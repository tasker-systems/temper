# Auth seam Stage 4 (M2M `client_credentials`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an agent (the deployed T6 steward) authenticate *as itself* via OAuth `client_credentials` and be provisioned as its own accountable principal, never proxying a human.

**Architecture:** A thin normalizer in the shared `temper-services::auth` seam owns machine-vs-human claim detection (one pure fn, `normalize_machine`); each surface keeps its own JWKS `decode()`. `AuthClaims` gains a typed `PrincipalKind` discriminant; `resolve_from_claims` branches to a machine path that provisions a dedicated agent profile (link namespace `auth0-m2m`, NULL email) reusing the existing lookup/provisioning machinery. TypeScript 4a advertises the new grant.

**Tech Stack:** Rust (temper-core, temper-services, temper-api, temper-mcp), `jsonwebtoken`, `sqlx` (Postgres), TypeScript/Bun (temper-cloud), e2e crate (`tests/e2e`).

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-07-02-auth-seam-stage-4-m2m-implementation-design.md`. Contract: `docs/auth/machine-token-contract.md`.
- **Scope:** 4a (TS one-liner) + 4b (Rust seam) only. **4c is out of scope** — do not touch `buildAsMetadata` or `handleToken`.
- **Machine detection signal:** `gty == "client-credentials"` — NEVER `azp` presence (human Auth0 tokens carry `azp` too).
- **Client-id source:** `azp` primary; fall back to stripping `@clients` off `sub`.
- **Provider tag (link namespace):** exact string `auth0-m2m` (fits `varchar(32)`).
- **Discriminator:** typed `PrincipalKind` enum — never a stringly-typed provider match in branch logic.
- **Machine email:** written as SQL `NULL`; `AuthClaims.email` stays `String` (Machine branch never reads it).
- **Every `#[sqlx::test]` file** must carry `#![cfg(feature = "test-db")]` at the top or the CI unit-tests job fails.
- **After any `query!` macro change:** regenerate SQL cache — `cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-services` for temper-services test-target queries.
- **Before every commit:** `cargo make check` must pass (fmt + clippy `-D warnings` + docs + machete + TS typecheck + biome).
- `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development` must be exported for `#[sqlx::test]` under bare `cargo`; `cargo make` tasks set it.
- Local e2e uses a **stale prebuilt `temper` bin** — irrelevant here (no CLI change), but rebuild nothing CLI-side.

---

### Task 1: `PrincipalKind` discriminant on `AuthClaims`

Introduces the typed discriminant and updates every construction site to `Human` so the workspace stays green. No behavior change yet — a refactor whose gate is "compiles + existing suites pass."

**Files:**
- Modify: `crates/temper-core/src/types/auth.rs` (add enum + field)
- Modify: `crates/temper-api/src/middleware/auth.rs:91` (add field)
- Modify: `crates/temper-api/src/handlers/internal_saml.rs:22` (add field)
- Modify: `crates/temper-mcp/src/service.rs:63` (add field)
- Modify: `crates/temper-services/src/auth/mod.rs:102` (test helper)
- Modify: `crates/temper-services/src/services/profile_service.rs` (7 test-helper sites: lines ~403, 421, 431, 449, 459, 477, 487)

**Interfaces:**
- Produces: `PrincipalKind { Human, Machine }` and `AuthClaims.principal_kind: PrincipalKind`, both `pub` from `temper_core::types` (re-exported like `AuthClaims`).

- [ ] **Step 1: Add the enum and field**

In `crates/temper-core/src/types/auth.rs`, above `AuthClaims`:

```rust
/// Whether the authenticated principal is a human (interactive OAuth) or a
/// machine (M2M `client_credentials`). The normalizer sets this; the profile
/// resolver branches on it. A typed discriminant — never a stringly-typed
/// provider-string match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalKind {
    Human,
    Machine,
}
```

Add the field as the first member of `AuthClaims`:

