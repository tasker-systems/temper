# Cloud-Mode Auth0 Design — Unit B.4 Research Answers

**Date:** 2026-04-19
**Context:** `temper`
**Goal:** `temper-cloud-portable-memory`
**Task:** `2026-04-19-unit-b-4-auth0-research-for-cloud-mode-token-issuance`
**Branch:** `jct/temper-cloud-mode-portable-memory`
**Parent spec:** `docs/superpowers/specs/2026-04-18-cloud-mode-and-portable-memory-design.md` §Unit B.4
**Unblocks:** Unit B.2 (cloud-mode dispatch rewrites)
**Introduces:** Unit D (server-minted cloud session tokens — W2)

---

## Problem

Unit B.2 rewrites `resource::create` / `resource::update` / `push` / `pull` / `list` / `show` / `search` / `sync` to route through cloud-mode branches when `VaultState::Cloud`. Before B.2 can land, five Auth0-shaped questions from the parent spec's §Unit B.4 need concrete answers. This note makes one recommendation per question, names the Auth0 flows and endpoints B.2 will consume, and sketches the Unit D follow-on work that covers the capabilities this first cut intentionally defers.

The context constraints on cloud-mode are: no interactive browser, no persistent disk, token arrives via env var. The surfaces that share this token inside a cloud agent session are both the `temper` CLI REST client and any `mcp__temperkb_io__*` tool calls the agent makes — so whatever we decide for token lifecycle covers both.

---

## Recommendations at a glance

| # | Question | Recommendation for B.2 | Follow-on (Unit D) |
|---|----------|------------------------|--------------------|
| Q1 | Non-interactive token issuance | **W1** — `temper auth export-token` exports a refreshed access token from local auth. No refresh-token export. | **W2** — server-minted separate grant per cloud session via Auth0 Management API. |
| Q2 | In-memory refresh contract | Introduce `TokenStore` trait. `DiskTokenStore` (local default) and `MemoryTokenStore` (cloud / ephemeral). | No new refresh semantics — Q2's engine is consumed by W2 as-is. |
| Q3 | Scope/expiry trade-offs | Full user scope, Auth0 default AT/RT lifetimes. No reduced-scope tokens. | Audience-scoping to `temper` when Management API mints session grants. |
| Q4 | Security posture + revocation | Natural AT expiry is the **only** revocation path — JWTs are stateless-until-`exp` and `temper-api` does not consult a revocation list. | Per-session revocation enforced by `temper-api` checking a `cloud_sessions.revoked_at` column on each authenticated request, keyed by a session-id claim the Management API embeds in the minted AT. |
| Q5 | Provider abstraction | Convert `provider: String` → `Provider` enum with one `Auth0 { domain: String }` variant. Reset `auth.json` — no migration shim. | Extend enum only when a second provider is actually planned. |

---

## Q1 — Non-interactive token issuance

**Recommendation: W1 now, W2 as Unit D.**

### Why M2M and device-flow-reuse are rejected

- **M2M client credentials** are the wrong shape. Per Auth0's own documentation (`get-started/apis/api-access-policies-for-applications`): "Client access is intended for machine-to-machine communication using the Client Credentials Flow. User access covers flows where an access token is generated on behalf of an end-user, **excluding** the Client Credentials Flow." Our cloud session represents a human user (JWT `sub` = `profile_id`). Using M2M would lose user identity at the JWT layer, breaking every authorization path in `temper-api` that scopes through `resources_visible_to` / `can_modify_resource`.
- **Refresh-token export (naively)** is broken under Auth0's Refresh Token Rotation. Each `/oauth/token` exchange with `grant_type=refresh_token` issues a new RT and invalidates the previous one; reuse detection kills the entire grant family. If the user's local CLI and a cloud session both hold the same exported RT, whichever exchanges first invalidates the other's copy, and on the other's next refresh Auth0 flags a breach and kills the whole grant. Local auth dies alongside the cloud session. Not viable.

### W1 — access-token-only export

