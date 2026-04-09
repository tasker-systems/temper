# MCP-CLI Resource Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consolidate 14 MCP tools to 12 by merging overlapping resource/ingest tools, adding name-based resolution, response enrichment, and documentation updates.

**Architecture:** MCP tool handlers become the adapter layer — they resolve human-friendly names (context_name, doc_type_name, slug) to UUIDs and call the existing service layer. The service layer stays UUID-based, shared with the REST API. New service functions (`get_by_slug`, `list_visible` with doc_type filter, `get_name_by_id` for doc types) are additive.

**Tech Stack:** Rust (rmcp, schemars, sqlx, serde), SvelteKit (temper-ui), Markdown (agent-skills)

**Spec:** `docs/superpowers/specs/2026-04-07-mcp-cli-resource-parity-design.md`

**Branch:** `jct/mcp-cli-resource-parity`

**Test commands:**
- Unit tests: `cargo nextest run --workspace`
- Integration tests: `cargo nextest run --workspace --features test-db`
- E2E tests: `cargo nextest run -p temper-e2e --features test-db`
- Clippy: `cargo make check`
- Full: `cargo make test-all`

---

## File Map

### Service layer (additive changes)
- Modify: `crates/temper-api/src/services/resource_service.rs` — add `get_by_slug`, extend `list_visible` with doc_type filter
- Modify: `crates/temper-api/src/services/doc_type_service.rs` — add `get_name_by_id`

### MCP tools (consolidation)
- Rewrite: `crates/temper-mcp/src/tools/resources.rs` — new input structs, consolidated handlers, response enrichment
- Delete: `crates/temper-mcp/src/tools/ingest.rs` — logic absorbed into resources.rs
- Modify: `crates/temper-mcp/src/tools/mod.rs` — remove `ingest` module
- Modify: `crates/temper-mcp/src/service.rs` — update tool registrations and descriptions

### Tests
- Create: `tests/e2e/tests/mcp_resource_parity_test.rs` — e2e tests for consolidated tools
- Modify: `tests/e2e/tests/mcp_ingest_test.rs` — update or remove tests for deleted tools

### Documentation
- Modify: `agent-skills/SKILL.md` — update cloud access note
- Rewrite: `agent-skills/knowledge-base.md` — new tool reference
- Rewrite: `agent-skills/claude-desktop.md` — new tool list and workflows
- Modify: `packages/temper-ui/src/routes/docs/+page.svelte` — update MCP tools table
- Modify: `packages/temper-ui/src/routes/agents/+page.svelte` — verify/update tool references

---

## Task 1: Add `get_by_slug` to resource_service

**Files:**
- Modify: `crates/temper-api/src/services/resource_service.rs`
- Create: `tests/e2e/tests/mcp_resource_parity_test.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/e2e/tests/mcp_resource_parity_test.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

use temper_api::services::{context_service, ingest_service, resource_service};
use temper_core::types::ids::ProfileId;

/// Helper: resolve profile ID from e2e test user.
async fn resolve_test_profile(pool: &sqlx::PgPool) -> ProfileId {
    ProfileId::from(
        sqlx::query_scalar!(
            "SELECT id FROM kb_profiles WHERE id IN (SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider_user_id = 'e2e-test-user') LIMIT 1"
        )
        .fetch_one(pool)
        .await
        .expect("profile lookup"),
    )
}

/// Helper: SHA256 hex digest of content.
fn content_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// get_by_slug returns a resource by slug within a context.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn get_by_slug_finds_resource(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client.profile().get().await.expect("profile pre-flight");

    let profile_id = resolve_test_profile(&pool).await;

    let context = context_service::create(&pool, profile_id, "slug-test")
        .await
        .expect("context create");

    let doc_type_id = ingest_service::resolve_doc_type(&pool, "research")
        .await
        .expect("doc_type");

    let body_hash = content_hash("test content");
    let empty = serde_json::json!({});

    ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            profile_id,
            device_id: "test",
            context_id: context.id,
            doc_type_id,
            title: "Slug Lookup Test",
            slug: Some("slug-lookup-test"),
            origin_uri: "test://slug-lookup",
            content_hash: &body_hash,
            managed_meta: &empty,
            open_meta: &empty,
        },
    )
    .await
    .expect("create resource");

    let found = resource_service::get_by_slug(
        &pool,
        profile_id.into(),
        "slug-lookup-test",
        context.id.into(),
    )
    .await
    .expect("get_by_slug");

    assert_eq!(found.title, "Slug Lookup Test");
    assert_eq!(found.slug.as_deref(), Some("slug-lookup-test"));
}

/// get_by_slug returns NotFound for non-existent slug.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn get_by_slug_returns_not_found(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client.profile().get().await.expect("profile pre-flight");

    let profile_id = resolve_test_profile(&pool).await;
    let context = context_service::create(&pool, profile_id, "slug-missing-test")
        .await
        .expect("context create");

    let result = resource_service::get_by_slug(
        &pool,
        profile_id.into(),
        "nonexistent-slug",
        context.id.into(),
    )
    .await;

    assert!(result.is_err());
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run -p temper-e2e --features test-db -E 'test(get_by_slug)'`
Expected: FAIL — `get_by_slug` function does not exist yet.

- [ ] **Step 3: Implement `get_by_slug` in resource_service**

Add to `crates/temper-api/src/services/resource_service.rs` after the `get_visible` function (after line 99):

