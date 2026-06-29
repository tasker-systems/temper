# Chunk 6 — Admin / System-Settings Surface Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface the admin/provisioning configuration that today exists only as raw SQL — editing the instance access gate (`kb_system_settings`), promoting a second system admin, and reviewing join requests — as `temper admin …` CLI + HTTP API + client, all `is_system_admin`-gated. Also wire the deferred Chunk-1 hook so approving an invite_only join request enrolls the profile into the auto-join "everyone" pool.

**Architecture:** Service-direct (no Backend-trait command — settings/teams are infrastructure, per CLAUDE.md §"Persistence is its own layer"). New SQL write functions live in `access_service.rs`; thin handlers gate on `is_system_admin` then dispatch; a new `AdminClient` mirrors `TeamsClient`; a new `temper admin` command group mirrors `temper team`. Wire request types live in `temper-core`. The **first** admin + initial `gating_team_slug` remain the irreducible 2-UPDATE operator-with-DB-credentials root step (documented, not surfaced — nothing to authenticate against yet); everything after bootstraps through the surface.

**Tech Stack:** Rust (axum 0.8, sqlx with compile-time-checked macros, clap), `temper-client` (reqwest), `temper-core` typed wire structs with feature-gated `ts_rs`/`utoipa`/`schemars` derives. e2e via standalone `tests/e2e` crate (real Axum + Postgres + real `temper` binary).

## Global Constraints

- **Typed structs over inline JSON** — no `serde_json::json!()` for known-shape data. Wire types live in `temper-core` with the gated-derive header (see Task 1).
- **Params structs over >5 args** — `UpdateSettingsRequest` IS the params struct; pass it through handler → service.
- **Auth before writes** — every new handler calls `access_service::is_system_admin(&state.pool, ProfileId::from(auth.0.profile.id)).await?` and returns `Err(ApiError::Forbidden)` BEFORE any mutation. The `gated` router already stacks `require_system_access` + `require_auth`; the handler check is defense-in-depth and the precise admin gate.
- **Service layer owns SQL** — never inline `sqlx::query!()` in handlers, CLI actions, or client. All new SQL goes in `access_service.rs`.
- **Production SQL uses compile-time macros** — `sqlx::query!`/`query_as!`/`query_scalar!`. After Tasks 2 & 3, regenerate the per-crate cache: `cargo make prepare-api` (writes `crates/temper-api/.sqlx`). e2e tests use **runtime** `sqlx::query(...)` strings (uncached) so they need no prepare.
- **Runtime wrapper** — CLI actions never call raw `Runtime::new()`; use `temper_cli::actions::runtime::with_client`.
- **Agent-first output** — primary structured results render via `crate::format::render(&value, fmt)`; conversational feedback via `output::*`.
- **`cargo fmt` before every commit** — `cargo make check` gates on `cargo fmt --check` (exit 105). Run `cargo fmt` (or `cargo make fix`) before each commit.
- **`--all-features`** for all builds/clippy.
- **Promote semantics (resolved design):** `temper admin promote <profile-uuid> [--team +<slug>|<slug>|<uuid>]` grants the target profile `owner` on the target team (idempotent `ON CONFLICT DO UPDATE`); `--team` defaults to the configured `gating_team_slug` (resolved **server-side** so the slug never leaves the server). System-admin ≡ owner of the gating team, so the default case mints a second system admin. Does **not** touch `kb_profiles.system_access`.
- **No utoipa for these handlers** — the existing `handlers/access.rs` handlers carry zero `#[utoipa::path]` annotations and are not registered in `openapi.rs`. New admin handlers follow that same precedent (no OpenAPI registration this chunk) to stay consistent and tight.

---

## File Structure

**Create:**
- `crates/temper-core/src/types/admin.rs` — `UpdateSettingsRequest`, `PromoteAdminRequest` wire structs.
- `crates/temper-client/src/admin.rs` — `AdminClient` sub-client.
- `crates/temper-cli/src/commands/admin.rs` — `admin` action bodies.
- `tests/e2e/tests/admin_surface_e2e.rs` — access-semantics e2e gate.

**Modify:**
- `crates/temper-core/src/types/access_gate.rs` — add `Deserialize` to `SystemSettings`.
- `crates/temper-core/src/types/mod.rs` — `pub mod admin;`.
- `crates/temper-api/src/services/access_service.rs` — `update_system_settings`, `promote_admin`; wire `ensure_auto_join_memberships` into `review_request`.
- `crates/temper-api/src/handlers/access.rs` — `get_admin_settings`, `update_settings`, `promote_admin` handlers.
- `crates/temper-api/src/routes.rs` — register the three admin routes in the `gated` router.
- `crates/temper-client/src/lib.rs` — `pub mod admin;` + `pub fn admin(&self)` accessor.
- `crates/temper-cli/src/cli.rs` — `Admin` top-level variant + `AdminAction`/`AdminRequestsAction` enums.
- `crates/temper-cli/src/commands/mod.rs` — `pub mod admin;`.
- `crates/temper-cli/src/main.rs` — `Commands::Admin { action }` dispatch arm.

---

## Task 1: Wire types in `temper-core`

