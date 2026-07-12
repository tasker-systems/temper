# The issuer / resource-server boundary & the machine-token contract

This document is the **canonical home** for the single machine-token claim contract that
Temper's two token issuers conform to and the Rust seam normalizes. It is the unifying
artifact for M2M (machine-to-machine) agent principals — the auth-seam plan's Stage 4.

> **Status: fully shipped — both issuers mint machine tokens.** Stage 4a (the Auth0
> `client_credentials` advertisement) and 4b (the Rust classifier,
> `temper-services::auth::classify`) landed 2026-07-02; **4c — the Temper AS minting machine
> tokens itself — shipped in Phase B1** (`packages/temper-cloud/src/oauth/endpoints.ts`
> `handleToken`'s `client_credentials` branch, minting via `mint.ts::mintMachineAccessToken`).
> The claim shape below was validated against a real Auth0 M2M token (see
> [the flow](#end-to-end-flow-a-machine-token-becomes-an-agent-profile)) and is produced
> byte-for-byte by the Temper AS.
>
> Operator-facing companion (mint / reach / rotate / revoke):
> [../guides/machine-credentials.md](../guides/machine-credentials.md). Implementation
> designs: Stage 4
> ([spec](../superpowers/specs/2026-07-02-auth-seam-stage-4-m2m-implementation-design.md)),
> registration Phase A
> ([spec](../superpowers/specs/2026-07-10-machine-principal-registration-design.md)), the
> issuer grant Phase B1
> ([spec](../superpowers/specs/2026-07-10-machine-principal-phase-b1-issuer-grant-design.md)).

## The boundary: who mints vs. who validates

Temper's token boundary is **issuer-mints / resource-server-validates**, and it stays split:

- **Issuers (TypeScript or Auth0).** The OAuth Authorization Server is entirely TypeScript
  (`packages/temper-cloud`) or Auth0. Grant advertisement is single-sourced in TS
  (`src/oauth/metadata.ts`), which branches on `AS_ISSUER`:
  - **Auth0-fronted instance** (temperkb.io): `token_endpoint` points at Auth0; Auth0 mints.
    `grant_types_supported` — including `client_credentials` — is advertised by
    `buildAuth0AsMetadata`.
  - **Temper AS instance** (self-hosted SAML): `token_endpoint` → temper-cloud's own
    `handleToken`, which mints EdDSA tokens via `mintAccessToken` (human) and
    `mintMachineAccessToken` (machine). `buildAsMetadata` advertises `client_credentials`
    alongside `authorization_code` / `refresh_token`, plus the secret-bearing
    `token_endpoint_auth_methods_supported` the machine grant needs. Advertising it is not
    cosmetic — a conformant client reads that document to decide whether M2M is possible at
    all.
- **Resource server (Rust).** `temper-api` and `temper-mcp` are pure resource servers:
  they **validate and normalize** tokens. Rust **never advertises or mints**.

So there is **no cross-language advertisement split to unify** — advertisement already lives
only in TS. The only thing worth pinning across the boundary is the **token claim shape**.
That is this contract.

## The token *request* shape (client → issuer)

The claim shape below is the response half of the contract. The **request** half is pinned
too, in `tests/contracts/m2m-token-request.json` — a language-neutral file every client
(temper-rb today; temper-py / temper-ts next) and the AS's own integration suite assert
against. A contract asserted only against itself is not asserted at all: the Ruby gem minted
with a JSON body and proved it with a stub that parsed JSON, while temper's AS read the body
with `req.formData()` and proved *that* with a form-encoded request. Both suites were green
and no client could mint against temper's issuer.

| | Value |
|---|---|
| `Content-Type` | **`application/x-www-form-urlencoded`** (RFC 6749 §4 mandates it) |
| Required params | `grant_type=client_credentials`, `client_id`, `client_secret` |
| Client auth | HTTP Basic (RFC 6749 §2.3.1, preferred) **or** the two params in the form body — `readClientCredentials` accepts either |
| `audience` | **Auth0 requires it. Temper's AS ignores it entirely** — it mints with the server-side `AS_AUDIENCE`. A temper-issued client must be able to omit it. |
| JSON body | **`invalid_request`.** Auth0 tolerates JSON as an extension; temper's AS does not, and must *refuse* rather than 500. |

That last row is a real defense: `req.formData()` **throws** on a JSON body, so without the
guard in `handleToken` the caller gets a 500 — which reads as "the server is broken" rather
than "you encoded the request wrong". Adding a client language means pinning it against the
contract file too.

## The single machine-token claim shape

A machine token (an agent acting *as itself*, no human) carries a distinct claim shape that
**both** issuers must produce identically, and the Rust normalizer parses as exactly one
machine shape regardless of issuer:

| Claim | Value | Note |
|-------|-------|------|
| `azp` / `client_id` | `<clientid>` | the stable agent identity; key the agent profile on this |
| `sub` | `<clientid>@clients` | Auth0's M2M convention |
| `gty` | `client-credentials` | grant-type marker |
| `email` | *(absent)* | a machine has no verified human email |
| `aud` | the target API/MCP audience | set by the **issuer** (Auth0: from the request's `audience`; Temper AS: always the server-side `AS_AUDIENCE`), checked by the validating surface |

The classifier (`classify`, in `temper-services::auth`) **detects** this shape and, for a
machine, stamps a typed discriminant onto `AuthClaims` — `principal_kind:
PrincipalKind::Machine` plus the provider tag `auth0-m2m` as the link namespace.
`resolve_from_claims` then **branches on `principal_kind`** (a typed match, not a
provider-string compare):

- **`Human`** → the existing email-reconcile path
  ([jwt-verification.md](./jwt-verification.md) ladder).
- **`Machine`** → a `(auth0-m2m, client_id)` link lookup that is **lookup-or-reject**. Since
  G3 Phase A a machine principal must be **registered ahead of its first call**
  (`kb_machine_clients`; `temper admin machine provision` for an IdP-held secret, `temper
  admin machine issue` for a temper-minted one) — there is no just-in-time create branch, and
  an unregistered or revoked `client_id` is a 401. The agent profile is created by the
  *registration*, not by the token. It **never** enters `reconcile_by_email` — there is no
  verified email.

> **`auth0-m2m` is the link namespace, not the issuer.** Both issuers' machine tokens
> normalize to the provider tag `auth0-m2m`, and the registration lookup
> (`machine_client_service::lookup_by_client_id`) keys on the `client_id` **alone** — it is
> issuer-agnostic on purpose, because the token shape is. Which issuer holds the secret is
> recorded separately, in `kb_machine_clients.issuer` (`auth0-m2m` vs `temper`), and is
> consulted only by the *minting* side (`verifyMachineSecret` matches temper-issued rows
> only; an Auth0 row has a NULL `secret_hash` and verifies via JWKS). The tag's name is
> historical — it predates temper being an issuer.

> **Decisions locked in Stage 4 (validated against a real token):**
> - Detection keys on **`gty == "client-credentials"`**, *not* `azp` presence — a human Auth0
>   access token also carries `azp`.
> - client_id source is **`azp` directly**, with the `@clients`-suffix strip off `sub` only as
>   a fallback.
> - provider tag is **`auth0-m2m`** (the link namespace); the human/machine branch itself is a
>   typed **`PrincipalKind`** enum, so it is not a stringly-typed match.
>
> Confirmed against a live token minted from the `Temper Steward M2M` app on
> `temperkb.us.auth0.com` (2026-07-02) and pinned as a KAT in
> `normalize.rs::real_auth0_m2m_token_shape_is_detected`.

> **Hardened (2026-07-11): classification is total — there is no default arm.**
> `classify` returns a **closed sum**, `Principal::{Machine, Human, Refuse}`, rather than the
> `Option<AuthClaims>` it began as. The `Option` shape meant every surface wrote
> `if let Some(machine) = … else { …human… }` — so an **unrecognized token silently became a
> human**, and the human path auto-provisions. Two tokens fell through it: one whose `sub` is
> `@clients`-suffixed but which lacks the `gty` marker, and one that *declares*
> `gty=client-credentials` but carries no derivable client_id. Both are now `Refuse`.
>
> Because the arms are a closed enum, the routing decision is total — no caller, including a
> surface not yet written, can spell "unrecognized ⇒ human". The invariant is held by the type,
> not by a convention. Since #388, `classify` and `Principal` are additionally **crate-private**:
> the *seam* is the only thing that matches on them, and a `Refuse` reaches a surface as
> `AuthzError::Refused`, already logged. A surface that could see `Principal` could pattern-match
> its way back to hand-building the human arm — which is the drift, one level down.
>
> **Two layers, deliberately.** The routing invariant in `classify` is the first;
> `resolve_human_from_claims` is the second, and it is **independent** — it refuses a
> machine-shaped identity (an `@clients` `external_user_id`, *or* the `auth0-m2m` provider tag —
> either signal is disqualifying on its own) at the *write site*, so even a caller that bypassed
> classification entirely cannot walk a machine past the registration gate by dressing it as a
> human. The machine path has a gate; the human path auto-provisions — a mislabeled machine is
> exactly the identity you would want to mislabel, so the human path checks for itself.

## Agent principals ride the ordinary rails

Once provisioned, an agent profile is an ordinary accountable principal — no auth-path
special-casing:

- It passes `is_active` and `system_access` on the **same rails** as a human (see the
  [two-level chain](./authorization-seam.md)).
- It takes **ordinary grants**: team membership for source read, `cogmap grant --write` for
  authoring. Registration (`--team` / `--cogmap`) is just a convenient way to confer those same
  ordinary grants at mint time, bounded by what the minter could confer on a human. It is its
  own accountable principal (fits the invocation-envelope model), never a proxied human.
- There is therefore **no machine-specific authorization path** — machine RBAC falls out of the
  ordinary team-and-grant predicates. The credential *is* the boundary.

## What centralized in the seam (and what did not)

Stage 4b moved **claim-shape detection** into the seam — the one thing that would drift into
two divergent copies. PR #388 then moved **principal construction** in behind it: the human
email ladder and the `AuthClaims` constructor both live in the seam now, so a surface hands in
a verified token and gets back an `AuthenticatedProfile` (see
[authorization-seam.md](./authorization-seam.md)).

What is *still* per-surface is exactly one thing: the JWKS `decode()`. The `JwksKeyStore` was
already shared, but the audience legitimately differs per surface, so each surface verifies
the signature itself and decodes into the shared `temper_services::auth::RawJwtClaims`. From
there it calls `authenticate_token(&state, &raw, &token)`, and the seam does everything:
`classify(&raw)` decides machine vs. human vs. refuse; the human arm runs the one ladder
(`auth/email.rs`); the machine arm skips it entirely.

**The ladder asymmetry is what made the closed sum necessary — and it is now gone.** Before
#388, temper-api's ladder ended in an Auth0 `/userinfo` call that 401s for a machine token, so
a misrouted machine failed closed **there** — but only by accident of that call. temper-mcp had
*no* ladder (it set `email: ""` and tolerated it), so the same misrouted token would have been
**auto-provisioned as a human**. One seam, two surfaces, and only one of them coincidentally
safe. Security that holds because of a downstream side effect is not security — so the closed
sum removed the routing drift (#384) and the shared ladder removed the construction drift
(#388). Both surfaces now run the same ladder and refuse the same unnamable human.

## End-to-end flow: a machine token becomes an agent profile

```text
1. Agent (e.g. the steward) → the issuer's /oauth/token, FORM-ENCODED:
      grant_type=client_credentials, client_id, client_secret [, audience]
      · Auth0:      mints; `audience` required.
      · Temper AS:  handleToken → verifyMachineSecret (sha256, constant-time, non-revoked,
                    issuer='temper') → mintMachineAccessToken. `audience` IGNORED —
                    minted with the server-side AS_AUDIENCE.
   Either way the access token carries: gty=client-credentials, azp=<client_id>,
      sub=<client_id>@clients, no email. One shape, two issuers.

2. Agent → temper-mcp with `Authorization: Bearer <token>`
      require_mcp_auth verifies the JWT (JwksKeyStore, aud=mcp_audience)
      → injects RawJwtClaims + BearerToken into request extensions.

3. ensure_profile_from_parts → temper_services::auth::authenticate_token(&state, &raw, tok):
      classify(&raw) sees gty=client-credentials
      → Principal::Machine(AuthClaims { principal_kind: Machine, provider: "auth0-m2m",
                                        external_user_id: azp, email: "" }).
      (A machine-SHAPED token without the gty marker → Principal::Refuse → 401.
       There is no "unrecognized ⇒ human" arm to fall through. The surface does not
       construct these claims — the seam does; that is the only constructor.)
      The machine arm does NOT run the email ladder.

4. authenticate (pub(crate)) → resolve_from_claims branches on principal_kind == Machine:
      lookup <client_id> in kb_machine_clients — LOOKUP-OR-REJECT.
      Unregistered or revoked ⇒ 401. The agent profile + link + emitters were created
      by `temper admin machine provision` / `issue`, ahead of this call.
      Never reconcile_by_email. Then the `is_active` gate.

5. require_system_access gates the agent on the ordinary rails: open mode admits;
      a gated instance (temperkb.io) denies until the agent profile holds gating-team
      membership. Registration enrolls it (as `watcher`) when the minter is themselves a
      gating-team member; authoring into a cogmap still needs an explicit write grant.
```

## Operator runbook: standing up an M2M agent

Two shapes, one Temper-side step. The operator-facing guide is
[../guides/machine-credentials.md](../guides/machine-credentials.md); this is the
contributor's view of what each command makes true.

**Either way, `temper admin machine …` is not optional.** Since G3 Phase A there is **no
auto-provisioning**: the agent profile is created by the *registration*, and an
unregistered `client_id` is a 401 no matter how valid its token is.

### A. Temper is the issuer (no external IdP)

```bash
# Temper mints the client_id (tmpr_…) AND the secret. The secret prints ONCE.
temper admin machine issue --label "Temper Steward" \
  --team <team-ref>:member \
  --cogmap <cogmap-ref>
```

### B. Auth0 holds the secret

One-time setup per agent principal, via the Auth0 CLI (`auth0 login` as a tenant user). The
`temperkb.us.auth0.com` steward app was provisioned this way on 2026-07-02:

```bash
# 1. Create the M2M application at the IdP (records client_id + client_secret).
auth0 apps create --name "Temper Steward M2M" --type m2m --reveal-secrets --no-input

# 2. Authorize it for the mcp audience (== the temper-api identifier; there is no separate
#    MCP_AUDIENCE API registered). Needs `create:client_grants` scope on your login.
auth0 api post client-grants \
  --data '{"client_id":"<CLIENT_ID>","audience":"https://temperkb.io/api","scope":[]}'

# 3. Register it with Temper — this creates the agent profile, its emitters, its gating-team
#    enrollment, and the reach you name. NOTHING is auto-provisioned by the token.
temper admin machine provision --client-id <CLIENT_ID> --label "Temper Steward" \
  --team <team-ref>:member \
  --cogmap <cogmap-ref>

# 4. (Verify) mint a token and confirm the claim shape. FORM-ENCODED — Auth0 tolerates JSON,
#    Temper's own AS does not, so form-encode everywhere and the same client works on both.
curl -s --request POST --url https://temperkb.us.auth0.com/oauth/token \
  --header 'content-type: application/x-www-form-urlencoded' \
  --data grant_type=client_credentials \
  --data client_id=<CLIENT_ID> \
  --data client_secret=<SECRET> \
  --data audience=https://temperkb.io/api
```

The `client_secret` is set as the deployed agent's env var (the steward's Vercel project).
**Rotating the IdP secret needs no Temper action** — the `client_id` is unchanged, so
authorship history stays continuous (`auth0 apps rotate-secret <client_id>`). Rotating the
IdP *application* — a new `client_id` — needs `temper admin machine rebind`, which binds the
new id to the existing agent profile. A temper-issued secret rotates with
`temper admin machine rotate-secret`.

Reach (`--team`, `--cogmap`) is **plural and explicit**, and is what clears the
`system_access` gate and the cogmap write gate on a gated instance (temperkb.io). It is never
inferred from `--owner-team`, which records the machine's *owner* and is never consulted for
authorization.

## Delivery split — all shipped

- **4a — Auth0 branch. ✅** `client_credentials` in `grant_types_supported`
  (`buildAuth0AsMetadata`, TS); the `Temper Steward M2M` app is provisioned. Auth0 mints.
- **4b — Rust resource-server side. ✅** The shared classifier +
  `principal_kind`-branched machine resolution described above.
- **4c — Temper AS branch. ✅ (Phase B1).** `handleToken` implements the
  `client_credentials` grant: `readClientCredentials` (Basic or form) → `verifyMachineSecret`
  → `touchMachineLastSeen` → `mintMachineAccessToken`, returning an access-token-only body
  (no refresh token — RFC 6749 §4.4.3). `buildAsMetadata` advertises the grant and the
  secret-bearing auth methods. The minted claims mirror an Auth0 M2M token exactly, so
  `classify` handles AS-issued and Auth0-issued machine tokens **identically** — no
  issuer-conditional branch anywhere in Rust.
- **Registration gate (G3 Phase A) ✅, team-owner registration + reach containment
  (Phase B2) ✅.** Lookup-or-reject on `kb_machine_clients`; a team owner may mint a machine
  for their own team, bounded to reach they could confer on a human.

**Not the destination:** `authorization_code + refresh_token` as a dedicated agent login
still works with temper-mcp (one-time browser consent) and was the pre-Stage-4 bridge. It is
an escape hatch. **Avoid** the `user`-subject-as-a-human path — it proxies as that human and
conflates authorship.

Spec:
[../superpowers/specs/2026-07-02-shared-auth-orchestration-seam-design.md](../superpowers/specs/2026-07-02-shared-auth-orchestration-seam-design.md)
(Stage 4). Creating the *Auth0 application* is an operator/console step outside the repo;
registering it with Temper (`temper admin machine provision`) is not — without it the token
is a 401.