```rust
/// Get a single resource by slug within a context, scoped to profile visibility.
pub async fn get_by_slug(
    pool: &PgPool,
    profile_id: Uuid,
    slug: &str,
    context_id: Uuid,
) -> ApiResult<ResourceRow> {
    let row = sqlx::query_as!(
        ResourceRow,
        r#"
        WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
        SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.origin_uri, r.title,
               r.slug,
               r.originator_profile_id, r.owner_profile_id, r.is_active,
               r.created, r.updated
          FROM kb_resources r
          JOIN visible v ON v.resource_id = r.id
         WHERE r.slug = $2
           AND r.kb_context_id = $3
           AND r.is_active = true
        "#,
        profile_id,
        slug,
        context_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;

    Ok(row)
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo nextest run -p temper-e2e --features test-db -E 'test(get_by_slug)'`
Expected: PASS — both tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/services/resource_service.rs tests/e2e/tests/mcp_resource_parity_test.rs
git commit -m "feat(api): add get_by_slug to resource_service for slug-based lookup"
```

---

## Task 2: Add doc_type filter to `list_visible` and add `get_name_by_id` to doc_type_service

**Files:**
- Modify: `crates/temper-api/src/services/resource_service.rs`
- Modify: `crates/temper-api/src/services/doc_type_service.rs`
- Modify: `tests/e2e/tests/mcp_resource_parity_test.rs`

- [ ] **Step 1: Write the failing tests**

Append to `tests/e2e/tests/mcp_resource_parity_test.rs`:

```rust
use temper_api::services::doc_type_service;

/// list_visible with doc_type filter returns only matching type.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_visible_filters_by_doc_type(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client.profile().get().await.expect("profile pre-flight");

    let profile_id = resolve_test_profile(&pool).await;
    let context = context_service::create(&pool, profile_id, "list-doctype-test")
        .await
        .expect("context create");

    let research_id = ingest_service::resolve_doc_type(&pool, "research").await.expect("research");
    let session_id = ingest_service::resolve_doc_type(&pool, "session").await.expect("session");

    let body_hash = content_hash("list test");
    let empty = serde_json::json!({});

    // Create one research and one session resource
    ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            profile_id,
            device_id: "test",
            context_id: context.id,
            doc_type_id: research_id,
            title: "Research One",
            slug: Some("research-one"),
            origin_uri: "test://list/research",
            content_hash: &body_hash,
            managed_meta: &empty,
            open_meta: &empty,
        },
    )
    .await
    .expect("create research");

    let session_hash = content_hash("session content");
    ingest_service::create_resource_with_manifest(
        &pool,
        &ingest_service::CreateResourceParams {
            profile_id,
            device_id: "test",
            context_id: context.id,
            doc_type_id: session_id,
            title: "Session One",
            slug: Some("session-one"),
            origin_uri: "test://list/session",
            content_hash: &session_hash,
            managed_meta: &empty,
            open_meta: &empty,
        },
    )
    .await
    .expect("create session");

    // List with doc_type filter = research
    let results = resource_service::list_visible(
        &pool,
        profile_id.into(),
        resource_service::ResourceListParams {
            kb_context_id: Some(context.id.into()),
            kb_doc_type_id: Some(research_id),
            limit: None,
            offset: None,
        },
    )
    .await
    .expect("list_visible with doc_type");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Research One");
}

