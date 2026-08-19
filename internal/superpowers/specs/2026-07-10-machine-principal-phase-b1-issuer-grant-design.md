# Machine-principal Phase B1 — temper issues `client_credentials` credentials

**Date:** 2026-07-10
**Goal:** `019f4910` — temper-rb, a native Ruby client for the temper API
**Task:** `019f4c36` — Phase B (this spec covers **B1 only**)
**Depends on:** Phase A (`kb_machine_clients` registration gate, PR #351, merged to `main`)
**Status:** ✅ **SHIPPED** as PR #374. B2 is no longer deferred either — it shipped as PR #377.

> **One thing this spec could not foresee.** B1 proved a temper-*issued token* authenticates through
> the gate, but never that a *client library could mint one*: every M2M client sent a JSON body,
> which Auth0 tolerates and this AS — which reads the request with `req.formData()` — does not. The
> token endpoint requires `application/x-www-form-urlencoded` (RFC 6749 §4). Closed on the branch
> that added `tests/contracts/m2m-token-request.json`, the cross-language wire contract both the
> clients and the AS now assert against.

---

## Problem

Phase A made temper a *governed verifier*: `kb_machine_clients` is a fail-closed allowlist,
`resolve_machine_from_claims` (`temper-services`) is lookup-or-401, and `temper admin machine
provision|rebind|list|show|revoke` registers, rotates the *application*, and revokes. But temper
still does not *issue* machine credentials — every machine principal must first obtain a token from
Auth0.

Three consequences, and they are the motivation:

1. **Auth0 bills and rate-limits per M2M application.** "One application per tenant agent" does not
   scale. Every temper-rb user who wants a Sidekiq worker principal must first create an Auth0 M2M
   app and grant it the temper audience.

2. **A self-hosted instance not fronted by Auth0 cannot mint a machine credential at all.** There is
   no IdP behind it. Phase A registers a principal but has nothing to register.

3. **Phase A's revocation is temper-side deny of a token that remains valid at the IdP** — correct,
   but not the same as never issuing it.

B1 closes all three by adding a `client_credentials` grant to the OAuth Authorization Server temper
**already runs**.

### What already exists (verified 2026-07-10)

- **temper runs its own AS.** `packages/temper-cloud/src/oauth/` mints EdDSA (Ed25519) access
  tokens (`mint.ts`, via `jose`), publishes a JWKS (`keys.ts`, `metadata.ts`), and rotates
  single-use refresh tokens with a revocation chain (`kb_oauth_refresh_tokens`). `handleToken`
  (`endpoints.ts`) supports `authorization_code` and `refresh_token` and returns
  `unsupported_grant_type` for everything else. The API already trusts the AS JWKS — that is how
  every temper-minted token validates today.
- **Opaque tokens are already SHA-256 hashed at rest.** `hashToken` (`mint.ts:60`) is
  `createHash("sha256")`; it stores refresh-token and auth-code hashes. There is **no argon2** in
  either the Rust or the TypeScript dependency tree.
- **`sha2::Sha256` is already used across the Rust workspace** (temper-ingest, temper-cli,
  temper-substrate). SHA-256 hex output is byte-identical Rust↔TS.
- **`normalize_machine` (`temper-services/src/auth/normalize.rs`) is issuer-agnostic on shape.** It
  detects a machine token by `gty == "client-credentials"`, resolves the client id from `azp`
  (primary) or by stripping the `@clients` suffix off `sub` (fallback), and emits the `auth0-m2m`
  provider tag. It has a known-answer test pinning the real Auth0 claim shape.
- **Phase A's `kb_machine_clients`** carries `client_id UNIQUE`, `issuer` (default `'auth0-m2m'`,
  the forward slot for `'temper'`), `profile_id`, `team_id` (owner, never reach), `label`,
  `registered_by_profile_id`, `last_seen_at`, and the revocation pair. The registration service
  (`machine_registration_service.rs`: `provision`, `rebind`), query service
  (`machine_client_service.rs`: `lookup_by_client_id`, `touch_last_seen`, `get`, `list`, `revoke`),
  handlers (`handlers/machine_clients.rs`), and CLI (`commands/admin_machine.rs`) are all in place.

The key structural fact: **a temper-issued machine token is just a JWT carrying `gty:"client-
credentials"` + `azp:<client_id>`, signed by the AS key and validated under the JWKS the API
already trusts.** So `normalize_machine` and Phase A's gate run **unmodified**. B1 adds an issuance
path and a verification grant; it changes no verifier.

---

## Design

