# Auth seam Stage 4 — M2M `client_credentials` for agent principals (implementation design)

**Scope of this doc:** the concrete, pinned implementation decisions for Stage 4 sub-parts
**4a + 4b**. It is the child of two parent artifacts and does not restate them:

- Parent spec (arc + rationale): [2026-07-02-shared-auth-orchestration-seam-design.md](./2026-07-02-shared-auth-orchestration-seam-design.md) (Stage 4).
- Canonical claim contract (the wire shape both issuers conform to): [../../auth/machine-token-contract.md](../../auth/machine-token-contract.md).

Those left three decisions open ("lock in Stage 4"). This doc locks them, chooses the seam
shape, and pins the test plan. **4c (Temper AS-mint `client_credentials`) is out of scope** —
deferred until a self-hosted instance wants agents.

## Why

The deployed T6 steward authenticates as a Vercel Connect `principalType: "app"` (a machine,
no human). The OAuth AS advertises only `authorization_code` + `refresh_token`, so an app
principal can never mint a token → the MCP connection never establishes → no profile is
created. Stage 4 lets an agent authenticate **as itself** via `client_credentials` and be
provisioned as its own accountable principal (never proxying a human).

## Decisions locked

1. **Seam shape — thin normalizer, not a full `verify_and_normalize`.** Each surface keeps its
   own `decode()` call (JWKS is already shared via `jwks_store`). The seam owns only the
   drift-prone claim-shape logic. Rationale: a full verify-in-the-seam would drag the
   human-only email-resolution machinery (api's userinfo/OIDC-discovery fetch, which mcp does
   not have) into the shared layer or behind a callback. The load-bearing win — *detection
   lives once* — is fully achieved by the thin normalizer; the rest is YAGNI.

2. **Machine detection signal — `gty == "client-credentials"`, NOT `azp` presence.** Auth0
   **human** access tokens also carry `azp` (the client). `gty` is the definitive grant-type
   marker. Misreading this would misclassify every human token, so it is the first correctness
   test.

3. **Client-id source — `azp` primary, `sub`-strip fallback.** Prefer reading `azp` directly
   (the stable agent identity) over stripping `@clients` off `sub`. Fall back to the `sub`
   strip only if `azp` is absent. (Contract-preferred; validate against a real token — see
   Follow-ups.)

4. **Provider tag — `auth0-m2m`.** This is the `kb_profile_auth_links.auth_provider`
   **link-namespace key** for machine links, distinct from the human `auth0` namespace. It
   fits `varchar(32)`. The `UNIQUE(auth_provider, auth_provider_user_id)` constraint means
   `(auth0-m2m, <client_id>)` can never collide with a human `(auth0, <sub>)`.

5. **Discriminator — a typed `PrincipalKind` enum on `AuthClaims`, not a stringly-typed
   provider match.** `resolve_from_claims` branches on `PrincipalKind`, honoring the project's
   no-stringly-typed-match / parse-don't-validate rule. The provider **string** still exists,
   but only as the link namespace, never as the branch signal. (This refines the contract's
   literal "branch on that tag" wording toward a typed discriminant.)

## Components & changes

### `temper-core` — `types/auth.rs`

- New `pub enum PrincipalKind { Human, Machine }` (`Debug, Clone, Copy, PartialEq, Eq`).
- `AuthClaims` gains `principal_kind: PrincipalKind`. `email` **stays `String`** (not
  `Option`): the `Machine` branch never reads it, and the DB write is NULL for machines
  regardless. Making it `Option` would churn the entire human path
  (`reconcile_by_email`, `create_new_profile_and_link`, `lookup_cached_email`) for zero
  machine-path benefit — a deliberate scope boundary.

### `temper-services` — `src/auth/` (the seam)

- New shared `RawJwtClaims` struct (superset both surfaces decode into):
  `sub, email?, email_verified?, azp?, gty?, exp, iat`. Replaces api's `JwtClaims` and mcp's
  `McpClaims`.
- New `MachineIdentity { client_id: String }`.
- New `fn detect_machine(raw: &RawJwtClaims) -> Option<MachineIdentity>` — owns decision (2)
  and the `azp`-vs-`sub` extraction (3). Returns `None` for humans.
