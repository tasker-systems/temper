# Auth-Orchestration Seam — Stage 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract the two-level authenticate/authorize gate sequence into a single `temper-services::auth` module that both temper-api and temper-mcp are forced to call, so an auth gate can never again be added to one surface and miss the other.

**Architecture:** A two-level chain in `temper-services/src/auth/`: `authenticate` (resolve profile + `is_active`) returns an `AuthenticatedProfile`; `require_system_access` consumes it and returns a `SystemAuthorized` token. Both surfaces call these and map a shared `AuthzError` to their own transport (HTTP `ApiError` vs `rmcp::ErrorData`). A cross-surface e2e parity suite drives the *real* caller on both surfaces to prove `is_active` and `system_access` are enforced identically.

**Tech Stack:** Rust, sqlx (Postgres, compile-time-checked macros), axum (temper-api middleware), rmcp (temper-mcp), cargo-nextest, e2e crate at `tests/e2e/`.

**Scope:** This plan covers **Stage 1 only** (the seam + parity test) from the spec `docs/superpowers/specs/2026-07-02-shared-auth-orchestration-seam-design.md`. Stages 2 (docs/auth), 3 (HMAC), and 4 (M2M) are tracked as separate temper build tasks and get their own plans.

## Global Constraints

- **No behavior change on the happy path.** Active + system-approved accounts on both surfaces behave exactly as today. Deactivated + no-access accounts are refused on both surfaces (already true after the MCP hotfix; this plan makes it *structural*).
- **Persistence stays in services; surfaces only map transport.** The seam calls existing `profile_service` / `access_service` functions — no new SQL in a surface, no gate logic duplicated in a surface. (CLAUDE.md: "surfaces dispatch through the service layer".)
- **SQL macro cache discipline.** The seam adds no new SQL (it composes existing service fns), so `.sqlx` should not need regeneration for Task 1–2. Test-target queries in the new e2e file use **runtime** `sqlx::query`/`query_scalar` (the test-fixture convention — see `mcp_round_trip_test.rs`'s `resolve_test_profile`), so they need no cache entry. Run the prepare ritual in Task 7 only if `cargo make check` reports a missing entry.
- **`test-db` feature gate.** Every `#[sqlx::test]` file starts with `#![cfg(feature = "test-db")]` or the unit-test CI job fails fast.
- **`cargo make check` (fmt + clippy `-D warnings` + machete + TS) must pass before every commit.** Run `cargo make fix` first.
- **Run focused tests + the touched crate's suite per task; full workspace/e2e only at branch end (Task 7).**
- Branch: continue on `jct/auth-seam-spec` (spec already committed there) — no new branch.

---

### Task 1: Seam module — `authenticate` + `AuthzError` (Level 1)

**Files:**
- Create: `crates/temper-services/src/auth/mod.rs`
- Modify: `crates/temper-services/src/lib.rs` (add `pub mod auth;`)
- Test: inline `#[cfg(test)]` module in `crates/temper-services/src/auth/mod.rs`

**Interfaces:**
- Consumes: `profile_service::resolve_from_claims(pool, &AuthClaims) -> ApiResult<Profile>` (`crates/temper-services/src/services/profile_service.rs:85`); `temper_core::types::{AuthClaims, AuthenticatedProfile}` (`crates/temper-core/src/types/auth.rs`); `temper_services::error::ApiError`.
- Produces:
  - `pub enum AuthzError { ProfileResolution(ApiError), Deactivated { profile_id: uuid::Uuid }, SystemAccessDenied { profile_id: uuid::Uuid } }`
  - `pub async fn authenticate(pool: &sqlx::PgPool, claims: &AuthClaims) -> Result<AuthenticatedProfile, AuthzError>`

> **Design note (refines the spec's error enum):** `SystemAccessDenied` carries `profile_id`, **not** a `SystemAccessDetails` struct. The seam decides *allow/deny*; building the denial payload (settings + own-request lookup, used only by temper-api's CLI-facing error) is transport presentation and stays surface-side. temper-mcp doesn't use details at all. This keeps the seam's boundary at the *decision*.

- [ ] **Step 1: Declare the module**

Add to `crates/temper-services/src/lib.rs`, alphabetically among the existing `pub mod` lines (after `pub mod backend;`):

```rust
pub mod auth;
```

- [ ] **Step 2: Write the seam skeleton with `authenticate`**

Create `crates/temper-services/src/auth/mod.rs`:

```rust
//! Shared authentication + authorization orchestration for both surfaces.
//!
//! The gate *sequence* lives here exactly once. temper-api and temper-mcp both
//! call these functions and map [`AuthzError`] to their own transport; neither
//! re-implements the ordering. Adding a future gate is one edit here, enforced
//! on every surface.
//!
//! Two levels form a typestate chain:
//! 1. [`authenticate`] — resolve the profile + `is_active`. Runs on every authed
//!    request on both surfaces. Yields [`AuthenticatedProfile`].
//! 2. [`require_system_access`] — consumes proof of Level 1, adds the access gate.
//!    Runs on the gated tier of both surfaces. Yields [`SystemAuthorized`].

use sqlx::PgPool;

use temper_core::types::ids::ProfileId;
use temper_core::types::{AuthClaims, AuthenticatedProfile};

use crate::error::ApiError;
use crate::services::profile_service;

/// The reason an authn/authz gate refused a request. Each surface maps these to
/// its own transport (HTTP status / rmcp error); the variants are the shared
/// vocabulary of *why*, never the words on the wire.
#[derive(Debug)]
pub enum AuthzError {
    /// `resolve_from_claims` failed (DB error, missing link data, etc.).
    ProfileResolution(ApiError),
    /// The resolved profile is soft-deleted (`is_active == false`).
    Deactivated { profile_id: uuid::Uuid },
    /// The profile is not an approved member of the gating team.
    /// Carries the id so a surface can build its own denial payload.
    SystemAccessDenied { profile_id: uuid::Uuid },
}

/// Level 1 — authentication. Verified+normalized claims → a resolved, active profile.
///
/// Runs on **every** authenticated request on **both** surfaces. Callers are
/// responsible for verifying the JWT and normalizing it into `claims` first
/// (each surface's audience differs); this function owns resolve + the
/// deactivation gate.
pub async fn authenticate(
    pool: &PgPool,
    claims: &AuthClaims,
) -> Result<AuthenticatedProfile, AuthzError> {
    let profile = profile_service::resolve_from_claims(pool, claims)
        .await
        .map_err(AuthzError::ProfileResolution)?;

    if !profile.is_active {
        return Err(AuthzError::Deactivated {
            profile_id: profile.id,
        });
    }

    Ok(AuthenticatedProfile {
        profile,
        claims: claims.clone(),
    })
}

// `require_system_access` + `SystemAuthorized` land in Task 2.

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;

    // Helper: build AuthClaims for a synthetic principal.
    fn claims(sub: &str, email: &str) -> AuthClaims {
        AuthClaims {
            provider: "test-provider".to_string(),
            external_user_id: sub.to_string(),
            email: email.to_string(),
            email_verified: Some(true),
            exp: 0,
            iat: 0,
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn authenticate_returns_active_profile(pool: PgPool) {
        let c = claims("seam-active", "active@example.test");
        let authed = authenticate(&pool, &c).await.expect("should authenticate");
        assert!(authed.profile.is_active);
        assert_eq!(authed.claims.external_user_id, "seam-active");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn authenticate_refuses_deactivated_profile(pool: PgPool) {
        // First resolve creates the profile.
        let c = claims("seam-deactivated", "deact@example.test");
        let authed = authenticate(&pool, &c).await.expect("first resolve");
        let id = authed.profile.id;

        // Soft-delete it (runtime query — test fixture, no macro cache needed).
        sqlx::query("UPDATE kb_profiles SET is_active = false WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("deactivate");

        let err = authenticate(&pool, &c).await.expect_err("should refuse");
        assert!(
            matches!(err, AuthzError::Deactivated { profile_id } if profile_id == id),
            "expected Deactivated, got {err:?}",
        );
    }
}
```

- [ ] **Step 3: Run the tests to verify they pass**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db -E 'test(auth::tests)'`
Expected: PASS — `authenticate_returns_active_profile`, `authenticate_refuses_deactivated_profile`. (Requires `cargo make docker-up` first.)

- [ ] **Step 4: `cargo make check` then commit**

```bash
cargo make fix && cargo make check
git add crates/temper-services/src/lib.rs crates/temper-services/src/auth/mod.rs
git commit -m "feat(services): auth seam Level 1 — authenticate + AuthzError"
```

---

### Task 2: Seam Level 2 — `require_system_access` + `SystemAuthorized`

**Files:**
- Modify: `crates/temper-services/src/auth/mod.rs` (add the fn, type, and tests)

**Interfaces:**
- Consumes: `access_service::has_system_access(pool, ProfileId) -> ApiResult<bool>` (`crates/temper-services/src/services/access_service.rs:34`); `AuthenticatedProfile`; `AuthzError` (Task 1).
- Produces:
  - `pub struct SystemAuthorized(pub AuthenticatedProfile)`
  - `pub async fn require_system_access(pool: &sqlx::PgPool, authed: &AuthenticatedProfile) -> Result<SystemAuthorized, AuthzError>`

- [ ] **Step 1: Add the failing tests**

Add these two tests inside the existing `#[cfg(test)] mod tests` in `crates/temper-services/src/auth/mod.rs` (a system-access-approved fixture requires a gating team + membership; the seam test asserts the *decision*, reusing whatever `has_system_access` returns for an open-mode instance where every authed profile is approved — matching `access_gate_test.rs::entitlements_in_open_mode`):

```rust
    #[sqlx::test(migrations = "../../migrations")]
    async fn require_system_access_allows_approved_profile(pool: PgPool) {
        // Open-mode default: an authenticated profile has system access.
        let c = claims("seam-approved", "approved@example.test");
        let authed = authenticate(&pool, &c).await.expect("authenticate");
        let ok = require_system_access(&pool, &authed).await;
        assert!(ok.is_ok(), "open-mode profile should be system-authorized: {ok:?}");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn require_system_access_refuses_when_gated(pool: PgPool) {
        // Enable invite-only so a fresh profile is NOT an approved member.
        // enable_invite_only lives in the e2e harness; here we set the gate
        // directly: point kb_system_settings at a gating team the profile
        // does not belong to.
        let c = claims("seam-gated", "gated@example.test");
        let authed = authenticate(&pool, &c).await.expect("authenticate");
        let id = authed.profile.id;

        sqlx::query(
            "UPDATE kb_system_settings SET access_mode = 'invite_only', \
             gating_team_slug = 'nonexistent-gating-team'",
        )
        .execute(&pool)
        .await
        .expect("enable gate");

        let err = require_system_access(&pool, &authed)
            .await
            .expect_err("gated profile should be refused");
        assert!(
            matches!(err, AuthzError::SystemAccessDenied { profile_id } if profile_id == id),
            "expected SystemAccessDenied, got {err:?}",
        );
    }
```

> **Verify before implementing:** confirm the `kb_system_settings` column names (`access_mode`, `gating_team_slug`) against the live schema — `psql "$DATABASE_URL" -c '\d kb_system_settings'`. The memory `reference_temperkb_admin_gating_bootstrap` records `gating_team_slug` as the admin/gating lever; adjust the UPDATE if the column set differs. If invite-only setup is fiddlier than one UPDATE, mirror the harness helper `enable_invite_only` (`tests/e2e/tests/common/mod.rs:207`) instead.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run -p temper-services --features test-db -E 'test(auth::tests::require_system_access)'`
Expected: FAIL to compile — `require_system_access` / `SystemAuthorized` not defined.

- [ ] **Step 3: Implement Level 2**

Replace the `// require_system_access + SystemAuthorized land in Task 2.` line in `crates/temper-services/src/auth/mod.rs` with:

```rust
/// Proof that a profile passed **both** levels: authenticated *and*
/// system-authorized. Only obtainable from [`require_system_access`], which
/// only accepts an [`AuthenticatedProfile`] — so the type makes it impossible
/// to run Level 2 without having passed Level 1.
pub struct SystemAuthorized(pub AuthenticatedProfile);

/// Level 2 — system authorization. Consumes proof of Level 1, adds the
/// gating-team access gate. Runs on the gated tier of both surfaces.
pub async fn require_system_access(
    pool: &PgPool,
    authed: &AuthenticatedProfile,
) -> Result<SystemAuthorized, AuthzError> {
    let has_access =
        crate::services::access_service::has_system_access(pool, ProfileId::from(authed.profile.id))
            .await
            .map_err(AuthzError::ProfileResolution)?;

    if !has_access {
        return Err(AuthzError::SystemAccessDenied {
            profile_id: authed.profile.id,
        });
    }

    Ok(SystemAuthorized(authed.clone()))
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo nextest run -p temper-services --features test-db -E 'test(auth::tests)'`
Expected: PASS — all four seam tests.

- [ ] **Step 5: `cargo make check` then commit**

```bash
cargo make fix && cargo make check
git add crates/temper-services/src/auth/mod.rs
git commit -m "feat(services): auth seam Level 2 — require_system_access + SystemAuthorized"
```

---

### Task 3: Rewire temper-api `require_auth` → `authenticate`

**Files:**
- Modify: `crates/temper-api/src/middleware/auth.rs:101-110` (the resolve + is_active block)

**Interfaces:**
- Consumes: `temper_services::auth::{authenticate, AuthzError}` (Tasks 1–2).
- Produces: no new public surface; behavior-preserving refactor.

- [ ] **Step 1: Replace the inline resolve + is_active with the seam call**

In `crates/temper-api/src/middleware/auth.rs`, replace this block (currently lines ~101–113):

```rust
    // 5. Resolve (or auto-provision) the profile.
    let profile = profile_service::resolve_from_claims(&state.pool, &claims).await?;

    // 5a. Reject deactivated accounts. This is the authn lever for soft-deleted
    //     profiles — it applies regardless of which auth provider resolved the
    //     claims (OAuth or SAML).
    if !profile.is_active {
        tracing::warn!(profile_id = %profile.id, "rejected: profile is deactivated");
        return Err(ApiError::Unauthorized("account is deactivated".to_string()));
    }

    tracing::Span::current().record("profile_id", tracing::field::display(profile.id));
```

with:

```rust
    // 5. Resolve + deactivation gate via the shared seam (Level 1). The gate
    //    sequence lives once in temper-services::auth; this surface only maps
    //    the refusal to its transport.
    let authed = temper_services::auth::authenticate(&state.pool, &claims)
        .await
        .map_err(|e| match e {
            temper_services::auth::AuthzError::Deactivated { profile_id } => {
                tracing::warn!(%profile_id, "rejected: profile is deactivated");
                ApiError::Unauthorized("account is deactivated".to_string())
            }
            temper_services::auth::AuthzError::ProfileResolution(err) => err,
            // Level 1 never denies system access.
            temper_services::auth::AuthzError::SystemAccessDenied { .. } => {
                ApiError::Internal("unexpected system-access error from authenticate".to_string())
            }
        })?;
    let profile = authed.profile.clone();

    tracing::Span::current().record("profile_id", tracing::field::display(profile.id));
```

Then update the extension insert at the end of the function to reuse `authed` instead of re-wrapping (find the `AuthenticatedProfile { profile, claims }` construction near line 128):

```rust
    // 7. Inject AuthenticatedProfile into extensions.
    request.extensions_mut().insert(authed);
```

Remove the now-unused `use temper_services::services::profile_service;` import if the compiler flags it (clippy `-D warnings` will).

- [ ] **Step 2: Run the temper-api auth-path integration tests**

Run: `cargo nextest run -p temper-api --features test-db --test auth_middleware_test 2>/dev/null || cargo nextest run -p temper-api --features test-db -E 'test(auth)'`
Expected: PASS. (If temper-api has no dedicated auth middleware integration test target, the e2e `auth_test.rs` / `access_gate_test.rs` in Task 6/7 are the guard — note that and move on.)

- [ ] **Step 3: `cargo make check` then commit**

```bash
cargo make fix && cargo make check
git add crates/temper-api/src/middleware/auth.rs
git commit -m "refactor(api): route require_auth through the shared auth seam"
```

---

### Task 4: Rewire temper-api `require_system_access` → seam

**Files:**
- Modify: `crates/temper-api/src/middleware/system_access.rs:38-61`

**Interfaces:**
- Consumes: `temper_services::auth::{require_system_access, AuthzError}`; keeps `access_service::{get_public_settings, get_own_request}` for the surface-side denial payload.
- Produces: behavior-preserving; the `ApiError::SystemAccessRequired` body is byte-identical to today.

- [ ] **Step 1: Replace the inline `has_system_access` check with the seam call**

In `crates/temper-api/src/middleware/system_access.rs`, replace the body from `let has_access =` through the `if !has_access { … }` block (lines ~38–61) with:

```rust
    let authed = request
        .extensions()
        .get::<AuthenticatedProfile>()
        .ok_or_else(|| {
            ApiError::Internal("AuthenticatedProfile not found in request extensions".to_string())
        })?;

    match temper_services::auth::require_system_access(&state.pool, authed).await {
        Ok(_authorized) => {}
        Err(temper_services::auth::AuthzError::SystemAccessDenied { .. }) => {
            // Surface-side presentation: build the CLI-facing details payload.
            let settings = access_service::get_public_settings(&state.pool).await?;
            let own_request =
                access_service::get_own_request(&state.pool, ProfileId::from(authed.profile.id))
                    .await?;
            // SECURITY NOTE: email and display_name are safe to return here because
            // the caller already proved ownership of this identity through OAuth.
            let details = temper_core::types::access_gate::SystemAccessDetails {
                email: authed.profile.email.clone(),
                display_name: Some(authed.profile.display_name.clone()),
                access_mode: settings.access_mode,
                join_request_status: own_request.map(|r| r.status),
                request_url: Some("https://temperkb.io/request-access".to_string()),
                cli_command: Some("temper team join --message \"...\"".to_string()),
            };
            return Err(ApiError::SystemAccessRequired {
                details: Box::new(details),
            });
        }
        Err(temper_services::auth::AuthzError::ProfileResolution(err)) => return Err(err),
        Err(temper_services::auth::AuthzError::Deactivated { .. }) => {
            // require_auth already gated deactivation before this layer runs.
            return Err(ApiError::Unauthorized("account is deactivated".to_string()));
        }
    }
```

The `profile` local (previously bound via `.get::<AuthenticatedProfile>()`) is now `authed`; ensure the earlier binding in the function is renamed/removed so there's one `authed` binding. Keep the existing `use` lines for `access_service`, `ProfileId`, `AuthenticatedProfile`.

- [ ] **Step 2: Run the access-gate integration/e2e guard**

Run: `cargo nextest run -p temper-api --features test-db -E 'test(system_access)' 2>/dev/null; echo "primary guard is e2e access_gate_test in Task 7"`
Expected: no compile errors; primary behavioral guard is `access_gate_test.rs` (Task 7).

- [ ] **Step 3: `cargo make check` then commit**

```bash
cargo make fix && cargo make check
git add crates/temper-api/src/middleware/system_access.rs
git commit -m "refactor(api): route require_system_access through the shared auth seam"
```

---

### Task 5: Rewire temper-mcp `ensure_profile_from_parts` → seam

**Files:**
- Modify: `crates/temper-mcp/src/service.rs:86-140` (`ensure_profile_from_parts`, and `resolve_profile` becomes the AuthClaims builder feeding the seam)

**Interfaces:**
- Consumes: `temper_services::auth::{authenticate, require_system_access, AuthzError}`; keeps `AuthClaims` construction from `McpClaims`.
- Produces: behavior-preserving; the two rmcp terminal-error messages (deactivated / access-required) are byte-identical to today.

- [ ] **Step 1: Replace the inline gate sequence with two seam calls**

In `crates/temper-mcp/src/service.rs`, rewrite `ensure_profile_from_parts` so the middle (resolve → is_active → has_system_access) becomes seam calls. Keep the `AuthClaims` construction currently in `resolve_profile` — inline it or keep `resolve_profile` returning `AuthClaims` (rename to `claims_from(&McpClaims) -> AuthClaims`). New body:

```rust
    pub async fn ensure_profile_from_parts(
        &self,
        parts: &http::request::Parts,
    ) -> Result<(), rmcp::ErrorData> {
        let claims = parts.extensions.get::<McpClaims>().ok_or_else(|| {
            tracing::warn!("McpClaims not found in HTTP request extensions");
            rmcp::ErrorData::internal_error("Not authenticated".to_string(), None)
        })?;

        let auth_claims = self.claims_from(claims);

        // Level 1: resolve + deactivation gate (shared seam).
        let authed = temper_services::auth::authenticate(&self.api_state.pool, &auth_claims)
            .await
            .map_err(map_authz_error)?;

        tracing::debug!(profile_id = %authed.profile.id, sub = %claims.sub, "Profile resolved");

        // Level 2: system-access gate (shared seam).
        temper_services::auth::require_system_access(&self.api_state.pool, &authed)
            .await
            .map_err(map_authz_error)?;

        let mut guard = self.profile.lock().await;
        *guard = Some(authed.profile);
        Ok(())
    }
```

- [ ] **Step 2: Add the `AuthzError` → rmcp mapper (preserving today's exact messages)**

Add a free function in `crates/temper-mcp/src/service.rs` (module scope). The two terminal strings are copied verbatim from the current inline blocks so the wire behavior is unchanged:

```rust
/// Map the shared seam's refusal vocabulary onto rmcp transport errors.
/// The deactivation and access-required strings are terminal ("do not retry")
/// and byte-identical to the pre-seam inline messages.
fn map_authz_error(e: temper_services::auth::AuthzError) -> rmcp::ErrorData {
    use temper_services::auth::AuthzError;
    match e {
        AuthzError::Deactivated { profile_id } => {
            tracing::warn!(%profile_id, "rejected: profile is deactivated");
            rmcp::ErrorData::new(
                rmcp::model::ErrorCode::INVALID_REQUEST,
                "This account has been deactivated. This error is terminal and should not be retried."
                    .to_string(),
                None,
            )
        }
        AuthzError::SystemAccessDenied { .. } => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            "Access to this temper instance requires approval. \
             Visit https://temperkb.io/request-access or run \
             `temper team join` in the CLI to request access. \
             This error is terminal and should not be retried."
                .to_string(),
            None,
        ),
        AuthzError::ProfileResolution(err) => {
            rmcp::ErrorData::internal_error(format!("Failed to resolve profile: {err}"), None)
        }
    }
}
```

Rename `resolve_profile` to `claims_from` returning `AuthClaims` (drop the `profile_service::resolve_from_claims` call — the seam now owns resolve):

```rust
    /// Build normalized `AuthClaims` from MCP JWT claims. MCP tokens may omit
    /// email; the profile service resolves it from cached auth links downstream.
    fn claims_from(&self, claims: &McpClaims) -> temper_core::types::AuthClaims {
        temper_core::types::AuthClaims {
            provider: self.api_state.config.auth_provider_name.clone(),
            external_user_id: claims.sub.clone(),
            email: String::new(),
            email_verified: None,
            exp: claims.exp,
            iat: 0,
        }
    }
```

Remove the now-unused `profile_service` import if clippy flags it.

- [ ] **Step 3: Build + run any temper-mcp tests**

Run: `cargo nextest run -p temper-mcp 2>/dev/null; cargo build -p temper-mcp`
Expected: compiles clean; existing temper-mcp tests pass.

- [ ] **Step 4: `cargo make check` then commit**

```bash
cargo make fix && cargo make check
git add crates/temper-mcp/src/service.rs
git commit -m "refactor(mcp): route profile gating through the shared auth seam"
```

---

### Task 6: Cross-surface parity e2e test

**Files:**
- Create: `tests/e2e/tests/auth_seam_parity_e2e.rs`

**Interfaces:**
- Consumes: `common::{setup, E2eTestApp, generate_test_jwt}` (`tests/e2e/tests/common/mod.rs`); the MCP-service construction pattern from `act_authorship_mcp_e2e.rs`; `temper_mcp::service::TemperMcpService`; `temper_mcp::middleware::McpClaims`.
- Produces: the acceptance test — a deactivated profile AND a no-access profile refused **identically** on both surfaces. This is the test a per-surface gate would have failed.

> **Grounding:** the MCP surface is driven by constructing a `TemperMcpService` over the test pool and calling `ensure_profile_from_parts` with hand-built request `Parts` carrying `McpClaims` — mirror the service-construction block in `act_authorship_mcp_e2e.rs` (`mcp_service(pool)`), which builds `JwksKeyStore::with_static_key` + `ApiConfig` + `AppState`. The API surface is driven over HTTP through the real middleware stack via `app.reqwest_client`. Copy the `ApiConfig`/`AppState` construction verbatim from `act_authorship_mcp_e2e.rs` rather than re-deriving it.

- [ ] **Step 1: Write the parity test**

Create `tests/e2e/tests/auth_seam_parity_e2e.rs`:

```rust
#![cfg(feature = "test-db")]
//! Cross-surface auth parity: the gate a per-surface implementation would have
//! missed. Proves `is_active` (deactivation) and `system_access` are enforced
//! identically on temper-api (HTTP) and temper-mcp (TemperMcpService) — both
//! now routing through the shared temper-services::auth seam.

mod common;

use reqwest::StatusCode;

/// Build request Parts carrying McpClaims for `sub`, to drive the MCP surface's
/// production gate (`ensure_profile_from_parts`).
fn mcp_parts(sub: &str) -> http::request::Parts {
    let claims = temper_mcp::middleware::McpClaims {
        sub: sub.to_string(),
        exp: 0,
    };
    let mut req = http::Request::builder().body(()).expect("build request");
    req.extensions_mut().insert(claims);
    req.into_parts().0
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn deactivated_profile_refused_on_both_surfaces(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Resolve the e2e principal on API (creates the profile), then deactivate it.
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("preflight");
    assert_eq!(resp.status(), StatusCode::OK);

    sqlx::query(
        "UPDATE kb_profiles SET is_active = false WHERE id IN \
         (SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider_user_id = 'e2e-test-user')",
    )
    .execute(&pool)
    .await
    .expect("deactivate");

    // API surface: refused.
    let api = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("api request");
    assert_eq!(api.status(), StatusCode::UNAUTHORIZED, "API must refuse deactivated");

    // MCP surface: refused (terminal rmcp error) through the real service gate.
    let svc = build_mcp_service(&pool).await;
    let err = svc
        .ensure_profile_from_parts(&mcp_parts("e2e-test-user"))
        .await
        .expect_err("MCP must refuse deactivated");
    assert!(
        err.message.contains("deactivated"),
        "MCP deactivation message, got: {}",
        err.message
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn no_system_access_refused_on_both_surfaces(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Preflight to create the profile.
    let profile = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("preflight")
        .json::<serde_json::Value>()
        .await
        .expect("profile json");
    let admin_id: uuid::Uuid = profile["id"].as_str().unwrap().parse().unwrap();

    // Flip to invite-only so the (non-member) e2e principal loses system access.
    common::enable_invite_only(&pool, admin_id).await;

    // A *second* user with no membership: refused on both surfaces.
    let second = common::generate_second_user_jwt();
    let api = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {second}"))
        .send()
        .await
        .expect("api request");
    assert_eq!(api.status(), StatusCode::FORBIDDEN, "API must refuse no-access");

    let svc = build_mcp_service(&pool).await;
    let err = svc
        .ensure_profile_from_parts(&mcp_parts("second-user"))
        .await
        .expect_err("MCP must refuse no-access");
    assert!(
        err.message.contains("requires approval"),
        "MCP access-required message, got: {}",
        err.message
    );
}
```

> **Two things to verify while implementing, both against existing tests:**
> 1. `build_mcp_service(&pool)` — lift the exact `mcp_service` helper from `act_authorship_mcp_e2e.rs` (it constructs `JwksKeyStore::with_static_key` + `ApiConfig` + `AppState` + `TemperMcpService::new`). Copy it into this file (or a shared `common` helper if you prefer — but a local copy matches the existing per-file pattern).
> 2. The `sub` values (`e2e-test-user`, `second-user`) must match what `common::setup` / `generate_second_user_jwt` mint. Confirm `generate_second_user_jwt`'s `sub` in `common/mod.rs:167` and the expected API status for a no-access user (`FORBIDDEN` vs the `SystemAccessRequired` mapping — check `access_gate_test.rs` for the exact status the gate returns) and align the assertion.

- [ ] **Step 2: Run the parity test**

Run: `cargo build -p temper-cli --bin temper && cargo make test-e2e 2>&1 | grep -E "auth_seam_parity|FAIL|PASS|error"`
(Rebuild the CLI bin first — `test-e2e` does not rebuild it, per the `project_e2e_stale_temper_bin` gotcha.)
Expected: both parity tests PASS.

- [ ] **Step 3: Commit**

```bash
cargo make fix && cargo make check
git add tests/e2e/tests/auth_seam_parity_e2e.rs
git commit -m "test(e2e): cross-surface auth parity — is_active + system_access on both surfaces"
```

---

### Task 7: Full verification, SQL cache, and PR

**Files:** none (verification + PR).

- [ ] **Step 1: Regenerate SQL caches only if `check` demands it**

The seam adds no macro queries, so this is usually a no-op. If `cargo make check` reports a missing `.sqlx` entry, run the ritual in order:

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make prepare-api
cargo make prepare-e2e
```

- [ ] **Step 2: Full workspace + e2e run**

```bash
cargo make check
cargo make test          # unit
cargo make test-db       # integration (docker up first)
cargo build -p temper-cli --bin temper && cargo make test-e2e
```

Expected: green. Confirm by exit code / grep `FAIL [` — not the per-binary Summary line (`feedback_nextest_summary_lies`).

- [ ] **Step 3: Push and open the PR**

```bash
git push -u origin jct/auth-seam-spec
gh pr create --title "Auth seam Stage 1: shared authenticate/require_system_access + cross-surface parity test" --body "$(cat <<'EOF'
Extracts the two-level auth gate sequence (resolve+is_active, then system_access)
into `temper-services::auth`, called by both temper-api and temper-mcp. Neither
surface re-implements the ordering; a future gate is one edit, enforced everywhere.

Cross-surface e2e parity test proves `is_active` and `system_access` are enforced
identically on both surfaces — the test the per-surface `is_active` gap (SAML
Phase 2, IMPORTANT-1) would have failed.

Spec: docs/superpowers/specs/2026-07-02-shared-auth-orchestration-seam-design.md
Plan: docs/superpowers/plans/2026-07-02-auth-seam-stage1.md
Stages 2 (docs/auth), 3 (HMAC), 4 (M2M) tracked separately.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review

**Spec coverage (Stage 1 scope):**
- Two-level chain `authenticate` / `require_system_access` → Tasks 1–2. ✓
- Both surfaces route through it → Tasks 3 (api auth), 4 (api system_access), 5 (mcp). ✓
- Single `AuthzError` mapped per transport → Tasks 3–5 mappers. ✓
- Typestate `SystemAuthorized` obtainable only from Level 1 → Task 2. ✓
- Cross-surface parity test at the production caller (both surfaces) → Task 6. ✓
- "Add a gate = one edit per level" — structurally true once Tasks 3–5 land (surfaces only map). ✓
- Stages 2–4 explicitly out of scope, tracked separately → header + PR body. ✓

**Placeholder scan:** No TBD/TODO. Two "verify against live schema / existing test" notes (Task 2 `kb_system_settings` columns, Task 6 `sub` values + status code) are deliberate grounding checks with the exact file/command to run, not deferred work.

**Type consistency:** `AuthzError` variants (`ProfileResolution`/`Deactivated`/`SystemAccessDenied`) identical across Tasks 1,3,4,5. `authenticate` returns `AuthenticatedProfile`; `require_system_access` takes `&AuthenticatedProfile`, returns `SystemAuthorized`. `map_authz_error` (mcp) and the inline match arms (api) cover all three variants. `claims_from` returns `AuthClaims`. Consistent.

**Known verify-points handed to the implementer (not gaps):** exact no-access HTTP status (Task 6 — `FORBIDDEN` vs the `SystemAccessRequired` mapping; confirm in `access_gate_test.rs`); `kb_system_settings` column names (Task 2); whether temper-api has a dedicated auth integration test target (Task 3).