**Rust issues, TypeScript verifies.** Issuance extends Phase A's CLI + registration service (where
`sha2` already lives and where the row's identity is already owned); verification adds a third grant
to the AS (where token minting and the `hashToken` precedent already live). SHA-256 hex crosses the
boundary unchanged, so neither side needs a new dependency.

### The secret is SHA-256, not argon2

temper **generates** the secret at full 256-bit entropy (32 random bytes), so a slow KDF buys
nothing: argon2 exists to resist brute-forcing *low-entropy human passwords*, and a 256-bit random
value is not brute-forceable regardless of hash. SHA-256 is cryptographically sufficient here, it
reuses `hashToken()` (the AS's existing at-rest hashing for refresh/code tokens — the "precedent"
the Phase A spec actually cited), it adds no dependency to either runtime, and it keeps the
`/oauth/token` verification path cheap rather than deliberately slow. Constant-time comparison
(`timingSafeEqual`) covers the only residual concern. The Phase A spec's "argon2id" was
over-specified against its own cited precedent; SHA-256 is the honest choice.

### 1. Schema — one additive migration

`20260711000070_machine_client_secrets.sql` (above the current head `...050`, leaving a `...060`
gap for a concurrent sibling session). Adds to `kb_machine_clients`, **all nullable** so `auth0-m2m`
rows are unaffected:

```sql
ALTER TABLE kb_machine_clients
  ADD COLUMN secret_hash                TEXT        NULL,
  ADD COLUMN secret_hash_previous       TEXT        NULL,
  ADD COLUMN secret_previous_expires_at TIMESTAMPTZ NULL,
  ADD COLUMN secret_rotated_at          TIMESTAMPTZ NULL;
```

- `secret_hash` — SHA-256 hex of the current secret. Present only on `issuer='temper'` rows;
  `auth0-m2m` rows keep it `NULL` and keep verifying against Auth0's JWKS. Two verification paths,
  keyed on `issuer`.
- `secret_hash_previous` + `secret_previous_expires_at` — the second live secret for zero-downtime
  rotation. Verification accepts the previous hash only while `now() < secret_previous_expires_at`.
- `secret_rotated_at` — audit stamp of the last rotation.

Column comments restate the `issuer`-keyed invariant and that no plaintext is ever stored. This
migration is purely additive; it applies cleanly on PG 17 (Neon) and PG 18 (local).

### 2. Issuance (Rust)

**temper mints both the `client_id` and the secret** — it *is* the Authorization Server, so it owns
the client identifier (unlike Phase A `provision`, which registers an *externally-issued* Auth0
`client_id`).

- `client_id` — a readable prefixed random, e.g. `tmpr_<base64url>`, `UNIQUE`.
- `client_secret` — 32 random bytes, base64url, returned **once** and never stored; only
  `sha256_hex(secret)` is persisted.