- New `fn machine_claims(m: MachineIdentity) -> AuthClaims` — stamps
  `PrincipalKind::Machine`, `provider = "auth0-m2m"`, `external_user_id = client_id`,
  `email = ""`.

Each surface's construction collapses to:
```rust
let raw = decode::<RawJwtClaims>(token, key, validation)?;   // stays per-surface
let claims = match auth::detect_machine(&raw) {
    Some(m) => auth::machine_claims(m),                       // shared
    None    => { /* existing human path — email resolution UNCHANGED */ }
};
```

### `temper-services` — `services/profile_service.rs`

- `resolve_from_claims` becomes a `match` on `claims.principal_kind`:
  - `Human` → `resolve_human_from_claims` (today's body, extracted verbatim).
  - `Machine` → `resolve_machine_from_claims` (new).
- `resolve_machine_from_claims`:
  1. `lookup_link_by_provider` — **reused as-is** (keyed `(auth0-m2m, client_id)`); found →
     `get_by_id` (idempotent second-sight).
  2. First sight → `create_agent_profile_and_link` (new: **NULL** email, `is_default = true`,
     handle from slugified `agent-{client_id}` via `generate_profile_handle`) →
     `provision_profile_entities` (**reused** — same `web`/`cli`/`mcp` emitters + default
     context a human gets; the steward writes through the `<handle>@mcp` emitter).
  3. `get_by_id`.
- The machine branch **never enters `reconcile_by_email`** — structurally skipped, no verified
  email exists.

### `temper-api` / `temper-mcp` — surface middleware

- Both decode into the shared `RawJwtClaims` and call `detect_machine` / `machine_claims`.
  api's human branch (email resolution via token → cached link → userinfo) is untouched.
- Machine profiles ride the ordinary gate rails: fresh `system_access = 'none'` (schema
  default) → open mode passes `require_system_access`; a gated instance denies until granted
  (see Follow-ups). No auth-path special-casing.

### `temper-cloud` (TS) — 4a

- `buildAuth0AsMetadata` in `oauth/metadata.ts`: append `"client_credentials"` to
  `grant_types_supported`. `buildAsMetadata` (Temper AS branch) is **4c — untouched**.

## Test plan (TDD)

**Committed (in-repo):**
- **Rust unit** (`temper-services::auth`): `detect_machine` truth table — machine shape
  (`gty=client-credentials`, `azp` set, no email) → `Some(client_id)`; **human token WITH
  `azp` but `gty=authorization_code` → `None`** (the critical guard); `sub`-strip fallback
  when `azp` absent.
- **Rust `#[sqlx::test]`** (`profile_service`): machine first-sight provisions agent profile +
  `(auth0-m2m, client_id)` link with NULL email + emitters + context; second call idempotent
  (no dup); a human is unaffected (no cross-branch reconcile).
- **Rust `#[sqlx::test]`** (`auth`): `authenticate` with machine claims → active profile;
  `require_system_access` open-passes / gated-denies.
- **TS unit** (`temper-cloud`): metadata advertises `client_credentials`.
- **e2e (`test-db`):** synthetic M2M-shaped JWT signed by the test JWKS fixture, driven
  through an mcp tool call → asserts a fresh agent profile is provisioned ("e2e at the
  production caller").

## Follow-ups (operator / console — not code)

- Provision the Auth0 **M2M application** authorized for the **mcp audience** (`MCP_AUDIENCE`)
  — the steward hits MCP, so the M2M app targets the mcp audience, not the API audience.
- Once provisioned: obtain a **real M2M token** and validate `detect_machine` against it —
  confirms the `azp`/`gty`/`sub` assumptions in decisions (2)/(3). (Deferred real-token gate.)
- Grant the agent's client-id profile team membership + `cogmap grant --write` so the deployed
  steward passes the gate on temperkb.io.
- **4c** (self-hosted Temper AS `client_credentials` grant + machine `MintedClaims` variant,
  ideally a ts-rs-shared machine-claim type) remains deferred until a self-hosted instance
  wants agents.