/// get_name_by_id returns the doc type name.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn doc_type_get_name_by_id(pool: sqlx::PgPool) {
    let _app = common::setup(pool.clone()).await;

    let research_id = ingest_service::resolve_doc_type(&pool, "research").await.expect("research");
    let name = doc_type_service::get_name_by_id(&pool, research_id).await.expect("get_name_by_id");
    assert_eq!(name, "research");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p temper-e2e --features test-db -E 'test(list_visible_filters|doc_type_get_name)'`
Expected: FAIL — `kb_doc_type_id` field doesn't exist on `ResourceListParams`, `get_name_by_id` doesn't exist.

- [ ] **Step 3: Add `kb_doc_type_id` to `ResourceListParams`**

Edit `crates/temper-core/src/types/resource.rs`. Add the field after `kb_context_id`:

```rust
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceListParams {
    /// Filter by context ID.
    pub kb_context_id: Option<Uuid>,
    /// Filter by document type ID.
    pub kb_doc_type_id: Option<Uuid>,
    /// Maximum results to return (default 50, max 200).
    #[cfg_attr(feature = "typescript", ts(type = "number | null"))]
    pub limit: Option<i64>,
    /// Offset for pagination.
    #[cfg_attr(feature = "typescript", ts(type = "number | null"))]
    pub offset: Option<i64>,
}
```

- [ ] **Step 4: Update `list_visible` to handle the doc_type filter**

Rewrite `crates/temper-api/src/services/resource_service.rs` `list_visible` function. The current function branches on context_id. Now it needs to handle four combinations. Use a runtime query approach to avoid four static branches:

```rust
/// List resources visible to the given profile.
///
/// Uses the `resources_visible_to(profile_id)` SQL function to scope results.
/// Optionally filters by context ID and/or doc type ID.
pub async fn list_visible(
    pool: &PgPool,
    profile_id: Uuid,
    params: ResourceListParams,
) -> ApiResult<Vec<ResourceRow>> {
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0).max(0);

    let rows = match (params.kb_context_id, params.kb_doc_type_id) {
        (Some(ctx_id), Some(dt_id)) => {
            sqlx::query_as!(
                ResourceRow,
                r#"
                WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
                SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.origin_uri, r.title,
                       r.slug,
                       r.originator_profile_id, r.owner_profile_id, r.is_active,
                       r.created, r.updated
                  FROM kb_resources r
                  JOIN visible v ON v.resource_id = r.id
                 WHERE r.is_active = true
                   AND r.kb_context_id = $2
                   AND r.kb_doc_type_id = $3
                 ORDER BY r.updated DESC
                 LIMIT $4 OFFSET $5
                "#,
                profile_id,
                ctx_id,
                dt_id,
                limit,
                offset,
            )
            .fetch_all(pool)
            .await?
        }
        (Some(ctx_id), None) => {
            sqlx::query_as!(
                ResourceRow,
                r#"
                WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
                SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.origin_uri, r.title,
                       r.slug,
                       r.originator_profile_id, r.owner_profile_id, r.is_active,
                       r.created, r.updated
                  FROM kb_resources r
                  JOIN visible v ON v.resource_id = r.id
                 WHERE r.is_active = true
                   AND r.kb_context_id = $2
                 ORDER BY r.updated DESC
                 LIMIT $3 OFFSET $4
                "#,
                profile_id,
                ctx_id,
                limit,
                offset,
            )
            .fetch_all(pool)
            .await?
        }
        (None, Some(dt_id)) => {
            sqlx::query_as!(
                ResourceRow,
                r#"
                WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
                SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.origin_uri, r.title,
                       r.slug,
                       r.originator_profile_id, r.owner_profile_id, r.is_active,
                       r.created, r.updated
                  FROM kb_resources r
                  JOIN visible v ON v.resource_id = r.id
                 WHERE r.is_active = true
                   AND r.kb_doc_type_id = $2
                 ORDER BY r.updated DESC
                 LIMIT $3 OFFSET $4
                "#,
                profile_id,
                dt_id,
                limit,
                offset,
            )
            .fetch_all(pool)
            .await?
        }
        (None, None) => {
            sqlx::query_as!(
                ResourceRow,
                r#"
                WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
                SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.origin_uri, r.title,
                       r.slug,
                       r.originator_profile_id, r.owner_profile_id, r.is_active,
                       r.created, r.updated
                  FROM kb_resources r
                  JOIN visible v ON v.resource_id = r.id
                 WHERE r.is_active = true
                 ORDER BY r.updated DESC
                 LIMIT $2 OFFSET $3
                "#,
                profile_id,
                limit,
                offset,
            )
            .fetch_all(pool)
            .await?
        }
    };

    Ok(rows)
}
```

- [ ] **Step 5: Add `get_name_by_id` to doc_type_service**

Add to `crates/temper-api/src/services/doc_type_service.rs`:

```rust
/// Get a doc type name by its UUID.
pub async fn get_name_by_id(pool: &PgPool, id: uuid::Uuid) -> ApiResult<String> {
    let name = sqlx::query_scalar!(
        "SELECT name FROM kb_doc_types WHERE id = $1",
        id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(crate::error::ApiError::NotFound)?;

    Ok(name)
}
```

- [ ] **Step 6: Fix any compile errors from `ResourceListParams` change**

The new `kb_doc_type_id` field means all existing construction sites need updating. Search for `ResourceListParams {` across the codebase and add `kb_doc_type_id: None` where missing. Key locations:
- `crates/temper-mcp/src/service.rs` (the `list_resources` tool — will be rewritten in Task 4 but needs to compile now)
- `crates/temper-api/src/handlers/` (REST handlers)
- `tests/e2e/tests/resource_crud_test.rs`

Run: `cargo make check` to find all sites.

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo nextest run -p temper-e2e --features test-db -E 'test(list_visible_filters|doc_type_get_name)'`
Expected: PASS.

- [ ] **Step 8: Regenerate sqlx offline cache**

Run: `cargo sqlx prepare --workspace -- --all-features`

- [ ] **Step 9: Commit**

```bash
git add crates/temper-core/src/types/resource.rs crates/temper-api/src/services/resource_service.rs crates/temper-api/src/services/doc_type_service.rs tests/e2e/tests/mcp_resource_parity_test.rs .sqlx/
git commit -m "feat(api): add doc_type filter to list_visible, add get_name_by_id to doc_type_service"
```

---

## Task 3: Build response enrichment helper in MCP tools

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs`

- [ ] **Step 1: Add the enriched response struct and helper**

Add to the top of `crates/temper-mcp/src/tools/resources.rs` (replacing the existing input structs which will be rewritten in Task 4). For now, add these alongside the existing code:

```rust
use temper_api::services::{context_service, doc_type_service};
use temper_core::types::ids::{ContextId, ProfileId};

/// Enriched resource response with human-readable names.
#[derive(Debug, serde::Serialize)]
pub struct EnrichedResource {
    pub id: uuid::Uuid,
    pub title: String,
    pub slug: Option<String>,
    pub context_name: String,
    pub doc_type_name: String,
    pub owner: String,
    pub origin_uri: String,
    pub is_active: bool,
    pub created: chrono::DateTime<chrono::Utc>,
    pub updated: chrono::DateTime<chrono::Utc>,
}

/// Enrich a ResourceRow with context and doc type names.
pub async fn enrich_resource(
    pool: &sqlx::PgPool,
    profile_id: ProfileId,
    row: &temper_core::types::resource::ResourceRow,
) -> Result<EnrichedResource, rmcp::ErrorData> {
    let context = context_service::get_visible(pool, profile_id, row.kb_context_id)
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to resolve context: {e}"), None))?;

    let doc_type_name = doc_type_service::get_name_by_id(pool, row.kb_doc_type_id.into())
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to resolve doc_type: {e}"), None))?;

    Ok(EnrichedResource {
        id: row.id.into(),
        title: row.title.clone(),
        slug: row.slug.clone(),
        context_name: context.name,
        doc_type_name,
        owner: "@me".to_string(),
        origin_uri: row.origin_uri.clone(),
        is_active: row.is_active,
        created: row.created,
        updated: row.updated,
    })
}

/// Enrich a list of ResourceRows.
pub async fn enrich_resources(
    pool: &sqlx::PgPool,
    profile_id: ProfileId,
    rows: &[temper_core::types::resource::ResourceRow],
) -> Result<Vec<EnrichedResource>, rmcp::ErrorData> {
    let mut enriched = Vec::with_capacity(rows.len());
    for row in rows {
        enriched.push(enrich_resource(pool, profile_id, row).await?);
    }
    Ok(enriched)
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p temper-mcp --all-features`
Expected: compiles (the new code is not yet called but must type-check).

- [ ] **Step 3: Commit**

```bash
git add crates/temper-mcp/src/tools/resources.rs
git commit -m "feat(mcp): add enriched resource response helper"
```

---

## Task 4: Rewrite MCP tool handlers — consolidated resource tools

This is the core task. Replace all resource/ingest tool handlers with the consolidated versions.

**Files:**
- Rewrite: `crates/temper-mcp/src/tools/resources.rs`
- Delete: `crates/temper-mcp/src/tools/ingest.rs`
- Modify: `crates/temper-mcp/src/tools/mod.rs`
- Modify: `crates/temper-mcp/src/service.rs`

- [ ] **Step 1: Rewrite `resources.rs` with new input structs and handlers**

Replace the entire contents of `crates/temper-mcp/src/tools/resources.rs` with the consolidated implementation. The file should contain:

1. All new input structs (`CreateResourceInput`, `GetResourceInput`, `ListResourcesInput`, `UpdateResourceInput`, `DeleteResourceInput`)
2. The `EnrichedResource` struct and `enrich_resource`/`enrich_resources` helpers (from Task 3)
3. The `to_text` helper
4. Content hashing (`content_hash`) and bearer token extraction (`extract_bearer_token`) moved from the old `ingest.rs`
5. The `spawn_content_ingest_post` function moved from the old `ingest.rs`
6. Five tool handler functions: `create_resource`, `get_resource`, `list_resources`, `update_resource`, `delete_resource`

The full file structure:

```rust
//! Resource tools — unified CRUD with name-based resolution and optional content.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use temper_api::services::{context_service, doc_type_service, ingest_service, resource_service};
use temper_core::types::ids::{ProfileId, ResourceId};

use crate::service::TemperMcpService;

// ── Input structs ──────────────────────────────────────────────────

/// MCP input for create_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateResourceInput {
    /// Human-readable context name (must already exist).
    pub context_name: String,
    /// Human-readable doc type name (e.g. "task", "session", "research").
    pub doc_type_name: String,
    /// Resource title.
    pub title: String,
    /// Optional markdown content body. If provided, triggers async
    /// chunk/embed processing.
    pub content: Option<String>,
    /// Optional URL-friendly slug.
    pub slug: Option<String>,
    /// Optional origin URI. Defaults to mcp://agent/<uuid>.
    pub origin_uri: Option<String>,
    /// Optional owner (defaults to @me). Reserved for future team scoping.
    pub owner: Option<String>,
}

/// MCP input for get_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetResourceInput {
    /// UUID of the resource. Provide either id or slug (not both).
    pub id: Option<Uuid>,
    /// Slug of the resource. Requires context_name for disambiguation.
    pub slug: Option<String>,
    /// Context name. Required when looking up by slug.
    pub context_name: Option<String>,
    /// If true, includes the full reconstituted markdown content.
    pub include_content: Option<bool>,
}

/// MCP input for list_resources.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListResourcesInput {
    /// Filter by context name.
    pub context_name: Option<String>,
    /// Filter by doc type name (e.g. "task", "research").
    pub doc_type_name: Option<String>,
    /// Max results (default 50, max 200).
    pub limit: Option<i64>,
    /// Pagination offset.
    pub offset: Option<i64>,
}