A new service function (`machine_registration_service::issue`) reuses Phase A's registration
internals to create, in one transaction: the agent profile, its `('auth0-m2m', client_id)` auth
link (the provider tag `normalize_machine` emits is unchanged — see "Why the auth link stays
`auth0-m2m`" below), its per-surface emitter entities, and the `kb_machine_clients` row with
`issuer='temper'` and `secret_hash` set. It returns a typed one-time-credential DTO
`{ client_id, client_secret }`. `rotate_secret` (in `machine_client_service.rs`) implements the
two-live-secret state machine (§4).

**CLI** (`commands/admin_machine.rs`, alongside `provision`/`rebind`):

| Subcommand | Behavior |
|---|---|
| `issue --label <l> [--owner-team <ref>] [--team <ref>[:role]]… [--cogmap <ref>[:rw]]…` | Mint `client_id` + secret, run the full registration, apply each `--team`/`--cogmap` (reach is plural and explicit, never inferred from `--owner-team` — D10 from Phase A), enroll in the gating team (D14). Print the plaintext secret **once** with a "store this now; it will not be shown again" notice. |
| `rotate-secret <ref> [--grace <duration>]` | Rotate to a fresh secret with a grace window on the old one (§4). Print the new plaintext once. |

`issue` reuses Phase A's `TeamSpec`/`GrantSpec` request shapes for `--team`/`--cogmap`.

**API** (`handlers/machine_clients.rs`, routes in `routes.rs`):

- `POST /api/machine-clients/issue`
- `POST /api/machine-clients/{id}/rotate-secret`

Both `is_system_admin`-gated **explicitly in the handler** — load-bearing, because prod runs
`access_mode='open'` under which the system-gated router admits every profile (Phase A D12). Both
added to `.github/scripts/check-openapi-routes.sh`'s operator-only allowlist. Handlers stay thin:
validate → one service call → serialize. No `sqlx::query!()` in a handler; SQL lives in the service.
The one-time-credential response DTO is a typed struct (never `serde_json::json!()`).

### 3. Verification (TypeScript) — the third grant

In `handleToken` (`endpoints.ts`), before the final `unsupported_grant_type`, add
`grant_type === "client_credentials"`:

1. Read `client_id` + `client_secret` from the form body (`client_secret_post`) — matching the
   existing grants' `formData()` parsing, RFC 6749 §2.3.1, and both reference callers (the steward's
   `temper-auth.ts` and temper-rb's `credentials.rb`). **Also accept HTTP Basic**
   (`client_secret_basic`) — RFC's recommended default, cheap, and temper-rb's generated config
   already has the helper.
2. Look up `kb_machine_clients` by `client_id WHERE issuer='temper' AND revoked_at IS NULL`
   (a new query in a small `oauth/machine-clients.ts`, or extending `flow.ts`).
3. Constant-time (`timingSafeEqual`) compare `hashToken(secret)` against `secret_hash`, **or**
   against `secret_hash_previous` when `now() < secret_previous_expires_at`. No match → `invalid_client`.
4. Mint an **access-token-only** response — no refresh token (RFC 6749 §4.4.3) — via a new
   `mintMachineAccessToken(clientId)` in `mint.ts`: claims `sub:"<client_id>@clients"`,
   `azp:"<client_id>"`, `gty:"client-credentials"`, **no email**; `iss`/`aud`/`exp` from the same
   `AS_ISSUER`/`AS_AUDIENCE`/`AS_ACCESS_TTL_SECONDS` env as human tokens.
5. Coarsely touch `last_seen_at` (five-minute rule, mirroring Phase A's gate).

The response body is `{ access_token, token_type: "Bearer", expires_in }` — `expires_in` from
`accessTtlSeconds()` so it agrees exactly with the minted `exp`.

### 4. Rotation semantics — two live secrets, capped at two

`rotate-secret`:

```
secret_hash_previous       ← secret_hash          (old current becomes previous)
secret_previous_expires_at ← now() + grace         (default 24h; --grace overrides)
secret_hash                ← sha256_hex(new secret)
secret_rotated_at          ← now()
```

Verification accepts `secret_hash` always and `secret_hash_previous` until its expiry. The operator
issues the new secret, deploys it to the caller, and the old one auto-expires — **no window in which
the caller holds no valid credential.** A second rotation inside the grace window overwrites
`secret_hash_previous`, retiring the oldest immediately: only two secrets are ever live. Secret
rotation, unlike *application* rotation, requires nothing at the IdP because temper is the issuer.

### 5. Why the auth link stays `auth0-m2m`

The `kb_machine_clients.issuer` column is `'temper'` for B1 rows, but the
`kb_profile_auth_links.auth_provider` stays `'auth0-m2m'` (== `MACHINE_PROVIDER_TAG`). That tag is
the *machine-principal namespace* `normalize_machine` emits for **all** machine tokens regardless of
who signed them; it is what `resolve_machine_from_claims` joins on. Splitting it would fork the gate
into two lookup paths for no benefit. `issuer` records who *issued the credential*;
`auth_provider` records *what kind of principal* it is. They are different questions, and only
`issuer` distinguishes B1 from Phase A.

### 6. Testing & coverage split

The mint path is TypeScript/Vercel — outside the Rust e2e harness (which spawns a real Axum server
+ Postgres). Coverage therefore splits by half, each suite testing its own side:

- **TS integration (vitest, `packages/temper-cloud`):** issue a row, then `client_credentials` mint
  succeeds and returns **no** refresh token; wrong secret → `invalid_client`; revoked row → rejected;
  a rotated secret's *previous* hash succeeds within grace and fails past it; the minted token
  carries `gty`/`azp`/`sub` in the exact shape `normalize_machine` expects.
- **Rust `test-db` + e2e (both surfaces, per `feedback_access_semantics_changes_need_e2e_tier`):**
  a machine-shaped JWT for an `issuer='temper'` row authenticates through the **unmodified**
  `resolve_machine_from_claims` gate on temper-api **and** temper-mcp; an `auth0-m2m` row keeps
  authenticating untouched (the bite test — asserts B1 changed no verifier).
- **Issuance service unit tests:** `sha256_hex` round-trips a generated secret; the rotation state
  machine (current/previous/expiry) transitions correctly; the plaintext is never persisted (only
  its hash appears in the row).

`normalize_machine`'s known-answer test is untouched and continues to guard the claim boundary.

---

## Decisions

- **D1 — SHA-256, not argon2.** temper generates the secret at 256-bit entropy, so a KDF's
  brute-force resistance is moot. SHA-256 reuses `hashToken()`, adds no dependency to either runtime,
  and keeps the token hot path cheap. Constant-time compare covers timing. The Phase A spec's
  "argon2id" was over-specified against its own cited precedent (`token_hash`, which is SHA-256).

- **D2 — Rust issues, TypeScript verifies.** Issuance extends Phase A's CLI/service (identity owner,
  `sha2` present); verification extends the AS (token minter, `hashToken` present). SHA-256 hex is
  identical across the boundary, so nothing is duplicated in a second language.

- **D3 — temper mints the `client_id`.** As the Authorization Server it owns the client identifier.
  Unlike Phase A `provision` (which registers an external Auth0 id), `issue` generates both id and
  secret. Operator-named ids were rejected (collision risk, no benefit).

- **D4 — The verifier does not change.** A temper-issued token is a JWT with the machine claim shape
  `normalize_machine` already detects, validated under the AS JWKS the API already trusts.
  `resolve_machine_from_claims`, `normalize_machine`, and the JWKS trust set are untouched.

- **D5 — `issuer='temper'` distinguishes the credential; `auth_provider` stays `auth0-m2m`.** The
  auth-link tag is the machine-principal namespace for all machine tokens; only `issuer` marks B1.

- **D6 — Two live secrets, capped at two, with a grace expiry.** Zero-downtime secret rotation with
  no IdP involvement. A second rotation inside the window retires the oldest immediately.

- **D7 — Access-token-only response.** `client_credentials` issues no refresh token (RFC 6749
  §4.4.3). A new `mintMachineAccessToken` path, separate from `issueTokenPair`.

- **D8 — `client_secret_post` primary, HTTP Basic also accepted.** Matches the existing grants, both
  reference callers, and the RFC default. The plaintext secret is returned once at issuance and
  never stored.

- **D9 — The `is_system_admin` check on `issue`/`rotate-secret` is load-bearing.** Prod
  `access_mode='open'` admits every profile through the gated router; the explicit handler check is
  the only gate. (Phase A D12, reasserted for the new routes.)

## Rejected

- **argon2id per the Phase A spec's letter.** See D1: no security gain over SHA-256 for a
  temper-generated full-entropy secret, at the cost of a new dependency in two runtimes and a
  deliberately-slow hash on the verification hot path.

- **Storing secrets encrypted (`pgcrypto`).** temper never needs the plaintext back; a hash strictly
  dominates. Carried over verbatim from the Phase A spec's D1.

- **Issuing a refresh token to a machine.** Forbidden by RFC 6749 §4.4.3; a machine re-mints from its
  client secret, which it holds durably.

- **Operator-named `client_id`.** Collision risk and no benefit; temper owns the namespace.

- **Splitting the gate into an `issuer`-keyed lookup path in `resolve_machine_from_claims`.** The
  gate is issuer-agnostic by construction (it looks up `client_id`); a second path would be dead
  weight.

## Deferred

- **B2 — widen registration authorization** from `is_system_admin` to `is_system_admin OR
  is_team_owner(team_id)`. One predicate + tests, no migration (`team_id` landed in Phase A). Its own
  task, gated on the enterprise Vercel-Eve security review closing — that is where the security
  surface actually changes.

- **Repointing the steward from Auth0 to temper's issuer.** The steward stays on its Auth0 M2M app
  until the self-hosted infra is genuinely up and ready; B1 only makes temper *capable* of issuing.

- **`EMBED_DISPATCH_SECRET` / `INTERNAL_RECONCILE_SECRET` scheme harmonization.** Orthogonal to
  machine principals; carried from the Phase A spec's Deferred list.

## Open questions and risks

- **Deploy ordering.** B1's schema (the four nullable columns) must be applied to prod before the
  code that reads them ships, per the additive-only-on-`main` invariant. Migrate-ahead is inert
  (columns exist, nothing reads them); deploy-ahead 500s the `client_credentials` grant on a missing
  column. Same posture as Phase A — migrate, then deploy. Phase A's own migration must also be
  applied to prod before B1 ships (its `issuer` column is what B1's rows set); confirm with `SELECT`
  before migrating.

- **JWKS trust for temper-issued machine tokens.** A temper-issued token is signed by the AS key
  (`iss = AS_ISSUER`), which the API already trusts for human tokens. No JWKS change is needed. The
  bite test asserts this end-to-end rather than assuming it.

- **`invite_only` interaction.** Under the flip temperkb.io intends, `trg_sync_system_membership`
  stops auto-enrolling; `issue` must enroll the agent in the gating team explicitly (it reuses Phase
  A `provision`'s D14 enrollment, so this is inherited, not re-solved).
