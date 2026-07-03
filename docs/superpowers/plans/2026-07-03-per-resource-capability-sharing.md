# Per-resource Capability Sharing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a per-resource capability-grant surface (API + CLI + MCP) over the existing polymorphic `kb_access_grants` store, plus a `can()`-seam fix so a resource **owner** can administer grants on their own resource.

**Architecture:** The grant model already exists end-to-end — `kb_access_grants` (subject-polymorphic), `access_service::grant_capability`/`revoke_capability` (already accept `subject_table="kb_resources"`, auth-gated by `can_administer_grant`), and the read consumers `resources_visible_to`/`can_modify_resource`. This work adds only the **resource surface** on top (mirroring the cogmap grant surface) and closes one gap: `derived_access_profile` has no `'grant'` arm for resources, so a bare owner fails `can_administer_grant`. We add that arm (owner ⇒ grant) in the SQL seam.

**Tech Stack:** Rust (axum, sqlx, clap, rmcp), PostgreSQL 18 + pgvector, cargo-make + cargo-nextest.

## Global Constraints

- **Mirror, don't invent.** Every layer copies the cogmap grant precedent verbatim, swapping `subject_table "kb_cogmaps" → "kb_resources"` and the path `/api/cognitive-maps/{id}/grants → /api/resources/{id}/grants`. Cited precedent lines below.
- **Service-direct writes.** Grants are admin events — call `access_service::grant_capability`/`revoke_capability` directly from each surface. Do NOT route through `DbBackend`/operations. (`access_service.rs:51-56`.)
- **Coherence rule everywhere:** `can_read = read || write || grant` (a write/grant grant forces read on). `can_delete` stays `false` from the CLI/MCP surfaces (matches cogmap; the DB CHECK is `(write|delete|grant) ⇒ read`).
- **Owner seam scope:** `kb_resource_homes.owner_profile_id` ONLY — never `originator_profile_id` (provenance ≠ access).
- **Additive-only-on-main:** the migration is a `CREATE OR REPLACE FUNCTION` (non-destructive DDL).
- **Types at boundaries:** typed structs with the `ts_rs`/`utoipa`/`schemars` quad-derive; no `serde_json::json!()` for structured wire data.
- **Quality gate:** `cargo make check` (fmt + clippy `-D warnings` + docs + machete + TS) must pass before each commit. Follow TDD (red → green).
- **Reuse existing helpers:** `temper_workflow::operations::parse_ref` (trailing-UUID-only ref resolution), `crate::actions::cogmap::resolve_principal` (exactly-one-of profile/team).

---

## File Structure

- `migrations/<ts>_resource_grant_owner_seam.sql` — **new**. `CREATE OR REPLACE derived_access_profile` + owner⇒grant arm.
- `crates/temper-core/src/types/resource_grant.rs` — **new**. `ResourceGrantBody` / `ResourceRevokeBody`.
- `crates/temper-core/src/types/mod.rs` — **modify**. Register the module.
- `crates/temper-api/src/handlers/resources.rs` — **modify**. Add `grant` / `revoke` handlers.
- `crates/temper-api/src/routes.rs` — **modify**. Add the `/api/resources/{id}/grants` route.
- `crates/temper-api/src/openapi.rs` — **modify**. Register the two paths + two schemas.
- `crates/temper-client/src/resources.rs` — **modify**. Add `grant` / `revoke` client methods.
- `crates/temper-cli/src/cli.rs` — **modify**. Add `ResourceAction::Grant` / `Revoke`.
- `crates/temper-cli/src/main.rs` — **modify**. Dispatch the two variants.
- `crates/temper-cli/src/commands/resource.rs` — **modify**. `grant` / `revoke` command fns.
- `crates/temper-mcp/src/tools/resources.rs` — **modify**. `resource_grant` / `resource_revoke` tool fns + inputs.
- `crates/temper-mcp/src/service.rs` — **modify**. Register the two `#[tool]` methods.
- `tests/e2e/tests/resource_grant_e2e.rs` — **new**. Acceptance test through the real CLI.

---

## Task 1: Owner-grant SQL seam (migration)

Closes the one behavioral gap: make `can('kb_profiles', owner, 'grant', 'kb_resources', res)` true so the unchanged `can_administer_grant` passes for a resource owner.

**Files:**
- Create: `migrations/<ts>_resource_grant_owner_seam.sql`
- Test: `tests/e2e/tests/resource_grant_e2e.rs` (seam-only test in this task; the full acceptance test is Task 6)

**Interfaces:**
- Produces: no Rust symbols. SQL effect only: `derived_access_profile(profile,'grant','kb_resources',res)` returns `true` iff `profile` owns `res`.

- [ ] **Step 1: Pick a strictly-increasing migration timestamp**

Run: `ls migrations | tail -3`
Expected: the latest is `20260703140000_*`. Name the new file with a strictly greater stamp, e.g. `20260704000001_resource_grant_owner_seam.sql`. Use that exact name for every path below.

- [ ] **Step 2: Write the failing seam test**

Create `tests/e2e/tests/resource_grant_e2e.rs`:

