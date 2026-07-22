# Admin-authz Enclosure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the discipline-based `require_system_admin` calls with a sealed `SystemAdmin` type-state proof, threaded as a required `&SystemAdmin` param through every pure-admin service fn, so calling an admin action without being admin is a compile error and both surfaces (api + future mcp) inherit the gate by construction.

**Architecture:** Extend the existing `AuthenticatedProfile → SystemAuthorized` type-state ladder in `temper-services/src/auth/mod.rs` with a Level-3 `SystemAdmin` proof (private field, minted only by `require_system_admin`). Migrate the pure-admin service fns to take `&SystemAdmin` (dropping their `actor: ProfileId` param and internal gate); surface handlers mint the proof once and pass it. Prove the seal with `trybuild`. Compositional and conditional gates keep the raw `is_system_admin` bool — untouched.

**Tech Stack:** Rust, axum (temper-api), sqlx (unchanged — no SQL/schema change), cargo-nextest, trybuild (new dev-dependency in temper-services).

Design spec: `docs/superpowers/specs/2026-07-22-admin-authz-enclosure-design.md`. Read §3 (design), the "What gets the proof" section, and the "Authz-site inventory" before starting.

## Global Constraints

- **F-3 — authz lives in the service, never the handler.** A handler-side `is_system_admin` call reds the `audit-handler-authz-drift` tripwire. The proof is minted in the handler but the *requirement* is the service-fn signature.
- **Additive, no DB migration, no wire-contract change.** The `/api/access/admin/*` surface is OpenAPI-excluded; no `cargo make openapi` regen. No `query!`/SQL changes → **no sqlx cache regen** (signature changes rebind the same values into the same query strings).
- **Behavioral parity is mandatory.** Same authorization outcomes (admin required → `Forbidden`); the existing admin suites must stay green unchanged.
- **Compositional (Bucket 2) and conditional (Bucket 3) gates are OUT of scope** — do not touch `machine_authz`, `can_administer_grant`, `admin_ledger_service`, `team_service::create`'s `auto_join_role`, or any `is_system_admin` OR-branch. See the spec's audit inventory.
- **The `is_system_admin(pool, id) -> ApiResult<bool>` predicate stays** — the gate and every Bucket-2/3 site use it.
- **Run before committing any task:** `cargo make check` (includes the security tripwires). DB is on port 5437; `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development` for `#[sqlx::test]` under bare cargo.

---

### Task 1: The sealed `SystemAdmin` proof + `require_system_admin` gate; seal `SystemAuthorized`

