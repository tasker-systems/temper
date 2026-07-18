# Security Audit Playbook — replaying the surfaces-in AuthN/AuthZ/credential audit

A repeatable procedure (for a human or an agent) to re-verify Temper's trust boundary. It
encodes the method used in the 2026-07-18 audit
([docs/code-reviews/2026-07-18-authn-authz-credential-flow-audit.md](../code-reviews/2026-07-18-authn-authz-credential-flow-audit.md))
**and the blind spot that audit initially fell into**, so the next run cannot repeat it.

> **The one lesson that matters most.** A grant audit has **two independent axes**, and the
> first pass checked only one:
> 1. **Authority acquisition** — can a caller *without* grant authority *acquire* it? (Trace
>    `can_administer_grant` / the `'grant'` SQL arm. Visibility must never become grant
>    authority.)
> 2. **Capability attenuation** — can a caller *with* grant authority confer **more capability
>    than it holds**, including to *itself*? (Trace whether every grant write enforces
>    `conferred ⊆ held`.)
>
> The first pass verified axis 1 and declared grants clean. Axis 2 was the actual hole (F-0: a
> `read+grant` principal self-escalating to `write+delete+grant`). **Always trace both axes.
> "Who may grant" is not "how much may they grant."**

## 0. Scope and posture

- **Surfaces-in.** Start at every point of entry, trace inward with file:line evidence. Do not
  trust comments, spec claims, or `CLAUDE.md` — verify against shipped code and SQL.
- **Both Rust surfaces share one seam.** `temper-api` and `temper-mcp` both route through
  `temper-services`. A check that lives in one surface's handler but not the shared service is
  a hole for the other surface. **Every authZ predicate must live in the shared service (or its
  SQL), reachable identically from both surfaces.**