```rust
#![cfg(feature = "test-db")]
mod common;

use serde_json::Value;
use temper_core::types::team::{AddMemberRequest, TeamCreateRequest, TeamRole};
use uuid::Uuid;

/// GET /api/profile → this token's profile UUID (mints the profile on first hit).
async fn provision(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight");
    let body: Value = resp.json().await.expect("json");
    body["id"].as_str().expect("id").parse().expect("uuid")
}

/// POST /api/ingest → the new resource's UUID. Homes the resource in `context_id`.
async fn ingest_into_context(
    app: &common::E2eTestApp,
    token: &str,
    context_id: Uuid,
    title: &str,
    slug: &str,
) -> Uuid {
    let resp = app
        .reqwest_client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "title": title,
            "origin_uri": format!("test://resource-grant-e2e/{}", Uuid::new_v4()),
            "context_ref": context_id.to_string(),
            "doc_type_name": "research",
            "slug": slug,
            "content": "A resource owned by the granter, shared to a team by capability grant.",
        }))
        .send()
        .await
        .expect("ingest request failed");
    assert_eq!(resp.status(), reqwest::StatusCode::OK, "owner ingests into own context");
    let body: Value = resp.json().await.expect("ingest json");
    body["id"].as_str().expect("resource id").parse().expect("uuid")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn owner_can_administer_grant_seam(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let owner_id = provision(&app, &app.token).await;
    let stranger_token = common::generate_second_user_jwt();
    let stranger_id = provision(&app, &stranger_token).await;

    let context = app.client.contexts().create("seam-ctx", None).await.expect("ctx");
    let resource_id =
        ingest_into_context(&app, &app.token, *context.id, "seam doc", "seam-doc").await;

    // The owner-grant seam: the resource's owner CAN administer grants; a stranger cannot.
    let owner_can: Option<bool> = sqlx::query_scalar(
        "SELECT can('kb_profiles', $1, 'grant', 'kb_resources', $2)",
    )
    .bind(owner_id)
    .bind(resource_id)
    .fetch_one(&pool)
    .await
    .expect("can() query");
    assert_eq!(owner_can, Some(true), "resource owner may administer grants (the new seam)");

    let stranger_can: Option<bool> = sqlx::query_scalar(
        "SELECT can('kb_profiles', $1, 'grant', 'kb_resources', $2)",
    )
    .bind(stranger_id)
    .bind(resource_id)
    .fetch_one(&pool)
    .await
    .expect("can() query");
    assert_eq!(stranger_can, Some(false), "a non-owner, non-admin cannot administer grants");
}
```

- [ ] **Step 3: Run it to verify it fails (seam absent)**

Run: `cargo make docker-up && cargo nextest run -p temper-e2e --features test-db owner_can_administer_grant_seam`
Expected: FAIL — `owner_can` is `Some(false)` because `derived_access_profile` has no `'grant'` arm yet.

> Note: the e2e crate's package name is `temper-e2e` (confirm with `grep '^name' tests/e2e/Cargo.toml`). If nextest can't find it, run from the repo root with `cargo make test-e2e` filtered, or `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db owner_can_administer_grant_seam`.

- [ ] **Step 4: Write the migration**

Create `migrations/<ts>_resource_grant_owner_seam.sql` — reproduces the current `derived_access_profile` body (`migrations/20260630000001_access_grants_seam.sql:75-91`, the only definition, never redefined) verbatim, adding ONE arm:

```sql
-- Owner-grant seam for per-resource capability sharing.
--
-- `derived_access_profile` (the non-explicit-grant reach behind `can()`) had no 'grant'
-- arm for resources, so a bare resource OWNER failed `can_administer_grant` and could not
-- share their own resource. Add owner ⇒ grant, symmetric with `can_modify_resource`'s
-- "the home confers modify to its principals". Scoped to owner_profile_id ONLY —
-- originator is provenance, not access. Non-destructive CREATE OR REPLACE (additive-only-on-main).
CREATE OR REPLACE FUNCTION derived_access_profile(
    p_profile uuid, p_action text, p_subject_table text, p_subject_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE
        WHEN p_subject_table = 'kb_resources' AND p_action = 'read'  THEN
            p_subject_id IN (SELECT resource_id FROM resources_visible_to(p_profile))
        WHEN p_subject_table = 'kb_resources' AND p_action = 'write' THEN
            can_modify_resource(p_profile, p_subject_id)
        WHEN p_subject_table = 'kb_resources' AND p_action = 'grant' THEN
            EXISTS (SELECT 1 FROM kb_resource_homes h
                    WHERE h.resource_id = p_subject_id
                      AND h.owner_profile_id = p_profile)
        WHEN p_subject_table = 'kb_cogmaps'  AND p_action = 'read'  THEN
            cogmap_readable_by_profile(p_profile, p_subject_id)
        WHEN p_subject_table = 'kb_cogmaps'  AND p_action = 'write' THEN
            cogmap_authorable_by_profile(p_profile, p_subject_id)
        WHEN p_subject_table = 'kb_contexts' AND p_action = 'read'  THEN
            context_visible_to(p_profile, p_subject_id)
        ELSE false
    END;
$$;
```

> ⚠️ Plan/reality guard: before writing, re-read `migrations/20260630000001_access_grants_seam.sql:75-91` and copy the `WHEN`-arms EXACTLY (function names `cogmap_readable_by_profile`, `cogmap_authorable_by_profile`, `context_visible_to`). If any arm differs from what's shown here, use the on-disk version — only ADD the `'grant'` arm.

- [ ] **Step 5: Run the seam test to verify it passes**