/// MCP input for update_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateResourceInput {
    /// UUID of the resource to update.
    pub id: Uuid,
    /// New title.
    pub title: Option<String>,
    /// New slug.
    pub slug: Option<String>,
    /// New markdown content. Replaces existing content and triggers
    /// async re-processing.
    pub content: Option<String>,
}

/// MCP input for delete_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteResourceInput {
    /// UUID of the resource to delete.
    pub id: Uuid,
}

// ── Response enrichment ────────────────────────────────────────────

/// Enriched resource response with human-readable names.
#[derive(Debug, serde::Serialize)]
pub struct EnrichedResource {
    pub id: Uuid,
    pub title: String,
    pub slug: Option<String>,
    pub context_name: String,
    pub doc_type_name: String,
    pub owner: String,
    pub origin_uri: String,
    pub is_active: bool,
    pub created: chrono::DateTime<chrono::Utc>,
    pub updated: chrono::DateTime<chrono::Utc>,
}

async fn enrich_resource(
    pool: &sqlx::PgPool,
    profile_id: ProfileId,
    row: &temper_core::types::resource::ResourceRow,
) -> Result<EnrichedResource, rmcp::ErrorData> {
    let context = context_service::get_visible(pool, profile_id, row.kb_context_id)
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to resolve context: {e}"), None))?;

    let doc_type_name = doc_type_service::get_name_by_id(pool, row.kb_doc_type_id.into())
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to resolve doc_type: {e}"), None))?;

    Ok(EnrichedResource {
        id: row.id.into(),
        title: row.title.clone(),
        slug: row.slug.clone(),
        context_name: context.name,
        doc_type_name,
        owner: "@me".to_string(),
        origin_uri: row.origin_uri.clone(),
        is_active: row.is_active,
        created: row.created,
        updated: row.updated,
    })
}