- **The router gate is vacuous in prod.** `has_system_access` returns `true` under
  `access_mode='open'` (production's setting), so `require_system_access` admits everyone. The
  **per-endpoint service checks are the only real authorization.** Never conclude "the gated
  router protects it."
- **Fail-closed is the bar.** Every secret gate must reject when its secret is unset/empty.

## 1. AuthN — front-door pass

For every entry point, produce a row: `route | mechanism | enforcement site (file:line) |
verdict`. Entry points to enumerate:

- **temper-api** — `crates/temper-api/src/routes.rs`. Each sub-router and its `.layer(...)`:
  `public` (health only), `auth_only` (`require_auth`), `gated` (`require_auth` +
  `require_system_access`), `internal`/`slack_link_internal` (HMAC), `slack_link_public`
  (PKCE+state), `embed_internal` (self-gated secret). Confirm `require_auth` is the **outermost**
  layer on gated routes and that it **survives `split_for_parts()`** (empirically proven by
  e2e `no_auth_returns_401` hitting a gated route — keep that test).
- **temper-mcp** — `crates/temper-mcp/src/router.rs` (which routes carry `require_mcp_auth`) and
  `service.rs` (every `#[tool]` must call the `ensure_profile_from_parts` chokepoint →
  `authenticate_token` + `require_system_access` as its first line). **Count `#[tool(` vs
  chokepoint calls — they must be 1:1.** Discovery / `/oauth/register` / health are by-design
  unauth and must touch no user data.
- **TypeScript** — `api/oauth/*`, `api/auth/*`, `packages/temper-cloud/src`. OAuth AS + SAML SP
  only. Confirm PKCE, SAML signature validation, allowlist fail-closed, constant-time secret
  compares.
- **The shared seam** — `temper-services/src/auth`. `classify` is a closed 3-variant sum (no
  fall-open default arm); machines are lookup-or-401; deactivated profiles gated post-resolve.

Automatable slice: **`.github/scripts/audit-route-auth.sh`** (planned) — list every `.route(`
/ `routes!(` and the layer stack it sits under, flag any route with no auth layer and no
self-gate.

## 2. AuthZ — admin, grants, RBAC

### 2a. Admin & credential-minting
Every admin / machine / connection endpoint must gate on `is_system_admin` OR the scoped
owner predicate **before** any write. Confirm `is_system_admin` is fail-closed when
`gating_team_slug IS NULL` (no first-admin self-serve HTTP path). Confirm machine-reach
containment is **type-enforced** (`AuthorizedReach` has no public constructor) — and remember
axis 2: verify it also attenuates the *capability bits* it confers, not just the team/grant
authority.

### 2b. Grants / share / role — **run BOTH axes** (§ lesson above)
For every access-mutating op (`grant_capability`, `resource_grant/revoke`, `cogmap_grant`,
`cogmap_bind`, connection `grant_reach`, context `share/transfer`, team
`add_member/change_role`, machine `apply_reach`, cogmap genesis):
- Axis 1: what authorizes the grantor? (owner / `can_grant` / admin — never visibility.)
- Axis 2: is `conferred ⊆ held` enforced? Can `principal_id = self`? Can a `read+grant` holder
  confer `write/delete/grant`? **This is where F-0 lived.**
- Delegated vs self: is a **delegated** administrator attenuated while a **system admin** stays
  unrestricted (bootstrap/repair)? Is **revoke** deliberately NOT attenuated (de-escalation must
  never be harder than escalation)?

Automatable slice: **`.github/scripts/audit-grant-sinks.sh`** (shipped) — enumerates every
production write-site to `kb_access_grants` against a reviewed baseline, so a **new grant sink
cannot be added without a reviewer acknowledging the attenuation question** (this is the "fifth
`insert_grant` caller the plan missed" trap, made mechanical).

### 2c. RBAC on data mutation
Every resource/edge/facet/ingest write must pass `check_can_modify_next` → `can_modify_resource`
**before** writing, keyed on the **JWT-derived** `profile_id` (no mutating tool accepts a
`profile_id`/`on_behalf` field). Async writers (steward, dispatch) must run with the principal's
own access, not ambient authority. Watch two by-design asymmetries: edge asserts authorize
**source only** (F-1), and create-into-context uses the **read** gate not write (F-2).

## 3. Credential-flow pen-test

Threat-model each flow: token replay, audience/issuer confusion across surfaces under JWKS
rotation, algorithm confusion, signature forgery, fail-open-on-unset-secret, state-nonce reuse
(must be an atomic `UPDATE … WHERE consumed_at IS NULL … RETURNING`), machine-shape confusion,
JIT-provision side effects, and grant-vault crypto (AEAD AAD must bind principal‖field; fresh
nonce per seal; fail-closed decrypt; redacted `Debug` on key/token types). **Delegated-authz
tripwire:** any function that mints an act-as-a-human credential from a bare principal id (e.g.
`slack_grant_vault_service::mint_access_token`) must have its caller derive that principal from
an HMAC-verified server-to-server event, never a client-supplied field.

## 4. How to fan this out to subagents

The 2026-07-18 run used parallel adversarial subagents, one per domain, each returning a
`op | predicate (file:line) | before-write? | shared-across-surfaces? | verdict` table:
1. admin / machine / connection (with the B2 reach-containment invariants);
2. grants / share / role — **must be told to trace capability attenuation (axis 2), not just
   authority acquisition** (the fix for this audit's miss);
3. resource / edge RBAC;
4. credential flows.
Then reconcile against the specs and the shipped SQL yourself, and cross-check any "clean"
verdict on the grant surface against `audit-grant-sinks.sh` and the two-axis rule.

## 5. Static checks (this directory's companions)

| script | status | what it pins |
|---|---|---|
| `.github/scripts/audit-grant-sinks.sh` | shipped | every `kb_access_grants` write-site vs a reviewed baseline — a new sink fails CI until acknowledged (attenuation review) |
| `.github/scripts/audit-route-auth.sh` | planned | every axum route vs its auth layer / self-gate — flags an ungated route |
| `.github/scripts/audit-handler-authz-drift.sh` | planned | `is_system_admin`/authz predicates invoked from `handlers/` rather than a shared service (the F-3 `promote_admin` drift shape) |

These are deliberately *enumerators and tripwires*, not provers: they make the reviewable set
explicit and fail when it grows silently. The judgment stays human/agent; the script guarantees
the judgment is asked.