**Files:**
- Create: `crates/temper-core/src/types/admin.rs`
- Modify: `crates/temper-core/src/types/access_gate.rs:96` (add `Deserialize` derive)
- Modify: `crates/temper-core/src/types/mod.rs` (add `pub mod admin;`)
- Test: `crates/temper-core/src/types/admin.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `UpdateSettingsRequest { access_mode: Option<String>, gating_team_slug: Option<String>, instance_name: Option<String>, terms_version: Option<String>, terms_resource_uri: Option<String> }` — each `Some` overwrites that column, each `None` leaves it unchanged (COALESCE semantics; clearing a field to NULL is out of scope this chunk).
  - `PromoteAdminRequest { profile_id: Uuid, team_id: Option<Uuid> }` — `team_id: None` ⇒ server uses the configured gating team.
  - `SystemSettings` (existing, in `access_gate.rs`) gains `Deserialize` so `AdminClient` can decode the admin GET response.

- [ ] **Step 1: Add `Deserialize` to `SystemSettings`**

In `crates/temper-core/src/types/access_gate.rs`, change the derive on `SystemSettings` (currently `access_gate.rs:97`) from:

```rust
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SystemSettings {
```

to:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SystemSettings {
```

(`Deserialize` is already imported in this module — `PublicSystemSettings` derives it.)

- [ ] **Step 2: Create `crates/temper-core/src/types/admin.rs`**

```rust
//! Wire types for the admin / system-settings surface (Chunk 6).
//!
//! `UpdateSettingsRequest` is a partial-update payload: every `Some` field
//! overwrites that `kb_system_settings` column, every `None` leaves it
//! unchanged (COALESCE on the server). `access_mode` is a raw string validated
//! server-side against `{open, invite_only}` — mirrors how `SystemSettings`
//! keeps `access_mode` as `String` rather than a sqlx-decoded enum.
//!
//! `PromoteAdminRequest` grants the target profile `owner` on a team. A `None`
//! `team_id` means "the configured gating team" (resolved server-side so the
//! gating slug never leaves the server). System-admin ≡ owner of the gating
//! team, so the default case mints a second system admin.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Partial-update body for `PATCH /api/access/admin/settings`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "admin.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateSettingsRequest {
    /// `"open"` or `"invite_only"`. Validated server-side.
    pub access_mode: Option<String>,
    /// Slug of the team that gates the instance in `invite_only` mode.
    pub gating_team_slug: Option<String>,
    /// Human-facing instance name.
    pub instance_name: Option<String>,
    /// Terms-of-service version label.
    pub terms_version: Option<String>,
    /// URI of the terms-of-service resource.
    pub terms_resource_uri: Option<String>,
}

impl UpdateSettingsRequest {
    /// True when no field is set — the caller wants a read, not a write.
    pub fn is_empty(&self) -> bool {
        self.access_mode.is_none()
            && self.gating_team_slug.is_none()
            && self.instance_name.is_none()
            && self.terms_version.is_none()
            && self.terms_resource_uri.is_none()
    }
}

/// Body for `POST /api/access/admin/promote`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "admin.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteAdminRequest {
    /// Profile to promote (grant `owner` on the target team).
    pub profile_id: Uuid,
    /// Target team; `None` ⇒ the configured gating team (mints a system admin).
    pub team_id: Option<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_settings_is_empty_detects_no_fields() {
        assert!(UpdateSettingsRequest::default().is_empty());
        let one = UpdateSettingsRequest {
            instance_name: Some("Acme".to_owned()),
            ..Default::default()
        };
        assert!(!one.is_empty());
    }

    #[test]
    fn promote_request_roundtrips_through_json() {
        let req = PromoteAdminRequest {
            profile_id: Uuid::nil(),
            team_id: None,
        };
        let json = serde_json::to_string(&req).expect("serialize");
        let back: PromoteAdminRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.profile_id, req.profile_id);
        assert!(back.team_id.is_none());
    }
}
```

- [ ] **Step 3: Register the module**

In `crates/temper-core/src/types/mod.rs`, add `pub mod admin;` in alphabetical position (before `pub mod access_gate;`? — match the existing ordering; `admin` sorts before `access_gate` alphabetically only if you sort by full name — place it next to the other type modules following the file's existing convention).

- [ ] **Step 4: Run the unit tests**

Run: `cargo test -p temper-core --lib types::admin`
Expected: PASS (2 tests).

- [ ] **Step 5: Verify it compiles under the feature matrix used by consumers**

Run: `cargo build -p temper-core --all-features`
Expected: builds clean (exercises `ts_rs`/`utoipa`/`schemars` derives).

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add crates/temper-core/src/types/admin.rs crates/temper-core/src/types/mod.rs crates/temper-core/src/types/access_gate.rs
git commit -m "Chunk 6: admin wire types + SystemSettings Deserialize"
```

---

## Task 2: Service — `update_system_settings` + `promote_admin`

**Files:**
- Modify: `crates/temper-api/src/services/access_service.rs` (append two functions after `get_public_settings`, ~`access_service.rs:114`)
- Test: `crates/temper-api/tests/admin_settings_test.rs` (new)

**Interfaces:**
- Consumes: `temper_core::types::admin::UpdateSettingsRequest`; `SystemSettings`, `AccessMode`, `ProfileId`, `ApiError`/`ApiResult` (already imported in this file); `TeamMemberRow`, `TeamRole` from `temper_core::types::team`.
- Produces:
  - `pub async fn update_system_settings(pool: &PgPool, req: &UpdateSettingsRequest) -> ApiResult<SystemSettings>`
  - `pub async fn promote_admin(pool: &PgPool, profile_id: Uuid, team_id: Option<Uuid>) -> ApiResult<TeamMemberRow>`

- [ ] **Step 1: Write the failing tests**

Create `crates/temper-api/tests/admin_settings_test.rs`. (Follows the `cogmap_authz_test.rs` style: `#[sqlx::test(migrator = "temper_api::MIGRATOR")]`, `common::fixtures` for profiles, raw `sqlx::query` for setup.)

```rust
#![cfg(feature = "test-db")]

mod common;

use temper_api::services::access_service;
use temper_core::types::admin::UpdateSettingsRequest;
use uuid::Uuid;

/// Seed the singleton settings row to a known baseline (the seed migration
/// inserts `id=1` already, but be explicit so the test is self-contained).
async fn reset_settings(pool: &sqlx::PgPool) {
    sqlx::query(
        "UPDATE kb_system_settings \
         SET access_mode='open', gating_team_slug=NULL, instance_name=NULL, \
             terms_version=NULL, terms_resource_uri=NULL WHERE id=1",
    )
    .execute(pool)
    .await
    .expect("reset settings");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_settings_partial_coalesces(pool: sqlx::PgPool) {
    reset_settings(&pool).await;

    let req = UpdateSettingsRequest {
        instance_name: Some("Acme Temper".to_owned()),
        ..Default::default()
    };
    let updated = access_service::update_system_settings(&pool, &req)
        .await
        .expect("update");

    assert_eq!(updated.instance_name.as_deref(), Some("Acme Temper"));
    assert_eq!(updated.access_mode, "open"); // untouched field preserved
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_settings_rejects_unknown_access_mode(pool: sqlx::PgPool) {
    reset_settings(&pool).await;

    let req = UpdateSettingsRequest {
        access_mode: Some("banana".to_owned()),
        ..Default::default()
    };
    let err = access_service::update_system_settings(&pool, &req)
        .await
        .expect_err("should reject");
    assert!(matches!(err, temper_api::error::ApiError::BadRequest(_)));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_settings_invite_only_requires_gating_team(pool: sqlx::PgPool) {
    reset_settings(&pool).await; // gating_team_slug is NULL

    let req = UpdateSettingsRequest {
        access_mode: Some("invite_only".to_owned()),
        ..Default::default()
    };
    let err = access_service::update_system_settings(&pool, &req)
        .await
        .expect_err("invite_only without a gating team should be rejected");
    assert!(matches!(err, temper_api::error::ApiError::BadRequest(_)));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn promote_admin_defaults_to_gating_team(pool: sqlx::PgPool) {
    reset_settings(&pool).await;
    // Configure a gating team that exists.
    let team_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='temper-system' WHERE id=1")
        .execute(&pool)
        .await
        .expect("set gating");

    let profile = common::fixtures::create_test_profile(&pool, "promotee@test.example.com").await;

    let row = access_service::promote_admin(&pool, profile, None)
        .await
        .expect("promote");

    assert_eq!(row.team_id, team_id);
    assert_eq!(row.profile_id, profile);
    assert!(matches!(row.role, temper_core::types::team::TeamRole::Owner));

    // is_system_admin now true for the promotee.
    let is_admin = access_service::is_system_admin(
        &pool,
        temper_core::ids::ProfileId::from(profile),
    )
    .await
    .expect("check");
    assert!(is_admin);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn promote_admin_without_gating_or_team_is_bad_request(pool: sqlx::PgPool) {
    reset_settings(&pool).await; // gating_team_slug NULL, no --team
    let profile = common::fixtures::create_test_profile(&pool, "x@test.example.com").await;
    let err = access_service::promote_admin(&pool, profile, None)
        .await
        .expect_err("no target team");
    assert!(matches!(err, temper_api::error::ApiError::BadRequest(_)));
}
```

> Note: confirm the exact `ProfileId` import path while implementing — the codebase exposes the newtype as `temper_core::ids::ProfileId` (re-exported; `access_service.rs` already uses `ProfileId`). Match whatever path `access_service.rs` imports at its top.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-api --features test-db --test admin_settings_test`
Expected: FAIL — `update_system_settings` / `promote_admin` not found.

- [ ] **Step 3: Implement `update_system_settings`**

Append to `crates/temper-api/src/services/access_service.rs` (after `get_public_settings`, ~line 114). Add `use temper_core::types::admin::UpdateSettingsRequest;` and `use temper_core::types::team::{TeamMemberRow, TeamRole};` to the imports at the top of the file.

```rust
/// Admin-only partial update of the singleton `kb_system_settings` row.
///
/// COALESCE semantics: each `Some` field overwrites its column; each `None`
/// leaves the column unchanged. `access_mode` is validated against
/// `{open, invite_only}`. Guards against the lockout footgun: an effective
/// `invite_only` mode with no `gating_team_slug` would make `has_system_access`
/// false for everyone, so it is rejected.
pub async fn update_system_settings(
    pool: &PgPool,
    req: &UpdateSettingsRequest,
) -> ApiResult<SystemSettings> {
    // Validate access_mode (parse-don't-validate against the DB CHECK).
    if let Some(mode) = req.access_mode.as_deref() {
        if AccessMode::from_db_str(mode).is_none() {
            return Err(ApiError::BadRequest(format!(
                "invalid access_mode {mode:?} (expected 'open' or 'invite_only')"
            )));
        }
    }

    // Compute the EFFECTIVE post-update mode + gating slug to guard lockout.
    let current = get_system_settings(pool).await?;
    let effective_mode = req
        .access_mode
        .clone()
        .unwrap_or(current.access_mode.clone());
    let effective_gating = req
        .gating_team_slug
        .clone()
        .or(current.gating_team_slug.clone());
    if effective_mode == "invite_only" && effective_gating.is_none() {
        return Err(ApiError::BadRequest(
            "invite_only mode requires a gating_team_slug (set --gating-team in the same call \
             or beforehand) — otherwise no one can access the instance"
                .to_string(),
        ));
    }

    let row = sqlx::query_as!(
        SystemSettings,
        r#"
        UPDATE kb_system_settings
           SET access_mode        = COALESCE($1, access_mode),
               gating_team_slug   = COALESCE($2, gating_team_slug),
               instance_name      = COALESCE($3, instance_name),
               terms_version      = COALESCE($4, terms_version),
               terms_resource_uri = COALESCE($5, terms_resource_uri),
               updated            = now()
         WHERE id = 1
        RETURNING id, access_mode, gating_team_slug, terms_version,
                  terms_resource_uri, instance_name, updated
        "#,
        req.access_mode,
        req.gating_team_slug,
        req.instance_name,
        req.terms_version,
        req.terms_resource_uri,
    )
    .fetch_one(pool)
    .await?;

    Ok(row)
}
```

- [ ] **Step 4: Implement `promote_admin`**

```rust
/// Admin-only: grant `profile_id` the `owner` role on a team (idempotent).
///
/// `team_id == None` resolves to the configured gating team — system-admin ≡
/// owner of the gating team, so this mints a second system admin. Decoupled
/// from `kb_profiles.system_access` (the auth gate reads gating-team ownership,
/// not the enum). Auth is enforced by the caller (handler `is_system_admin`).
pub async fn promote_admin(
    pool: &PgPool,
    profile_id: Uuid,
    team_id: Option<Uuid>,
) -> ApiResult<TeamMemberRow> {
    // Resolve the target team: explicit, else the configured gating team.
    let target_team = match team_id {
        Some(id) => id,
        None => {
            let settings = get_system_settings(pool).await?;
            let Some(slug) = settings.gating_team_slug else {
                return Err(ApiError::BadRequest(
                    "no gating team configured; pass --team to promote on a specific team"
                        .to_string(),
                ));
            };
            sqlx::query_scalar!("SELECT id FROM kb_teams WHERE slug = $1", slug)
                .fetch_optional(pool)
                .await?
                .ok_or_else(|| {
                    ApiError::BadRequest(format!("gating team '{slug}' does not exist"))
                })?
        }
    };

    let row = sqlx::query_as!(
        TeamMemberRow,
        r#"
        INSERT INTO kb_team_members (team_id, profile_id, role)
        VALUES ($1, $2, 'owner')
        ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role
        RETURNING team_id, profile_id, role AS "role: TeamRole", created
        "#,
        target_team,
        profile_id,
    )
    .fetch_one(pool)
    .await?;

    Ok(row)
}
```

- [ ] **Step 5: Regenerate the per-crate sqlx cache (new `query!`/`query_as!` macros)**

Run: `cargo make prepare-api`
Expected: updates `crates/temper-api/.sqlx/` with new query entries; exits 0.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo nextest run -p temper-api --features test-db --test admin_settings_test`
Expected: PASS (5 tests).

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add crates/temper-api/src/services/access_service.rs crates/temper-api/tests/admin_settings_test.rs crates/temper-api/.sqlx
git commit -m "Chunk 6: settings-write + promote-admin service functions"
```

---

## Task 3: Wire `ensure_auto_join_memberships` into join-request approval

**Files:**
- Modify: `crates/temper-api/src/services/access_service.rs:346-358` (the approve branch in `review_request`)
- Test: `crates/temper-api/tests/admin_settings_test.rs` (add one test)

**Interfaces:**
- Consumes: existing `ensure_auto_join_memberships(p_profile uuid)` SQL function (`migrations/20260629000002_auto_join_team_generalization.sql:41-56`). It is a no-op unless `has_system_access(profile)` is true, so it must run **after** the gating-team membership is granted (which establishes access).
- Produces: no new signature — `review_request` behavior gains auto-join enrollment on approval.

- [ ] **Step 1: Write the failing test**

Add to `crates/temper-api/tests/admin_settings_test.rs`:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn approval_enrolls_into_other_auto_join_teams(pool: sqlx::PgPool) {
    // Gating team = temper-system (auto_join_role watcher, seeded by migration).
    let gating_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("gating team");
    // A SECOND auto-join team that is NOT the gating team — proves the hook does
    // more than the direct gating-team insert.
    let other_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name, auto_join_role) \
         VALUES ('everyone','Everyone','member') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("other auto-join team");
    sqlx::query(
        "UPDATE kb_system_settings SET access_mode='invite_only', gating_team_slug='temper-system' WHERE id=1",
    )
    .execute(&pool)
    .await
    .expect("invite_only");

    let admin = common::fixtures::create_test_profile(&pool, "admin@test.example.com").await;
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1,$2,'owner') \
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role=EXCLUDED.role",
    )
    .bind(gating_id)
    .bind(admin)
    .execute(&pool)
    .await
    .expect("make admin");

    let joiner = common::fixtures::create_test_profile(&pool, "joiner@test.example.com").await;

    // Joiner submits a request for the gating team.
    let request_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_join_requests (id, team_id, requesting_profile_id, status, source) \
         VALUES (gen_random_uuid(), $1, $2, 'pending', 'test') RETURNING id",
    )
    .bind(gating_id)
    .bind(joiner)
    .fetch_one(&pool)
    .await
    .expect("join request");

    // Admin approves via the service.
    access_service::review_request(
        &pool,
        access_service::ReviewRequestParams {
            request_id,
            reviewer_profile_id: temper_core::ids::ProfileId::from(admin),
            decision: temper_core::types::access_gate::JoinRequestStatus::Approved,
            decision_note: None,
        },
    )
    .await
    .expect("approve");

    // The joiner is now enrolled in the OTHER auto-join team via the hook.
    let in_other: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM kb_team_members WHERE team_id=$1 AND profile_id=$2)",
    )
    .bind(other_id)
    .bind(joiner)
    .fetch_one(&pool)
    .await
    .expect("check");
    assert!(in_other, "approval should enroll the profile into auto-join teams");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p temper-api --features test-db --test admin_settings_test approval_enrolls`
Expected: FAIL — the joiner is enrolled only in the gating team today, not `everyone`.

- [ ] **Step 3: Add the hook call in `review_request`**

In `crates/temper-api/src/services/access_service.rs`, inside the `if params.decision == JoinRequestStatus::Approved { … }` block (currently `access_service.rs:346-358`), AFTER the existing `INSERT INTO kb_team_members … 'watcher' … ON CONFLICT … DO NOTHING` statement (which grants access via the gating team), add:

```rust
        // Now that gating-team membership establishes access, enroll the
        // profile into the rest of the auto-join "everyone" pool (Chunk 1's
        // deferred call site). No-op if has_system_access is still false.
        sqlx::query!(
            "SELECT ensure_auto_join_memberships($1)",
            row.requesting_profile_id,
        )
        .execute(&mut *tx)
        .await?;
```

Keep the existing gating-team insert exactly as-is — it is the grant that flips `has_system_access` to true; the hook is additive.

- [ ] **Step 4: Regenerate the sqlx cache (new `query!` macro)**

Run: `cargo make prepare-api`
Expected: exits 0, cache updated.

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo nextest run -p temper-api --features test-db --test admin_settings_test`
Expected: PASS (6 tests — the 5 from Task 2 plus this one).

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add crates/temper-api/src/services/access_service.rs crates/temper-api/tests/admin_settings_test.rs crates/temper-api/.sqlx
git commit -m "Chunk 6: enroll auto-join pool on invite_only approval"
```

---

## Task 4: API handlers + routes

**Files:**
- Modify: `crates/temper-api/src/handlers/access.rs` (add three handlers + import the request types)
- Modify: `crates/temper-api/src/routes.rs:133-148` (register routes in the `gated` router)

**Interfaces:**
- Consumes: `access_service::{is_system_admin, get_system_settings, update_system_settings, promote_admin}`; `temper_core::types::admin::{UpdateSettingsRequest, PromoteAdminRequest}`; `SystemSettings`, `TeamMemberRow`.
- Produces (handler fns, mirroring the existing `list_pending`/`review_request` gating shape):
  - `get_admin_settings(State, AuthUser) -> ApiResult<Json<SystemSettings>>`
  - `update_settings(State, AuthUser, Json<UpdateSettingsRequest>) -> ApiResult<Json<SystemSettings>>`
  - `promote_admin(State, AuthUser, Json<PromoteAdminRequest>) -> ApiResult<Json<TeamMemberRow>>`
- Routes (in `gated`): `GET`+`PATCH /api/access/admin/settings`, `POST /api/access/admin/promote`.

- [ ] **Step 1: Add the three handlers**

In `crates/temper-api/src/handlers/access.rs`, add imports near the top: `use temper_core::types::admin::{PromoteAdminRequest, UpdateSettingsRequest};`, `use temper_core::types::access_gate::SystemSettings;`, `use temper_core::types::team::TeamMemberRow;`. Append the handlers after `review_request` (~`access.rs:126`):

```rust
/// GET /api/access/admin/settings — read FULL system settings (admin only).
///
/// Unlike the public `GET /api/access/settings`, this returns `gating_team_slug`
/// and `updated`, which an admin needs to administer the gate.
pub async fn get_admin_settings(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<SystemSettings>> {
    let is_admin =
        access_service::is_system_admin(&state.pool, ProfileId::from(auth.0.profile.id)).await?;
    if !is_admin {
        return Err(ApiError::Forbidden);
    }
    access_service::get_system_settings(&state.pool)
        .await
        .map(Json)
}

/// PATCH /api/access/admin/settings — partial update of system settings (admin only).
pub async fn update_settings(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<UpdateSettingsRequest>,
) -> ApiResult<Json<SystemSettings>> {
    let is_admin =
        access_service::is_system_admin(&state.pool, ProfileId::from(auth.0.profile.id)).await?;
    if !is_admin {
        return Err(ApiError::Forbidden);
    }
    access_service::update_system_settings(&state.pool, &body)
        .await
        .map(Json)
}

/// POST /api/access/admin/promote — grant a profile `owner` on a team (admin only).
///
/// `team_id` omitted ⇒ the configured gating team (mints a second system admin).
pub async fn promote_admin(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<PromoteAdminRequest>,
) -> ApiResult<Json<TeamMemberRow>> {
    let is_admin =
        access_service::is_system_admin(&state.pool, ProfileId::from(auth.0.profile.id)).await?;
    if !is_admin {
        return Err(ApiError::Forbidden);
    }
    access_service::promote_admin(&state.pool, body.profile_id, body.team_id)
        .await
        .map(Json)
}
```

- [ ] **Step 2: Register the routes**

In `crates/temper-api/src/routes.rs`, in the `gated` router block (next to the existing `/api/access/admin/requests` routes, ~`routes.rs:135-142`), add:

```rust
        .route(
            "/api/access/admin/settings",
            get(handlers::access::get_admin_settings)
                .patch(handlers::access::update_settings),
        )
        .route(
            "/api/access/admin/promote",
            post(handlers::access::promote_admin),
        )
```

(`get` and `post` are already imported at `routes.rs:16`; chaining `.patch(...)` onto the `.route(...)` avoids needing the bare `patch` import. Placement inside `gated` means both `require_system_access` and `require_auth` already apply; the handler `is_system_admin` check is the admin gate.)

- [ ] **Step 3: Build to verify handlers + routes compile**

Run: `cargo build -p temper-api --all-features`
Expected: builds clean.

- [ ] **Step 4: Commit**

```bash
cargo fmt
git add crates/temper-api/src/handlers/access.rs crates/temper-api/src/routes.rs
git commit -m "Chunk 6: admin settings + promote HTTP handlers and routes"
```

---

## Task 5: `temper-client` `AdminClient`

**Files:**
- Create: `crates/temper-client/src/admin.rs`
- Modify: `crates/temper-client/src/lib.rs` (declare module + accessor)

**Interfaces:**
- Consumes: `HttpClient` (`get`/`patch`/`post`, `resolve_token`, `send_json`); `temper_core::types::admin::{UpdateSettingsRequest, PromoteAdminRequest}`; `SystemSettings`; `JoinRequest`, `JoinRequestWithProfile`, `JoinRequestStatus` (`access_gate`); `TeamMemberRow` (`team`).
- Produces: `AdminClient<'a>` with `get_settings`, `update_settings`, `promote`, `list_requests`, `review_request`; accessor `TemperClient::admin(&self) -> AdminClient<'_>`.

- [ ] **Step 1: Create `crates/temper-client/src/admin.rs`**

Mirror `teams.rs` exactly (borrowed `&HttpClient`, `pub(crate) fn new`, `Debug` impl, per-method `resolve_token` → builder → `send_json`).

```rust
//! Typed sub-client for the `/api/access/admin/*` endpoints (Chunk 6).

use reqwest::Method;
use uuid::Uuid;

use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::access_gate::{
    JoinRequest, JoinRequestStatus, JoinRequestWithProfile, SystemSettings,
};
use temper_core::types::admin::{PromoteAdminRequest, UpdateSettingsRequest};
use temper_core::types::team::TeamMemberRow;

/// Sub-client for admin / system-settings operations.
pub struct AdminClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for AdminClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminClient").finish_non_exhaustive()
    }
}

impl<'a> AdminClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Read full system settings (admin only).
    pub async fn get_settings(&self) -> Result<SystemSettings> {
        let token = self.http.resolve_token()?;
        let path = "/api/access/admin/settings";
        let req = self.http.get(path);
        self.http.send_json(&Method::GET, path, req, Some(&token)).await
    }

    /// Partial-update system settings (admin only).
    pub async fn update_settings(&self, body: &UpdateSettingsRequest) -> Result<SystemSettings> {
        let token = self.http.resolve_token()?;
        let path = "/api/access/admin/settings";
        let req = self.http.patch(path).json(body);
        self.http.send_json(&Method::PATCH, path, req, Some(&token)).await
    }

    /// Promote a profile to `owner` on a team (admin only).
    pub async fn promote(&self, body: &PromoteAdminRequest) -> Result<TeamMemberRow> {
        let token = self.http.resolve_token()?;
        let path = "/api/access/admin/promote";
        let req = self.http.post(path).json(body);
        self.http.send_json(&Method::POST, path, req, Some(&token)).await
    }

    /// List pending join requests for the gating team (admin only).
    pub async fn list_requests(&self) -> Result<Vec<JoinRequestWithProfile>> {
        let token = self.http.resolve_token()?;
        let path = "/api/access/admin/requests";
        let req = self.http.get(path);
        self.http.send_json(&Method::GET, path, req, Some(&token)).await
    }

    /// Approve or reject a join request (admin only).
    pub async fn review_request(
        &self,
        request_id: Uuid,
        decision: JoinRequestStatus,
        decision_note: Option<String>,
    ) -> Result<JoinRequest> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/access/admin/requests/{request_id}");
        let body = ReviewBody { status: decision, decision_note };
        let req = self.http.patch(&path).json(&body);
        self.http.send_json(&Method::PATCH, &path, req, Some(&token)).await
    }
}

/// Mirrors `handlers::access::ReviewRequestBody` (the handler's private body type).
#[derive(serde::Serialize)]
struct ReviewBody {
    status: JoinRequestStatus,
    decision_note: Option<String>,
}
```

- [ ] **Step 2: Register the module + accessor in `lib.rs`**

In `crates/temper-client/src/lib.rs`, add `pub mod admin;` to the module block (alongside `pub mod teams;`). Add the accessor next to `teams()` (~`lib.rs:146-148`):

```rust
    /// Admin / system-settings sub-client (settings, promote, request review).
    pub fn admin(&self) -> admin::AdminClient<'_> {
        admin::AdminClient::new(&self.http)
    }
```

- [ ] **Step 3: Build to verify it compiles**

Run: `cargo build -p temper-client --all-features`
Expected: builds clean.

- [ ] **Step 4: Commit**

```bash
cargo fmt
git add crates/temper-client/src/admin.rs crates/temper-client/src/lib.rs
git commit -m "Chunk 6: AdminClient (settings, promote, request review)"
```

---

## Task 6: CLI `temper admin` command group

**Files:**
- Create: `crates/temper-cli/src/commands/admin.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs` (`pub mod admin;`)
- Modify: `crates/temper-cli/src/cli.rs` (`Admin` variant + `AdminAction` + `AdminRequestsAction`)
- Modify: `crates/temper-cli/src/main.rs` (dispatch arm)

**Interfaces:**
- Consumes: `temper_client::TemperClient::admin()`; `crate::actions::cogmap::resolve_team_id` (handles `+<slug>`/bare-slug/UUID → team UUID); `crate::format::{render, OutputFormat}`; `crate::commands::client_err`; `temper_core::types::admin::{UpdateSettingsRequest, PromoteAdminRequest}`; `JoinRequestStatus`.
- Produces: `temper admin settings [flags]`, `temper admin promote <profile> [--team]`, `temper admin requests list`, `temper admin requests review <id> --approve|--reject [--note]`.

- [ ] **Step 1: Add the clap enums in `cli.rs`**

In `crates/temper-cli/src/cli.rs`, add a top-level variant to `Commands` (near `Team`, ~`cli.rs:197`):

```rust
    /// Administer the instance (system settings, promote admins, review requests)
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },
```

Add the subcommand enums (near `TeamAction`, ~`cli.rs:506`):

```rust
#[derive(Subcommand)]
pub enum AdminAction {
    /// Show system settings, or update them when any flag is provided
    Settings {
        /// Access mode: open | invite_only
        #[arg(long = "access-mode")]
        access_mode: Option<String>,
        /// Gating team slug (the team that gates invite_only access)
        #[arg(long = "gating-team")]
        gating_team_slug: Option<String>,
        /// Human-facing instance name
        #[arg(long = "instance-name")]
        instance_name: Option<String>,
        /// Terms-of-service version label
        #[arg(long = "terms-version")]
        terms_version: Option<String>,
        /// Terms-of-service resource URI
        #[arg(long = "terms-uri")]
        terms_resource_uri: Option<String>,
    },
    /// Promote a profile to admin (owner on a team; defaults to the gating team)
    Promote {
        /// Profile ID (UUID) to promote
        profile: String,
        /// Team ref (`+slug`, bare slug, or UUID); defaults to the gating team
        #[arg(long)]
        team: Option<String>,
    },
    /// Review pending join requests
    Requests {
        #[command(subcommand)]
        action: AdminRequestsAction,
    },
}

#[derive(Subcommand)]
pub enum AdminRequestsAction {
    /// List pending join requests for the gating team
    List,
    /// Approve or reject a join request
    Review {
        /// Join request ID (UUID)
        id: String,
        /// Approve the request
        #[arg(long, conflicts_with = "reject")]
        approve: bool,
        /// Reject the request
        #[arg(long)]
        reject: bool,
        /// Optional decision note
        #[arg(long)]
        note: Option<String>,
    },
}
```

- [ ] **Step 2: Create `crates/temper-cli/src/commands/admin.rs`**

```rust
//! Admin commands: system-settings show/update, promote, request review.
//! Round-trips CLI → AdminClient → API → access_service.

use crate::error::{Result, TemperError};
use temper_core::types::access_gate::JoinRequestStatus;
use temper_core::types::admin::{PromoteAdminRequest, UpdateSettingsRequest};

/// Show settings when no flag is set; otherwise PATCH and render the result.
#[expect(clippy::too_many_arguments, reason = "thin CLI passthrough of optional flags")]
pub async fn settings_remote(
    client: &temper_client::TemperClient,
    access_mode: Option<&str>,
    gating_team_slug: Option<&str>,
    instance_name: Option<&str>,
    terms_version: Option<&str>,
    terms_resource_uri: Option<&str>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let req = UpdateSettingsRequest {
        access_mode: access_mode.map(str::to_owned),
        gating_team_slug: gating_team_slug.map(str::to_owned),
        instance_name: instance_name.map(str::to_owned),
        terms_version: terms_version.map(str::to_owned),
        terms_resource_uri: terms_resource_uri.map(str::to_owned),
    };

    let settings = if req.is_empty() {
        client.admin().get_settings().await.map_err(crate::commands::client_err)?
    } else {
        client.admin().update_settings(&req).await.map_err(crate::commands::client_err)?
    };

    let rendered = crate::format::render(&settings, fmt)?;
    println!("{rendered}");
    Ok(())
}
```

> Note: the `#[expect(clippy::too_many_arguments)]` here is borderline against CLAUDE.md's "params struct over >5 args" rule. Prefer building the `UpdateSettingsRequest` in the `main.rs` dispatch arm and passing it by value instead — i.e. signature `settings_remote(client, req: UpdateSettingsRequest, fmt)`. Implement that cleaner form: it drops the `#[expect]` and satisfies the rule. (Shown verbose above only to make the field mapping explicit; the implementer should pass the struct.)

Continue `admin.rs`:

```rust
/// Promote a profile to owner on a team (defaults to the gating team).
pub async fn promote_remote(
    client: &temper_client::TemperClient,
    profile: &str,
    team: Option<&str>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let profile_id = uuid::Uuid::parse_str(profile)
        .map_err(|e| TemperError::Api(format!("invalid profile id '{profile}': {e}")))?;

    // Resolve --team to a UUID when provided; None ⇒ server uses the gating team.
    let team_id = match team {
        Some(t) => Some(crate::actions::cogmap::resolve_team_id(client, t).await?),
        None => None,
    };

    let req = PromoteAdminRequest { profile_id, team_id };
    let row = client.admin().promote(&req).await.map_err(crate::commands::client_err)?;

    let rendered = crate::format::render(&row, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// List pending join requests.
pub async fn requests_list_remote(
    client: &temper_client::TemperClient,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let rows = client.admin().list_requests().await.map_err(crate::commands::client_err)?;
    let rendered = crate::format::render(&rows, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Approve or reject a join request.
pub async fn requests_review_remote(
    client: &temper_client::TemperClient,
    id: &str,
    approve: bool,
    reject: bool,
    note: Option<&str>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let request_id = uuid::Uuid::parse_str(id)
        .map_err(|e| TemperError::Api(format!("invalid request id '{id}': {e}")))?;

    let decision = match (approve, reject) {
        (true, false) => JoinRequestStatus::Approved,
        (false, true) => JoinRequestStatus::Rejected,
        _ => {
            return Err(TemperError::Api(
                "specify exactly one of --approve or --reject".to_string(),
            ))
        }
    };

    let row = client
        .admin()
        .review_request(request_id, decision, note.map(str::to_owned))
        .await
        .map_err(crate::commands::client_err)?;

    let rendered = crate::format::render(&row, fmt)?;
    println!("{rendered}");
    Ok(())
}
```

- [ ] **Step 3: Register the module**

In `crates/temper-cli/src/commands/mod.rs`, add `pub mod admin;` (alphabetical, before `pub mod auth;`).

- [ ] **Step 4: Add the dispatch arm in `main.rs`**

In `crates/temper-cli/src/main.rs`, add alongside `Commands::Team` (~`main.rs:299`). (Using the cleaner `settings_remote(client, req, fmt)` form — build the `UpdateSettingsRequest` here.)

```rust
        Commands::Admin { action } => match action {
            AdminAction::Settings {
                access_mode,
                gating_team_slug,
                instance_name,
                terms_version,
                terms_resource_uri,
            } => temper_cli::actions::runtime::with_client(|client| {
                Box::pin(async move {
                    let req = temper_core::types::admin::UpdateSettingsRequest {
                        access_mode,
                        gating_team_slug,
                        instance_name,
                        terms_version,
                        terms_resource_uri,
                    };
                    temper_cli::commands::admin::settings_remote(client, req, output_format).await
                })
            }),
            AdminAction::Promote { profile, team } => {
                temper_cli::actions::runtime::with_client(|client| {
                    Box::pin(async move {
                        temper_cli::commands::admin::promote_remote(
                            client,
                            &profile,
                            team.as_deref(),
                            output_format,
                        )
                        .await
                    })
                })
            }
            AdminAction::Requests { action } => match action {
                AdminRequestsAction::List => {
                    temper_cli::actions::runtime::with_client(|client| {
                        Box::pin(async move {
                            temper_cli::commands::admin::requests_list_remote(client, output_format)
                                .await
                        })
                    })
                }
                AdminRequestsAction::Review { id, approve, reject, note } => {
                    temper_cli::actions::runtime::with_client(|client| {
                        Box::pin(async move {
                            temper_cli::commands::admin::requests_review_remote(
                                client,
                                &id,
                                approve,
                                reject,
                                note.as_deref(),
                                output_format,
                            )
                            .await
                        })
                    })
                }
            },
        },
```

> Adjust the `settings_remote` signature to `(client, req: UpdateSettingsRequest, fmt)` (drop the per-field args + `#[expect]`) to match this dispatch and satisfy the params-struct rule. Ensure `AdminAction`/`AdminRequestsAction` are imported in `main.rs` (same `use` site as `TeamAction`).

- [ ] **Step 5: Build + smoke-test the CLI surface**

Run: `cargo build -p temper-cli --all-features`
Then verify the help renders (no server needed):
Run: `cargo run -p temper-cli -- admin --help` and `cargo run -p temper-cli -- admin settings --help`
Expected: subcommands and flags listed; exits 0.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add crates/temper-cli/src/commands/admin.rs crates/temper-cli/src/commands/mod.rs crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs
git commit -m "Chunk 6: temper admin command group (settings, promote, requests)"
```

---

## Task 7: e2e access-semantics gate

**Files:**
- Create: `tests/e2e/tests/admin_surface_e2e.rs`

**Interfaces:**
- Consumes: `common::{setup, E2eTestApp, generate_second_user_jwt, run_temper_cli}`; raw `sqlx::query` against `app.pool`; `app.reqwest_client` + tokens. Uses runtime `sqlx::query(...)` strings (no `query!` macro) ⇒ no `prepare-e2e` needed.

This is the **mandatory access-semantics e2e tier** for an admin/membership change (per `feedback_access_semantics_changes_need_e2e_tier`). Two tests.

- [ ] **Step 1: Write the test file**

```rust
#![cfg(feature = "test-db")]

mod common;

use reqwest::StatusCode;
use serde_json::{json, Value};
use uuid::Uuid;

/// Provision a profile by hitting an authed endpoint (auto-provision on first request).
async fn provision(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("json");
    body["id"].as_str().expect("id").parse().expect("uuid")
}

/// The irreducible 2-UPDATE operator root step: configure gating + mint first admin.
async fn root_bootstrap_first_admin(pool: &sqlx::PgPool, admin_id: Uuid) {
    sqlx::query(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name",
    )
    .execute(pool)
    .await
    .expect("team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='temper-system' WHERE id=1")
        .execute(pool)
        .await
        .expect("gating");
    sqlx::query("UPDATE kb_profiles SET system_access='admin' WHERE id=$1")
        .bind(admin_id)
        .execute(pool)
        .await
        .expect("promote first admin"); // trigger mints owner of temper-system
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn admin_can_set_settings_and_promote_second_admin(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    let second_token = common::generate_second_user_jwt();
    let second_id = provision(&app, &second_token).await;

    root_bootstrap_first_admin(&pool, admin_id).await;

    // First admin sets an instance name via the CLI (runs as app.token).
    let out = common::run_temper_cli(
        &app,
        &["admin", "settings", "--instance-name", "Acme Temper", "--format", "json"],
    )
    .await
    .expect("cli settings");
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
    let settings: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(settings["instance_name"], "Acme Temper");

    // Read-back round-trip via CLI (no flags ⇒ show).
    let out = common::run_temper_cli(&app, &["admin", "settings", "--format", "json"])
        .await
        .expect("cli show");
    let shown: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(shown["instance_name"], "Acme Temper");
    assert_eq!(shown["gating_team_slug"], "temper-system");

    // First admin promotes the second admin via the CLI (default = gating team).
    let out = common::run_temper_cli(
        &app,
        &["admin", "promote", &second_id.to_string(), "--format", "json"],
    )
    .await
    .expect("cli promote");
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));

    // The second user is now a system admin: can read admin settings (200).
    let resp = app
        .reqwest_client
        .get(app.url("/api/access/admin/settings"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("second admin settings");
    assert_eq!(resp.status(), StatusCode::OK, "promoted admin should have access");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_admin_is_forbidden_on_all_admin_endpoints(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    let second_token = common::generate_second_user_jwt();
    let second_id = provision(&app, &second_token).await;

    root_bootstrap_first_admin(&pool, admin_id).await;

    // Second user is a watcher member (has system access, NOT admin).
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) \
         SELECT id, $1, 'watcher' FROM kb_teams WHERE slug='temper-system' \
         ON CONFLICT (team_id, profile_id) DO NOTHING",
    )
    .bind(second_id)
    .execute(&pool)
    .await
    .expect("watcher");

    // GET admin settings → 403 FORBIDDEN.
    let resp = app
        .reqwest_client
        .get(app.url("/api/access/admin/settings"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("get");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: Value = resp.json().await.expect("json");
    assert_eq!(body["error"]["code"], "FORBIDDEN");

    // PATCH admin settings → 403.
    let resp = app
        .reqwest_client
        .patch(app.url("/api/access/admin/settings"))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&json!({"instance_name": "hijack"}))
        .send()
        .await
        .expect("patch");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // POST promote → 403.
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/admin/promote"))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&json!({"profile_id": admin_id}))
        .send()
        .await
        .expect("promote");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
```

- [ ] **Step 2: Reinstall the `temper` binary the e2e CLI runner invokes**

The e2e `run_temper_cli` spawns the compiled `temper` binary resolved relative to `current_exe()` — a `cargo nextest`/`cargo test` build rebuilds it, so no manual install is needed for the test run itself. (Manual PATH reinstall is only relevant after merge — see Task 8.)

- [ ] **Step 3: Run the e2e suite**

Run: `cargo make test-e2e`
Expected: PASS, including the two new `admin_surface_e2e` tests and no regression in `access_gate_test`.

> If only this file is desired during iteration:
> `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db --test admin_surface_e2e`

- [ ] **Step 4: Commit**

```bash
cargo fmt
git add tests/e2e/tests/admin_surface_e2e.rs
git commit -m "Chunk 6: e2e access-semantics gate (settings, promote, forbidden)"
```

---

## Task 8: Full verification + ship

**Files:** none (verification + PR).

- [ ] **Step 1: Regenerate workspace + per-crate sqlx caches (belt-and-suspenders)**

Run: `cargo sqlx prepare --workspace -- --all-features` then `cargo make prepare-api`
Expected: no diff beyond what Tasks 2/3 already produced; commit any delta.

- [ ] **Step 2: Full quality gate (honest offline probe)**

Run: `cargo make check`
Expected: fmt clean, clippy `-D warnings` clean, machete clean, TS unaffected. (Recall `cargo make` forces `SQLX_OFFLINE=true`, so this validates the committed caches.)

- [ ] **Step 3: Unit + integration tests**

Run: `cargo make test` then `cargo nextest run -p temper-api --features test-db --test admin_settings_test`
Expected: all PASS.

- [ ] **Step 4: e2e (access-semantics tier — mandatory)**

Run: `cargo make test-e2e`
Expected: PASS. (No embed feature needed — the admin path does no embedding.)

- [ ] **Step 5: Verify the e2e CLI bootstrap under a clean HOME (CI parity)**

Run: `HOME=$(mktemp -d) cargo make test-e2e`
Expected: PASS (guards the `run_temper_cli` env-wiring against hidden `~/.config` dependencies — see `feedback_e2e_cli_binary_bootstrap`).

- [ ] **Step 6: Push + open PR (never merge to main locally)**

```bash
git checkout -b jct/org-provisioning-chunk6-admin-settings
git merge origin/main   # surface sibling-PR drift before pushing
git push -u origin jct/org-provisioning-chunk6-admin-settings
gh pr create --title "org-provisioning chunk 6: admin / system-settings surface" --body "<summary>"
```

- [ ] **Step 7: After merge — reinstall the PATH binary**

```bash
cargo install --path crates/temper-cli
```

(A merged-but-not-installed CLI change behaves like the old bug — see `feedback_reinstall_temper_after_cli_merge`.)

---

## Self-Review

**Spec coverage (spec §4 Chunk 6):**
- ✅ Admin-gated `PATCH /api/access/admin/settings` (access_mode, gating_team_slug, instance_name, terms) → Tasks 2, 4; CLI `temper admin settings` → Task 6.
- ✅ `temper admin promote <profile>` = owner-grant (defaults to gating team, decoupled from `system_access`) → Tasks 2, 4, 6. Resolved-design `--team` reuses `resolve_team_id` for `+slug`/slug/UUID.
- ✅ `temper admin requests {list,review}` binding the existing handlers → Tasks 5, 6 (client + CLI over the already-shipped `list_pending`/`review_request`).
- ✅ Deferred Chunk-1 hook: `review_request` approval → `ensure_auto_join_memberships` → Task 3.
- ✅ First admin + initial gating remain the 2-UPDATE SQL root (documented in this plan + exercised as the e2e `root_bootstrap_first_admin` helper), not surfaced.
- ✅ Gate: e2e — root mints first admin → admin promotes second admin via surface → settings round-trip → non-admin Forbidden on all → invite_only approval enrolls profile → Tasks 3, 7.

**Type consistency:** `UpdateSettingsRequest`/`PromoteAdminRequest` (Task 1) are consumed identically in service (Task 2), handlers (Task 4), client (Task 5), CLI (Task 6). `SystemSettings` gains `Deserialize` (Task 1) so the client (Task 5) can decode the admin GET. `TeamMemberRow` is the promote return across service/handler/client. `JoinRequestStatus` is shared by CLI review and client.

**Placeholder scan:** none — every step carries complete code or an exact command.

**Open implementation choices flagged inline (resolve at code time, not blockers):**
- Prefer the `settings_remote(client, req, fmt)` signature over the per-field form (drops the `#[expect(too_many_arguments)]`, satisfies the params-struct rule). Task 6 Step 4 already passes the struct.
- Confirm the `ProfileId` import path used by `access_service.rs` and reuse it verbatim in the new test.