async fn enrich_resources(
    pool: &sqlx::PgPool,
    profile_id: ProfileId,
    rows: &[temper_core::types::resource::ResourceRow],
) -> Result<Vec<EnrichedResource>, rmcp::ErrorData> {
    let mut enriched = Vec::with_capacity(rows.len());
    for row in rows {
        enriched.push(enrich_resource(pool, profile_id, row).await?);
    }
    Ok(enriched)
}

// ── Helpers ────────────────────────────────────────────────────────

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

fn extract_bearer_token(parts: &http::request::Parts) -> Option<String> {
    let header = parts.headers.get(http::header::AUTHORIZATION)?;
    let value = header.to_str().ok()?;
    value.strip_prefix("Bearer ").map(|s| s.to_string())
}

fn spawn_content_ingest_post(
    resource_id: ResourceId,
    content: String,
    replace: bool,
    bearer_token: Option<String>,
    context_id: String,
    body_hash: String,
) {
    tokio::spawn(async move {
        let base_url = match std::env::var("MCP_BASE_URL") {
            Ok(url) => url,
            Err(_) => {
                tracing::warn!("MCP_BASE_URL not set; skipping content-ingest POST");
                return;
            }
        };

        let url = format!("{base_url}/api/content-ingest");
        let payload = temper_core::types::ingest::ContentIngestRequest {
            resource_id: resource_id.to_string(),
            content,
            replace,
            context_id: Some(context_id),
            body_hash: Some(body_hash),
        };
        let client = reqwest::Client::new();
        let mut req = client.post(&url).json(&payload);

        if let Some(token) = bearer_token {
            req = req.bearer_auth(token);
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!(resource_id = %resource_id, "content-ingest POST accepted");
            }
            Ok(resp) => {
                tracing::warn!(resource_id = %resource_id, status = %resp.status(), "content-ingest POST returned non-success");
            }
            Err(e) => {
                tracing::warn!(resource_id = %resource_id, error = %e, "content-ingest POST failed");
            }
        }
    });
}

// ── Tool handlers ──────────────────────────────────────────────────

pub async fn create_resource(
    svc: &TemperMcpService,
    input: CreateResourceInput,
    parts: &http::request::Parts,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    // Validate owner format if provided (stub for R11)
    if let Some(ref owner) = input.owner {
        if !owner.starts_with('@') && !owner.starts_with('+') {
            return Err(rmcp::ErrorData::invalid_params(
                "owner must start with @ (profile) or + (team)".to_string(),
                None,
            ));
        }
    }

    // 1. Resolve context by name — error if not found
    let context = context_service::resolve_by_name(pool, profile_id, &input.context_name)
        .await
        .map_err(|e| match e {
            temper_api::error::ApiError::NotFound => rmcp::ErrorData::invalid_params(
                format!("Context '{}' not found. Use create_context to create it first.", input.context_name),
                None,
            ),
            other => rmcp::ErrorData::internal_error(format!("Failed to resolve context: {other}"), None),
        })?;

    // 2. Resolve doc type by name
    let doc_type_id = ingest_service::resolve_doc_type(pool, &input.doc_type_name)
        .await
        .map_err(|e| rmcp::ErrorData::invalid_params(
            format!("Unknown doc_type '{}'. Use list_doc_types to see available types. Error: {e}", input.doc_type_name),
            None,
        ))?;

    // 3. Content handling — hash, dedup, ingest post
    let body_hash = input.content.as_ref().map(|c| content_hash(c));

    if let Some(ref hash) = body_hash {
        if let Some(existing) = ingest_service::find_by_body_hash(pool, profile_id, hash)
            .await
            .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to check body hash: {e}"), None))?
        {
            let enriched = enrich_resource(pool, profile_id, &existing).await?;
            return Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                to_text(&serde_json::json!({
                    "resource": enriched,
                    "status": "existing"
                })),
            )]));
        }
    }

    // 4. Default origin_uri
    let origin_uri = input.origin_uri.unwrap_or_else(|| format!("mcp://agent/{}", Uuid::new_v4()));

    // 5. Create resource + manifest + event
    let empty_json = serde_json::json!({});
    let hash_for_manifest = body_hash.as_deref().unwrap_or("sha256:empty");

    let resource = ingest_service::create_resource_with_manifest(
        pool,
        &ingest_service::CreateResourceParams {
            profile_id,
            device_id: "mcp",
            context_id: context.id,
            doc_type_id,
            title: &input.title,
            slug: input.slug.as_deref(),
            origin_uri: &origin_uri,
            content_hash: hash_for_manifest,
            managed_meta: &empty_json,
            open_meta: &empty_json,
        },
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to create resource: {e}"), None))?;

    // 6. Fire content-ingest POST if content provided
    if let (Some(content), Some(hash)) = (input.content, body_hash) {
        let bearer_token = extract_bearer_token(parts);
        spawn_content_ingest_post(
            resource.id,
            content,
            false,
            bearer_token,
            context.id.to_string(),
            hash,
        );
    }

    let enriched = enrich_resource(pool, profile_id, &resource).await?;
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&serde_json::json!({
            "resource": enriched,
            "status": "created"
        })),
    )]))
}