```rust
pub struct AuthClaims {
    /// Human (interactive) vs machine (M2M) principal.
    pub principal_kind: PrincipalKind,
    /// Which provider issued this token
    pub provider: String,
    // ... rest unchanged ...
}
```

Confirm `PrincipalKind` is exported wherever `AuthClaims` is (same `pub use` in `crates/temper-core/src/types/mod.rs` — add `PrincipalKind` to the existing `auth::{...}` re-export).

- [ ] **Step 2: Verify it fails to compile (missing field at every site)**

Run: `cargo check -p temper-core -p temper-services -p temper-api -p temper-mcp`
Expected: FAIL — `missing field \`principal_kind\` in initializer of \`AuthClaims\`` at the 8 sites above.

- [ ] **Step 3: Add `principal_kind: PrincipalKind::Human` at every construction site**

Add `principal_kind: temper_core::types::PrincipalKind::Human,` (or the site's local import path) as the first field in each of these initializers:
- `crates/temper-api/src/middleware/auth.rs:91`
- `crates/temper-api/src/handlers/internal_saml.rs:22`
- `crates/temper-mcp/src/service.rs:63`
- `crates/temper-services/src/auth/mod.rs:102` (test helper `claims`)
- `crates/temper-services/src/services/profile_service.rs` — all 7 test-helper `AuthClaims { ... }` blocks.

For the temper-services sites, import via `use temper_core::types::PrincipalKind;` at the top of each module (or fully-qualify).

- [ ] **Step 4: Verify build + existing suites green**

Run: `cargo check --workspace` then `cargo make test`
Expected: PASS (no behavior change).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core crates/temper-api crates/temper-mcp crates/temper-services
git commit -m "feat(auth): add PrincipalKind discriminant to AuthClaims"
```

---

### Task 2: The normalizer — `RawJwtClaims` + `normalize_machine`

The seam's shared claim struct and the single pure detection/normalization function. Pure logic, full TDD.

**Files:**
- Create: `crates/temper-services/src/auth/normalize.rs`
- Modify: `crates/temper-services/src/auth/mod.rs` (add `mod normalize; pub use normalize::*;`)

**Interfaces:**
- Consumes: `temper_core::types::{AuthClaims, PrincipalKind}` (Task 1).
- Produces:
  - `pub const MACHINE_PROVIDER_TAG: &str = "auth0-m2m";`
  - `pub struct RawJwtClaims { pub sub: String, pub email: Option<String>, pub email_verified: Option<bool>, pub azp: Option<String>, pub gty: Option<String>, pub exp: i64, pub iat: i64 }` — `#[derive(Debug, Clone, Deserialize)]`.
  - `pub fn normalize_machine(raw: &RawJwtClaims) -> Option<AuthClaims>` — `Some(machine claims)` iff `gty == "client-credentials"`, else `None`.

- [ ] **Step 1: Write the failing tests**

Create `crates/temper-services/src/auth/normalize.rs` with the module skeleton and tests first:

```rust
//! Shared machine-token claim normalization. The single place that decides
//! whether a decoded JWT is a machine (M2M `client_credentials`) principal and,
//! if so, produces normalized `AuthClaims`. Both surfaces decode into
//! `RawJwtClaims` and call `normalize_machine`; the human branch stays
//! per-surface (email resolution differs by surface).

use serde::Deserialize;

use temper_core::types::{AuthClaims, PrincipalKind};

/// Link-namespace provider tag for Auth0 M2M agent principals. Distinct from the
/// human `auth0` namespace so `(auth0-m2m, client_id)` never collides with a
/// human `(auth0, sub)` under the `UNIQUE(auth_provider, auth_provider_user_id)`
/// constraint.
pub const MACHINE_PROVIDER_TAG: &str = "auth0-m2m";

/// Superset of JWT claims both surfaces decode into. Optional fields absorb the
/// human/machine shape difference in one struct.
#[derive(Debug, Clone, Deserialize)]
pub struct RawJwtClaims {
    pub sub: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub email_verified: Option<bool>,
    /// Authorized party (the client id). Present on Auth0 human AND machine
    /// tokens — NOT a machine signal on its own.
    #[serde(default)]
    pub azp: Option<String>,
    /// Grant-type marker. `client-credentials` is the definitive machine signal.
    #[serde(default)]
    pub gty: Option<String>,
    pub exp: i64,
    #[serde(default)]
    pub iat: i64,
}

/// If `raw` is a machine (`client_credentials`) token, return normalized machine
/// `AuthClaims`; otherwise `None` (caller handles the human branch).
///
/// Detection is on `gty`, never `azp` presence. Client-id source: `azp` primary,
/// `sub` `@clients`-suffix strip as fallback.
pub fn normalize_machine(raw: &RawJwtClaims) -> Option<AuthClaims> {
    if raw.gty.as_deref() != Some("client-credentials") {
        return None;
    }
    let client_id = raw
        .azp
        .clone()
        .or_else(|| raw.sub.strip_suffix("@clients").map(str::to_string))?;
    Some(AuthClaims {
        principal_kind: PrincipalKind::Machine,
        provider: MACHINE_PROVIDER_TAG.to_string(),
        external_user_id: client_id,
        email: String::new(),
        email_verified: None,
        exp: raw.exp,
        iat: raw.iat,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(gty: Option<&str>, azp: Option<&str>, sub: &str, email: Option<&str>) -> RawJwtClaims {
        RawJwtClaims {
            sub: sub.to_string(),
            email: email.map(str::to_string),
            email_verified: None,
            azp: azp.map(str::to_string),
            gty: gty.map(str::to_string),
            exp: 9999,
            iat: 1111,
        }
    }

    #[test]
    fn machine_token_via_azp() {
        let c = normalize_machine(&raw(
            Some("client-credentials"),
            Some("abc123"),
            "abc123@clients",
            None,
        ))
        .expect("should detect machine");
        assert_eq!(c.principal_kind, PrincipalKind::Machine);
        assert_eq!(c.provider, "auth0-m2m");
        assert_eq!(c.external_user_id, "abc123"); // azp preferred
        assert_eq!(c.exp, 9999);
        assert_eq!(c.iat, 1111);
    }

    #[test]
    fn machine_token_sub_strip_fallback_when_azp_absent() {
        let c = normalize_machine(&raw(
            Some("client-credentials"),
            None,
            "abc123@clients",
            None,
        ))
        .expect("should detect machine via sub strip");
        assert_eq!(c.external_user_id, "abc123");
    }

    #[test]
    fn human_token_with_azp_is_not_machine() {
        // The critical guard: a human authorization_code token also carries azp.
        assert!(normalize_machine(&raw(
            Some("authorization_code"),
            Some("abc123"),
            "auth0|user",
            Some("u@example.test"),
        ))
        .is_none());
    }

    #[test]
    fn human_token_without_gty_is_not_machine() {
        assert!(normalize_machine(&raw(None, Some("abc123"), "auth0|user", Some("u@x.test")))
            .is_none());
    }
}
```

Wire it in `crates/temper-services/src/auth/mod.rs` — add near the top:

```rust
mod normalize;
pub use normalize::{normalize_machine, RawJwtClaims, MACHINE_PROVIDER_TAG};
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p temper-services normalize`
Expected: initially FAILs to compile only if the skeleton is incomplete; since Step 1 includes the impl, instead first stub the body as `todo!()` to see red. Practically: temporarily replace the `normalize_machine` body with `{ let _ = raw; todo!() }`, run, expect FAIL (`not yet implemented`), then restore the real body.

- [ ] **Step 3: Restore the real implementation (shown in Step 1)**

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-services normalize`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
cargo make check
git add crates/temper-services/src/auth
git commit -m "feat(auth): shared machine-token normalizer (normalize_machine)"
```

---

### Task 3: `resolve_from_claims` machine branch + agent-profile provisioning

**Files:**
- Modify: `crates/temper-services/src/services/profile_service.rs`

**Interfaces:**
- Consumes: `PrincipalKind` (Task 1), `MACHINE_PROVIDER_TAG` (Task 2), existing `lookup_link_by_provider`, `get_by_id`, `provision_profile_entities`, `generate_profile_handle`.
- Produces: `resolve_from_claims` now branches on `PrincipalKind`; new private `resolve_machine_from_claims` + `create_agent_profile_and_link`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(all(test, feature = "test-db"))]` tests module in `profile_service.rs` (reuse the file's existing `AuthClaims` helper style, setting `principal_kind: PrincipalKind::Machine`):

```rust
fn machine_claims(client_id: &str) -> AuthClaims {
    AuthClaims {
        principal_kind: PrincipalKind::Machine,
        provider: crate::auth::MACHINE_PROVIDER_TAG.to_string(),
        external_user_id: client_id.to_string(),
        email: String::new(),
        email_verified: None,
        exp: 0,
        iat: 0,
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn machine_first_sight_provisions_agent_profile(pool: PgPool) {
    let c = machine_claims("agent-client-xyz");
    let p = resolve_from_claims(&pool, &c).await.expect("provision agent");

    // Link created under the machine namespace with NULL email.
    let link = sqlx::query!(
        "SELECT auth_provider, email FROM kb_profile_auth_links \
         WHERE auth_provider = $1 AND auth_provider_user_id = $2",
        "auth0-m2m",
        "agent-client-xyz",
    )
    .fetch_one(&pool)
    .await
    .expect("link row");
    assert_eq!(link.auth_provider, "auth0-m2m");
    assert!(link.email.is_none(), "machine link email must be NULL");
    assert!(p.is_active);
}

#[sqlx::test(migrations = "../../migrations")]
async fn machine_resolution_is_idempotent(pool: PgPool) {
    let c = machine_claims("agent-idem");
    let a = resolve_from_claims(&pool, &c).await.expect("first");
    let b = resolve_from_claims(&pool, &c).await.expect("second");
    assert_eq!(a.id, b.id, "same agent profile on second sight");

    let n = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM kb_profile_auth_links WHERE auth_provider_user_id = $1",
        "agent-idem",
    )
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(n, Some(1), "exactly one link, no duplicate");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p temper-services --features test-db machine_`
Expected: FAIL — machine claims currently flow through the human path (no `auth0-m2m` link created; email non-NULL).

- [ ] **Step 3: Implement the branch + machine creator**

In `profile_service.rs`, change `resolve_from_claims` to dispatch on kind (keep the existing body as the human arm):

```rust
pub async fn resolve_from_claims(pool: &PgPool, claims: &AuthClaims) -> ApiResult<Profile> {
    match claims.principal_kind {
        PrincipalKind::Human => resolve_human_from_claims(pool, claims).await,
        PrincipalKind::Machine => resolve_machine_from_claims(pool, claims).await,
    }
}

/// Human path: link lookup → email reconcile → new profile. (Body moved verbatim
/// from the former `resolve_from_claims`.)
async fn resolve_human_from_claims(pool: &PgPool, claims: &AuthClaims) -> ApiResult<Profile> {
    if let Some(link) = lookup_link_by_provider(pool, claims).await? {
        return get_by_id(pool, ProfileId::from(link.profile_id)).await;
    }
    if let Some(profile) = reconcile_by_email(pool, claims).await? {
        return Ok(profile);
    }
    let (profile_id, handle) = create_new_profile_and_link(pool, claims).await?;
    provision_profile_entities(pool, profile_id, &handle).await?;
    get_by_id(pool, ProfileId::from(profile_id)).await
}

/// Machine path: link lookup under the `auth0-m2m` namespace → on first sight,
/// provision a dedicated agent profile. NEVER enters email reconciliation.
async fn resolve_machine_from_claims(pool: &PgPool, claims: &AuthClaims) -> ApiResult<Profile> {
    if let Some(link) = lookup_link_by_provider(pool, claims).await? {
        return get_by_id(pool, ProfileId::from(link.profile_id)).await;
    }
    let (profile_id, handle) = create_agent_profile_and_link(pool, claims).await?;
    provision_profile_entities(pool, profile_id, &handle).await?;
    get_by_id(pool, ProfileId::from(profile_id)).await
}

/// Create a brand-new agent profile and its default machine auth link. Email is
/// SQL NULL (a machine has none); display name / handle derive from the client id.
async fn create_agent_profile_and_link(
    pool: &PgPool,
    claims: &AuthClaims,
) -> ApiResult<(Uuid, String)> {
    let display_name = format!("agent-{}", claims.external_user_id);
    let handle = generate_profile_handle(pool, &display_name).await?;
    let profile_id = Uuid::now_v7();

    sqlx::query!(
        r#"
        INSERT INTO kb_profiles (id, handle, display_name, email, preferences)
        VALUES ($1, $2, $3, NULL, '{}')
        "#,
        profile_id,
        &handle,
        &display_name,
    )
    .execute(pool)
    .await?;

    let auth_link_id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO kb_profile_auth_links
            (id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at)
        VALUES ($1, $2, $3, $4, NULL, true, now())
        "#,
        auth_link_id,
        profile_id,
        &claims.provider,
        &claims.external_user_id,
    )
    .execute(pool)
    .await?;

    Ok((profile_id, handle))
}
```

Add `use temper_core::types::PrincipalKind;` to the imports if not already present.

- [ ] **Step 4: Regenerate SQL cache, then run tests**

Run:
```bash
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo nextest run -p temper-services --features test-db profile_service
```
Expected: PASS (new machine tests + all existing human tests).

- [ ] **Step 5: Commit**

```bash
cargo make check
git add crates/temper-services .sqlx
git commit -m "feat(auth): agent-profile provisioning on the machine resolve branch"
```

---

### Task 4: Seam gate rides ordinary rails for machines

Proves a machine principal passes Level 1 (`authenticate`) and Level 2 (`require_system_access`) with no auth-path special-casing.

**Files:**
- Modify: `crates/temper-services/src/auth/mod.rs` (tests module)

**Interfaces:**
- Consumes: `authenticate`, `require_system_access` (unchanged), `MACHINE_PROVIDER_TAG`, `PrincipalKind`.

- [ ] **Step 1: Write the failing test**

Add to the tests module in `auth/mod.rs`:

```rust
fn machine_claims(client_id: &str) -> AuthClaims {
    AuthClaims {
        principal_kind: temper_core::types::PrincipalKind::Machine,
        provider: crate::auth::MACHINE_PROVIDER_TAG.to_string(),
        external_user_id: client_id.to_string(),
        email: String::new(),
        email_verified: None,
        exp: 0,
        iat: 0,
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn machine_principal_rides_ordinary_gate_rails(pool: PgPool) {
    let c = machine_claims("agent-rails");
    let authed = authenticate(&pool, &c).await.expect("authenticate machine");
    assert!(authed.profile.is_active);
    assert_eq!(authed.claims.principal_kind, temper_core::types::PrincipalKind::Machine);
    // Open mode: an authenticated agent has system access, same rail as a human.
    require_system_access(&pool, &authed)
        .await
        .expect("open-mode machine should be system-authorized");
}
```

- [ ] **Step 2: Run to verify it passes** (implementation already exists from Task 3 — this is a characterization test of the seam)

Run: `cargo nextest run -p temper-services --features test-db machine_principal_rides`
Expected: PASS. (If it fails, the Task 3 branch is wrong — fix there.)

- [ ] **Step 3: Commit**

```bash
git add crates/temper-services/src/auth/mod.rs
git commit -m "test(auth): machine principal rides the ordinary gate rails"
```

---

### Task 5: Wire temper-api middleware to the normalizer

**Files:**
- Modify: `crates/temper-api/src/middleware/auth.rs`

**Interfaces:**
- Consumes: `temper_services::auth::{RawJwtClaims, normalize_machine}`, `temper_core::types::PrincipalKind`.
- Replaces the local `JwtClaims` struct with `RawJwtClaims`; `resolve_email_from_claims` signature `&JwtClaims` → `&RawJwtClaims`.

- [ ] **Step 1: Replace the decode target and add the machine branch**

Delete the local `struct JwtClaims { ... }` (lines ~15–23). Change the decode + claims-build block (lines ~80–98) to:

```rust
let token_data: TokenData<temper_services::auth::RawJwtClaims> =
    decode(&token, &vk.key, &validation).map_err(|e| {
        tracing::debug!("JWT verification failed: {e}");
        ApiError::Unauthorized("Invalid or expired token".to_string())
    })?;
let raw = token_data.claims;

let claims = if let Some(machine) = temper_services::auth::normalize_machine(&raw) {
    machine
} else {
    // Human path — resolve email (token → cached link → userinfo), unchanged.
    let (email, email_verified) = resolve_email_from_claims(&state, &raw, &token).await?;
    AuthClaims {
        principal_kind: temper_core::types::PrincipalKind::Human,
        provider: state.config.auth_provider_name.clone(),
        external_user_id: raw.sub.clone(),
        email,
        email_verified,
        exp: raw.exp,
        iat: raw.iat,
    }
};
```

Update `resolve_email_from_claims` signature: `claims: &JwtClaims` → `claims: &temper_services::auth::RawJwtClaims`. Its body already reads `claims.email` (`Option<String>`) and `claims.sub` — both present on `RawJwtClaims`, no body change.

- [ ] **Step 2: Build + run existing api integration tests**

Run: `cargo nextest run -p temper-api --features test-db --test relationship_handler_test` (a representative authed suite) and `cargo check -p temper-api`
Expected: PASS / clean. (Behavioral proof for the machine path is the e2e in Task 8; human-path regression is covered here.)

- [ ] **Step 3: Commit**

```bash
cargo make check
git add crates/temper-api/src/middleware/auth.rs
git commit -m "feat(auth): temper-api middleware normalizes via the shared seam"
```

---

### Task 6: Wire temper-mcp to the normalizer

Replaces `McpClaims` with the shared `RawJwtClaims` so mcp — the surface the steward actually hits — detects machine tokens. Ripples into two e2e helpers that reference `McpClaims`.

**Files:**
- Modify: `crates/temper-mcp/src/middleware.rs` (decode + inject `RawJwtClaims`; delete `McpClaims`)
- Modify: `crates/temper-mcp/src/service.rs` (`claims_from` + the two `parts.extensions.get::<McpClaims>()` sites)
- Modify: `tests/e2e/tests/auth_seam_parity_e2e.rs` (`mcp_parts` injects `RawJwtClaims`)
- Modify: `tests/e2e/tests/act_authorship_mcp_e2e.rs` (same `McpClaims` → `RawJwtClaims` swap)

**Interfaces:**
- Consumes: `temper_services::auth::{RawJwtClaims, normalize_machine}`.
- Produces: mcp injects `RawJwtClaims` into request extensions (was `McpClaims`).

- [ ] **Step 1: middleware — decode into `RawJwtClaims`, drop `McpClaims`**

In `crates/temper-mcp/src/middleware.rs`: delete `pub struct McpClaims { ... }`. Change the decode to `decode::<temper_services::auth::RawJwtClaims>(...)` and inject that. Update the doc comment referencing `McpClaims`.

- [ ] **Step 2: service — normalize in `claims_from`**

In `crates/temper-mcp/src/service.rs`, change `claims_from` and the two extension reads from `McpClaims` to `temper_services::auth::RawJwtClaims`:

```rust
fn claims_from(&self, raw: &temper_services::auth::RawJwtClaims) -> AuthClaims {
    if let Some(machine) = temper_services::auth::normalize_machine(raw) {
        return machine;
    }
    // Human (mcp): email resolved downstream from cached auth links.
    AuthClaims {
        principal_kind: temper_core::types::PrincipalKind::Human,
        provider: self.api_state.config.auth_provider_name.clone(),
        external_user_id: raw.sub.clone(),
        email: String::new(),
        email_verified: None,
        exp: raw.exp,
        iat: raw.iat,
    }
}
```

Update `use crate::middleware::McpClaims;` → import removed; use `temper_services::auth::RawJwtClaims` where the extensions are read (lines ~82, ~591). The `parts.extensions.get::<...>()` type becomes `RawJwtClaims`.

- [ ] **Step 3: Update the two e2e helpers to inject `RawJwtClaims`**

In `tests/e2e/tests/auth_seam_parity_e2e.rs`, `mcp_parts` becomes:

```rust
fn mcp_parts(sub: &str) -> axum::http::request::Parts {
    axum::http::Request::builder()
        .extension(temper_services::auth::RawJwtClaims {
            sub: sub.to_string(),
            email: None,
            email_verified: None,
            azp: None,
            gty: None,
            exp: 0,
            iat: 0,
        })
        .body(())
        .expect("build request")
        .into_parts()
        .0
}
```

Apply the identical `McpClaims { sub, exp }` → `RawJwtClaims { .. }` swap in `tests/e2e/tests/act_authorship_mcp_e2e.rs`.

- [ ] **Step 4: Build + run mcp and existing e2e parity**

Run:
```bash
cargo check -p temper-mcp
cargo nextest run -p temper-e2e --features test-db --test auth_seam_parity_e2e
```
Expected: PASS (human parity unchanged through the new injected type).

- [ ] **Step 5: Commit**

```bash
cargo make check
git add crates/temper-mcp tests/e2e
git commit -m "feat(auth): temper-mcp normalizes via the shared seam (RawJwtClaims)"
```

---

### Task 7: 4a — advertise `client_credentials` (TypeScript)

**Files:**
- Modify: `packages/temper-cloud/src/oauth/metadata.ts` (`buildAuth0AsMetadata` only)
- Test: `packages/temper-cloud/src/oauth/metadata.test.ts` (create if absent, else extend)

**Interfaces:**
- Produces: `buildAuth0AsMetadata(...).grant_types_supported` includes `"client_credentials"`.

- [ ] **Step 1: Write the failing test**

In `packages/temper-cloud/src/oauth/metadata.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { buildAuth0AsMetadata } from "./metadata.js";

describe("buildAuth0AsMetadata", () => {
  it("advertises client_credentials for M2M agent principals", () => {
    const md = buildAuth0AsMetadata({
      base: "https://mcp.example.com",
      auth0Domain: "https://t.auth0.com/",
      mcpAudience: "https://api.example.com",
    });
    expect(md.grant_types_supported).toContain("client_credentials");
    // Existing grants remain.
    expect(md.grant_types_supported).toContain("authorization_code");
    expect(md.grant_types_supported).toContain("refresh_token");
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd packages/temper-cloud && bun run test metadata`
Expected: FAIL — `client_credentials` not present.

- [ ] **Step 3: Add the grant (one line)**

In `metadata.ts`, `buildAuth0AsMetadata` return:

```ts
    grant_types_supported: ["authorization_code", "refresh_token", "client_credentials"],
```

Leave `buildAsMetadata` (the Temper AS branch) untouched — that is 4c.

- [ ] **Step 4: Run to verify it passes**

Run: `cd packages/temper-cloud && bun run test metadata`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cd packages/temper-cloud && bun run check && cd -
git add packages/temper-cloud/src/oauth/metadata.ts packages/temper-cloud/src/oauth/metadata.test.ts
git commit -m "feat(auth): advertise client_credentials grant (Stage 4a)"
```

---

### Task 8: e2e — machine token provisions an agent profile through the mcp gate

Drives the production caller (`ensure_profile_from_parts`) with a machine-shaped `RawJwtClaims` and asserts agent-profile provisioning end-to-end.

**Files:**
- Create: `tests/e2e/tests/auth_seam_m2m_e2e.rs`

**Interfaces:**
- Consumes: `common::setup`, `TemperMcpService::{new, ensure_profile_from_parts}`, `temper_services::auth::RawJwtClaims`, the `build_mcp_service` pattern from `auth_seam_parity_e2e.rs`.

- [ ] **Step 1: Write the failing test**

```rust
#![cfg(feature = "test-db")]
//! Stage 4b: a machine (`client_credentials`) token, driven through the real mcp
//! gate `ensure_profile_from_parts`, provisions a dedicated agent profile under
//! the `auth0-m2m` link namespace with a NULL email — never the email-reconcile
//! path.

mod common;

use temper_services::config::ApiConfig;
use temper_services::state::{AppState, JwksKeyStore};

async fn build_mcp_service(pool: &sqlx::PgPool) -> temper_mcp::service::TemperMcpService {
    let decoding_key =
        jsonwebtoken::DecodingKey::from_rsa_pem(include_bytes!("fixtures/test_rsa.pub"))
            .expect("decoding key");
    let jwks_store = JwksKeyStore::with_static_key(decoding_key, jsonwebtoken::Algorithm::RS256);
    let api_config = ApiConfig {
        database_url: "unused".to_string(),
        jwks_url: "unused".to_string(),
        auth_issuer: "test-issuer".to_string(),
        auth_audience: None,
        auth_provider_name: "test-provider".to_string(),
        cors_origins: vec![],
        port: 0,
        enable_swagger: false,
        internal_reconcile_secret: None,
    };
    let state = AppState::new(pool.clone(), jwks_store, api_config);
    temper_mcp::service::TemperMcpService::new(state)
}

fn machine_parts(client_id: &str) -> axum::http::request::Parts {
    axum::http::Request::builder()
        .extension(temper_services::auth::RawJwtClaims {
            sub: format!("{client_id}@clients"),
            email: None,
            email_verified: None,
            azp: Some(client_id.to_string()),
            gty: Some("client-credentials".to_string()),
            exp: 0,
            iat: 0,
        })
        .body(())
        .expect("build request")
        .into_parts()
        .0
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn machine_token_provisions_agent_profile_via_mcp(pool: sqlx::PgPool) {
    let _app = common::setup(pool.clone()).await;
    let svc = build_mcp_service(&pool).await;

    svc.ensure_profile_from_parts(&machine_parts("steward-client-1"))
        .await
        .expect("mcp gate must admit + provision the agent");

    let link = sqlx::query!(
        "SELECT auth_provider, email FROM kb_profile_auth_links \
         WHERE auth_provider = $1 AND auth_provider_user_id = $2",
        "auth0-m2m",
        "steward-client-1",
    )
    .fetch_one(&pool)
    .await
    .expect("agent link row exists");
    assert_eq!(link.auth_provider, "auth0-m2m");
    assert!(link.email.is_none(), "agent link email must be NULL");
}
```

- [ ] **Step 2: Regenerate e2e SQL cache, run the test**

Run:
```bash
cargo make prepare-e2e
cargo nextest run -p temper-e2e --features test-db --test auth_seam_m2m_e2e
```
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
cargo make check
git add tests/e2e
git commit -m "test(auth): e2e machine token provisions an agent profile via mcp"
```

---

## Final verification

- [ ] `cargo make check` — clean.
- [ ] `cargo make test` — unit suites green.
- [ ] `cargo make test-e2e` — e2e green (includes the new m2m test).
- [ ] `cd packages/temper-cloud && bun run test && bun run typecheck` — TS green.
- [ ] Confirm no `.sqlx` orphans introduced; caches regenerated (`--workspace`, `prepare-services`, `prepare-e2e`).
- [ ] Update the design doc's Follow-ups status if any operator step was completed in-session.

## Operator follow-ups (documented, not in this plan's code)

- Provision the Auth0 M2M application authorized for `MCP_AUDIENCE`.
- Validate `normalize_machine` against a real Auth0 M2M token (`azp`/`gty`/`sub` assumptions).
- Grant the agent client-id profile team membership + `cogmap grant --write` on temperkb.io.
