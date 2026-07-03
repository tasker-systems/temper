# The issuer / resource-server boundary & the machine-token contract

This document is the **canonical home** for the single machine-token claim contract that
Temper's two token issuers conform to and the Rust seam normalizes. It is the unifying
artifact for M2M (machine-to-machine) agent principals — the auth-seam plan's Stage 4.

> **Status:** **shipped (Stage 4a + 4b, 2026-07-02).** The Rust normalizer
> (`temper-services::auth::normalize_machine`) and the Auth0 `client_credentials`
> advertisement are live; the contract below was validated against a real Auth0 M2M token
> (see [the flow](#end-to-end-flow-a-machine-token-becomes-an-agent-profile)). **4c** — the
> self-hosted Temper AS minting machine tokens — remains deferred until an instance wants it.
> Implementation design:
> [../superpowers/specs/2026-07-02-auth-seam-stage-4-m2m-implementation-design.md](../superpowers/specs/2026-07-02-auth-seam-stage-4-m2m-implementation-design.md).

## The boundary: who mints vs. who validates

Temper's token boundary is **issuer-mints / resource-server-validates**, and it stays split:

- **Issuers (TypeScript or Auth0).** The OAuth Authorization Server is entirely TypeScript
  (`packages/temper-cloud`) or Auth0. Grant advertisement is single-sourced in TS
  (`src/oauth/metadata.ts`), which branches on `AS_ISSUER`:
  - **Auth0-fronted instance** (temperkb.io): `token_endpoint` points at Auth0; Auth0 mints.
    `grant_types_supported` is advertised by `buildAuth0AsMetadata`.
  - **Temper AS instance** (self-hosted SAML): `token_endpoint` → temper-cloud's own
    `handleToken`, which mints EdDSA tokens via `mintAccessToken`.
- **Resource server (Rust).** `temper-api` and `temper-mcp` are pure resource servers:
  they **validate and normalize** tokens. Rust **never advertises or mints**.

So there is **no cross-language advertisement split to unify** — advertisement already lives
only in TS. The only thing worth pinning across the boundary is the **token claim shape**.
That is this contract.

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
| `aud` | the target API/MCP audience | passed in by the validating surface |

The normalizer (`normalize_machine`, in `temper-services::auth`) **detects** this shape and,
for a machine, stamps a typed discriminant onto `AuthClaims` — `principal_kind:
PrincipalKind::Machine` plus the provider tag `auth0-m2m` as the link namespace.
`resolve_from_claims` then **branches on `principal_kind`** (a typed match, not a
provider-string compare):

- **`Human`** → the existing email-reconcile path
  ([jwt-verification.md](./jwt-verification.md) ladder).
- **`Machine`** → a `(auth0-m2m, client_id)` link lookup; on first sight, provision a
  **dedicated agent profile** (a `kb_profile_auth_links` row under the `auth0-m2m` provider,
  NULL email). It **never** enters `reconcile_by_email` — there is no verified email.

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

## Agent principals ride the ordinary rails

Once provisioned, an agent profile is an ordinary accountable principal — no auth-path
special-casing:

- It passes `is_active` and `system_access` on the **same rails** as a human (see the
  [two-level chain](./authorization-seam.md)).
- It takes **ordinary grants**: team membership for source read, `cogmap grant --write` for
  authoring. It is its own accountable principal (fits the invocation-envelope model), never
  a proxied human.

## What centralized in the seam (and what did not)

Stage 4b moved **claim-shape detection** into the seam — the one thing that would drift into
two divergent copies. Each surface still owns its own JWKS `decode()` (the `JwksKeyStore` was
already shared; the audience legitimately differs per surface), decoding into the shared
`temper_services::auth::RawJwtClaims`. It then calls `normalize_machine(&raw)` — the single
place that decides machine vs. human and, for a machine, stamps the normalized `AuthClaims`.

This is a **thin normalizer**, chosen deliberately over moving full JWT verification into the
seam: the human email-resolution ladder (token → cached link → OIDC `/userinfo`) is genuinely
per-surface (temper-api has it, temper-mcp does not), so dragging it behind a shared
`verify_and_normalize` would trade a real drift risk (claim-shape) for speculative
abstraction. "Detection lives once" is the load-bearing win, and the thin normalizer delivers
it in full.

## End-to-end flow: a machine token becomes an agent profile

```text
1. Agent (e.g. the steward) → Auth0 /oauth/token
      grant_type=client_credentials, client_id, client_secret, audience=<mcp audience>
   Auth0 mints an access token: gty=client-credentials, azp=<client_id>,
      sub=<client_id>@clients, no email.

2. Agent → temper-mcp with `Authorization: Bearer <token>`
      require_mcp_auth verifies the JWT (JwksKeyStore, aud=mcp_audience)
      → injects RawJwtClaims into request extensions.

3. ensure_profile_from_parts → claims_from(&raw):
      normalize_machine(&raw) sees gty=client-credentials
      → AuthClaims { principal_kind: Machine, provider: "auth0-m2m",
                     external_user_id: azp, email: "" }.

4. authenticate → resolve_from_claims branches on principal_kind == Machine:
      lookup (auth0-m2m, <client_id>); on first sight, create a dedicated agent
      profile + link (NULL email) + emitters + default context. Never reconcile_by_email.

5. require_system_access gates the agent on the ordinary rails: open mode admits;
      a gated instance (temperkb.io) denies until the agent profile is granted team
      membership + `cogmap grant --write`.
```

## Operator runbook: provisioning an Auth0 M2M agent

One-time setup per agent principal, via the Auth0 CLI (`auth0 login` as a tenant user). The
`temperkb.us.auth0.com` steward app was provisioned this way on 2026-07-02:

```bash
# 1. Create the M2M application (records client_id + client_secret).
auth0 apps create --name "Temper Steward M2M" --type m2m --reveal-secrets --no-input

# 2. Authorize it for the mcp audience (== the temper-api identifier; there is no separate
#    MCP_AUDIENCE API registered). Needs `create:client_grants` scope on your login.
auth0 api post client-grants \
  --data '{"client_id":"<CLIENT_ID>","audience":"https://temperkb.io/api","scope":[]}'

# 3. (Verify) mint a token and confirm the claim shape.
curl -s --request POST --url https://temperkb.us.auth0.com/oauth/token \
  --header 'content-type: application/json' \
  --data '{"client_id":"<CLIENT_ID>","client_secret":"<SECRET>","audience":"https://temperkb.io/api","grant_type":"client_credentials"}'
```

The `client_secret` is set as the deployed agent's env var (the steward's Vercel project);
rotate with `auth0 apps rotate-secret <client_id>`. **Still pending after provisioning:** grant
the agent's auto-provisioned profile team membership + `cogmap grant --write` so it clears the
`system_access` gate on temperkb.io.

## Stage-4 delivery split

- **4a — Auth0 branch. ✅ shipped.** `client_credentials` added to `grant_types_supported` in
  `buildAuth0AsMetadata` (TS); the `Temper Steward M2M` app is provisioned. Auth0 mints; no
  token-minting code in the repo.
- **4b — Rust resource-server side. ✅ shipped.** The shared `normalize_machine` +
  `principal_kind`-branched agent-profile provisioning described above.
- **4c — Temper AS branch. ⏸ deferred.** `handleToken` would implement the `client_credentials`
  grant and mint via a **machine variant of `MintedClaims`** matching this contract — ideally
  a ts-rs-shared machine-claim type in temper-core, so `normalize_machine` handles AS-issued
  and Auth0-issued machine tokens identically. Needed only for a self-hosted instance that
  wants agents; the Temper AS still returns `unsupported_grant_type` for `client_credentials`.

**Bridge, if an agent must go live before Stage 4:** `authorization_code + refresh_token` as
a dedicated login works with temper-mcp as-is (one-time browser consent). It is the escape
hatch, not the destination. **Avoid** the `user`-subject-as-a-human path — it proxies as
that human and conflates authorship.

Spec:
[../superpowers/specs/2026-07-02-shared-auth-orchestration-seam-design.md](../superpowers/specs/2026-07-02-shared-auth-orchestration-seam-design.md)
(Stage 4). Auth0 M2M app provisioning is an operator/console runbook step outside the repo.