pub async fn get_resource(
    svc: &TemperMcpService,
    input: GetResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    // Validate input: exactly one of id or slug
    let row = match (input.id, input.slug.as_deref()) {
        (Some(id), None) => {
            resource_service::get_visible(pool, profile.id.into(), id)
                .await
                .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None))?
        }
        (None, Some(slug)) => {
            let context_name = input.context_name.as_deref().ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    "context_name is required when looking up by slug".to_string(),
                    None,
                )
            })?;
            let context = context_service::resolve_by_name(pool, profile_id, context_name)
                .await
                .map_err(|e| rmcp::ErrorData::invalid_params(
                    format!("Context '{context_name}' not found: {e}"),
                    None,
                ))?;
            resource_service::get_by_slug(pool, profile.id.into(), slug, context.id.into())
                .await
                .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None))?
        }
        (Some(_), Some(_)) => {
            return Err(rmcp::ErrorData::invalid_params(
                "Provide either id or slug, not both".to_string(),
                None,
            ));
        }
        (None, None) => {
            return Err(rmcp::ErrorData::invalid_params(
                "Provide either id or slug".to_string(),
                None,
            ));
        }
    };

    let enriched = enrich_resource(pool, profile_id, &row).await?;

    if input.include_content.unwrap_or(false) {
        let markdown = resource_service::get_content(pool, profile.id.into(), row.id.into())
            .await
            .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to get content: {e}"), None))?;

        Ok(CallToolResult::success(vec![
            rmcp::model::Content::text(to_text(&enriched)),
            rmcp::model::Content::text(markdown),
        ]))
    } else {
        Ok(CallToolResult::success(vec![rmcp::model::Content::text(
            to_text(&enriched),
        )]))
    }
}

pub async fn list_resources(
    svc: &TemperMcpService,
    input: ListResourcesInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    // Resolve names to IDs
    let context_id = if let Some(ref name) = input.context_name {
        Some(
            context_service::resolve_by_name(pool, profile_id, name)
                .await
                .map_err(|e| rmcp::ErrorData::invalid_params(
                    format!("Context '{name}' not found: {e}"),
                    None,
                ))?
                .id
                .into(),
        )
    } else {
        None
    };

    let doc_type_id = if let Some(ref name) = input.doc_type_name {
        Some(
            ingest_service::resolve_doc_type(pool, name)
                .await
                .map_err(|e| rmcp::ErrorData::invalid_params(
                    format!("Unknown doc_type '{name}': {e}"),
                    None,
                ))?,
        )
    } else {
        None
    };

    let params = resource_service::ResourceListParams {
        kb_context_id: context_id,
        kb_doc_type_id: doc_type_id,
        limit: input.limit,
        offset: input.offset,
    };

    let rows = resource_service::list_visible(pool, profile.id.into(), params)
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to list resources: {e}"), None))?;

    let enriched = enrich_resources(pool, profile_id, &rows).await?;
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&enriched),
    )]))
}

pub async fn update_resource(
    svc: &TemperMcpService,
    input: UpdateResourceInput,
    parts: &http::request::Parts,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);
    let resource_id = ResourceId::from(input.id);

    // Auth check
    let can_modify = sqlx::query_scalar!(
        "SELECT true FROM can_modify_resource($1, $2)",
        *profile_id,
        *resource_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to check permissions: {e}"), None))?;

    if can_modify.is_none() {
        return Err(rmcp::ErrorData::internal_error(
            "Resource not found or not modifiable".to_string(),
            None,
        ));
    }

    // Update title/slug if provided
    if input.title.is_some() || input.slug.is_some() {
        let update_req = temper_core::types::resource::ResourceUpdateRequest {
            title: input.title.clone(),
            slug: input.slug.clone(),
        };
        resource_service::update(pool, profile.id.into(), input.id, update_req)
            .await
            .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to update resource: {e}"), None))?;
    }

    // Update content if provided
    if let Some(content) = input.content {
        let body_hash = content_hash(&content);
        let empty_json = serde_json::json!({});

        let mut tx = pool.begin().await.map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to begin transaction: {e}"), None)
        })?;

        ingest_service::update_resource_manifest(
            &mut tx,
            profile_id,
            "mcp",
            resource_id,
            &body_hash,
            &empty_json,
            &empty_json,
        )
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to update manifest: {e}"), None))?;

        tx.commit().await.map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to commit: {e}"), None)
        })?;

        // Fire content-ingest POST
        let resource = resource_service::get_visible(pool, profile.id.into(), input.id)
            .await
            .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None))?;

        let bearer_token = extract_bearer_token(parts);
        spawn_content_ingest_post(
            resource_id,
            content,
            true,
            bearer_token,
            resource.kb_context_id.to_string(),
            body_hash,
        );
    }

    // Return enriched current state
    let row = resource_service::get_visible(pool, profile.id.into(), input.id)
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None))?;

    let enriched = enrich_resource(pool, profile_id, &row).await?;
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&enriched),
    )]))
}