**Flow:**
1. User runs `temper auth export-token` on their local machine.
2. Command loads local auth via `DiskTokenStore::default()` (post-Q2 refactor), calls `get_valid_token(&store, token_url, client_id)` so the AT is refreshed locally if near expiry. The refresh uses the user's local RT and writes the rotated RT back to disk (standard local behavior). The RT itself never leaves the local machine.
3. Command prints the AT to **stdout** as plain text. No `--json` flag — the token is already a single string, and JSON wrapping only expands the capture surface for agent-framework structured-output loggers. Plain stdout is correct.
4. Command prints a security warning to **stderr** (stdout stays cleanly pipeable into `pbcopy` / stdin):
   ```
   ⚠  This access token grants full access to your temper account at
      your current permission levels until it expires (~24 hours).
      Once issued, the token cannot be revoked early — treat leaked
      tokens as live for their full lifetime. Per-session revocation
      is coming in Unit D of the cloud-mode goal.

      Recommended handling:
        temper auth export-token | pbcopy          # macOS clipboard
        temper auth export-token | wl-copy         # Linux wayland
        temper auth export-token | <agent-secret-input>
      AVOID:
        temper auth export-token > token.txt       # file lands in
                                                     backups (Time
                                                     Machine / iCloud
                                                     / Dropbox)
        TEMPER_TOKEN=$(temper auth export-token)   # shell history +
                                                     exposed via
                                                     /proc/<pid>/environ
   ```
5. User pastes the AT into Claude Web / Cursor cloud agent / Devin session secrets as `TEMPER_TOKEN`. B.1's env-var floor (shipped) picks it up.

**Properties:**
- Cloud session uses the AT for both `temper` CLI REST calls and any `mcp__temperkb_io__*` tool calls the agent makes. Single token lifecycle.
- 24h (Auth0 default AT lifetime) ceiling. Re-export to renew; no in-cloud refresh because no RT is exported.
- **Zero grant entanglement with Claude Desktop MCP.** Claude Desktop runs its own device flow against `temper-mcp`'s OAuth discovery endpoints (`crates/temper-mcp/src/discovery.rs`) and holds its own grant.
- Local CLI is untouched by anything that happens in the cloud session. The cloud session never holds an RT from the user's local grant.
- **Post-Unit-D framing:** once `temper auth create-cloud-session` ships (Unit D), `export-token` remains available as a dev-expedient for quick tests and throwaway sessions but is **permanently non-revocable-early** because it exports from the user's local grant. `create-cloud-session` becomes the recommended path for durable / production use because it mints a separately-revocable grant. `export-token` is not deprecated, but the recommendation shifts.

**Command implementation notes:**
- Lives alongside `login`, `logout`, `status`, `token` in `crates/temper-cli/src/commands/auth.rs`.
- The existing `temper auth token <jwt>` command imports a JWT via **positional arg**, which leaks the token to shell history, `ps auxww`, and `/proc/<pid>/cmdline`. **Migrate `temper auth token` to stdin-only input** (`temper auth export-token | temper auth token` would be the new round-trip). This is a breaking change to the CLI shape; per the "no premature backward compat" rule (repo is ~1 month old), acceptable.
- No new `temper-client` API needed beyond Q2's `TokenStore` refactor — the command just calls `get_valid_token(&DiskTokenStore::default(), …)` and prints the result.

### W2 (Unit D) — server-minted separate grant

Detailed in the "Unit D sketch" section below. Short version: `temper-api` gains endpoints that use the Auth0 Management API (with its own M2M credentials) to mint a completely separate grant per cloud session. That grant carries `offline_access` so cloud sessions can refresh in-memory using the RT via Q2's `TokenStore`. Revocation is per-session via `/api/v2/users/{id}/revoke-access`.

This is the correct long-term architecture and lands alongside B.2/B.3 as part of this same goal.

---

## Q2 — In-memory refresh contract

**Recommendation: `TokenStore` trait in `temper-client`.**

This work lives entirely on the client side — `temper-api` never holds user tokens; it validates incoming JWTs via JWKS. The trait abstracts over "where does the token live" so one refresh code path serves local CLI, cloud mode, and MCP-from-cloud.

### Current code

In `crates/temper-client/src/auth.rs`:

- `refresh_token(auth, token_url, client_id) -> Result<StoredAuth>` at lines 308–348 computes a new `StoredAuth` and calls `save_auth(&updated)` at line 346 — coupled directly to `~/.config/temper/auth.json`.
- `get_valid_token(token_url, client_id) -> Result<String>` at lines 351–360 calls `load_auth()` (disk or env, per B.1's env-var floor) + `refresh_token()` (disk-write).

### Proposed trait

```rust
pub trait TokenStore: Send + Sync {
    fn load(&self) -> Result<Option<StoredAuth>>;
    fn save(&self, auth: &StoredAuth) -> Result<()>;
}
```

### Structural secrets hardening (must land with the refactor)

Today `StoredAuth` at `auth.rs:23-33` derives `Debug, Clone, Serialize, Deserialize` and carries `access_token: String` / `refresh_token: Option<String>` directly. Any of these leak paths is one line from active:

- `tracing::error!("auth state: {auth:?}")`
- `dbg!(auth)` during debugging, committed by accident
- `panic!` over a `Result` carrying `StoredAuth` → token in stderr backtrace
- `#[instrument]` on a function taking `&StoredAuth` → token in span fields
- Any future `serde_json::to_string(&store)` → token in JSON

"Verify via grep during B.2" is not a durable defense. Fix structurally, in the same PR as the `TokenStore` refactor:

- **Introduce `SecretString` newtype** in `temper-client` (or pull in the `secrecy` crate, which provides `SecretString` with `zeroize` on drop and a `Debug` impl that prints `[REDACTED]`).
- **Replace `access_token: String` → `access_token: SecretString`** on `StoredAuth`. Same for `refresh_token: Option<SecretString>`.
- **Replace the derived `Debug` on `StoredAuth`** with a manual impl that prints field names but redacts token values (or rely on `SecretString`'s Debug if the crate is used).
- **`MemoryTokenStore` should not implement `Serialize` at all.** Mark with `#[serde(skip)]` where applicable or simply don't derive. Prevents future "log the whole client state as JSON" accidents.
- **`StoredAuth::clone()` stays available** (the refresh flow needs it), but reviewers should flag any `auth.clone()` followed by logging.

Acceptance: grep for `.access_token` / `.refresh_token` string extraction in the refactored code; each site extracts via `expose_secret()` (or equivalent) immediately before network use, never stored in a `String` variable that outlives the request.

### Two impls

- **`DiskTokenStore`** — the local CLI default. Wraps the existing `load_auth_from` / `save_auth_to` with a configurable path (default `auth_json_path()`). `auth.json` format unchanged on disk (base64-ish token strings are already what Auth0 returns).
- **`MemoryTokenStore`** — ephemeral. Holds `Arc<RwLock<Option<StoredAuth>>>`. Constructed **once at session start** from `TEMPER_TOKEN` (+ optional `TEMPER_REFRESH_TOKEN` when Unit D ships). Does not re-parse env on every `load()` call (the current `stored_auth_from_env` path at `auth.rs:217` re-parses on every `load_auth()` — acceptable for B.1's no-refresh case, but not for cloud mode once refresh lands in Unit D, because the env var never changes and the post-refresh authoritative state lives in the store).

### Refactored API

- `refresh_token(store: &dyn TokenStore, token_url, client_id) -> Result<StoredAuth>` — exchanges the RT via `POST /oauth/token`, computes new `StoredAuth`, writes via `store.save()`. Disk path is observably unchanged.
- `get_valid_token(store: &dyn TokenStore, token_url, client_id) -> Result<String>` — reads via `store.load()`, checks `needs_refresh`, refreshes through the trait if needed.
- Free-function `get_valid_token(token_url, client_id)` / `refresh_token(auth, …)` shims are **eliminated**. Per the "no premature backward compat" rule, call sites migrate to pass an explicit `TokenStore`. Expected change: ~5-10 call sites across `temper-cli` and `temper-client`, each replacing a bare call with a `DiskTokenStore::default()` argument.

### Lands in B.2 — ordering is load-bearing

The refactor is a **hard prerequisite** for `temper auth export-token` and for cloud-mode dispatch. Ordering within B.2:

1. `TokenStore` trait + `DiskTokenStore` + `SecretString` newtype + manual `Debug` on `StoredAuth`. Free-function shims eliminated.
2. `MemoryTokenStore` impl + env-var-to-store bootstrap at client construction time.
3. `temper auth export-token` command (depends on `DiskTokenStore` already being the default path).
4. `temper auth token` migrated to stdin-only.
5. Remaining cloud-mode dispatch (the bulk of B.2 per the parent spec).

**Why the ordering matters:** today's `refresh_token()` at `auth.rs:346` calls `save_auth(&updated)` unconditionally. If `export-token` ships before the refactor, a cloud runner that happens to have a writable `$HOME` (e.g., a GitHub Actions runner, a Vercel function with a tmpfs home, a hostile but writable container mount) will persist refreshed tokens to `~/.config/temper/auth.json`. The `TokenStore` abstraction is what makes "cloud mode never writes tokens to disk" a structural property, not a per-call discipline. Ship it first.

### Known latent bug to fix during the Q2 work

`parse_jwt_claims` at `auth.rs:166` extracts `sub` and runs `uuid::Uuid::parse_str(s).ok()`. When `sub` parses as a UUID it becomes `profile_id`; when it doesn't, `profile_id` silently becomes `None`. Today this is fine — our own issuer emits profile UUIDs. It will break silently the moment Unit D introduces Management-API-minted tokens whose `sub` is an Auth0 user-id string (`auth0|6123abc...`) rather than the profile UUID. Two paths:

- **Fix at parse time.** Extend `JwtClaims` so `profile_id` carries either a `Uuid` or a `String`, and callers decide how to map. Keeps the failure loud.
- **Accept the None for now, document it.** Revisit as part of Unit D's "how do we embed claims in minted tokens" research.

Recommendation: **fix at parse time** in the same Q2 refactor. Small change, preempts a hard-to-debug Unit D integration failure.

---

## Q3 — Scope and expiry trade-offs

**Recommendation: full user scope, standard expiry.**

- B.2 cloud sessions do arbitrary user-level work: create/update across any context, push/pull any resource, search anything the user can see. There is no principled way to classify operations by sensitivity — we don't make that distinction anywhere else in the product. Scope reduction would require inventing one.
- Accept Auth0's default AT lifetime (~24h, per-client configurable via the Management API if we want to tune later). RT rotation with the default 30-day absolute lifetime. These are already implied by the `offline_access` scope the local device flow requests.
- Revisit only if and when a specific use case emerges (e.g., a read-only browsing agent, a "graph explorer only" tool). Not speculative scope for B.2.

**Unit D angle:** when the Management API mints a session grant, audience-scope it to the `temper` API only. Session tokens shouldn't be valid for any Auth0-protected resource beyond ours. This is a natural scope-narrowing that doesn't complicate the CLI surface.

---

## Q4 — Security posture and revocation

**Recommendation: accept grant-family blast radius in W1, track per-session revocation as Unit D.**

### Where the token lives in a cloud session

- **In the agent host's secrets manager** — Claude Web session secrets, Cursor cloud agent env, GitHub Actions `secrets.TEMPER_TOKEN`, etc. Injected into the agent process as `TEMPER_TOKEN`.
- **In process memory only.** Never written to disk in cloud mode — B.1's env-var path has no `save_auth` call, and Q2's `MemoryTokenStore` makes this structural.
- **Never logged.** Existing `temper-client` code doesn't log bearer tokens; this should remain true across all B.2 work. Verify via grep during B.2 implementation.

### Revocation behavior — the important nuance

**Access tokens are stateless until `exp`.** Our JWT ATs carry claims and a signature; `temper-api` validates them via JWKS (signature + standard claims, see `crates/temper-mcp/src/middleware.rs:27` for the MCP-side pattern and the equivalent in `temper-api`). **No revocation list is consulted.** This has a consequence that needs to be explicit: once an AT is issued, it is valid until its `exp` claim, and there is no way to invalidate it early without adding server-side revocation machinery.

**`POST /oauth/revoke`** with a refresh token revokes the entire grant family — but only for purposes of **future token issuance**. Any AT already issued under that grant keeps working until its natural expiry. In W1 this is doubly moot because the cloud session never holds an RT.

**`POST /api/v2/users/{id}/revoke-access`** (Management API) can target a `session_id` and requires `delete:sessions` scope. Useful for Unit D, but again: on its own it only affects future token issuance from that session. To make it a live-AT revocation lever, Unit D must **also** add a revocation check inside `temper-api`'s JWT validator.

**Therefore, in W1, the only revocation mechanism is natural AT expiry.** The user cannot revoke a leaked cloud AT early. The window is bounded at Auth0's AT lifetime (~24h) and that is the full story. This is explicit in the `temper auth export-token` security warning.

**`temper auth login` re-login** issues a new grant for the user's local CLI going forward. It does **not** invalidate outstanding ATs from the prior grant. The local CLI simply uses the new grant from that moment on. Prior cloud sessions keep working until their AT expires.

### Blast radius comparison

| Scenario | W1 blast radius | Unit D (W2) blast radius |
|----------|-----------------|--------------------------|
| Cloud AT leaks | Up to ~24h of user-scope access; attacker can't refresh; **no early-revoke lever** | Session-scope access; per-session revocation enforced within request-round-trip once the validator check lands |
| Attempt to revoke a single cloud session | **Not possible** — only natural AT expiry | `temper auth revoke-cloud-session <id>` → marks row in `cloud_sessions`; temper-api rejects subsequent requests immediately |
| Attempt to revoke "everything" | Attacker keeps the AT until it expires; user can rotate going forward via `temper auth login` but outstanding ATs are live | Bulk revoke all user's cloud sessions via a list+revoke loop |
| Compromised cloud session detected | Accept ~24h window | Targeted revoke; session blocked within a round-trip |

### Why W1 is acceptable as the first cut

- Leaked AT has no refresh, so the attacker's window is bounded at Auth0's AT lifetime (~24h). A stateless-JWT system without a revocation check has the same bound for any leaked AT today — we're not making it worse for cloud tokens relative to, say, a leaked `temper auth login` AT.
- Claude Desktop MCP (the other active authenticated surface) already runs its own separate grant via the device flow against `temper-mcp`'s OAuth discovery endpoints. A compromised cloud session does not extend to the user's Claude Desktop MCP work.
- The alternative for W1 (server-side revocation list without Unit D's full machinery) is essentially building Unit D. Not worth splitting.

### Explicit follow-on

**Per-session revocation is tracked as Unit D in `temper-cloud-portable-memory`.** Goal doc, session notes, and the Unit D task description all carry this framing so it's not implicit or forgettable.

---

## Q5 — Provider abstraction

**Recommendation: convert `provider: String` → `Provider` enum as part of B.2 housekeeping.**

### Current code

`StoredAuth` at `crates/temper-client/src/auth.rs:23-33`:

```rust
pub struct StoredAuth {
    pub provider: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub profile_id: Option<uuid::Uuid>,
    pub device_id: Option<String>,
}
```

`provider` is effectively hardcoded: `stored_auth_from_env` at line 225–228 defaults to `"auth0"` when `TEMPER_PROVIDER` is unset; `temper auth login` passes `"auth0"` directly; `temper auth token <jwt> --provider auth0` is the only entry that varies it and no one types anything else.

### Proposed shape

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provider {
    Auth0 { domain: String },
}

impl FromStr for Provider {
    // "auth0" with no colon → pulls domain from config default
    // "auth0:temper.us.auth0.com" → explicit
}
```

One variant. Domain carried on the variant so it's authoritative at the token-issuer layer (today the domain is in `temper-client`'s config; enum carries it so `Provider` is self-describing).

### Migration

Per the "no premature backward compat" rule (repo is ~1 month old), **reset `auth.json` rather than preserve old format**. Users run `temper auth login` once after upgrade. Code stays tight. This is acceptable because:

- Only developer machines currently have `auth.json` files in the wild.
- `temper auth login` already re-issues a grant cleanly.
- The alternative (serde-default + two-format deserialize) adds code for zero user-visible win.

### Forcing function

Cloud mode is the right moment because Unit D will plausibly carry `Provider` context into Management API calls (which Auth0 tenant to target). Converting stringly → enum now avoids a second refactor later.

### Scope limit

**One variant only.** No `SelfHosted`, no `GitHub`, no hypothetical IdP. The enum shape is the change; a second provider is a separate design question with no current forcing function.

---

## Integration picture — how the recommendations pull together

The five recommendations are not independent. They form one coherent first cut:

1. **Q2's `TokenStore` trait is the spine.** Everything else consumes it.
2. **W1 (Q1) exports an AT via the trait.** `temper auth export-token` uses `get_valid_token(&DiskTokenStore::default(), …)` to produce a refreshed-if-needed AT from the user's local grant. No RT leaves the user's machine.
3. **Cloud mode uses `MemoryTokenStore` (Q2) with the pasted AT.** Refresh is not active in W1 (no RT in the store), so the cloud session is read-the-token-once-use-until-expiry. Q2's trait is still the right shape because it's what Unit D will plug refresh into without a second refactor.
4. **Q3 keeps scope decisions out of the refactor.** Full scope, default expiry. The Management API work in Unit D is where scope-narrowing gets a natural home.
5. **Q4's revocation answer for W1 is "AT expiry, period."** JWT ATs are stateless-until-`exp` and `temper-api` does not consult a revocation list. Surfaced explicitly in the `export-token` security warning. Unit D's server-side revocation check is what closes this gap.
6. **Q5's `Provider` enum lands with Q2's refactor.** Both touch the same `auth.rs` surface; bundling them avoids a second round of churn on the same file.

### B.2 ordering implied by this

1. Q2 refactor (first): `TokenStore` trait + `DiskTokenStore` + `MemoryTokenStore`; `SecretString` newtype on `StoredAuth` tokens + manual redacted `Debug`; `parse_jwt_claims` fix so non-UUID `sub` doesn't silently drop `profile_id`; free-function shims eliminated.
2. Q5 refactor: `Provider` enum, reset `auth.json` format.
3. Migrate `temper auth token` from positional-arg to stdin-only (breaking).
4. `temper auth export-token` command with stderr security warning and structured Do/Avoid guidance (no `--json` flag).
5. Cloud-mode dispatch (the bulk of B.2 per the parent spec): `resource::create` / `resource::update` / `push` / `pull` / `list` / `show` / `search` / `sync` branches against `VaultState::Cloud`, all consuming `MemoryTokenStore`-backed `Client`.

Steps 1–4 are the security-critical ones — getting them in order keeps the blast radius of an accidentally-shipped intermediate state bounded to "the command doesn't exist yet" rather than "the command leaks tokens to disk."

---

## Unit D sketch — server-minted cloud session tokens

Enough shape to create the task. Full design and plan are Unit D's own artifacts.

### Goal

Per-session revocation + in-cloud refresh via grants that are fully isolated from the user's local grant. Adds a fourth unit to `temper-cloud-portable-memory` and should be taken up alongside or immediately after B.2/B.3.

### `temper-api` side

- **New Auth0 Management API M2M application.** `temper-api` holds this credential; the CLI never does. Config lives in the secrets manager (Vercel env, etc.), loaded at process start.
  - **Scope the M2M credential minimally.** Only `create:users_access_tokens` (or whatever exact scope the chosen mint flow requires) and `delete:sessions`. **No `read:*` scopes.** This credential is the ultimate blast-radius key: compromise of the Vercel env → ability to mint tokens for any user. Keep its capability surface as narrow as possible.
  - **Rotate regularly.** At least quarterly, with an automated rotation pipeline. Rotation cadence belongs in Unit D's acceptance criteria.
  - **Audit-log every mint and revoke.** Append-only `cloud_session_audit` table (or equivalent) with `(event, profile_id, session_id, actor, at)`. Graduate audit-logging from "out-of-scope" (per current §Out of scope) to **in-scope for Unit D** — given the M2M credential's blast radius, an audit trail is a baseline, not a future polish.
- **Service:** `cloud_session_service` in `crates/temper-api/src/services/`. Functions: `mint(profile_id, label, ttl_hours)`, `list(profile_id)`, `revoke(profile_id, session_id)`.
- **Endpoints:**
  - `POST /auth/cloud-sessions` (mint) — body carries `label`, optional `ttl_hours`; response carries `access_token`, optional `refresh_token`, `session_id`, `expires_at`.
  - `GET /auth/cloud-sessions` (list) — profile-scoped; response carries `[{ session_id, label, created_at, last_used_at }]`.
  - `DELETE /auth/cloud-sessions/{session_id}` (revoke) — profile-scoped; server calls Management API.
- **Schema:** `cloud_sessions` table — `id` (uuidv7), `profile_id`, `label`, `auth0_grant_id`, `created_at`, `last_used_at`, `revoked_at` (nullable). Plus `cloud_session_audit` as above.
- **Token validation extension.** For revocation to actually stop outstanding ATs (not just future issuance), `temper-api`'s JWT validator must check `cloud_sessions.revoked_at IS NULL` on each authenticated request carrying a cloud-session AT. Traps to avoid:
  - **URL-namespace the custom claim.** Auth0 strips non-namespaced custom claims from the AT. Use `https://temperkb.io/session_id` (not bare `session_id`). A bare claim that Auth0 silently strips means the validator never sees a `session_id` → the revocation check becomes a fails-open no-op, which is **worse than no revocation at all** because we'd believe sessions are revocable when they're not. Lock this in Unit D's implementation plan.
  - **Extend the claims struct in BOTH `temper-api` and `temper-mcp`.** Today `temper-mcp` uses its own `McpClaims` struct in `middleware.rs`; `temper-api` uses a parallel one. Both need the new namespaced `session_id` field and both need the revocation check. Forgetting `temper-mcp` means MCP-from-cloud calls bypass revocation entirely.
  - **Run the revocation check AFTER signature validation.** Invalid tokens must fail at the JWKS step, not at the DB lookup. Otherwise a volumetric DOS against an unauthenticated endpoint carrying random bearer tokens exhausts the DB connection pool.
  - **Fail closed on unknown `session_id`.** If a token carries a `session_id` that doesn't exist in `cloud_sessions`, reject it (401). Not "assume it's an older non-session token and let it through" — a compromised signing config could mint tokens with arbitrary claims including nonexistent session ids, and the only safe default is to reject anything claiming session-origin that doesn't check out.
  - **Indexed lookup.** `cloud_sessions` primary key on `session_id`; constant-time check with a connection-pool budget. Expected negligible given the table is small.
- **TOCTOU on in-flight long-running requests.** Revocation stops **new** requests within a round-trip; an in-flight request that already passed the validator completes normally. For short REST calls this is trivially bounded. For long-lived MCP streams (SSE-style) or future long-running HTTP responses, either (a) document "in-flight completes, new requests blocked" as an accepted property, or (b) have the streaming handler re-check `revoked_at` periodically. **Default to (a); flag (b) as a future enhancement.**
- **Authorization:** profile-scoped throughout. Users can only mint/list/revoke their own sessions.

### `temper-client` + CLI side

- `temper auth create-cloud-session --label <str> [--ttl 24h] [--include-refresh]` — authenticated call to `POST /auth/cloud-sessions`. Prints AT + optional RT + session id with security warnings mirroring W1.
- `temper auth list-cloud-sessions` — prints labels, created, last-used, session ids.
- `temper auth revoke-cloud-session <id>` — authenticated `DELETE`.
- Cloud-mode `Client` constructs `MemoryTokenStore` from `TEMPER_TOKEN` (+ optional `TEMPER_REFRESH_TOKEN`); Q2's refresh engine handles rotation transparently.

### Research sub-block inside Unit D

- **Exactly which Auth0 Management API call creates a separable grant on a user's behalf.** Candidates: `POST /api/v2/users/{id}/authenticate`-flavored endpoints, Privileged Worker Token Exchange with Token Vault, or a tenant-specific custom flow. Answer determines the service implementation shape.
- **How to embed a `session_id` claim in the minted AT.** Auth0 custom claims are added via login flow Actions, Rules, or the Management API's token customization — which path fits a non-interactive mint?
- **Audience scoping.** Session-minted tokens should be valid only for the `temper` API audience, not the full Auth0 tenant surface.
- **TTL defaults and caps.** What's the right default session lifetime (24h? 72h? 7d?), and what maximum should we enforce server-side?

### Task shape (to create)

- **Title:** `Unit D: server-minted cloud session tokens`
- **Mode/effort:** `plan/medium` initially — research sub-block before build. Bump to `build/large` when the Auth0 flow is picked.
- **Goal:** `temper-cloud-portable-memory`
- **Acceptance (rough):** `temper auth create-cloud-session` mints a revocable grant separate from the user's local. Revoking it leaves local CLI + Claude Desktop MCP untouched. Cloud session refreshes its AT in-memory via RT rotation.

---

## Out of scope for this research block

- **B.2 implementation** — lives in a separate plan artifact (`docs/superpowers/plans/2026-04-XX-unit-b-2-cloud-mode-dispatch.md`, to be written at B.2 task start).
- **Working directory layout** — Unit B.3's problem.
- **Claude Desktop MCP auth flow changes** — Claude Desktop runs its own device flow via `temper-mcp`'s discovery endpoints; no changes needed here.
- **Second token provider** — the `Provider` enum shape is the change; building a `SelfHosted` variant is a separate future spec.

---

## Open questions carried into follow-on work

- **Management API M2M credential rotation.** How often does `temper-api`'s Management API credential rotate, and where does the rotation happen? Belongs in Unit D.
- **Cloud session TTL defaults.** Unit D.
- **Behavior when a cloud session's refresh token fails mid-session.** Expected in Unit D when W2 is implemented — for now it's irrelevant because W1 doesn't refresh in-cloud.
- **Whether to add a `last_used_at` update throttle.** Hitting the DB on every authenticated request is expensive; a 5-minute debounce is probably fine. Unit D implementation detail.

---

## Security review summary

The design was security-reviewed on 2026-04-19 (commit `79bb273` state). The review verified:

- M2M client credentials correctly rejected for user-representing cloud sessions.
- Claude Desktop MCP grant isolation correctly characterized (verified via `crates/temper-mcp/src/discovery.rs`).
- `/oauth/revoke` correctly described as non-invalidating for outstanding ATs (verified via the JWKS-decode pattern in `crates/temper-mcp/src/middleware.rs`).
- `TokenStore` trait as the right abstraction.
- `Provider` enum scope discipline (one variant, no speculative expansion).

It raised concrete risks that have been addressed inline in this spec:

- **Secrets-in-memory hygiene** (§Q2: `SecretString` newtype, manual redacted `Debug` on `StoredAuth`, no `Serialize` on `MemoryTokenStore`) — **addressed in Q2 structural hardening**.
- **`refresh_token()` unconditional disk write** — **addressed by enforcing the Q2-refactor-before-export-token ordering in Integration Picture**.
- **`parse_jwt_claims` silently dropping non-UUID `sub`** — **addressed as a known latent bug to fix in Q2**.
- **`temper auth token <jwt>` positional arg leaks to shell history / `ps` / `/proc`** — **addressed by migrating to stdin-only input**.
- **`--json` on `export-token` expands capture surface** — **dropped; stdout is plain token only**.
- **Shell-redirection to file risk** — **addressed in stderr warning's Do/Avoid guidance**.
- **Env-var bootstrap re-parsing on every call** — **addressed in `MemoryTokenStore` read-once-at-construction note**.
- **Unit D trap: bare (non-namespaced) `session_id` claim silently stripped by Auth0** — **addressed in Unit D's token-validation extension**.
- **Unit D trap: claim struct must be extended in both `temper-api` and `temper-mcp`** — **addressed**.
- **Unit D trap: revocation check ordering (after signature validation)** — **addressed**.
- **Unit D trap: fail-closed on unknown `session_id`** — **addressed**.
- **Unit D: M2M credential scope minimization, rotation cadence, audit-log** — **addressed; audit-log graduated from out-of-scope to in-scope for Unit D**.
- **Unit D: TOCTOU on in-flight long-running requests** — **addressed; "in-flight completes, new requests blocked" is the accepted property**.

## Acceptance (for this research note)

- One actionable recommendation per Q1–Q5. ✓
- In-memory refresh contract specified at `temper-client` API level (trait shape + two impls + refactored function signatures + secrets hardening). ✓
- Revocation/rotation story explicit enough to implement without re-research: W1 uses AT-expiry; W2 uses Management API `/api/v2/users/{id}/revoke-access` + namespaced `session_id` claim + per-request revocation check. ✓
- Provider abstraction decision made and scoped: enum with one variant, reset `auth.json`. ✓
- Follow-on work (Unit D) shaped concretely enough to create a task, with the critical security traps pinned in the sketch. ✓
- Independent security review completed and findings integrated. ✓