**Files:**
- Modify: `crates/temper-services/src/auth/mod.rs` (add `SystemAdmin` + `require_system_admin` beside `SystemAuthorized:263`; seal `SystemAuthorized`'s field)
- Test: `crates/temper-services/tests/system_admin_proof_test.rs` (new)

**Interfaces:**
- Produces: `temper_services::auth::SystemAdmin` (opaque; `fn actor(&self) -> ProfileId`); `temper_services::auth::require_system_admin(pool: &PgPool, authed: &AuthenticatedProfile) -> ApiResult<SystemAdmin>`.
- Consumes: `access_service::is_system_admin(pool, ProfileId) -> ApiResult<bool>` (exists, `access_service.rs:47`); `AuthenticatedProfile` (`temper-core`, field `profile.id: Uuid`); `ApiError::Forbidden`, `ApiResult` (`crate::error`).

- [ ] **Step 1: Write the failing test**

Create `crates/temper-services/tests/system_admin_proof_test.rs`:

```rust
#![cfg(feature = "test-db")]
//! The Level-3 `SystemAdmin` proof (spec §3.1): minted only by `require_system_admin`, which checks
//! governance. A non-admin cannot obtain one; the actor it carries is the caller.

use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_services::auth::{require_system_admin, AuthenticatedProfile};
use temper_services::error::ApiError;
use temper_services::test_support;

// Build an AuthenticatedProfile for a seeded profile id — the auth path's Level-1 output.
async fn authed(pool: &PgPool, handle: &str) -> AuthenticatedProfile {
    let id: uuid::Uuid =
        sqlx::query_scalar("INSERT INTO kb_profiles (handle, display_name) VALUES ($1,$1) RETURNING id")
            .bind(handle)
            .fetch_one(pool)
            .await
            .unwrap();
    test_support::authenticated_profile_for(pool, id).await // helper added in Step 3
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn admin_gets_a_proof_carrying_its_actor(pool: PgPool) {
    let a = authed(&pool, "admin").await;
    let id = ProfileId::from(a.profile.id);
    test_support::grant_governance(&pool, a.profile.id).await;

    let proof = require_system_admin(&pool, &a).await.expect("admin mints a proof");
    assert_eq!(proof.actor(), id, "the proof carries the acting admin");
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn non_admin_is_refused(pool: PgPool) {
    let a = authed(&pool, "not-admin").await;
    let err = require_system_admin(&pool, &a).await.expect_err("a non-admin cannot mint one");
    assert!(matches!(err, ApiError::Forbidden));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-services --features test-db --test system_admin_proof_test`
Expected: FAIL to compile — `require_system_admin` and `test_support::authenticated_profile_for` do not exist yet.

- [ ] **Step 3: Add the test-support helper**

In `crates/temper-services/src/test_support.rs`, append (it needs `AuthClaims`; build a minimal one — the proof only reads `profile.id`):

```rust
use temper_core::types::auth::{AuthClaims, AuthenticatedProfile};
use crate::services::profile_service;

/// Load a real `AuthenticatedProfile` for a seeded profile id — for tests that exercise the auth
/// ladder directly. Minimal claims: the proofs downstream only read `profile.id`.
pub async fn authenticated_profile_for(pool: &PgPool, profile_id: Uuid) -> AuthenticatedProfile {
    let profile = profile_service::get_profile_by_id(pool, profile_id)
        .await
        .expect("load profile")
        .expect("profile exists");
    AuthenticatedProfile {
        profile,
        claims: AuthClaims {
            sub: format!("test|{profile_id}"),
            email: None,
            email_verified: None,
            exp: 0,
            iat: 0,
        },
    }
}
```

> ⚠️ Grounding note: verify `profile_service::get_profile_by_id`'s exact name/signature and `AuthClaims`'s exact fields on disk (GD-2) — the field list above mirrors `auth.rs:63` but re-confirm before relying on it. If a loader by id doesn't exist, load the row inline with `sqlx::query_as!`.

- [ ] **Step 4: Add the type and gate**

In `crates/temper-services/src/auth/mod.rs`, after `SystemAuthorized` (`:263`), add:

```rust
/// Proof the caller is a system admin (D10 governance / spec §3). SEALED: the private field means the
/// only way to hold one is `require_system_admin`, which checks the DB. Forging one is a compile error
/// outside this module.
#[derive(Debug)]
pub struct SystemAdmin(ProfileId);

impl SystemAdmin {
    /// The acting admin — recorded as `actor` on every governance/standing/ledger write.
    pub fn actor(&self) -> ProfileId {
        self.0
    }
}

/// Level 3 — governance check. Consumes proof of Level 1 (like `require_system_access`), returns a
/// plain `Forbidden` on denial: admin denial needs none of `AuthzError::SystemAccessDenied`'s
/// CLI-presentation payload, and `Forbidden` is exactly what the admin gate returns today.
pub async fn require_system_admin(
    pool: &PgPool,
    authed: &AuthenticatedProfile,
) -> crate::error::ApiResult<SystemAdmin> {
    let actor = ProfileId::from(authed.profile.id);
    if crate::services::access_service::is_system_admin(pool, actor).await? {
        Ok(SystemAdmin(actor))
    } else {
        Err(crate::error::ApiError::Forbidden)
    }
}
```

Then **seal Level 2**: change `pub struct SystemAuthorized(pub AuthenticatedProfile);` to a private field with an accessor:

```rust
#[derive(Debug)]
pub struct SystemAuthorized(AuthenticatedProfile);

impl SystemAuthorized {
    /// The authenticated identity this proof was minted for.
    pub fn authenticated(&self) -> &AuthenticatedProfile {
        &self.0
    }
}
```

> ⚠️ Grounding note: `grep -rn "SystemAuthorized(" crates/` and `grep -rn "\.0" ` around `SystemAuthorized` uses. `require_system_access:284` constructs it (same module — fine). Any *external* `.0` access on a `SystemAuthorized` becomes `.authenticated()`. From the earlier audit, `require_system_access`'s result is discarded at call sites (`system_access.rs:38` matches `Ok(_)`), so expect zero external field reads — but confirm before compiling.

- [ ] **Step 5: Run tests to verify they pass**

Run: `DATABASE_URL=… cargo nextest run -p temper-services --features test-db --test system_admin_proof_test`
Expected: PASS (both tests).

- [ ] **Step 6: `cargo make check` then commit**

```bash
cargo make check
git add crates/temper-services/src/auth/mod.rs crates/temper-services/src/test_support.rs crates/temper-services/tests/system_admin_proof_test.rs
git commit -m "feat(auth): sealed SystemAdmin proof + require_system_admin gate; seal SystemAuthorized (Task 1)"
```

---

### Task 2: Migrate the service-gated admin acts to `&SystemAdmin`

The five acts in `access_service.rs` that already gate via the internal `require_system_admin` helper. Drop the `actor: ProfileId` param and the internal check; take `admin: &SystemAdmin`; read `admin.actor()`. Update their temper-api handlers to mint the proof.

**Files:**
- Modify: `crates/temper-services/src/services/access_service.rs` (`admin_approve:716`, `admin_revoke:733`, `admin_deactivate:754`, `admin_reactivate:775`, `demote_admin:~794`; and the private `require_system_admin` helper `:705`)
- Modify: `crates/temper-api/src/handlers/access.rs` (`approve_principal:296`, `revoke_principal:311`, `deactivate_principal:328`, `reactivate_principal:343`, `demote_admin:~276`)
- Test: existing `crates/temper-services/tests/admin_demotion_test.rs` + `crates/temper-api`/e2e suites (parity)

**Interfaces:**
- Consumes: `SystemAdmin`, `require_system_admin` (Task 1).
- Produces: `admin_approve(pool, admin: &SystemAdmin, subject: ProfileId)`, `admin_revoke(pool, admin: &SystemAdmin, subject: ProfileId, reason: String)`, `admin_deactivate(pool, admin: &SystemAdmin, subject: ProfileId)`, `admin_reactivate(pool, admin: &SystemAdmin, subject: ProfileId)`, `demote_admin(pool, admin: &SystemAdmin, subject: ProfileId)` — all `-> ApiResult<()>`.

- [ ] **Step 1: Update the failing test first (it encodes the new signature)**

In `crates/temper-services/tests/admin_demotion_test.rs`, the calls currently pass `ProfileId::from(actor)`. Change each `access_service::admin_revoke(&pool, ProfileId::from(subject), ProfileId::from(actor), "test".into())` to mint a proof and pass it:

```rust
// helper near the top of the test module:
async fn admin_proof(pool: &PgPool, admin_id: uuid::Uuid) -> temper_services::auth::SystemAdmin {
    let a = temper_services::test_support::authenticated_profile_for(pool, admin_id).await;
    temper_services::auth::require_system_admin(pool, &a).await.expect("admin proof")
}

// call site becomes:
let proof = admin_proof(&pool, actor).await;
access_service::admin_revoke(&pool, &proof, ProfileId::from(subject), "test".into()).await.unwrap();
```

Apply the same shape to the `admin_deactivate` / `admin_reactivate` / `demote_admin` calls in that file. The `demote_admin_requires_the_caller_be_admin` test (non-admin actor → Forbidden) now asserts at the **proof-minting** step: `require_system_admin(&pool, &non_admin_authed).await.expect_err(...)` returns `ApiError::Forbidden`, and `demote_admin` is never reached.

- [ ] **Step 2: Run to verify it fails**

Run: `DATABASE_URL=… cargo nextest run -p temper-services --features test-db --test admin_demotion_test`
Expected: FAIL to compile — the service fns still take `actor: ProfileId`.

- [ ] **Step 3: Migrate the service fns**

In `access_service.rs`, for each of `admin_approve/revoke/deactivate/reactivate`: replace the `actor: ProfileId` param with `admin: &SystemAdmin`, delete the `require_system_admin(pool, actor).await?;` line, and replace `actor` with `admin.actor()` inside. Example (`admin_revoke`):

```rust
pub async fn admin_revoke(
    pool: &PgPool,
    admin: &SystemAdmin,
    subject: ProfileId,
    reason: String,
) -> ApiResult<()> {
    standing_service::apply(
        pool,
        ApplyStandingParams {
            subject,
            act: Act::Revoke { reason },
            actor: Some(admin.actor()),
            authority: ActorAuthority::Admin,
        },
    )
    .await?;
    Ok(())
}
```

`admin_approve`, `admin_deactivate`, `admin_reactivate` follow identically (their `act:` and params differ; only the signature + `actor: Some(admin.actor())` change). For `demote_admin`:

```rust
pub async fn demote_admin(pool: &PgPool, admin: &SystemAdmin, subject: ProfileId) -> ApiResult<()> {
    sqlx::query_scalar!(
        "SELECT principal_governance_set($1, false, $2, 'system admin demotion')",
        *subject,
        *admin.actor(),
    )
    .fetch_one(pool)
    .await?;
    Ok(())
}
```

Add `use crate::auth::SystemAdmin;` to `access_service.rs`. **Delete** the now-unused private `require_system_admin` helper (`:705`). Confirm nothing else calls it: `grep -rn "require_system_admin" crates/temper-services/src/services/`.

- [ ] **Step 4: Update the handlers to mint**

In `crates/temper-api/src/handlers/access.rs`, each standing-act handler currently passes `ProfileId::from(auth.0.profile.id)` as actor. Mint the proof and pass it. Example (`revoke_principal`):

```rust
pub async fn revoke_principal(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(profile_id): Path<Uuid>,
    Json(body): Json<RevokePrincipalBody>,
) -> ApiResult<StatusCode> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    access_service::admin_revoke(&state.pool, &admin, ProfileId::from(profile_id), body.reason).await?;
    Ok(StatusCode::OK)
}
```

Apply identically to `approve_principal`, `deactivate_principal`, `reactivate_principal`, and the `demote_admin` handler (which becomes `require_system_admin(...)?` then `access_service::demote_admin(&state.pool, &admin, ProfileId::from(body.profile_id))`).

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo nextest run -p temper-services --features test-db --test admin_demotion_test
cargo nextest run -p temper-api --features test-db --test admin_settings_test   # if present; else skip
cargo make test-e2e   # admin_surface_e2e must stay green (parity)
```
Expected: PASS. The e2e `non_admin_is_forbidden_on_all_admin_endpoints` still returns 403 for every act.

- [ ] **Step 6: `cargo make check` then commit**

```bash
cargo make check
git add crates/temper-services/src/services/access_service.rs crates/temper-api/src/handlers/access.rs crates/temper-services/tests/admin_demotion_test.rs
git commit -m "refactor(access): service-gated admin acts take &SystemAdmin (Task 2)"
```

---

### Task 3: Migrate the handler-gated admin endpoints to `&SystemAdmin`; remove inline handler checks

These endpoints check `is_system_admin` **in the handler** today. Move the requirement into the service by adding a `&SystemAdmin` param; the handler mints the proof and drops the inline check. For the **shared** settings reader, add an admin-specific wrapper (spec §3.4) — do NOT gate `get_system_settings` (the public path uses it).

**Files:**
- Modify: `crates/temper-services/src/services/access_service.rs` (`promote_admin:580`, `update_system_settings:~500`, `list_pending_requests`, `review_request`; add `admin_get_settings` wrapper)
- Modify: `crates/temper-api/src/handlers/access.rs` (`list_pending:179`, `review_request:195`, `get_admin_settings:223`, `update_settings:238`, `promote_admin:256`)
- Modify: `crates/temper-api/src/handlers/embed.rs` (`reembed:~183` handler) + its embed service fn
- Test: existing e2e `admin_surface_e2e` (promote path + non-admin 403s) — parity

**Interfaces:**
- Produces: `promote_admin(pool, admin: &SystemAdmin, profile_id: Uuid, team_id: Option<Uuid>) -> ApiResult<TeamMemberRow>`; `admin_get_settings(pool, _admin: &SystemAdmin) -> ApiResult<SystemSettings>`; `update_system_settings(pool, _admin: &SystemAdmin, body: &UpdateSettingsRequest) -> ApiResult<SystemSettings>`; `list_pending_requests(pool, _admin: &SystemAdmin) -> ApiResult<Vec<JoinRequestWithProfile>>`; `review_request(pool, admin: &SystemAdmin, params: ReviewRequestParams)` with the reviewer taken from `admin.actor()`.

- [ ] **Step 1: Update/confirm the parity tests encode the behavior**

No new test needed — `admin_surface_e2e::{admin_can_set_settings_and_promote_second_admin, non_admin_is_forbidden_on_all_admin_endpoints}` already exercise these endpoints end-to-end and must stay green. If any temper-api unit test calls these service fns directly, update its call to mint a proof (as in Task 2 Step 1). Run them first to capture the green baseline.

- [ ] **Step 2: Migrate `promote_admin`**

In `access_service.rs`, `promote_admin` currently takes `actor: Option<ProfileId>`. Change to require the proof and derive the actor from it:

```rust
pub async fn promote_admin(
    pool: &PgPool,
    admin: &SystemAdmin,
    profile_id: Uuid,
    team_id: Option<Uuid>,
) -> ApiResult<TeamMemberRow> {
    // ... body unchanged, except every `actor.map(|a| *a)` becomes `Some(*admin.actor())`
    //     and `actor` in the governance/standing writes becomes `admin.actor()`.
}
```

Update the two write sites inside (the `principal_governance_set($1, true, $2, …)` and `principal_standing_apply(…, $2, …)` binds) from `actor.map(|a| *a)` to `Some(*admin.actor())`.

- [ ] **Step 3: Add the settings wrapper + gate update/list/review**

In `access_service.rs`:

```rust
/// Admin-authority read of the FULL settings (spec §3.4). The shared `get_system_settings` reader
/// stays ungated (the public route + internal callers use it); the *admin act* gets its own gated entry.
pub async fn admin_get_settings(pool: &PgPool, _admin: &SystemAdmin) -> ApiResult<SystemSettings> {
    get_system_settings(pool).await
}
```

Add `_admin: &SystemAdmin` as the first-after-`pool` param to `update_system_settings` and `list_pending_requests` (bodies unchanged — the param is the capability). For `review_request`, add `admin: &SystemAdmin` and take the reviewer from it — replace `ReviewRequestParams { reviewer_profile_id, .. }`'s source: the handler stops passing `reviewer_profile_id`; the service sets it from `admin.actor()`. Adjust `ReviewRequestParams` construction accordingly (keep the struct; populate `reviewer_profile_id: admin.actor()` inside `review_request`).

> ⚠️ Grounding note: confirm `review_request`'s current signature and `ReviewRequestParams`'s fields (`access_service.rs` + `handlers/access.rs:207`). If the reviewer is threaded as a struct field, set it from `admin.actor()` inside the service; do not keep a separate handler-supplied reviewer.

- [ ] **Step 4: Update the handlers — mint, drop the inline `is_system_admin`**

In `handlers/access.rs`, for `list_pending`, `review_request`, `get_admin_settings`, `update_settings`, `promote_admin`: delete the `let is_admin = …is_system_admin…; if !is_admin { return Err(ApiError::Forbidden); }` block and replace with a mint. Example (`get_admin_settings`):

```rust
pub async fn get_admin_settings(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<SystemSettings>> {
    let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
    access_service::admin_get_settings(&state.pool, &admin).await.map(Json)
}
```

`promote_admin` handler: mint, then `access_service::promote_admin(&state.pool, &admin, body.profile_id, body.team_id)`. Do the same for `reembed` in `handlers/embed.rs` (`:195` inline check → mint; its embed service fn gains `_admin: &SystemAdmin`).

- [ ] **Step 5: Run the parity suites**

```bash
cargo make test-e2e   # admin_surface_e2e green: promote still works, non-admins still 403
cargo make check      # audit-handler-authz-drift must PASS — no handler-side is_system_admin remains
```
Expected: PASS. Critically, `cargo make check`'s `audit-handler-authz-drift` tripwire should now find **fewer** handler `is_system_admin` sites (these are gone).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-services/src/services/access_service.rs crates/temper-api/src/handlers/access.rs crates/temper-api/src/handlers/embed.rs
git commit -m "refactor(access): handler-gated admin endpoints take &SystemAdmin; drop inline checks (Task 3)"
```

---

### Task 4: Enclose `machine_registration::rebind` (the audit-found pure-admin site)

**Files:**
- Modify: `crates/temper-services/src/services/machine_registration_service.rs` (`rebind:392`)
- Modify: `crates/temper-api/src/handlers/machine_clients.rs` (the `rebind` handler)
- Test: existing rebind test(s) in `machine_registration_service.rs` (`:636` region) + any e2e

**Interfaces:**
- Produces: `machine_registration::rebind(pool, admin: &SystemAdmin, req: &RebindMachineRequest) -> ApiResult<MachineClient>` (drops `caller: ProfileId`).

- [ ] **Step 1: Update the rebind test to the new signature**

In `machine_registration_service.rs`'s test module (around `:636`), the rebind test calls `rebind(pool, caller, &req)`. Change it to mint a proof for an admin caller and pass it:

```rust
let admin = {
    let a = crate::test_support::authenticated_profile_for(&pool, admin_id).await;
    crate::auth::require_system_admin(&pool, &a).await.expect("admin proof")
};
let out = rebind(&pool, &admin, &req).await;
```

If a test asserted "a plain team owner is refused" by calling `rebind` with a non-admin caller and expecting `Forbidden`, that assertion now moves to the **mint**: `require_system_admin(&pool, &owner_authed).await` returns `Err(ApiError::Forbidden)`, and `rebind` is never called.

- [ ] **Step 2: Run to verify it fails**

Run: `DATABASE_URL=… cargo nextest run -p temper-services --features test-db --test <the rebind test target>` (or the in-crate `#[cfg(test)]` module via `cargo nextest run -p temper-services --features test-db -E 'test(rebind)'`)
Expected: FAIL to compile — `rebind` still takes `caller: ProfileId`.

- [ ] **Step 3: Migrate `rebind`**

```rust
pub async fn rebind(
    pool: &PgPool,
    admin: &SystemAdmin,
    req: &RebindMachineRequest,
) -> ApiResult<MachineClient> {
    let old = machine_client_service::get(pool, req.from_machine_client_id).await?;

    // Auth before writes. Admin-only (see the fn doc): team ownership cannot bound the reach a rebind
    // inherits — which is why this takes a SystemAdmin proof, NOT machine_authz. Do not widen it.
    // (The proof itself IS the check; the old inline `is_system_admin` is gone.)

    // ... rest of the body unchanged (revoked-source guard, writes) ...
}
```

Delete the `if !access_service::is_system_admin(pool, caller).await? { return Err(ApiError::Forbidden); }` block. Add `use crate::auth::SystemAdmin;`. Anywhere the body used `caller` (e.g. as an actor on a write), use `admin.actor()`.

> ⚠️ Grounding note: check whether `rebind`'s body uses `caller` beyond the gate (e.g. recording who rebound). If so, thread `admin.actor()` there. `grep -n "caller" crates/temper-services/src/services/machine_registration_service.rs` within the fn.

- [ ] **Step 4: Update the handler to mint**

In `handlers/machine_clients.rs`, the `rebind` handler mints and passes the proof:

```rust
let admin = temper_services::auth::require_system_admin(&state.pool, &auth.0).await?;
machine_registration::rebind(&state.pool, &admin, &body).await.map(Json)
```

Remove any handler-side `is_system_admin` for rebind (grep the handler file).

- [ ] **Step 5: Run tests + check**

```bash
cargo nextest run -p temper-services --features test-db -E 'test(rebind)'
cargo make check
```
Expected: PASS; `audit-handler-authz-drift` green.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-services/src/services/machine_registration_service.rs crates/temper-api/src/handlers/machine_clients.rs
git commit -m "refactor(machine): rebind takes &SystemAdmin (audit-found pure-admin site) (Task 4)"
```

---

### Task 5: Prove the seal with `trybuild`

A compile-fail fixture: `SystemAdmin` cannot be constructed outside `auth`, and a gated fn cannot be called without one. This is the guarantee, demonstrated.

**Files:**
- Modify: `crates/temper-services/Cargo.toml` (add `trybuild` under `[dev-dependencies]`)
- Create: `crates/temper-services/tests/compile_fail.rs`
- Create: `crates/temper-services/tests/compile_fail/forge_system_admin.rs`
- Create: `crates/temper-services/tests/compile_fail/call_admin_fn_without_proof.rs`

- [ ] **Step 1: Add the trybuild harness**

`crates/temper-services/tests/compile_fail.rs`:

```rust
//! Proof of the enclosure: the sealed SystemAdmin proof (spec §3.1, Testing) — these MUST NOT compile.
#[test]
fn system_admin_is_unforgeable_and_gates_admin_fns() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
```

Add to `Cargo.toml`:

```toml
[dev-dependencies]
trybuild = "1"
```

- [ ] **Step 2: Write the two must-not-compile fixtures**

`tests/compile_fail/forge_system_admin.rs`:

```rust
// SystemAdmin has a private field — constructing one outside `temper_services::auth` must not compile.
use temper_services::auth::SystemAdmin;
use temper_core::types::ids::ProfileId;

fn main() {
    let _forged = SystemAdmin(ProfileId::from(uuid::Uuid::nil())); // E0603 / private field
}
```

`tests/compile_fail/call_admin_fn_without_proof.rs`:

```rust
// A pure-admin fn requires &SystemAdmin — calling it without one must not compile.
use temper_services::services::access_service;
use temper_core::types::ids::ProfileId;

async fn nope(pool: &sqlx::PgPool) {
    // wrong arity / wrong type: no SystemAdmin in hand
    let _ = access_service::admin_revoke(pool, ProfileId::from(uuid::Uuid::nil()), "x".into()).await;
}

fn main() {}
```

- [ ] **Step 3: Run to verify the fixtures fail to compile (i.e. the test passes)**

Run: `cargo nextest run -p temper-services --test compile_fail` (or `cargo test -p temper-services --test compile_fail` — trybuild prints the expected errors on first run and writes `.stderr` snapshots).
Expected: PASS (both fixtures fail to compile for the right reason). On first run, if trybuild reports a mismatch, run with `TRYBUILD=overwrite` once to capture the `.stderr`, then inspect it to confirm the error is the private-field / arity error (not an unrelated one), and commit the snapshot.

> ⚠️ Grounding note: trybuild `.stderr` snapshots are toolchain-sensitive. If they prove flaky across rustc versions in CI, delete the `.stderr` files and keep only the `compile_fail` assertion (trybuild still fails the test if a fixture *compiles*, which is the property we care about). Prefer no-snapshot mode if CI uses a different rustc than local.

- [ ] **Step 4: `cargo make check` then commit**

```bash
cargo make check
git add crates/temper-services/Cargo.toml crates/temper-services/tests/compile_fail.rs crates/temper-services/tests/compile_fail/
git commit -m "test(auth): trybuild proof — SystemAdmin is unforgeable and gates admin fns (Task 5)"
```

---

### Task 6: Evolve the `audit-handler-authz-drift` tripwire to assert `/admin/*` dispatch

Today the tripwire flags *new* handler-side `is_system_admin`. Now that pure-admin handlers mint a proof and delegate, tighten/rebaseline it so a future admin route that *doesn't* dispatch to a `&SystemAdmin`-taking fn is caught.

**Files:**
- Modify: `.github/scripts/audit-handler-authz-drift.sh` (grounding: read it first — its exact assertion shape is unknown until read)
- Test: run it locally + via `cargo make check`

- [ ] **Step 1: Read the current tripwire and its baseline**

Run: `cat .github/scripts/audit-handler-authz-drift.sh`. Determine what it greps and how it baselines. The handler-gated `is_system_admin` sites removed in Task 3 should now be *absent*; if the script has an allowlist enumerating them, remove those entries (they'd be stale-allow).

- [ ] **Step 2: Tighten the assertion**

Add (or adjust) the check so the invariant is: **no handler under `crates/temper-api/src/handlers/` contains `is_system_admin(`** except any deliberately-compositional handler that documents why (there should be none after Tasks 2–4; the compositional gates live in services). If the script already asserts "zero handler-side `is_system_admin`", Tasks 2–4 make it pass with a smaller/empty allowlist — update the baseline accordingly and add a comment pointing at this plan.

> ⚠️ Grounding note: do not invent a new assertion mechanism — extend the existing script's shape. If it's a simple `grep -c` against a baseline number, lower the number to 0 (or the true residual) and comment why. Keep it consistent with the sibling tripwires (`audit-grant-sinks.sh` etc.).

- [ ] **Step 3: Run it**

```bash
bash .github/scripts/audit-handler-authz-drift.sh   # exit 0
cargo make check                                     # the tripwire runs here too
```
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add .github/scripts/audit-handler-authz-drift.sh
git commit -m "build(check): audit-handler-authz-drift asserts admin routes dispatch to &SystemAdmin (Task 6)"
```

---

### Task 7: Full verification + parity sweep

**Files:** none (verification only), unless the sweep surfaces a missed call site.

- [ ] **Step 1: Grep for any residual old-shape callers**

```bash
grep -rn "require_system_admin" crates/           # only auth/mod.rs (def) + handlers (mint) + tests
grep -rn "is_system_admin(" crates/temper-api/src/handlers/   # expect ZERO (all moved to services)
grep -rn "actor: Option<ProfileId>\|, actor: ProfileId" crates/temper-services/src/services/access_service.rs  # admin fns no longer take a bare actor
```
Expected: `require_system_admin` appears only as the new gate + handler mints + tests; no handler `is_system_admin`. Fix any straggler.

- [ ] **Step 2: The full DB-backed suites**

```bash
cargo make check
cargo nextest run -p temper-services --features test-db
cargo nextest run -p temper-api --features test-db --test admin_settings_test   # if present
cargo make test-e2e
```
Expected: all green. `admin_surface_e2e::non_admin_is_forbidden_on_all_admin_endpoints` unchanged (parity). No sqlx cache diff (no SQL changed) — if `cargo make check` reds on sqlx, something touched a `query!`; investigate.

- [ ] **Step 3: Confirm no wire/migration artifacts changed**

```bash
git status --short   # expect ONLY Rust + the tripwire script + trybuild fixtures. NO migrations/, NO openapi.json, NO .sqlx/, NO generated TS.
```
Expected: clean of any generated artifact. If `.sqlx/` or `openapi.json` appear, a `query!` or a documented DTO changed unexpectedly — reconcile before proceeding.

- [ ] **Step 4: Final commit (if the sweep changed anything) + PR**

```bash
git add -A && git commit -m "chore(access): admin-authz enclosure verification sweep (Task 7)"   # only if Step 1 found stragglers
git push -u origin jct/admin-authz-enclosure
gh pr create --base main --title "refactor(access): sealed SystemAdmin proof enclosing admin-authz" --body "<summary per the spec>"
```

Stop at **PR up + CI green + summary** — Pete reviews and merges.

---

## Self-Review

**Spec coverage:**
- §3.1 type + gate + L2 seal → Task 1. ✅
- §3.2/3.3 service-boundary enclosure + surface minting → Tasks 2, 3, 4. ✅
- §3.4 shared-reader wrapper → Task 3 (`admin_get_settings`). ✅
- "What gets the proof" buckets → Tasks 2–4 enclose Bucket 1 (incl. `rebind`); Buckets 2/3 untouched (Global Constraints). ✅
- Audit inventory `rebind` → Task 4. ✅
- Testing: trybuild → Task 5; parity → Tasks 2/3/7. ✅
- Honest limits: tripwire evolution → Task 6. ✅
- Rollout (no migration/wire change) → Task 7 Step 3 guard. ✅
- Non-goals (L1 sealing, DomainAuthzAction, write-primitive deepening) → not in plan, correctly. ✅

**Placeholder scan:** the `⚠️ Grounding note` blocks are deliberate GD-2 re-verify prompts (the plan is grounded at read-time signatures that may drift), not TODOs — each names the exact grep/file to confirm. Task 6 is intentionally read-first because the tripwire script's internal shape wasn't read during planning; its steps say so explicitly rather than inventing an assertion.

**Type consistency:** `SystemAdmin` / `require_system_admin` / `.actor()` used identically across Tasks 1–5. Admin fn signatures in Task 2/3 "Produces" match their call sites in the handler steps and the Task 5 trybuild fixture (`admin_revoke(pool, &SystemAdmin, subject, reason)`).