pub async fn delete_resource(
    svc: &TemperMcpService,
    input: DeleteResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    temper_api::services::resource_service::delete(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        ResourceId::from(input.id),
        "mcp",
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to delete resource: {e}"), None))?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&serde_json::json!({ "deleted": true, "id": input.id })),
    )]))
}
```

- [ ] **Step 2: Delete `ingest.rs` and update `mod.rs`**

Delete `crates/temper-mcp/src/tools/ingest.rs`.

Update `crates/temper-mcp/src/tools/mod.rs` to:

```rust
pub mod contexts;
pub mod doc_types;
pub mod events;
pub mod profiles;
pub mod resources;
pub mod search;
```

- [ ] **Step 3: Rewrite tool registrations in `service.rs`**

Replace the `// ── Tools ──` section in `crates/temper-mcp/src/service.rs` (lines 104-246) with the consolidated 12 tools. Remove `ingest_content`, `update_resource_content`, `get_resource_content`. The `create_resource` and `update_resource` handlers now take `Extension(parts)` for bearer token extraction. Key changes:

- `create_resource` takes `Parameters<tools::resources::CreateResourceInput>` + `Extension(parts)`, delegates to `tools::resources::create_resource(self, input, &parts)`
- `get_resource` takes `Parameters<tools::resources::GetResourceInput>`, delegates to `tools::resources::get_resource(self, input)` (no parts needed)
- `list_resources` takes `Parameters<tools::resources::ListResourcesInput>`, delegates to `tools::resources::list_resources(self, input)`
- `update_resource` takes `Parameters<tools::resources::UpdateResourceInput>` + `Extension(parts)`, delegates to `tools::resources::update_resource(self, input, &parts)`
- `delete_resource` takes `Parameters<tools::resources::DeleteResourceInput>`, delegates to `tools::resources::delete_resource(self, input)`

Update all tool `#[tool(description = "...")]` attributes to match the spec descriptions.

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p temper-mcp --all-features`
Expected: compiles with no errors.

- [ ] **Step 5: Run the full test suite**

Run: `cargo nextest run --workspace`
Expected: PASS — unit tests still pass.

- [ ] **Step 6: Commit**

```bash
git add -A crates/temper-mcp/
git commit -m "feat(mcp): consolidate resource tools — merge ingest into CRUD, add name-based resolution"
```

---

## Task 5: Update existing e2e tests for the new tool shapes

**Files:**
- Modify: `tests/e2e/tests/mcp_ingest_test.rs`

- [ ] **Step 1: Review and update `mcp_ingest_test.rs`**

The existing `mcp_ingest_test.rs` tests the `create_resource_with_manifest` service function directly (not MCP tool handlers). These tests should still pass since the service layer is unchanged. Verify by running:

Run: `cargo nextest run -p temper-e2e --features test-db -E 'test(create_resource_with_manifest)'`
Expected: PASS — the service function is unchanged.

- [ ] **Step 2: Run the full e2e suite**

Run: `cargo nextest run -p temper-e2e --features test-db`
Expected: PASS — all e2e tests pass.

- [ ] **Step 3: Run clippy and check**

Run: `cargo make check`
Expected: all checks pass.

- [ ] **Step 4: Regenerate sqlx offline cache if needed**

Run: `cargo sqlx prepare --workspace -- --all-features`

- [ ] **Step 5: Commit if any test changes were needed**

```bash
git add tests/ .sqlx/
git commit -m "test: update e2e tests for consolidated MCP tools"
```

---

## Task 6: Update agent-skills documentation

**Files:**
- Modify: `agent-skills/SKILL.md`
- Rewrite: `agent-skills/knowledge-base.md`
- Rewrite: `agent-skills/claude-desktop.md`

- [ ] **Step 1: Update SKILL.md cloud access note**

In `agent-skills/SKILL.md`, replace the cloud access note (lines 26-28) that references `ingest_content`:

Replace:
```
> **Cloud access**: If a Temper MCP server is configured, you can also access the knowledge
> base through MCP resources and tools instead of (or alongside) local file access. The MCP
> server supports both reading and writing — use `ingest_content` to create resources with
> full content, and `search` for text-based discovery. See `knowledge-base.md` for MCP
> access patterns and `claude-desktop.md` for Claude Desktop setup.
```

With:
```
> **Cloud access**: If a Temper MCP server is configured, you can also access the knowledge
> base through MCP resources and tools instead of (or alongside) local file access. The MCP
> server supports both reading and writing — use `create_resource` with a `content` field to
> create resources with full content, and `search` for text-based discovery. See
> `knowledge-base.md` for MCP access patterns and `claude-desktop.md` for Claude Desktop setup.
```

- [ ] **Step 2: Rewrite knowledge-base.md**

Replace the entire "Resources vs Tools — Decision Table" (lines 15-29) with the consolidated tool list:

```markdown
| Intent | Use | Why |
|--------|-----|-----|
| Browse what's in a context | Resource: `temper://contexts/{name}/resources` | No tool call overhead, client can cache |
| Read a specific document | Resource: `temper://resources/{id}` | Returns metadata + full markdown |
| Get raw markdown only | Resource: `temper://resources/{id}/content` | Lighter than full resource read |
| Find something by topic | Tool: `search` | Semantic vector search, can't do with resources |
| Create a new resource | Tool: `create_resource` | Name-based, optional content |
| Get a resource by ID or slug | Tool: `get_resource` | Supports ID or slug+context lookup, optional content |
| List resources with filters | Tool: `list_resources` | Filter by context_name, doc_type_name |
| Update resource | Tool: `update_resource` | Title, slug, and/or content |
| Delete a resource | Tool: `delete_resource` | Soft-delete, tools only |
| Create a new context | Tool: `create_context` | Mutation — tools only |
| Check who you are | Tool: `get_profile` | Identity/settings |
| Audit recent activity | Tool: `list_events` | Debugging, event history |
| Discover valid document types | Tool: `list_doc_types` | Returns id and name for each type |
```

Replace the "Writing Content" section (lines 72-160) with the updated create/update patterns using the consolidated tool names and parameter shapes. Key changes:
- `ingest_content` becomes `create_resource` with `content` field
- `update_resource_content` becomes `update_resource` with `content` field
- `create_resource` (metadata-only) section removed — the unified tool handles both
- Remove the "Context handling" note about auto-creation — contexts must now exist first
- Show `context_name` and `doc_type_name` instead of UUID-based parameters

- [ ] **Step 3: Rewrite claude-desktop.md tool list**

Update the "Tools (Query & Mutate)" section (lines 48-68) to match the new 12-tool inventory:

```markdown
### Tools (Query & Mutate)