Run: `cargo nextest run -p temper-e2e --features test-db owner_can_administer_grant_seam`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add migrations/*_resource_grant_owner_seam.sql tests/e2e/tests/resource_grant_e2e.rs
git commit -m "feat(access): owner-grant seam for per-resource capability sharing

derived_access_profile gains a resource 'grant' arm (owner_profile_id ⇒ grant),
so the shared can_administer_grant passes for a resource owner. Additive
CREATE OR REPLACE. Scope #4 of the Teams-in-Temper goal.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Wire types (`ResourceGrantBody` / `ResourceRevokeBody`)

**Files:**
- Create: `crates/temper-core/src/types/resource_grant.rs`
- Modify: `crates/temper-core/src/types/mod.rs` (add `pub mod resource_grant;` in alpha order, between `reconcile` and `relationship_events`)
- Test: inline `#[cfg(test)]` in the new file.

**Interfaces:**
- Produces: `temper_core::types::resource_grant::ResourceGrantBody { principal_table: String, principal_id: Uuid, can_read: bool, can_write: bool, can_delete: bool, can_grant: bool }` and `ResourceRevokeBody { principal_table: String, principal_id: Uuid }`. Reuses `temper_core::types::cognitive_maps::{GrantOutcome, RevokeOutcome}` for responses.

- [ ] **Step 1: Write the failing serde test + the types**

Create `crates/temper-core/src/types/resource_grant.rs`:

```rust
//! HTTP/MCP body types for per-resource capability grants
//! (`POST/DELETE /api/resources/{id}/grants`). The subject is the path `{id}` (a resource),
//! so the body carries only the principal + capabilities. Handlers/tools widen these into a
//! `GrantCapabilityRequest`/`RevokeCapabilityRequest` with `subject_table = "kb_resources"`.
//! Structurally parallel to `CogmapGrantBody`/`CogmapRevokeBody`. Responses reuse the shared
//! `GrantOutcome`/`RevokeOutcome`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Mint/update a `kb_access_grants` row on a resource. Principal `{kb_teams,kb_profiles}`.
/// The DB coherence CHECK enforces `write|delete|grant ⇒ read`; pass a coherent set
/// (a write grant implies read).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource_grant.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceGrantBody {
    pub principal_table: String,
    pub principal_id: Uuid,
    pub can_read: bool,
    pub can_write: bool,
    pub can_delete: bool,
    pub can_grant: bool,
}

/// Delete a `kb_access_grants` row on a resource (the `(subject, principal)` pair). Absent ⇒ no-op.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource_grant.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRevokeBody {
    pub principal_table: String,
    pub principal_id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_grant_body_roundtrips() {
        let id = Uuid::now_v7();
        let body = ResourceGrantBody {
            principal_table: "kb_teams".to_string(),
            principal_id: id,
            can_read: true,
            can_write: true,
            can_delete: false,
            can_grant: false,
        };
        let json = serde_json::to_string(&body).unwrap();
        let back: ResourceGrantBody = serde_json::from_str(&json).unwrap();
        assert_eq!(back.principal_id, id);
        assert!(back.can_read && back.can_write && !back.can_grant);
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/temper-core/src/types/mod.rs`, add (keeping alpha order, after `pub mod reconcile;` / line 40):

```rust
pub mod resource_grant;
```

- [ ] **Step 3: Run the test**

Run: `cargo nextest run -p temper-core resource_grant_body_roundtrips`
Expected: PASS.

- [ ] **Step 4: Regenerate TS types (the new `resource_grant.ts` export)**

Run: `cargo make generate-ts-types`
Expected: creates `resource_grant.ts` under the generated types dir; no errors.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/resource_grant.rs crates/temper-core/src/types/mod.rs
git add -A  # picks up the generated resource_grant.ts
git commit -m "feat(types): ResourceGrantBody/ResourceRevokeBody for resource grants

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: API handlers + route + OpenAPI

**Files:**
- Modify: `crates/temper-api/src/handlers/resources.rs` (append two handlers)
- Modify: `crates/temper-api/src/routes.rs` (one route line)
- Modify: `crates/temper-api/src/openapi.rs` (2 paths + 2 schemas)

**Interfaces:**
- Consumes: `temper_core::types::resource_grant::{ResourceGrantBody, ResourceRevokeBody}` (Task 2); `temper_core::types::cognitive_maps::{GrantCapabilityRequest, RevokeCapabilityRequest, GrantOutcome, RevokeOutcome}`; `access_service::{grant_capability, revoke_capability}`.
- Produces: `handlers::resources::grant`, `handlers::resources::revoke`; route `POST/DELETE /api/resources/{id}/grants`.

- [ ] **Step 1: Add the handlers**

Append to `crates/temper-api/src/handlers/resources.rs` (add the imports to the existing `use` block at the top of the file — `access_service` from `temper_services::services`, the two body types, and the four cogmap types):

```rust
// add to the existing top-of-file imports:
use temper_core::types::cognitive_maps::{
    GrantCapabilityRequest, GrantOutcome, RevokeCapabilityRequest, RevokeOutcome,
};
use temper_core::types::resource_grant::{ResourceGrantBody, ResourceRevokeBody};
use temper_services::services::access_service;
```

```rust
#[utoipa::path(
    post,
    path = "/api/resources/{id}/grants",
    tag = "Resources",
    params(("id" = Uuid, Path, description = "Resource ID")),
    request_body = ResourceGrantBody,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Grant minted (or updated in place)", body = GrantOutcome),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Caller may not administer grants on this resource", body = ErrorBody),
    )
)]
pub async fn grant(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
    Json(body): Json<ResourceGrantBody>,
) -> ApiResult<Json<GrantOutcome>> {
    // Auth-before-writes lives in the service (`is_system_admin OR can(...,'grant',...)`,
    // and — via the new seam — the resource owner). Widen the resource-scoped body into the
    // polymorphic request by injecting subject_table + the path id.
    let req = GrantCapabilityRequest {
        subject_table: "kb_resources".to_string(),
        subject_id: resource_id,
        principal_table: body.principal_table,
        principal_id: body.principal_id,
        can_read: body.can_read,
        can_write: body.can_write,
        can_delete: body.can_delete,
        can_grant: body.can_grant,
    };
    let outcome =
        access_service::grant_capability(&state.pool, ProfileId::from(auth.0.profile.id), &req)
            .await?;
    Ok(Json(outcome))
}

#[utoipa::path(
    delete,
    path = "/api/resources/{id}/grants",
    tag = "Resources",
    params(("id" = Uuid, Path, description = "Resource ID")),
    request_body = ResourceRevokeBody,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Grant revoked (no-op safe)", body = RevokeOutcome),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Caller may not administer grants on this resource", body = ErrorBody),
    )
)]
pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
    Json(body): Json<ResourceRevokeBody>,
) -> ApiResult<Json<RevokeOutcome>> {
    let req = RevokeCapabilityRequest {
        subject_table: "kb_resources".to_string(),
        subject_id: resource_id,
        principal_table: body.principal_table,
        principal_id: body.principal_id,
    };
    let outcome =
        access_service::revoke_capability(&state.pool, ProfileId::from(auth.0.profile.id), &req)
            .await?;
    Ok(Json(outcome))
}
```

- [ ] **Step 2: Add the route**

In `crates/temper-api/src/routes.rs`, mirror the cogmap grants line (`:186-189`). Add near the other `/api/resources/...` routes:

```rust
        .route(
            "/api/resources/{id}/grants",
            post(handlers::resources::grant).delete(handlers::resources::revoke),
        )
```

- [ ] **Step 3: Register OpenAPI paths + schemas**

In `crates/temper-api/src/openapi.rs`, in the `paths(...)` list (near `crate::handlers::resources::create` / `:26`) add:

```rust
        crate::handlers::resources::grant,
        crate::handlers::resources::revoke,
```

Then find the `components(schemas(...))` block (grep `CogmapGrantBody` in the file to locate it) and add:

```rust
        temper_core::types::resource_grant::ResourceGrantBody,
        temper_core::types::resource_grant::ResourceRevokeBody,
```

> `GrantOutcome`/`RevokeOutcome` are already registered for cogmap grants — reuse, don't re-add.

- [ ] **Step 4: Build + check**

Run: `cargo make check`
Expected: clean. (Compile confirms handler signatures + OpenAPI derive registration; behavioral coverage is the Task 6 e2e.)

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/handlers/resources.rs crates/temper-api/src/routes.rs crates/temper-api/src/openapi.rs
git commit -m "feat(api): POST/DELETE /api/resources/{id}/grants

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Client methods

**Files:**
- Modify: `crates/temper-client/src/resources.rs` (add two methods + imports)

**Interfaces:**
- Consumes: `ResourceGrantBody`, `ResourceRevokeBody`, `GrantOutcome`, `RevokeOutcome`.
- Produces: `ResourceClient::grant(id: Uuid, body: &ResourceGrantBody) -> Result<GrantOutcome>` and `ResourceClient::revoke(id: Uuid, body: &ResourceRevokeBody) -> Result<RevokeOutcome>`.

- [ ] **Step 1: Add the methods**

In `crates/temper-client/src/resources.rs`, extend the top imports:

```rust
use temper_core::types::cognitive_maps::{GrantOutcome, RevokeOutcome};
use temper_core::types::resource_grant::{ResourceGrantBody, ResourceRevokeBody};
```

Add inside `impl<'a> ResourceClient<'a>` (mirroring `cognitive_maps.rs:170-190`):

```rust
    /// POST /api/resources/{id}/grants — mint/update a capability grant on the resource
    /// (system-admin, a can_grant holder, OR the resource owner). `granted: false` ⇒ an
    /// existing grant was updated in place.
    pub async fn grant(&self, id: Uuid, body: &ResourceGrantBody) -> Result<GrantOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{id}/grants");
        let req = self.http.post(&path).json(body);
        self.http
            .send_json(&Method::POST, &path, req, Some(&token))
            .await
    }

    /// DELETE /api/resources/{id}/grants — revoke a capability grant (no-op safe).
    /// `revoked: false` ⇒ no matching grant existed.
    pub async fn revoke(&self, id: Uuid, body: &ResourceRevokeBody) -> Result<RevokeOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{id}/grants");
        let req = self.http.delete(&path).json(body);
        self.http
            .send_json(&Method::DELETE, &path, req, Some(&token))
            .await
    }
```

- [ ] **Step 2: Build**

Run: `cargo build -p temper-client`
Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-client/src/resources.rs
git commit -m "feat(client): ResourceClient grant/revoke

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: CLI subcommands

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (add `ResourceAction::Grant` / `Revoke`)
- Modify: `crates/temper-cli/src/main.rs` (dispatch)
- Modify: `crates/temper-cli/src/commands/resource.rs` (command fns)

**Interfaces:**
- Consumes: `crate::actions::cogmap::resolve_principal`, `temper_workflow::operations::parse_ref`, `client.resources().grant/revoke` (Task 4).
- Produces: `temper resource grant <ref> [--to-profile <uuid> | --to-team <ref>] [--read] [--write] [--grant]` and `temper resource revoke <ref> [--from-profile <uuid> | --from-team <ref>]`.

- [ ] **Step 1: Add the clap variants**

In `crates/temper-cli/src/cli.rs`, add to `enum ResourceAction` (mirror `CogmapCmd::Grant`/`Revoke` at `:919-948`; NOTE the divergence: `--to-team`/`--from-team` are `Option<String>` (a decorated ref: UUID or `slug-<uuid>`, resolved via `parse_ref`), while `--to-profile`/`--from-profile` stay `Option<Uuid>`):

```rust
    /// Grant a capability on a resource to a profile or team (system-admin, a can_grant
    /// holder, or the resource owner).
    Grant {
        /// Resource ref: a UUID or the decorated `slug-<uuid>` form.
        r#ref: String,
        /// Grant to this profile (UUID). Mutually exclusive with `--to-team`.
        #[arg(long = "to-profile")]
        to_profile: Option<uuid::Uuid>,
        /// Grant to this team (a UUID or decorated `slug-<uuid>` ref). Mutually exclusive
        /// with `--to-profile`.
        #[arg(long = "to-team")]
        to_team: Option<String>,
        /// Grant read.
        #[arg(long)]
        read: bool,
        /// Grant write (implies read).
        #[arg(long)]
        write: bool,
        /// Grant delegated-grant authority (implies read).
        #[arg(long)]
        grant: bool,
    },
    /// Revoke a capability grant on a resource (system-admin, a can_grant holder, or the owner).
    Revoke {
        /// Resource ref: a UUID or the decorated `slug-<uuid>` form.
        r#ref: String,
        /// Revoke this profile's grant (UUID). Mutually exclusive with `--from-team`.
        #[arg(long = "from-profile")]
        from_profile: Option<uuid::Uuid>,
        /// Revoke this team's grant (a UUID or decorated `slug-<uuid>` ref). Mutually
        /// exclusive with `--from-profile`.
        #[arg(long = "from-team")]
        from_team: Option<String>,
    },
```

- [ ] **Step 2: Add the command fns**

In `crates/temper-cli/src/commands/resource.rs`, add (mirroring `commands/cogmap.rs:94-138`, but resolving the team ref via `parse_ref` instead of `resolve_team_id`):

```rust
/// `temper resource grant <ref> --to-profile|--to-team <ref> [--read] [--write] [--grant]`.
#[allow(clippy::too_many_arguments)]
pub fn grant(
    r#ref: &str,
    to_profile: Option<uuid::Uuid>,
    to_team: Option<String>,
    read: bool,
    write: bool,
    grant_cap: bool,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let resource_id = temper_workflow::operations::parse_ref(r#ref)?.0;
    // A team ref is a decorated ref (UUID or `slug-<uuid>`); parse_ref strips the slug half
    // and keeps the trailing UUID — no slug-uniqueness lookup needed.
    let to_team_id = to_team
        .as_deref()
        .map(temper_workflow::operations::parse_ref)
        .transpose()?
        .map(|r| r.0);
    let principal = crate::actions::cogmap::resolve_principal(to_profile, to_team_id)?;

    let body = temper_core::types::resource_grant::ResourceGrantBody {
        principal_table: principal.table,
        principal_id: principal.id,
        can_read: read || write || grant_cap,
        can_write: write,
        can_delete: false,
        can_grant: grant_cap,
    };

    let outcome = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .grant(uuid::Uuid::from(resource_id), &body)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let rendered = crate::format::render(&outcome, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}

/// `temper resource revoke <ref> --from-profile|--from-team <ref>`.
pub fn revoke(
    r#ref: &str,
    from_profile: Option<uuid::Uuid>,
    from_team: Option<String>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let resource_id = temper_workflow::operations::parse_ref(r#ref)?.0;
    let from_team_id = from_team
        .as_deref()
        .map(temper_workflow::operations::parse_ref)
        .transpose()?
        .map(|r| r.0);
    let principal = crate::actions::cogmap::resolve_principal(from_profile, from_team_id)?;

    let body = temper_core::types::resource_grant::ResourceRevokeBody {
        principal_table: principal.table,
        principal_id: principal.id,
    };

    let outcome = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .revoke(uuid::Uuid::from(resource_id), &body)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let rendered = crate::format::render(&outcome, fmt)?;
    crate::output::plain(rendered);
    Ok(())
}
```

> ⚠️ Plan/reality guards: (1) `parse_ref(x)?.0` yields a `ResourceId`-newtype whose `.0` is a `Uuid` in the cogmap path, but `client.resources().grant` takes a `Uuid` — the code above wraps with `uuid::Uuid::from(resource_id)`. If `parse_ref(...).0` is already a bare `Uuid` (as in `commands/cogmap.rs` where it's passed straight to a `Uuid` param), drop the `uuid::Uuid::from(...)` wrapper and pass `resource_id` directly. Verify which by reading `parse_ref`'s return type before writing. (2) Confirm the error-map helper name: `reassign` uses `crate::actions::runtime::client_err_to_temper`; cogmap uses `crate::commands::client_err`. Use whichever the surrounding `commands/resource.rs` already imports (check `reassign` in that file — it uses `client_err_to_temper`).

- [ ] **Step 3: Dispatch in main.rs**

In `crates/temper-cli/src/main.rs`, in the `ResourceAction` match (near `Reassign` at `:279`), add:

```rust
                ResourceAction::Grant {
                    r#ref,
                    to_profile,
                    to_team,
                    read,
                    write,
                    grant,
                } => temper_cli::commands::resource::grant(
                    &r#ref,
                    to_profile,
                    to_team,
                    read,
                    write,
                    grant,
                    output_format,
                ),
                ResourceAction::Revoke {
                    r#ref,
                    from_profile,
                    from_team,
                } => temper_cli::commands::resource::revoke(
                    &r#ref,
                    from_profile,
                    from_team,
                    output_format,
                ),
```

- [ ] **Step 4: Verify parsing + build**

Run: `cargo build -p temper-cli --bin temper && ./target/debug/temper resource grant --help`
Expected: help shows `--to-profile`, `--to-team`, `--read`, `--write`, `--grant`. `cargo make check` clean.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs crates/temper-cli/src/commands/resource.rs
git commit -m "feat(cli): temper resource grant/revoke

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: End-to-end acceptance (through the real CLI)

**Files:**
- Modify: `tests/e2e/tests/resource_grant_e2e.rs` (add the acceptance test alongside the Task 1 seam test; reuses its `provision` / `ingest_into_context` helpers)

**Interfaces:**
- Consumes: everything from Tasks 1–5, driven through the compiled `temper` binary via `common::run_temper_cli`.

- [ ] **Step 1: Write the acceptance test**

Append to `tests/e2e/tests/resource_grant_e2e.rs`:

```rust
/// Full-stack acceptance: a NON-admin resource owner grants a team read/write via the real
/// CLI, a team member gains visibility + modify, and revoke reverses it. Team creation is
/// open (no admin bootstrap), so the owner is never a system admin — this proves the seam.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn owner_grants_resource_to_team_via_cli_and_revokes(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Owner drives app.client/app.token (NON-admin). Member is a second profile.
    let _owner_id = provision(&app, &app.token).await;
    let member_token = common::generate_second_user_jwt();
    let member_id = provision(&app, &member_token).await;

    // Owner creates a team (becomes its owner — team creation is open) and adds the member.
    let team = app
        .client
        .teams()
        .create(&TeamCreateRequest {
            slug: "grant-team".to_owned(),
            name: None,
            parent: None,
            auto_join_role: None,
        })
        .await
        .expect("owner creates team");
    app.client
        .teams()
        .add_member(
            team.id,
            &AddMemberRequest { profile_id: member_id, role: TeamRole::Member },
        )
        .await
        .expect("owner adds member");

    // Owner homes a resource in their own context.
    let context = app.client.contexts().create("grant-ctx", None).await.expect("ctx");
    let resource_id =
        ingest_into_context(&app, &app.token, *context.id, "grant doc", "grant-doc").await;
    let resource_str = resource_id.to_string();
    let team_str = team.id.to_string();

    // Oracles.
    let show_status = |token: String, res: Uuid| {
        let app = &app;
        async move {
            app.reqwest_client
                .get(app.url(&format!("/api/resources/{res}")))
                .header("Authorization", format!("Bearer {token}"))
                .send()
                .await
                .expect("show")
                .status()
        }
    };
    async fn can_modify(pool: &sqlx::PgPool, profile: Uuid, res: Uuid) -> bool {
        sqlx::query_scalar::<_, bool>("SELECT can_modify_resource($1, $2)")
            .bind(profile)
            .bind(res)
            .fetch_one(pool)
            .await
            .expect("can_modify_resource")
    }

    // Pre-grant: member cannot see the resource.
    assert_eq!(
        show_status(member_token.clone(), resource_id).await,
        reqwest::StatusCode::NOT_FOUND,
        "member cannot see the resource before any grant"
    );

    // Owner grants READ to the team via the real CLI.
    let out = common::run_temper_cli(
        &app,
        &["resource", "grant", &resource_str, "--to-team", &team_str, "--read"],
    )
    .await
    .expect("spawn temper cli");
    assert!(
        out.status.success(),
        "grant --read failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // Member now sees it; but cannot modify (read-only grant).
    assert_eq!(
        show_status(member_token.clone(), resource_id).await,
        reqwest::StatusCode::OK,
        "member sees the resource after a read grant to their team"
    );
    assert!(!can_modify(&pool, member_id, resource_id).await, "read grant does not confer modify");

    // Owner upgrades to WRITE via the CLI → member can modify.
    let out = common::run_temper_cli(
        &app,
        &["resource", "grant", &resource_str, "--to-team", &team_str, "--write"],
    )
    .await
    .expect("spawn temper cli");
    assert!(out.status.success(), "grant --write failed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(can_modify(&pool, member_id, resource_id).await, "write grant confers modify");

    // Owner revokes via the CLI → visibility + modify gone.
    let out = common::run_temper_cli(
        &app,
        &["resource", "revoke", &resource_str, "--from-team", &team_str],
    )
    .await
    .expect("spawn temper cli");
    assert!(out.status.success(), "revoke failed: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        show_status(member_token.clone(), resource_id).await,
        reqwest::StatusCode::NOT_FOUND,
        "member loses visibility after revoke"
    );
    assert!(!can_modify(&pool, member_id, resource_id).await, "revoke removes modify");

    // Decorated-ref strip: grant using `some-slug-<uuid>` for --to-team still resolves.
    let decorated_team = format!("any-slug-here-{team_str}");
    let out = common::run_temper_cli(
        &app,
        &["resource", "grant", &resource_str, "--to-team", &decorated_team, "--read"],
    )
    .await
    .expect("spawn temper cli");
    assert!(out.status.success(), "decorated --to-team ref failed: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        show_status(member_token, resource_id).await,
        reqwest::StatusCode::OK,
        "decorated team ref (slug-<uuid>) resolves via parse_ref"
    );
}
```

> ⚠️ Plan/reality guards: (1) confirm `common::run_temper_cli(&app, &[...])` signature and `common::generate_second_user_jwt()` exist (they're used by `reassign_test.rs` / `context_share_e2e.rs`). (2) `run_temper_cli` spawns the compiled binary — rebuild it first (`cargo build -p temper-cli --bin temper`) or the test drives a stale `temper` (see memory: local e2e uses a stale bin). (3) If the closure-based `show_status` borrow of `app` fights the borrow checker, inline the GET at each call site instead.

- [ ] **Step 2: Rebuild the CLI binary, then run the acceptance test**

Run:
```bash
cargo build -p temper-cli --bin temper
cargo nextest run -p temper-e2e --features test-db owner_grants_resource_to_team_via_cli_and_revokes
```
Expected: PASS.

- [ ] **Step 3: Run the whole e2e file (both tests) to confirm no interference**

Run: `cargo nextest run -p temper-e2e --features test-db resource_grant`
Expected: both `owner_can_administer_grant_seam` and `owner_grants_resource_to_team_via_cli_and_revokes` PASS.

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/tests/resource_grant_e2e.rs
git commit -m "test(e2e): resource grant/revoke through the real CLI (owner→team visibility+modify)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: MCP tools (surface parity)

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs` (add inputs + `resource_grant`/`resource_revoke` fns + local `resolve_principal`/`map_api_error`)
- Modify: `crates/temper-mcp/src/service.rs` (register two `#[tool]` methods)

**Interfaces:**
- Consumes: `access_service::{grant_capability, revoke_capability}`, `GrantCapabilityRequest`/`RevokeCapabilityRequest`, `svc.require_profile()`, `svc.api_state.pool`.
- Produces: MCP tools `resource_grant`, `resource_revoke`.

- [ ] **Step 1: Add the tool inputs + functions**

In `crates/temper-mcp/src/tools/resources.rs`, add imports as needed (`GrantCapabilityRequest`/`RevokeCapabilityRequest` from `temper_core::types::cognitive_maps`, `ProfileId` from `temper_core::types::ids`, `access_service` from `temper_services::services`, `ApiError` from `temper_services::error`, `CallToolResult`/`Content` per the crate's existing rmcp imports) and append (mirroring `tools/cognitive_maps.rs:392-511`):

```rust
/// MCP input for resource_grant. `resource` is a ref; exactly one of `to_profile`/`to_team`
/// (raw UUID) names the principal. At least one capability must be set (read implied by write/grant).
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ResourceGrantInput {
    /// The resource, by ref (UUID or `slug-<uuid>`).
    pub resource: String,
    #[serde(default)]
    pub to_profile: Option<uuid::Uuid>,
    #[serde(default)]
    pub to_team: Option<uuid::Uuid>,
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub write: bool,
    #[serde(default)]
    pub grant: bool,
}

/// MCP input for resource_revoke. `resource` is a ref; exactly one of `from_profile`/`from_team`.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ResourceRevokeInput {
    pub resource: String,
    #[serde(default)]
    pub from_profile: Option<uuid::Uuid>,
    #[serde(default)]
    pub from_team: Option<uuid::Uuid>,
}

/// Resolve exactly one of (profile, team) into `(principal_table, principal_id)`.
fn resolve_principal(
    profile: Option<uuid::Uuid>,
    team: Option<uuid::Uuid>,
) -> Result<(String, uuid::Uuid), rmcp::ErrorData> {
    match (profile, team) {
        (Some(p), None) => Ok(("kb_profiles".to_string(), p)),
        (None, Some(t)) => Ok(("kb_teams".to_string(), t)),
        (Some(_), Some(_)) => Err(rmcp::ErrorData::invalid_params(
            "supply exactly one principal, not both a profile and a team".to_string(),
            None,
        )),
        (None, None) => Err(rmcp::ErrorData::invalid_params(
            "no principal — supply exactly one of a profile or a team".to_string(),
            None,
        )),
    }
}

fn map_api_error(context: &str, err: temper_services::error::ApiError) -> rmcp::ErrorData {
    match err {
        temper_services::error::ApiError::Forbidden => rmcp::ErrorData::invalid_params(
            format!("{context}: caller may not administer grants on this resource"),
            None,
        ),
        other => rmcp::ErrorData::internal_error(format!("{context} failed: {other}"), None),
    }
}

/// Grant a capability on a resource. SERVICE-DIRECT, gated by `is_system_admin OR can_grant OR
/// owner`. `read` forced on when `write`/`grant` is set.
pub async fn resource_grant(
    svc: &TemperMcpService,
    input: ResourceGrantInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let resource_id = temper_workflow::operations::parse_ref(&input.resource)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad resource ref: {e}"), None))?
        .0;
    let (principal_table, principal_id) = resolve_principal(input.to_profile, input.to_team)?;
    if !(input.read || input.write || input.grant) {
        return Err(rmcp::ErrorData::invalid_params(
            "no capability selected — set at least one of read/write/grant".to_string(),
            None,
        ));
    }
    let req = temper_core::types::cognitive_maps::GrantCapabilityRequest {
        subject_table: "kb_resources".to_string(),
        subject_id: uuid::Uuid::from(resource_id),
        principal_table,
        principal_id,
        can_read: input.read || input.write || input.grant,
        can_write: input.write,
        can_delete: false,
        can_grant: input.grant,
    };
    let outcome = temper_services::services::access_service::grant_capability(
        &svc.api_state.pool,
        temper_core::types::ids::ProfileId::from(profile.id),
        &req,
    )
    .await
    .map_err(|e| map_api_error("resource_grant", e))?;
    let text = serde_json::to_string_pretty(&outcome).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(text)]))
}

/// Revoke a capability grant on a resource. SERVICE-DIRECT, admin/can_grant/owner-gated. No-op safe.
pub async fn resource_revoke(
    svc: &TemperMcpService,
    input: ResourceRevokeInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let resource_id = temper_workflow::operations::parse_ref(&input.resource)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad resource ref: {e}"), None))?
        .0;
    let (principal_table, principal_id) = resolve_principal(input.from_profile, input.from_team)?;
    let req = temper_core::types::cognitive_maps::RevokeCapabilityRequest {
        subject_table: "kb_resources".to_string(),
        subject_id: uuid::Uuid::from(resource_id),
        principal_table,
        principal_id,
    };
    let outcome = temper_services::services::access_service::revoke_capability(
        &svc.api_state.pool,
        temper_core::types::ids::ProfileId::from(profile.id),
        &req,
    )
    .await
    .map_err(|e| map_api_error("resource_revoke", e))?;
    let text = serde_json::to_string_pretty(&outcome).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(text)]))
}

#[cfg(test)]
mod resource_grant_tests {
    use super::*;

    #[test]
    fn resource_grant_input_deserializes() {
        let id = uuid::Uuid::now_v7();
        let raw = serde_json::json!({ "resource": "r", "to_team": id.to_string(), "write": true });
        let input: ResourceGrantInput = serde_json::from_value(raw).unwrap();
        assert_eq!(input.to_team, Some(id));
        assert!(input.write);
        assert!(!input.grant);
    }
}
```

> ⚠️ Plan/reality guards: (1) match the existing `use` style in `tools/resources.rs` — it may already import `CallToolResult`, `TemperMcpService`, etc.; don't duplicate. If `tools/cognitive_maps.rs` already exposes a `map_api_error`/`resolve_principal` you can make `pub(crate)` and reuse instead of copying, prefer that (DRY) — but only if it's a one-line visibility change, else copy locally. (2) `parse_ref(...).0` newtype→Uuid: `uuid::Uuid::from(resource_id)` as above; drop the wrapper if `.0` is already `Uuid`.

- [ ] **Step 2: Register the tools in service.rs**

In `crates/temper-mcp/src/service.rs`, inside the `#[tool_router] impl TemperMcpService` block (mirror `:347-369`), add:

```rust
    #[tool(
        description = "Grant a capability on a resource to a profile or team (system-admin, a can_grant holder, OR the resource owner). Pass the resource by ref, exactly one principal (to_profile or to_team by UUID), and capability flags (read/write/grant; read is implied by write/grant)."
    )]
    async fn resource_grant(
        &self,
        Parameters(input): Parameters<tools::resources::ResourceGrantInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::resources::resource_grant(self, input).await
    }

    #[tool(
        description = "Revoke a capability grant on a resource (system-admin, a can_grant holder, or the resource owner). No-op safe. Pass the resource by ref and exactly one principal (from_profile or from_team by UUID)."
    )]
    async fn resource_revoke(
        &self,
        Parameters(input): Parameters<tools::resources::ResourceRevokeInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::resources::resource_revoke(self, input).await
    }
```

- [ ] **Step 3: Build + unit test**

Run: `cargo nextest run -p temper-mcp resource_grant_input_deserializes && cargo build -p temper-mcp`
Expected: PASS + compiles (the `#[tool_router]` macro auto-collects the new methods; no registry list to edit).

- [ ] **Step 4: Commit**

```bash
git add crates/temper-mcp/src/tools/resources.rs crates/temper-mcp/src/service.rs
git commit -m "feat(mcp): resource_grant/resource_revoke tools (surface parity)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: sqlx cache + full verification

**Files:** none (verification + possible cache regen).

- [ ] **Step 1: Regenerate the sqlx cache (defensive)**

The production surface adds NO new Rust `query!` macros (handlers reuse `access_service`; the e2e uses runtime `sqlx::query_scalar`). So the workspace cache is likely unchanged. Confirm honestly:

Run: `cargo make check`
Expected: clean. If it reports a missing/stale `.sqlx` entry, run `cargo sqlx prepare --workspace -- --all-features` (and `cargo make prepare-e2e` only if a new `query!` macro landed in a test target), then re-run `cargo make check`.

- [ ] **Step 2: Targeted crate suites**

Run:
```bash
cargo nextest run -p temper-core resource_grant
cargo nextest run -p temper-mcp resource_grant
cargo build -p temper-cli --bin temper
cargo nextest run -p temper-e2e --features test-db resource_grant
```
Expected: all PASS.

- [ ] **Step 3: Full check gate**

Run: `cargo make check`
Expected: fmt + clippy(`-D warnings`) + docs + machete + TS all clean.

- [ ] **Step 4: Commit any cache changes**

```bash
git add -A
git commit -m "chore(sqlx): refresh cache for resource-grant surface" --allow-empty
```

(Skip / `--allow-empty` if nothing changed.)

---

## Self-Review

**Spec coverage:**
- Migration (owner⇒grant seam) → Task 1 ✓
- Types (`ResourceGrantBody`/`ResourceRevokeBody`, reuse polymorphic req/outcome) → Task 2 ✓
- API handler + route + OpenAPI → Task 3 ✓
- Client → Task 4 ✓
- CLI grant/revoke (`--to-team` via parse_ref, per the user's decorated-ref simplification) → Task 5 ✓
- Tests: SQL/service seam + e2e through the real CLI → Tasks 1 & 6 ✓
- MCP parity → Task 7 ✓
- sqlx cache → Task 8 ✓

**Type consistency:** `ResourceGrantBody`/`ResourceRevokeBody` fields match across Task 2 (def), Task 3 (handler widen), Task 4 (client), Task 5 (CLI build). `GrantOutcome`/`RevokeOutcome` reused everywhere. `resolve_principal(Option<Uuid>, Option<Uuid>)` reused from `actions::cogmap` in Task 5. `parse_ref(...).0` handling flagged with a guard in Tasks 5 & 7.

**Placeholder scan:** no TBD/TODO; every code step carries full code. The two intentional variables — the migration timestamp `<ts>` (Task 1 Step 1 resolves it) and the `parse_ref` newtype/Uuid + `client_err` helper name (guarded, resolved by reading on disk) — are explicit verification steps, not placeholders.

**Ambiguity:** `can_delete` is always `false` from CLI/MCP (matches cogmap); a delete capability is not surfaced (YAGNI — add later if needed).