These are available as function calls during conversation:

**Read operations:**
- `list_contexts` — show all workspaces
- `list_resources` — list resources, filtered by context name and/or doc type name
- `get_resource` — get one resource by ID or slug (optionally with content)
- `search` — semantic search across all resources
- `list_doc_types` — discover available document types

**Write operations:**
- `create_resource` — create a resource with optional markdown content
- `update_resource` — change a resource's title, slug, or content
- `delete_resource` — soft-delete a resource
- `create_context` — create a new workspace

**Utility:**
- `get_profile` — see your authenticated identity and preferences
- `list_events` — view recent activity for debugging
```

Update the workflow examples to use `create_resource` instead of `ingest_content`.

- [ ] **Step 4: Commit**

```bash
git add agent-skills/
git commit -m "docs: update agent-skills for consolidated MCP tools"
```

---

## Task 7: Update temper-ui docs and agents pages

**Files:**
- Modify: `packages/temper-ui/src/routes/docs/+page.svelte`
- Modify: `packages/temper-ui/src/routes/agents/+page.svelte`

- [ ] **Step 1: Update docs page MCP tools table**

In `packages/temper-ui/src/routes/docs/+page.svelte`, replace the "Available Tools" table (lines 108-122) with the consolidated 12 tools:

```html
<h3>Available Tools</h3>
<table>
  <tbody>
    <tr><td><code>list_resources</code></td><td>List resources, filtered by context name and/or doc type name. Most recent first.</td></tr>
    <tr><td><code>get_resource</code></td><td>Get a resource by ID or slug, optionally with full markdown content</td></tr>
    <tr><td><code>create_resource</code></td><td>Create a resource with optional markdown content. Name-based context and doc type.</td></tr>
    <tr><td><code>update_resource</code></td><td>Update a resource's title, slug, or content. New content triggers re-indexing.</td></tr>
    <tr><td><code>delete_resource</code></td><td>Soft-delete a resource by ID</td></tr>
    <tr><td><code>search</code></td><td>Full-text and semantic search across the knowledge base</td></tr>
    <tr><td><code>list_contexts</code></td><td>List available contexts (workspaces)</td></tr>
    <tr><td><code>get_context</code></td><td>Get details of a specific context</td></tr>
    <tr><td><code>create_context</code></td><td>Create a new context (workspace)</td></tr>
    <tr><td><code>list_doc_types</code></td><td>List available document types</td></tr>
    <tr><td><code>list_events</code></td><td>List events, optionally filtered by resource or type</td></tr>
    <tr><td><code>get_profile</code></td><td>Get the authenticated user's profile</td></tr>
  </tbody>
</table>
```

- [ ] **Step 2: Verify agents page**

Read `packages/temper-ui/src/routes/agents/+page.svelte` — the demo transcripts use conceptual calls (`temper.search`, `temper.warmup`, `temper.session_save`) that don't map 1:1 to MCP tool names. These are aspirational demonstrations, not literal API calls. No changes needed unless a removed tool name appears literally.

- [ ] **Step 3: Run UI checks**

Run: `cd packages/temper-ui && bun run check`
Expected: svelte-check passes.

- [ ] **Step 4: Commit**

```bash
git add packages/temper-ui/
git commit -m "docs: update temper-ui MCP tools reference for consolidated tools"
```

---

## Task 8: Final verification and cleanup

**Files:** None new — full verification pass.

- [ ] **Step 1: Run the full Rust test suite**

Run: `cargo make test`
Expected: all unit tests pass.

- [ ] **Step 2: Run integration tests**

Run: `cargo nextest run -p temper-e2e --features test-db`
Expected: all e2e tests pass.

- [ ] **Step 3: Run full quality checks**

Run: `cargo make check`
Expected: clippy, fmt, docs all pass.

- [ ] **Step 4: Run TypeScript checks**

Run: `cargo make ts-test` and `cd packages/temper-ui && bun run check`
Expected: all pass.

- [ ] **Step 5: Verify tool count**

Grep for `#[tool(description` in `crates/temper-mcp/src/service.rs` and count the results. Should be exactly 12.

Run: `grep -c '#\[tool(description' crates/temper-mcp/src/service.rs`
Expected: `12`

- [ ] **Step 6: Regenerate sqlx offline cache**

Run: `cargo sqlx prepare --workspace -- --all-features`

- [ ] **Step 7: Final commit if any cleanup needed**

```bash
git add .sqlx/
git commit -m "chore: regenerate sqlx offline cache"
```
