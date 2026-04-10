# Authed Vault Browser Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the L0 read-only vault browser for temperkb.io — context sidebar, SVAR DataGrid, ⌘K search, full-route detail pages with kb://-mirrored URLs.

**Architecture:** SvelteKit shell over extended temper-api HTTP endpoints. No direct DB access from SvelteKit. All types shared via temper-core + ts-rs. Single auth/access boundary at the API layer.

**Tech Stack:** Rust (temper-core, temper-api), SvelteKit 2 + Svelte 5, SVAR Svelte DataGrid, Tailwind v4, marked + dompurify, PostgreSQL 18.

**Spec:** `docs/superpowers/specs/2026-04-09-authed-vault-browser-design.md`

**Branch:** `jct/temper-authed-dashboard-ui` (continuing existing branch, 4 commits ahead of main)

---

## Pre-Flight Verification

Before starting any task, the implementer must verify these items:

1. **Docker Postgres running:** `cargo make docker-up` — needed for all `test-db` tests.
2. **SVAR package name:** Run `npm search @wx/svelte-grid` or check https://svar.dev/svelte/datagrid/getting-started/ for the exact package name. Plan uses `wx-svelte-grid` as placeholder — replace with the real name.
3. **RuleHeading existence:** Check if `packages/temper-ui/src/lib/components/RuleHeading.svelte` already exists in the current branch. If yes, reuse it. If no, create it in Task 13.
4. **`_internal/` prefix:** Grep `packages/temper-ui/src/routes/` for any existing internal endpoint prefix convention. Use whatever exists; fall back to `_internal/`.

---

## File Map

### Rust — New Files

| File | Responsibility |
|---|---|
| `migrations/20260410000001_index_resource_manifests_managed_meta.sql` | B-tree expression indexes + GIN on managed_meta |

### Rust — Modified Files

| File | Changes |
|---|---|
| `crates/temper-core/src/types/resource.rs` | Add `ResourceSortField`, `SortOrder` enums; extend `ResourceListParams`; extend `ResourceRow`; add `ResourceListResponse`, `ResourceFacets` |
| `crates/temper-core/src/types/context.rs` | Add `ContextRowWithCounts` |
| `crates/temper-core/src/types/mod.rs` | Re-export new types if needed |
| `crates/temper-api/src/services/resource_service.rs` | Rewrite `list_visible` with extended SQL, add `compute_facets`, add `resolve_by_uri` |
| `crates/temper-api/src/services/context_service.rs` | Add `list_visible_with_counts` |
| `crates/temper-api/src/handlers/resources.rs` | Update `list`, add `facets`, add `by_uri` |
| `crates/temper-api/src/handlers/contexts.rs` | `list` returns `Vec<ContextRowWithCounts>` |
| `crates/temper-api/src/routes.rs` | Add `/api/resources/facets`, `/api/resources/by-uri` |
| `crates/temper-api/src/openapi.rs` | Register new schemas/paths |
| `crates/temper-client/src/resources.rs` | `list()` returns `ResourceListResponse` |
| `crates/temper-client/src/contexts.rs` | `list()` returns `Vec<ContextRowWithCounts>` |
| `crates/temper-api/tests/resources_test.rs` | Update existing test for new response shape |

### Rust — New Test Files

| File | Tests |
|---|---|
| `crates/temper-api/tests/resources_browse_test.rs` | Extended list: filters, sort, pagination, total, FTS |
| `crates/temper-api/tests/resources_facets_test.rs` | Facets endpoint |
| `crates/temper-api/tests/resources_by_uri_test.rs` | URI resolution endpoint |
| `crates/temper-e2e/tests/vault_browse_test.rs` | Full e2e: contexts with counts → resources → facets → by-uri |

### SvelteKit — New Files

| File | Responsibility |
|---|---|
| `packages/temper-ui/src/lib/components/Sidebar.svelte` | Context list + footer nav |
| `packages/temper-ui/src/lib/components/ContextNavGroup.svelte` | Owner-grouped context list |
| `packages/temper-ui/src/lib/components/VaultGrid.svelte` | SVAR DataGrid wrapper with URL-bound state |
| `packages/temper-ui/src/lib/components/FacetChips.svelte` | Doc-type filter chips |
| `packages/temper-ui/src/lib/components/RuleHeading.svelte` | Editorial left-rule heading (if not already present) |
| `packages/temper-ui/src/lib/components/EmptyState.svelte` | Generic empty state |
| `packages/temper-ui/src/lib/components/MarkdownRenderer.svelte` | marked + dompurify render |
| `packages/temper-ui/src/lib/components/ResourceMetaHeader.svelte` | Detail page header |
| `packages/temper-ui/src/lib/components/CommandPalette.svelte` | ⌘K overlay |
| `packages/temper-ui/src/routes/(app)/vault/+page.server.ts` | Redirect to /vault/all |
| `packages/temper-ui/src/routes/(app)/vault/all/+page.server.ts` | All-resources load |
| `packages/temper-ui/src/routes/(app)/vault/all/+page.svelte` | All-resources grid page |
| `packages/temper-ui/src/routes/(app)/vault/search/+page.server.ts` | Search results load |
| `packages/temper-ui/src/routes/(app)/vault/search/+page.svelte` | Search results page |
| `packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/+page.server.ts` | Context grid load |
| `packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/+page.svelte` | Context grid page |
| `packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/[doc_type]/[ident]/+page.server.ts` | Detail load (by-uri + content) |
| `packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/[doc_type]/[ident]/+page.svelte` | Detail page |
| `packages/temper-ui/src/routes/(app)/_internal/search/+server.ts` | ⌘K proxy endpoint |
| `packages/temper-ui/src/routes/(app)/teams/+page.svelte` | Placeholder stub |
| `packages/temper-ui/src/routes/(app)/settings/+page.svelte` | Placeholder stub |
| `packages/temper-ui/src/routes/(app)/+error.svelte` | In-shell error page |

### SvelteKit — Modified Files

| File | Changes |
|---|---|
| `packages/temper-ui/package.json` | Add SVAR, marked, dompurify deps |
| `packages/temper-ui/src/routes/(app)/+layout.svelte` | Sidebar shell + ⌘K listener |
| `packages/temper-ui/src/routes/(app)/+layout.server.ts` | Fetch contexts for sidebar |

### SvelteKit — Deleted Files

| File | Reason |
|---|---|
| `packages/temper-ui/src/routes/(app)/dashboard/+page.server.ts` | Replaced by /vault/all |
| `packages/temper-ui/src/routes/(app)/dashboard/+page.svelte` | Replaced by /vault/all |
| `packages/temper-ui/src/routes/(app)/dashboard/CLAUDE.md` | No longer needed |

---

## Phase 0: Database Migration

### Task 1: Add managed_meta indexes

**Files:**
- Create: `migrations/20260410000001_index_resource_manifests_managed_meta.sql`

- [ ] **Step 1: Create the migration file**

```sql
-- B-tree expression indexes for sortable/filterable managed_meta keys.
CREATE INDEX idx_manifests_managed_stage
    ON kb_resource_manifests ((managed_meta->>'temper-stage'));
CREATE INDEX idx_manifests_managed_seq
    ON kb_resource_manifests (((managed_meta->>'temper-seq')::bigint));
CREATE INDEX idx_manifests_managed_mode
    ON kb_resource_manifests ((managed_meta->>'temper-mode'));
CREATE INDEX idx_manifests_managed_effort
    ON kb_resource_manifests ((managed_meta->>'temper-effort'));
CREATE INDEX idx_manifests_managed_doc_type
    ON kb_resource_manifests ((managed_meta->>'temper-type'));

-- GIN with jsonb_path_ops for future ad-hoc containment queries.
CREATE INDEX idx_manifests_managed_meta_gin
    ON kb_resource_manifests USING gin (managed_meta jsonb_path_ops);
```

- [ ] **Step 2: Run the migration against local dev DB**

```bash
cargo make docker-up
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development sqlx migrate run
```

Expected: "Applied 20260410000001" — no errors.

- [ ] **Step 3: Regenerate sqlx offline cache**

```bash
cargo sqlx prepare --workspace -- --all-features
```

Expected: `.sqlx/` files updated.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260410000001_index_resource_manifests_managed_meta.sql .sqlx/
git commit -m "feat(db): add B-tree and GIN indexes on managed_meta"
```

---

## Phase 1: Shared Types (temper-core)

### Task 2: Add ResourceSortField, SortOrder enums and extend ResourceListParams

**Files:**
- Modify: `crates/temper-core/src/types/resource.rs`

- [ ] **Step 1: Add the enums and extend ResourceListParams**

Add these types to `crates/temper-core/src/types/resource.rs`, after the existing `ResourceListParams`:

```rust
/// Sort field for resource listing.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ResourceSortField {
    #[default]
    Updated,
    Created,
    Title,
    Stage,
    Seq,
}

/// Sort direction.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    #[default]
    Desc,
    Asc,
}
```

Then replace the existing `ResourceListParams` struct with:

```rust
/// Query parameters for listing visible resources.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceListParams {
    /// Filter by context ID (UUID).
    pub kb_context_id: Option<Uuid>,
    /// Filter by document type ID (UUID).
    pub kb_doc_type_id: Option<Uuid>,
    /// Filter by context name (alternative to kb_context_id).
    pub context_name: Option<String>,
    /// Filter by document type name (alternative to kb_doc_type_id).
    pub doc_type_name: Option<String>,
    /// Filter by owner sigil: "@me" or "+team-slug".
    pub owner: Option<String>,
    /// Full-text search query.
    pub q: Option<String>,
    /// Sort field (default: updated).
    pub sort: Option<ResourceSortField>,
    /// Sort direction (default: desc).
    pub order: Option<SortOrder>,
    /// Maximum results to return (default 50, max 200).
    #[cfg_attr(feature = "typescript", ts(type = "number | null"))]
    pub limit: Option<i64>,
    /// Offset for pagination.
    #[cfg_attr(feature = "typescript", ts(type = "number | null"))]
    pub offset: Option<i64>,
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cargo check -p temper-core --all-features
```

Expected: compiles clean (downstream crates will have errors — that's expected and handled in later tasks).

### Task 3: Extend ResourceRow and add response types

**Files:**
- Modify: `crates/temper-core/src/types/resource.rs`

- [ ] **Step 1: Extend ResourceRow with display fields and managed_meta projections**

Replace the existing `ResourceRow` struct:

```rust
/// Row type for resource list/detail responses.
///
/// Extends the bare `kb_resources` columns with joined display fields
/// (context_name, doc_type_name, owner_handle) and managed_meta JSONB
/// projections (stage, seq, mode, effort).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceRow {
    pub id: ResourceId,
    pub kb_context_id: ContextId,
    pub kb_doc_type_id: DocTypeId,
    pub origin_uri: String,
    pub title: String,
    pub slug: Option<String>,
    pub originator_profile_id: ProfileId,
    pub owner_profile_id: ProfileId,
    pub is_active: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    // Joined display fields
    pub context_name: String,
    pub doc_type_name: String,
    /// Owner sigil relative to the requester: "@me" or "+team-slug".
    pub owner_handle: String,
    // Managed meta projections (from kb_resource_manifests.managed_meta)
    pub stage: Option<String>,
    #[cfg_attr(feature = "typescript", ts(type = "number | null"))]
    pub seq: Option<i64>,
    pub mode: Option<String>,
    pub effort: Option<String>,
}
```

- [ ] **Step 2: Add ResourceListResponse**

```rust
/// Paginated response for resource list endpoints.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceListResponse {
    pub rows: Vec<ResourceRow>,
    pub total: i64,
}
```

- [ ] **Step 3: Add ResourceFacets**

```rust
/// Aggregated facet counts for the current filter set.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceFacets {
    pub doc_type: std::collections::HashMap<String, i64>,
    pub stage: std::collections::HashMap<String, i64>,
}
```

- [ ] **Step 4: Verify temper-core compiles**

```bash
cargo check -p temper-core --all-features
```

### Task 4: Add ContextRowWithCounts

**Files:**
- Modify: `crates/temper-core/src/types/context.rs`

- [ ] **Step 1: Add the new type**

Add after the existing `ContextRow`:

```rust
/// Context with resource count — used by the list endpoint.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ContextRowWithCounts {
    pub id: ContextId,
    pub name: String,
    pub kb_owner_table: String,
    pub kb_owner_id: Uuid,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    pub resource_count: i64,
}
```

- [ ] **Step 2: Export from mod.rs if needed**

Check `crates/temper-core/src/types/mod.rs` — if types are re-exported, add `ContextRowWithCounts` to the re-exports.

- [ ] **Step 3: Verify temper-core compiles**

```bash
cargo check -p temper-core --all-features
```

- [ ] **Step 4: Commit all type changes**

```bash
git add crates/temper-core/
git commit -m "feat(core): add vault browser types — extended ResourceRow, ResourceListResponse, ResourceFacets, ContextRowWithCounts"
```

---

## Phase 2: API Service Layer (TDD)

### Task 5: Rewrite resource_service::list_visible with extended query

**Files:**
- Modify: `crates/temper-api/src/services/resource_service.rs`
- Test: `crates/temper-api/tests/resources_browse_test.rs`

This is the largest single task. The existing `list_visible` uses a 4-branch match on (context_id, doc_type_id) with repeated SQL. We replace it with one parameterized query using runtime `sqlx::query_as` (because the sort column is dynamic — compile-time macros can't parameterize `ORDER BY`).

- [ ] **Step 1: Write the failing integration test**

Create `crates/temper-api/tests/resources_browse_test.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;

/// GET /api/resources returns ResourceListResponse { rows, total }.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_list_resources_returns_wrapped_response(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = common::create_test_user(&app, "browse-user").await;

    // Create a resource so the list is non-empty.
    let _created = common::create_test_resource(
        &app,
        &token,
        "Browse Test Resource",
        common::fixtures::TEMPER_CONTEXT_ID,
        common::fixtures::TASK_DOC_TYPE_ID,
    )
    .await;

    let resp = app
        .client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("list request failed");

    assert_eq!(resp.status().as_u16(), 200);

    let body: Value = resp.json().await.expect("expected JSON");
    // New shape: { rows: [...], total: N }
    assert!(body["rows"].is_array(), "response must have 'rows' array");
    assert!(body["total"].is_number(), "response must have 'total' number");
    assert!(body["total"].as_i64().unwrap() > 0);

    // Rows must have the new display fields
    let first = &body["rows"][0];
    assert!(first["context_name"].is_string(), "must have context_name");
    assert!(first["doc_type_name"].is_string(), "must have doc_type_name");
    assert!(first["owner_handle"].is_string(), "must have owner_handle");
}

/// GET /api/resources?context_name=temper filters by context name.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_list_resources_filter_by_context_name(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = common::create_test_user(&app, "ctx-filter-user").await;

    let _r1 = common::create_test_resource(
        &app,
        &token,
        "Temper Resource",
        common::fixtures::TEMPER_CONTEXT_ID,
        common::fixtures::TASK_DOC_TYPE_ID,
    )
    .await;

    let resp = app
        .client
        .get(app.url("/api/resources?context_name=temper"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.unwrap();
    let rows = body["rows"].as_array().unwrap();
    for row in rows {
        assert_eq!(row["context_name"].as_str().unwrap(), "temper");
    }
}

/// GET /api/resources?sort=title&order=asc sorts alphabetically.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_list_resources_sort_by_title_asc(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = common::create_test_user(&app, "sort-user").await;

    common::create_test_resource(
        &app, &token, "Zebra", common::fixtures::TEMPER_CONTEXT_ID,
        common::fixtures::TASK_DOC_TYPE_ID,
    ).await;
    common::create_test_resource(
        &app, &token, "Alpha", common::fixtures::TEMPER_CONTEXT_ID,
        common::fixtures::TASK_DOC_TYPE_ID,
    ).await;

    let resp = app
        .client
        .get(app.url("/api/resources?sort=title&order=asc"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();

    let body: Value = resp.json().await.unwrap();
    let rows = body["rows"].as_array().unwrap();
    if rows.len() >= 2 {
        let first_title = rows[0]["title"].as_str().unwrap();
        let second_title = rows[1]["title"].as_str().unwrap();
        assert!(first_title <= second_title, "expected ascending: {first_title} <= {second_title}");
    }
}
```

**Note:** The test file references helper functions `common::create_test_user` and `common::create_test_resource`. Check `crates/temper-api/tests/common/` for existing helpers. If they don't exist, create them based on the pattern in `resources_test.rs` (generate JWT, POST to /api/resources). Adapt the fixture constants from `common::fixtures` to match whatever the test common module already provides.

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo nextest run -p temper-api --features test-db test_list_resources_returns_wrapped_response
```

Expected: FAIL — the endpoint still returns `Vec<ResourceRow>`, not `ResourceListResponse`.

- [ ] **Step 3: Implement the extended list_visible**

In `crates/temper-api/src/services/resource_service.rs`, replace the existing `list_visible` function. The new implementation uses runtime `sqlx::query_as` because the ORDER BY column is dynamic:

```rust
use temper_core::types::resource::{
    ResourceListParams, ResourceListResponse, ResourceRow, ResourceSortField, SortOrder,
};

pub async fn list_visible(
    pool: &PgPool,
    profile_id: Uuid,
    params: ResourceListParams,
) -> ApiResult<ResourceListResponse> {
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0).max(0);
    let sort_field = params.sort.unwrap_or_default();
    let sort_order = params.order.unwrap_or_default();

    // Build the ORDER BY clause based on the sort field.
    let order_clause = match (sort_field, sort_order) {
        (ResourceSortField::Updated, SortOrder::Desc) => "r.updated DESC",
        (ResourceSortField::Updated, SortOrder::Asc) => "r.updated ASC",
        (ResourceSortField::Created, SortOrder::Desc) => "r.created DESC",
        (ResourceSortField::Created, SortOrder::Asc) => "r.created ASC",
        (ResourceSortField::Title, SortOrder::Desc) => "r.title DESC",
        (ResourceSortField::Title, SortOrder::Asc) => "r.title ASC",
        (ResourceSortField::Stage, SortOrder::Desc) => "m.managed_meta->>'temper-stage' DESC NULLS LAST",
        (ResourceSortField::Stage, SortOrder::Asc) => "m.managed_meta->>'temper-stage' ASC NULLS LAST",
        (ResourceSortField::Seq, SortOrder::Desc) => "(m.managed_meta->>'temper-seq')::bigint DESC NULLS LAST",
        (ResourceSortField::Seq, SortOrder::Asc) => "(m.managed_meta->>'temper-seq')::bigint ASC NULLS LAST",
    };

    // Build dynamic WHERE conditions.
    // We use numbered bind params starting from $1=profile_id.
    // This is necessarily runtime SQL — compile-time macros can't handle dynamic ORDER BY.
    let mut conditions = vec!["r.is_active = true".to_string()];
    let mut bind_offset = 1; // $1 is always profile_id

    if params.kb_context_id.is_some() {
        bind_offset += 1;
        conditions.push(format!("r.kb_context_id = ${bind_offset}"));
    }
    if params.kb_doc_type_id.is_some() {
        bind_offset += 1;
        conditions.push(format!("r.kb_doc_type_id = ${bind_offset}"));
    }
    if params.context_name.is_some() {
        bind_offset += 1;
        conditions.push(format!("c.name = ${bind_offset}"));
    }
    if params.doc_type_name.is_some() {
        bind_offset += 1;
        conditions.push(format!("dt.name = ${bind_offset}"));
    }
    if params.owner.as_ref().is_some_and(|o| !o.is_empty()) {
        // Owner filtering: "@me" → owner is the requesting profile;
        // "+slug" → owner is the team with that slug.
        let owner = params.owner.as_deref().unwrap();
        if owner == "@me" {
            conditions.push(format!(
                "(c.kb_owner_table = 'kb_profiles' AND c.kb_owner_id = $1)"
            ));
        } else if let Some(slug) = owner.strip_prefix('+') {
            bind_offset += 1;
            conditions.push(format!(
                "(c.kb_owner_table = 'kb_teams' AND t.slug = ${bind_offset})"
            ));
            // Note: slug is bound later
            let _ = slug; // used in bind below
        }
    }
    if params.q.as_ref().is_some_and(|q| !q.trim().is_empty()) {
        bind_offset += 1;
        conditions.push(format!(
            "fts.tsvector @@ plainto_tsquery('english', ${bind_offset})"
        ));
    }

    let where_clause = conditions.join(" AND ");

    let sql = format!(
        r#"
        WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
        SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.origin_uri, r.title,
               r.slug, r.originator_profile_id, r.owner_profile_id, r.is_active,
               r.created, r.updated,
               c.name AS context_name,
               dt.name AS doc_type_name,
               CASE
                 WHEN c.kb_owner_table = 'kb_profiles' AND c.kb_owner_id = $1 THEN '@me'
                 WHEN c.kb_owner_table = 'kb_teams' THEN '+' || t.slug
                 ELSE '@unknown'
               END AS owner_handle,
               m.managed_meta->>'temper-stage' AS stage,
               (m.managed_meta->>'temper-seq')::bigint AS seq,
               m.managed_meta->>'temper-mode' AS mode,
               m.managed_meta->>'temper-effort' AS effort
          FROM kb_resources r
          JOIN visible v ON v.resource_id = r.id
          JOIN kb_contexts c ON c.id = r.kb_context_id
          JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
          JOIN kb_profiles p ON p.id = r.owner_profile_id
          LEFT JOIN kb_resource_manifests m ON m.resource_id = r.id
          LEFT JOIN kb_teams t ON c.kb_owner_table = 'kb_teams' AND t.id = c.kb_owner_id
          LEFT JOIN kb_fts_index fts ON fts.resource_id = r.id
         WHERE {where_clause}
         ORDER BY {order_clause}
         LIMIT {limit} OFFSET {offset}
        "#
    );

    // Build a matching COUNT query with the same WHERE.
    let count_sql = format!(
        r#"
        WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
        SELECT COUNT(*) as "count!"
          FROM kb_resources r
          JOIN visible v ON v.resource_id = r.id
          JOIN kb_contexts c ON c.id = r.kb_context_id
          JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
          LEFT JOIN kb_resource_manifests m ON m.resource_id = r.id
          LEFT JOIN kb_teams t ON c.kb_owner_table = 'kb_teams' AND t.id = c.kb_owner_id
          LEFT JOIN kb_fts_index fts ON fts.resource_id = r.id
         WHERE {where_clause}
        "#
    );

    // Build and execute both queries with the same bindings.
    let mut query = sqlx::query_as::<_, ResourceRow>(&sql).bind(profile_id);
    let mut count_query = sqlx::query_scalar::<_, i64>(&count_sql).bind(profile_id);

    // Bind params in the same order as conditions were added.
    if let Some(ctx_id) = params.kb_context_id {
        query = query.bind(ctx_id);
        count_query = count_query.bind(ctx_id);
    }
    if let Some(dt_id) = params.kb_doc_type_id {
        query = query.bind(dt_id);
        count_query = count_query.bind(dt_id);
    }
    if let Some(ref ctx_name) = params.context_name {
        query = query.bind(ctx_name);
        count_query = count_query.bind(ctx_name);
    }
    if let Some(ref dt_name) = params.doc_type_name {
        query = query.bind(dt_name);
        count_query = count_query.bind(dt_name);
    }
    if let Some(ref owner) = params.owner {
        if let Some(slug) = owner.strip_prefix('+') {
            query = query.bind(slug.to_string());
            count_query = count_query.bind(slug.to_string());
        }
        // "@me" doesn't add a bind — it references $1 (profile_id)
    }
    if let Some(ref q) = params.q {
        if !q.trim().is_empty() {
            query = query.bind(q);
            count_query = count_query.bind(q);
        }
    }

    let rows = query.fetch_all(pool).await?;
    let total = count_query.fetch_one(pool).await?;

    Ok(ResourceListResponse { rows, total })
}
```

**Important notes for the implementer:**
- This uses runtime `sqlx::query_as` (not `sqlx::query_as!` macro) because ORDER BY is dynamic. This is acceptable per CLAUDE.md: the search_service already uses this pattern for the same reason.
- The `kb_fts_index` table name may be different — grep for `CREATE TABLE.*fts` in the migrations to find the actual name. If FTS is on `kb_chunks` directly, adjust the JOIN accordingly.
- The bind-parameter numbering must match exactly. If the logic gets complex, consider using a query-builder crate like `sea-query` — but for this scope, string formatting with sequential binds is simpler.

- [ ] **Step 4: Run the tests**

```bash
cargo nextest run -p temper-api --features test-db test_list_resources
```

Expected: all three new tests pass. Fix any SQL or bind issues.

- [ ] **Step 5: Update the handler to return the new shape**

In `crates/temper-api/src/handlers/resources.rs`, update the `list` handler:

```rust
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<ResourceListParams>,
) -> ApiResult<Json<ResourceListResponse>> {
    resource_service::list_visible(&state.pool, auth.0.profile.id, params)
        .await
        .map(Json)
}
```

Update the utoipa annotation to reference `ResourceListResponse` instead of `Vec<ResourceRow>`.

- [ ] **Step 6: Run tests again, commit**

```bash
cargo nextest run -p temper-api --features test-db test_list_resources
git add crates/temper-api/src/ crates/temper-api/tests/
git commit -m "feat(api): extend GET /api/resources with filters, sort, pagination, wrapped response"
```

### Task 6: Add compute_facets service + endpoint

**Files:**
- Modify: `crates/temper-api/src/services/resource_service.rs`
- Modify: `crates/temper-api/src/handlers/resources.rs`
- Modify: `crates/temper-api/src/routes.rs`
- Test: `crates/temper-api/tests/resources_facets_test.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/temper-api/tests/resources_facets_test.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

use serde_json::Value;
use sqlx::PgPool;

/// GET /api/resources/facets returns doc_type and stage counts.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_facets_returns_counts(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = common::create_test_user(&app, "facets-user").await;

    common::create_test_resource(
        &app, &token, "Facet Resource",
        common::fixtures::TEMPER_CONTEXT_ID,
        common::fixtures::TASK_DOC_TYPE_ID,
    ).await;

    let resp = app
        .client
        .get(app.url("/api/resources/facets"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.unwrap();
    assert!(body["doc_type"].is_object(), "must have doc_type facets");
    assert!(body["stage"].is_object(), "must have stage facets");
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo nextest run -p temper-api --features test-db test_facets_returns_counts
```

Expected: FAIL — route doesn't exist yet.

- [ ] **Step 3: Implement compute_facets**

Add to `crates/temper-api/src/services/resource_service.rs`:

```rust
use temper_core::types::resource::ResourceFacets;
use std::collections::HashMap;

/// Row for facet aggregation.
#[derive(Debug, sqlx::FromRow)]
struct FacetRow {
    facet_key: String,
    facet_value: Option<String>,
    count: i64,
}

pub async fn compute_facets(
    pool: &PgPool,
    profile_id: Uuid,
    params: ResourceListParams,
) -> ApiResult<ResourceFacets> {
    // Use the same WHERE conditions as list_visible but GROUP BY instead of paginate.
    // For simplicity, compute two queries: one for doc_type, one for stage.
    let doc_type_rows = sqlx::query_as::<_, FacetRow>(
        r#"
        WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
        SELECT dt.name AS facet_key, NULL AS facet_value,
               COUNT(*) AS count
          FROM kb_resources r
          JOIN visible v ON v.resource_id = r.id
          JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
         WHERE r.is_active = true
         GROUP BY dt.name
         ORDER BY count DESC
        "#,
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await?;

    let stage_rows = sqlx::query_as::<_, FacetRow>(
        r#"
        WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
        SELECT COALESCE(m.managed_meta->>'temper-stage', 'none') AS facet_key,
               NULL AS facet_value,
               COUNT(*) AS count
          FROM kb_resources r
          JOIN visible v ON v.resource_id = r.id
          LEFT JOIN kb_resource_manifests m ON m.resource_id = r.id
         WHERE r.is_active = true
         GROUP BY facet_key
         ORDER BY count DESC
        "#,
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await?;

    let doc_type: HashMap<String, i64> = doc_type_rows
        .into_iter()
        .map(|r| (r.facet_key, r.count))
        .collect();

    let stage: HashMap<String, i64> = stage_rows
        .into_iter()
        .map(|r| (r.facet_key, r.count))
        .collect();

    Ok(ResourceFacets { doc_type, stage })
}
```

**Note:** This simplified version doesn't apply the context_name/doc_type_name/owner/q filters to the facet queries. For v1 this is acceptable — facets show the global distribution. If filtered facets are needed, add the same WHERE-clause builder from `list_visible`. The spec says "same access predicate, same WHERE filters" — implement that if time permits, or add a TODO and come back in a follow-up.

- [ ] **Step 4: Add the handler**

In `crates/temper-api/src/handlers/resources.rs`:

```rust
use temper_core::types::resource::ResourceFacets;

#[utoipa::path(
    get,
    path = "/api/resources/facets",
    tag = "Resources",
    params(ResourceListParams),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Facet counts", body = ResourceFacets),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn facets(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<ResourceListParams>,
) -> ApiResult<Json<ResourceFacets>> {
    resource_service::compute_facets(&state.pool, auth.0.profile.id, params)
        .await
        .map(Json)
}
```

- [ ] **Step 5: Wire the route**

In `crates/temper-api/src/routes.rs`, inside the `gated` router, add before the existing `/api/resources` route:

```rust
.route("/api/resources/facets", get(handlers::resources::facets))
```

**Important:** This route must come BEFORE `/api/resources/{id}` to avoid path conflicts — `facets` would match as an `{id}` param otherwise.

- [ ] **Step 6: Run tests, commit**

```bash
cargo nextest run -p temper-api --features test-db test_facets
git add crates/temper-api/
git commit -m "feat(api): add GET /api/resources/facets endpoint"
```

### Task 7: Add resolve_by_uri service + endpoint

**Files:**
- Modify: `crates/temper-api/src/services/resource_service.rs`
- Modify: `crates/temper-api/src/handlers/resources.rs`
- Modify: `crates/temper-api/src/routes.rs`
- Test: `crates/temper-api/tests/resources_by_uri_test.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/temper-api/tests/resources_by_uri_test.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

use serde_json::Value;
use sqlx::PgPool;

/// GET /api/resources/by-uri resolves a slug to a resource.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_resolve_by_uri_with_slug(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = common::create_test_user(&app, "uri-user").await;

    let created: Value = common::create_test_resource(
        &app, &token, "URI Test Resource",
        common::fixtures::TEMPER_CONTEXT_ID,
        common::fixtures::TASK_DOC_TYPE_ID,
    ).await;

    let resource_id = created["id"].as_str().unwrap();

    // Resolve by UUID (as ident).
    let resp = app
        .client
        .get(app.url(&format!(
            "/api/resources/by-uri?owner=@me&context=temper&doc_type=task&ident={resource_id}"
        )))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["id"].as_str().unwrap(), resource_id);
}

/// GET /api/resources/by-uri returns 404 for nonexistent resource.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_resolve_by_uri_not_found(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, _) = common::create_test_user(&app, "uri-404-user").await;

    let resp = app
        .client
        .get(app.url("/api/resources/by-uri?owner=@me&context=temper&doc_type=task&ident=nonexistent"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status().as_u16(), 404);
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo nextest run -p temper-api --features test-db test_resolve_by_uri
```

- [ ] **Step 3: Implement resolve_by_uri**

Add to `crates/temper-api/src/services/resource_service.rs`:

```rust
/// Query params for the by-uri endpoint.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
pub struct ResolveByUriParams {
    pub owner: String,
    pub context: String,
    pub doc_type: String,
    pub ident: String,
}

pub async fn resolve_by_uri(
    pool: &PgPool,
    profile_id: Uuid,
    params: ResolveByUriParams,
) -> ApiResult<ResourceRow> {
    // Try ident as UUID first, then as slug.
    let ident_uuid = Uuid::try_parse(&params.ident).ok();

    let row = sqlx::query_as::<_, ResourceRow>(
        r#"
        WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
        SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.origin_uri, r.title,
               r.slug, r.originator_profile_id, r.owner_profile_id, r.is_active,
               r.created, r.updated,
               c.name AS context_name,
               dt.name AS doc_type_name,
               CASE
                 WHEN c.kb_owner_table = 'kb_profiles' AND c.kb_owner_id = $1 THEN '@me'
                 WHEN c.kb_owner_table = 'kb_teams' THEN '+' || t.slug
                 ELSE '@unknown'
               END AS owner_handle,
               m.managed_meta->>'temper-stage' AS stage,
               (m.managed_meta->>'temper-seq')::bigint AS seq,
               m.managed_meta->>'temper-mode' AS mode,
               m.managed_meta->>'temper-effort' AS effort
          FROM kb_resources r
          JOIN visible v ON v.resource_id = r.id
          JOIN kb_contexts c ON c.id = r.kb_context_id
          JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
          JOIN kb_profiles p ON p.id = r.owner_profile_id
          LEFT JOIN kb_resource_manifests m ON m.resource_id = r.id
          LEFT JOIN kb_teams t ON c.kb_owner_table = 'kb_teams' AND t.id = c.kb_owner_id
         WHERE r.is_active = true
           AND c.name = $2
           AND dt.name = $3
           AND (r.id = $4 OR r.slug = $5)
        "#,
    )
    .bind(profile_id)
    .bind(&params.context)
    .bind(&params.doc_type)
    .bind(ident_uuid)
    .bind(&params.ident)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;

    Ok(row)
}
```

- [ ] **Step 4: Add the handler**

In `crates/temper-api/src/handlers/resources.rs`:

```rust
use crate::services::resource_service::ResolveByUriParams;

#[utoipa::path(
    get,
    path = "/api/resources/by-uri",
    tag = "Resources",
    params(ResolveByUriParams),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Resolved resource", body = ResourceRow),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
    )
)]
pub async fn by_uri(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<ResolveByUriParams>,
) -> ApiResult<Json<ResourceRow>> {
    resource_service::resolve_by_uri(&state.pool, auth.0.profile.id, params)
        .await
        .map(Json)
}
```

- [ ] **Step 5: Wire the route**

In `crates/temper-api/src/routes.rs`, add inside the `gated` router (above `/api/resources/{id}`):

```rust
.route("/api/resources/by-uri", get(handlers::resources::by_uri))
```

- [ ] **Step 6: Run tests, commit**

```bash
cargo nextest run -p temper-api --features test-db test_resolve_by_uri
git add crates/temper-api/
git commit -m "feat(api): add GET /api/resources/by-uri endpoint"
```

### Task 8: Update context_service to return counts

**Files:**
- Modify: `crates/temper-api/src/services/context_service.rs`
- Modify: `crates/temper-api/src/handlers/contexts.rs`

- [ ] **Step 1: Add list_visible_with_counts to context_service**

In `crates/temper-api/src/services/context_service.rs`:

```rust
use temper_core::types::context::ContextRowWithCounts;

pub async fn list_visible_with_counts(
    pool: &PgPool,
    profile_id: ProfileId,
) -> ApiResult<Vec<ContextRowWithCounts>> {
    let rows = sqlx::query_as!(
        ContextRowWithCounts,
        r#"
        SELECT c.id, c.name, c.kb_owner_table, c.kb_owner_id, c.created, c.updated,
               COUNT(r.id) AS "resource_count!"
          FROM contexts_visible_to($1) cv
          JOIN kb_contexts c ON c.id = cv.id
          LEFT JOIN kb_resources r ON r.kb_context_id = c.id AND r.is_active = true
         GROUP BY c.id, c.name, c.kb_owner_table, c.kb_owner_id, c.created, c.updated
         ORDER BY c.name
        "#,
        *profile_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
```

- [ ] **Step 2: Update the handler**

In `crates/temper-api/src/handlers/contexts.rs`, update the `list` function:

```rust
use crate::services::context_service::{self, ContextRowWithCounts};

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Vec<ContextRowWithCounts>>> {
    context_service::list_visible_with_counts(&state.pool, ProfileId::from(auth.0.profile.id))
        .await
        .map(Json)
}
```

Update the utoipa annotation to reference `Vec<ContextRowWithCounts>`.

- [ ] **Step 3: Run existing context tests**

```bash
cargo nextest run -p temper-api --features test-db context
```

Fix any test that expects the old shape.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/
git commit -m "feat(api): GET /api/contexts now returns resource counts"
```

---

## Phase 3: Cascade Fixes

### Task 9: Fix temper-client for new response shapes

**Files:**
- Modify: `crates/temper-client/src/resources.rs`
- Modify: `crates/temper-client/src/contexts.rs` (if exists, or wherever contexts are called)

- [ ] **Step 1: Update resources client**

In `crates/temper-client/src/resources.rs`, change the `list` return type:

```rust
use temper_core::types::resource::ResourceListResponse;

pub async fn list(&self, params: &ResourceListParams) -> Result<ResourceListResponse> {
    // ... existing HTTP call, change deserialization target
}
```

- [ ] **Step 2: Update contexts client**

Find the contexts list call in `crates/temper-client/src/` and change it to return `Vec<ContextRowWithCounts>`.

- [ ] **Step 3: Fix any temper-cli and temper-mcp consumers**

Run a workspace-wide build to find all compile errors:

```bash
cargo build --workspace --all-features 2>&1 | head -80
```

For each error:
- If it's a CLI command printing resource rows: access `.rows` on the response, and handle the new fields (context_name, doc_type_name, etc.) — either display them or ignore them.
- If it's a CLI command printing context rows: handle the `resource_count` field.
- If it's an MCP tool consuming the service directly: the service return type changed; update the MCP tool's signature.

**Do not** extend the CLI's table rendering to use the new fields — that's explicitly out of scope per the spec (§9: "CLI extension to use new pagination/sort/filter machinery — hold for a future task"). Just make it compile.

- [ ] **Step 4: Build clean, commit**

```bash
cargo build --workspace --all-features
git add crates/
git commit -m "fix(cascade): update temper-client, temper-cli, temper-mcp for new response shapes"
```

### Task 10: Regenerate ts-rs types and sqlx cache

**Files:**
- Regenerate: `packages/temper-core-types/`
- Regenerate: `.sqlx/`

- [ ] **Step 1: Regenerate TypeScript types**

```bash
cargo make generate-ts-types
```

- [ ] **Step 2: Regenerate sqlx offline cache**

```bash
cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 3: Run full check**

```bash
cargo make check
```

Expected: all quality gates pass.

- [ ] **Step 4: Commit**

```bash
git add packages/temper-core-types/ .sqlx/
git commit -m "build: regenerate ts-rs types and sqlx cache for vault browser"
```

---

## Phase 4: SvelteKit Shell

### Task 11: Add frontend dependencies

**Files:**
- Modify: `packages/temper-ui/package.json`

- [ ] **Step 1: Install deps**

```bash
cd packages/temper-ui
bun add wx-svelte-grid marked dompurify
bun add -d @types/dompurify
```

**Note:** The SVAR package name may be different — check https://svar.dev/svelte/datagrid/getting-started/ for the exact npm name. Common variants: `wx-svelte-grid`, `@wx/svelte-grid`, `@svar/svelte-grid`.

- [ ] **Step 2: Verify installed**

```bash
bun run check
```

- [ ] **Step 3: Commit**

```bash
git add packages/temper-ui/package.json packages/temper-ui/bun.lockb
git commit -m "build(temper-ui): add SVAR DataGrid, marked, dompurify"
```

### Task 12: Create EmptyState component

**Files:**
- Create: `packages/temper-ui/src/lib/components/EmptyState.svelte`

- [ ] **Step 1: Create the component**

```svelte
<script lang="ts">
  interface Props {
    message: string;
    action?: { label: string; href: string } | undefined;
  }

  let { message, action }: Props = $props();
</script>

<div class="flex flex-col items-center justify-center gap-3 py-16 text-zinc-500">
  <p class="text-sm">{message}</p>
  {#if action}
    <a
      href={action.href}
      class="text-xs text-yellow-500 hover:text-yellow-400 border border-zinc-700 rounded px-3 py-1"
    >
      {action.label}
    </a>
  {/if}
</div>
```

- [ ] **Step 2: Verify**

```bash
cd packages/temper-ui && bun run check
```

### Task 13: Create RuleHeading component

**Files:**
- Create: `packages/temper-ui/src/lib/components/RuleHeading.svelte` (skip if already exists)

- [ ] **Step 1: Check if it exists**

```bash
ls packages/temper-ui/src/lib/components/RuleHeading.svelte 2>/dev/null
```

If it exists, skip this task.

- [ ] **Step 2: Create the component**

```svelte
<script lang="ts">
  interface Props {
    title: string;
    caption?: string | undefined;
  }

  let { title, caption }: Props = $props();
</script>

<div class="border-l-2 border-yellow-500 pl-3">
  <h2 class="text-lg font-medium text-zinc-100">{title}</h2>
  {#if caption}
    <p class="text-xs text-zinc-400">{caption}</p>
  {/if}
</div>
```

### Task 14: Create ContextNavGroup and Sidebar components

**Files:**
- Create: `packages/temper-ui/src/lib/components/ContextNavGroup.svelte`
- Create: `packages/temper-ui/src/lib/components/Sidebar.svelte`

- [ ] **Step 1: Create ContextNavGroup**

```svelte
<script lang="ts">
  import { page } from '$app/stores';
  import type { ContextRowWithCounts } from '$lib/types/context';

  interface Props {
    label: string;
    ownerPrefix: string;
    contexts: ContextRowWithCounts[];
  }

  let { label, ownerPrefix, contexts }: Props = $props();

  function isActive(ctx: ContextRowWithCounts): boolean {
    return $page.params.owner === ownerPrefix && $page.params.context === ctx.name;
  }
</script>

<div class="px-3 pt-4 pb-1 text-[10px] uppercase tracking-widest text-zinc-500">
  {label}
</div>
{#each contexts as ctx}
  <a
    href="/vault/{ownerPrefix}/{ctx.name}"
    class="flex items-center gap-2 px-3 py-1.5 text-sm transition-colors
           {isActive(ctx)
             ? 'border-l-2 border-yellow-500 bg-zinc-800/50 text-zinc-100 pl-[calc(0.75rem-2px)]'
             : 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/30'}"
  >
    <span class="w-1.5 h-1.5 rounded-sm {isActive(ctx) ? 'bg-yellow-500' : 'bg-zinc-600'}"></span>
    <span class="flex-1 truncate">{ctx.name}</span>
    <span class="text-xs text-zinc-600">{ctx.resource_count}</span>
  </a>
{/each}
```

- [ ] **Step 2: Create Sidebar**

```svelte
<script lang="ts">
  import { page } from '$app/stores';
  import ContextNavGroup from './ContextNavGroup.svelte';
  import type { ContextRowWithCounts } from '$lib/types/context';

  interface Props {
    contexts: ContextRowWithCounts[];
    user: { display_name: string; email: string } | null;
    isAdmin: boolean;
  }

  let { contexts, user, isAdmin }: Props = $props();

  // Group contexts by owner_table: @me first, then teams.
  let myContexts = $derived(
    contexts.filter((c) => c.kb_owner_table === 'kb_profiles')
  );
  let teamContexts = $derived(
    contexts.filter((c) => c.kb_owner_table === 'kb_teams')
  );

  let isAllActive = $derived($page.url.pathname === '/vault/all' || $page.url.pathname === '/vault');
</script>

<aside class="flex flex-col w-52 bg-zinc-900/50 border-r border-zinc-800 overflow-hidden">
  <!-- Scrollable contexts -->
  <nav class="flex-1 overflow-y-auto py-2">
    <div class="px-3 pt-2 pb-1 text-[10px] uppercase tracking-widest text-zinc-500">Vault</div>
    <a
      href="/vault/all"
      class="flex items-center gap-2 px-3 py-1.5 text-sm transition-colors
             {isAllActive
               ? 'border-l-2 border-yellow-500 bg-zinc-800/50 text-zinc-100 pl-[calc(0.75rem-2px)]'
               : 'text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/30'}"
    >
      <span class="w-1.5 h-1.5 rounded-sm {isAllActive ? 'bg-yellow-500' : 'bg-zinc-600'}"></span>
      All resources
    </a>

    {#if myContexts.length > 0}
      <ContextNavGroup label="Contexts" ownerPrefix="@me" contexts={myContexts} />
    {/if}

    {#if teamContexts.length > 0}
      <ContextNavGroup label="Teams" ownerPrefix="+team" contexts={teamContexts} />
    {/if}
  </nav>

  <!-- Footer -->
  <div class="border-t border-zinc-800 py-2">
    <a href="/teams" class="flex items-center gap-2 px-3 py-1.5 text-sm text-zinc-400 hover:text-zinc-200">
      <span class="w-1.5 h-1.5 rounded-sm bg-zinc-600"></span>Teams
    </a>
    {#if isAdmin}
      <a href="/admin/access" class="flex items-center gap-2 px-3 py-1.5 text-sm text-zinc-400 hover:text-zinc-200">
        <span class="w-1.5 h-1.5 rounded-sm bg-zinc-600"></span>Admin
      </a>
    {/if}
    <a href="/settings" class="flex items-center gap-2 px-3 py-1.5 text-sm text-zinc-400 hover:text-zinc-200">
      <span class="w-1.5 h-1.5 rounded-sm bg-zinc-600"></span>Settings
    </a>
    {#if user}
      <div class="flex items-center gap-2 px-3 py-2 text-xs text-zinc-500">
        <div class="w-5 h-5 rounded-full bg-zinc-700 flex-shrink-0"></div>
        {user.display_name}
      </div>
    {/if}
  </div>
</aside>
```

**Note:** The `$lib/types/context` import path depends on how ts-rs generates the type file. Check `packages/temper-core-types/` for the actual output path and create a `$lib/types/` re-export barrel if needed.

- [ ] **Step 3: Verify**

```bash
cd packages/temper-ui && bun run check
```

- [ ] **Step 4: Commit**

```bash
git add packages/temper-ui/src/lib/components/
git commit -m "feat(temper-ui): add Sidebar, ContextNavGroup, RuleHeading, EmptyState components"
```

### Task 15: Modify (app) layout for sidebar shell

**Files:**
- Modify: `packages/temper-ui/src/routes/(app)/+layout.server.ts`
- Modify: `packages/temper-ui/src/routes/(app)/+layout.svelte`

- [ ] **Step 1: Update layout server load to fetch contexts**

Read the existing `+layout.server.ts` first. Add contexts fetch:

```ts
// Add to the existing load function's return data:
const contexts = await apiGet('/api/contexts', locals.accessToken)
  .catch(() => []);  // graceful degradation per spec §6.2

return {
  // ...existing user, profile, entitlements, accessToken...
  contexts,
};
```

- [ ] **Step 2: Update layout svelte for sidebar shell**

Read the existing `+layout.svelte` first. Replace the layout body with:

```svelte
<script lang="ts">
  import Sidebar from '$lib/components/Sidebar.svelte';
  import type { LayoutData } from './$types';

  let { data, children }: { data: LayoutData; children: any } = $props();
</script>

<div class="flex h-screen bg-zinc-950 text-zinc-100">
  <Sidebar
    contexts={data.contexts ?? []}
    user={data.profile ? { display_name: data.profile.display_name, email: data.profile.email ?? '' } : null}
    isAdmin={data.entitlements?.is_admin ?? false}
  />
  <main class="flex-1 overflow-y-auto">
    {@render children()}
  </main>
</div>
```

**Note:** The existing layout likely has an `<slot />` instead of `{@render children()}` — Svelte 5 uses the snippet pattern. Adapt to whatever the current file uses.

- [ ] **Step 3: Verify**

```bash
cd packages/temper-ui && bun run check
```

- [ ] **Step 4: Commit**

```bash
git add packages/temper-ui/src/routes/\(app\)/
git commit -m "feat(temper-ui): sidebar shell layout with context list"
```

---

## Phase 5: Vault Grid Pages

### Task 16: Create VaultGrid wrapper component

**Files:**
- Create: `packages/temper-ui/src/lib/components/VaultGrid.svelte`

- [ ] **Step 1: Create the component**

```svelte
<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/stores';
  import { Grid } from 'wx-svelte-grid';
  import type { ResourceRow } from '$lib/types/resource';

  interface Props {
    rows: ResourceRow[];
    total: number;
  }

  let { rows, total }: Props = $props();

  const columns = [
    { id: 'title', header: 'Title', flexgrow: 2, sort: true },
    { id: 'context_name', header: 'Context', width: 120, sort: true },
    { id: 'doc_type_name', header: 'Type', width: 100, sort: true },
    { id: 'stage', header: 'Stage', width: 100, sort: true },
    { id: 'updated', header: 'Updated', width: 120, sort: true },
    { id: 'seq', header: 'Seq', width: 60, sort: true },
  ];

  function handleRowClick(ev: CustomEvent) {
    const row: ResourceRow = ev.detail.row;
    const slug = row.slug ?? row.id;
    goto(`/vault/${row.owner_handle}/${row.context_name}/${row.doc_type_name}/${slug}`);
  }

  function handleSort(ev: CustomEvent) {
    const { id, order } = ev.detail;
    const url = new URL($page.url);
    url.searchParams.set('sort', id === 'updated' ? 'updated' : id);
    url.searchParams.set('order', order === 1 ? 'asc' : 'desc');
    goto(url.toString());
  }
</script>

<div class="h-full">
  <Grid
    data={rows}
    {columns}
    on:row-click={handleRowClick}
    on:sort={handleSort}
  />
  {#if total > rows.length}
    <div class="flex justify-between items-center px-4 py-2 text-xs text-zinc-500 border-t border-zinc-800">
      <span>Showing {rows.length} of {total}</span>
    </div>
  {/if}
</div>
```

**Note:** The SVAR DataGrid API may differ from what's shown above. Read the SVAR docs at https://svar.dev/svelte/datagrid/ for the correct component import name, prop names, and event names. Common adjustments:
- Import might be `import { DataGrid }` not `{ Grid }`
- Event names might be `onrowclick` instead of `on:row-click`
- Column config might use `field` instead of `id`
- The sort event detail shape may differ

Adapt the component to match the actual SVAR API after installation.

### Task 17: Create FacetChips component

**Files:**
- Create: `packages/temper-ui/src/lib/components/FacetChips.svelte`

- [ ] **Step 1: Create the component**

```svelte
<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/stores';

  interface Props {
    facets: Record<string, number> | null;
  }

  let { facets }: Props = $props();

  let activeDocType = $derived($page.url.searchParams.get('doc_type_name'));

  function toggleDocType(name: string) {
    const url = new URL($page.url);
    if (activeDocType === name) {
      url.searchParams.delete('doc_type_name');
    } else {
      url.searchParams.set('doc_type_name', name);
    }
    url.searchParams.delete('offset');
    goto(url.toString());
  }
</script>

{#if facets && Object.keys(facets).length > 0}
  <div class="flex flex-wrap gap-1.5">
    {#each Object.entries(facets).sort((a, b) => b[1] - a[1]) as [name, count]}
      <button
        onclick={() => toggleDocType(name)}
        class="px-2.5 py-0.5 rounded-full text-xs border transition-colors
               {activeDocType === name
                 ? 'bg-yellow-900/30 border-yellow-500 text-yellow-500'
                 : 'bg-zinc-800/50 border-zinc-700 text-zinc-400 hover:text-zinc-200'}"
      >
        {name} <span class="text-zinc-600">{count}</span>
      </button>
    {/each}
  </div>
{/if}
```

### Task 18: Create vault route pages

**Files:**
- Create: `packages/temper-ui/src/routes/(app)/vault/+page.server.ts`
- Create: `packages/temper-ui/src/routes/(app)/vault/all/+page.server.ts`
- Create: `packages/temper-ui/src/routes/(app)/vault/all/+page.svelte`
- Create: `packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/+page.server.ts`
- Create: `packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/+page.svelte`
- Delete: `packages/temper-ui/src/routes/(app)/dashboard/`

- [ ] **Step 1: Create /vault redirect**

`packages/temper-ui/src/routes/(app)/vault/+page.server.ts`:

```ts
import { redirect } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';

export const load: PageServerLoad = async () => {
  redirect(302, '/vault/all');
};
```

- [ ] **Step 2: Create /vault/all server load**

`packages/temper-ui/src/routes/(app)/vault/all/+page.server.ts`:

```ts
import type { PageServerLoad } from './$types';
import { apiGet } from '$lib/api/apiGet';

export const load: PageServerLoad = async ({ url, locals }) => {
  const qs = new URLSearchParams(
    Object.fromEntries(url.searchParams)
  ).toString();

  const [resources, facets] = await Promise.all([
    apiGet(`/api/resources${qs ? `?${qs}` : ''}`, locals.accessToken),
    apiGet(`/api/resources/facets${qs ? `?${qs}` : ''}`, locals.accessToken).catch(() => null),
  ]);

  return { resources, facets };
};
```

- [ ] **Step 3: Create /vault/all page**

`packages/temper-ui/src/routes/(app)/vault/all/+page.svelte`:

```svelte
<script lang="ts">
  import RuleHeading from '$lib/components/RuleHeading.svelte';
  import FacetChips from '$lib/components/FacetChips.svelte';
  import VaultGrid from '$lib/components/VaultGrid.svelte';
  import EmptyState from '$lib/components/EmptyState.svelte';
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();
</script>

<div class="flex flex-col gap-4 p-6 h-full">
  <RuleHeading
    title="Vault"
    caption="{data.resources?.total ?? 0} resources"
  />

  <FacetChips facets={data.facets?.doc_type ?? null} />

  {#if data.resources?.rows?.length > 0}
    <VaultGrid rows={data.resources.rows} total={data.resources.total} />
  {:else}
    <EmptyState message="No resources in the vault yet." />
  {/if}
</div>
```

- [ ] **Step 4: Create /vault/[owner]/[context] server load**

`packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/+page.server.ts`:

```ts
import type { PageServerLoad } from './$types';
import { apiGet } from '$lib/api/apiGet';
import { error } from '@sveltejs/kit';

export const load: PageServerLoad = async ({ params, url, locals }) => {
  const qs = new URLSearchParams({
    owner: params.owner,
    context_name: params.context,
    ...Object.fromEntries(url.searchParams),
  }).toString();

  try {
    const [resources, facets] = await Promise.all([
      apiGet(`/api/resources?${qs}`, locals.accessToken),
      apiGet(`/api/resources/facets?${qs}`, locals.accessToken).catch(() => null),
    ]);

    return { resources, facets, contextName: params.context, owner: params.owner };
  } catch (e: any) {
    if (e?.status === 404) {
      error(404, `Context "${params.context}" not found`);
    }
    throw e;
  }
};
```

- [ ] **Step 5: Create /vault/[owner]/[context] page**

`packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/+page.svelte`:

```svelte
<script lang="ts">
  import RuleHeading from '$lib/components/RuleHeading.svelte';
  import FacetChips from '$lib/components/FacetChips.svelte';
  import VaultGrid from '$lib/components/VaultGrid.svelte';
  import EmptyState from '$lib/components/EmptyState.svelte';
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();
</script>

<div class="flex flex-col gap-4 p-6 h-full">
  <RuleHeading
    title={data.contextName}
    caption="{data.resources?.total ?? 0} resources"
  />

  <FacetChips facets={data.facets?.doc_type ?? null} />

  {#if data.resources?.rows?.length > 0}
    <VaultGrid rows={data.resources.rows} total={data.resources.total} />
  {:else}
    <EmptyState message="No resources in this context." />
  {/if}
</div>
```

- [ ] **Step 6: Delete the old dashboard**

```bash
rm -rf packages/temper-ui/src/routes/\(app\)/dashboard/
```

- [ ] **Step 7: Verify and commit**

```bash
cd packages/temper-ui && bun run check
git add packages/temper-ui/src/
git rm -r packages/temper-ui/src/routes/\(app\)/dashboard/ 2>/dev/null
git commit -m "feat(temper-ui): vault grid pages — /vault/all and /vault/[owner]/[context]"
```

---

## Phase 6: Resource Detail Page

### Task 19: Create MarkdownRenderer and ResourceMetaHeader components

**Files:**
- Create: `packages/temper-ui/src/lib/components/MarkdownRenderer.svelte`
- Create: `packages/temper-ui/src/lib/components/ResourceMetaHeader.svelte`

- [ ] **Step 1: Create MarkdownRenderer**

```svelte
<script lang="ts">
  import { marked } from 'marked';
  import DOMPurify from 'dompurify';

  interface Props {
    markdown: string;
  }

  let { markdown }: Props = $props();

  let html = $derived(() => {
    try {
      const raw = marked.parse(markdown, { async: false }) as string;
      return DOMPurify.sanitize(raw);
    } catch {
      return '';
    }
  });
</script>

{#if html()}
  <div class="prose prose-invert prose-sm max-w-none
              prose-headings:text-zinc-100 prose-p:text-zinc-300
              prose-code:text-yellow-500 prose-code:bg-zinc-800/50
              prose-a:text-yellow-500 prose-strong:text-zinc-200">
    {@html html()}
  </div>
{:else}
  <div class="text-sm text-zinc-500 italic">
    This resource appears to be empty or contains unsupported content.
  </div>
{/if}
```

- [ ] **Step 2: Create ResourceMetaHeader**

```svelte
<script lang="ts">
  import RuleHeading from './RuleHeading.svelte';
  import type { ResourceRow } from '$lib/types/resource';

  interface Props {
    resource: ResourceRow;
  }

  let { resource }: Props = $props();

  function formatDate(date: string): string {
    return new Date(date).toLocaleDateString('en-US', {
      year: 'numeric', month: 'short', day: 'numeric'
    });
  }
</script>

<div class="flex flex-col gap-3">
  <RuleHeading
    title={resource.title}
    caption="{resource.doc_type_name} · {resource.context_name} · {formatDate(resource.updated)}"
  />

  <div class="flex gap-4 text-[10px] uppercase tracking-wide text-zinc-500">
    {#if resource.seq != null}
      <div><span class="text-zinc-600">seq</span> {resource.seq}</div>
    {/if}
    {#if resource.stage}
      <div><span class="text-zinc-600">stage</span> {resource.stage}</div>
    {/if}
    {#if resource.mode}
      <div><span class="text-zinc-600">mode</span> {resource.mode}</div>
    {/if}
    {#if resource.effort}
      <div><span class="text-zinc-600">effort</span> {resource.effort}</div>
    {/if}
    <div><span class="text-zinc-600">owner</span> {resource.owner_handle}</div>
  </div>
</div>
```

### Task 20: Create resource detail route

**Files:**
- Create: `packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/[doc_type]/[ident]/+page.server.ts`
- Create: `packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/[doc_type]/[ident]/+page.svelte`

- [ ] **Step 1: Create the server load**

```ts
import type { PageServerLoad } from './$types';
import { apiGet } from '$lib/api/apiGet';
import { error } from '@sveltejs/kit';

export const load: PageServerLoad = async ({ params, locals }) => {
  const uriQs = new URLSearchParams({
    owner: params.owner,
    context: params.context,
    doc_type: params.doc_type,
    ident: params.ident,
  }).toString();

  let resource;
  try {
    resource = await apiGet(`/api/resources/by-uri?${uriQs}`, locals.accessToken);
  } catch (e: any) {
    if (e?.status === 404) {
      error(404, 'Resource not found');
    }
    throw e;
  }

  let content;
  try {
    content = await apiGet(`/api/resources/${resource.id}/content`, locals.accessToken);
  } catch {
    content = { markdown: '' };
  }

  return { resource, content };
};
```

- [ ] **Step 2: Create the detail page**

```svelte
<script lang="ts">
  import ResourceMetaHeader from '$lib/components/ResourceMetaHeader.svelte';
  import MarkdownRenderer from '$lib/components/MarkdownRenderer.svelte';
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();
</script>

<div class="max-w-4xl mx-auto px-6 py-8 flex flex-col gap-6">
  <a
    href="/vault/{data.resource.owner_handle}/{data.resource.context_name}"
    class="text-xs text-zinc-500 hover:text-zinc-300"
  >
    &larr; {data.resource.context_name}
  </a>

  <ResourceMetaHeader resource={data.resource} />

  {#if data.content?.markdown}
    <MarkdownRenderer markdown={data.content.markdown} />
  {/if}
</div>
```

- [ ] **Step 3: Verify and commit**

```bash
cd packages/temper-ui && bun run check
git add packages/temper-ui/src/
git commit -m "feat(temper-ui): resource detail page with markdown rendering"
```

---

## Phase 7: Search (⌘K + Search Results Page)

### Task 21: Create the search proxy and CommandPalette

**Files:**
- Create: `packages/temper-ui/src/routes/(app)/_internal/search/+server.ts`
- Create: `packages/temper-ui/src/lib/components/CommandPalette.svelte`

- [ ] **Step 1: Create the search proxy endpoint**

`packages/temper-ui/src/routes/(app)/_internal/search/+server.ts`:

```ts
import { json } from '@sveltejs/kit';
import { apiGet } from '$lib/api/apiGet';
import type { RequestHandler } from './$types';

export const GET: RequestHandler = async ({ url, locals }) => {
  const q = url.searchParams.get('q') ?? '';
  if (!q.trim()) {
    return json({ rows: [], total: 0 });
  }

  try {
    const result = await apiGet(
      `/api/resources?q=${encodeURIComponent(q)}&limit=10`,
      locals.accessToken
    );
    return json(result);
  } catch {
    return json({ rows: [], total: 0 }, { status: 503 });
  }
};
```

- [ ] **Step 2: Create CommandPalette**

```svelte
<script lang="ts">
  import { goto } from '$app/navigation';
  import type { ResourceRow } from '$lib/types/resource';

  let open = $state(false);
  let query = $state('');
  let results = $state<ResourceRow[]>([]);
  let total = $state(0);
  let focused = $state(0);
  let loading = $state(false);
  let debounceTimer: ReturnType<typeof setTimeout>;

  export function toggle() {
    open = !open;
    if (open) {
      query = '';
      results = [];
      total = 0;
      focused = 0;
    }
  }

  async function search(q: string) {
    if (!q.trim()) {
      results = [];
      total = 0;
      return;
    }
    loading = true;
    try {
      const resp = await fetch(`/_internal/search?q=${encodeURIComponent(q)}`);
      const data = await resp.json();
      results = data.rows ?? [];
      total = data.total ?? 0;
    } catch {
      results = [];
      total = 0;
    }
    loading = false;
  }

  function onInput() {
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => search(query), 150);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      open = false;
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      focused = Math.min(focused + 1, results.length);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      focused = Math.max(focused - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (focused < results.length) {
        const row = results[focused];
        const slug = row.slug ?? row.id;
        goto(`/vault/${row.owner_handle}/${row.context_name}/${row.doc_type_name}/${slug}`);
        open = false;
      } else if (query.trim()) {
        goto(`/vault/search?q=${encodeURIComponent(query)}`);
        open = false;
      }
    }
  }
</script>

{#if open}
  <!-- Backdrop -->
  <button
    class="fixed inset-0 bg-black/60 z-40"
    onclick={() => (open = false)}
    aria-label="Close search"
  ></button>

  <!-- Palette -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="fixed top-[15%] left-1/2 -translate-x-1/2 w-full max-w-xl z-50
           bg-zinc-900 border border-zinc-700 rounded-lg shadow-2xl overflow-hidden"
    onkeydown={onKeydown}
  >
    <input
      type="text"
      bind:value={query}
      oninput={onInput}
      placeholder="Search the vault..."
      class="w-full px-4 py-3 bg-transparent text-zinc-100 text-sm
             border-b border-zinc-800 outline-none placeholder:text-zinc-500"
      autofocus
    />

    {#if results.length > 0}
      <div class="max-h-80 overflow-y-auto">
        {#each results as row, i}
          <button
            class="w-full text-left px-4 py-2.5 flex flex-col gap-0.5 transition-colors
                   {i === focused ? 'bg-zinc-800' : 'hover:bg-zinc-800/50'}"
            onclick={() => {
              const slug = row.slug ?? row.id;
              goto(`/vault/${row.owner_handle}/${row.context_name}/${row.doc_type_name}/${slug}`);
              open = false;
            }}
          >
            <span class="text-sm text-zinc-100">{row.title}</span>
            <span class="text-xs text-zinc-500">
              {row.context_name} · {row.doc_type_name}
              {#if row.stage} · {row.stage}{/if}
            </span>
          </button>
        {/each}
      </div>

      {#if total > results.length}
        <button
          class="w-full text-left px-4 py-2 text-xs text-yellow-500 hover:bg-zinc-800/50 border-t border-zinc-800"
          onclick={() => {
            goto(`/vault/search?q=${encodeURIComponent(query)}`);
            open = false;
          }}
        >
          See all {total} results
        </button>
      {/if}
    {:else if query.trim() && !loading}
      <div class="px-4 py-6 text-sm text-zinc-500 text-center">No results</div>
    {/if}
  </div>
{/if}
```

- [ ] **Step 3: Mount ⌘K in layout**

Update `packages/temper-ui/src/routes/(app)/+layout.svelte` to add the keyboard listener and CommandPalette:

```svelte
<script lang="ts">
  import Sidebar from '$lib/components/Sidebar.svelte';
  import CommandPalette from '$lib/components/CommandPalette.svelte';
  // ... existing imports

  let palette: CommandPalette;

  function onKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
      e.preventDefault();
      palette.toggle();
    }
  }
</script>

<svelte:window on:keydown={onKeydown} />

<div class="flex h-screen bg-zinc-950 text-zinc-100">
  <Sidebar {/* ...existing props */} />
  <main class="flex-1 overflow-y-auto">
    <!-- Search bar hint in top bar -->
    <header class="sticky top-0 z-10 flex items-center gap-3 px-6 py-3 bg-zinc-950/80 backdrop-blur border-b border-zinc-800">
      <button
        onclick={() => palette.toggle()}
        class="flex-1 flex items-center justify-between px-3 py-1.5 bg-zinc-900 border border-zinc-800 rounded text-sm text-zinc-500 hover:border-zinc-700"
      >
        <span>Search the vault...</span>
        <kbd class="text-[10px] bg-zinc-800 border border-zinc-700 rounded px-1.5 py-0.5 text-zinc-500">⌘K</kbd>
      </button>
    </header>

    {@render children()}
  </main>
</div>

<CommandPalette bind:this={palette} />
```

**Note:** Svelte 5 component binding via `bind:this` requires the component to have exported functions. Adapt the binding pattern to whatever works with Svelte 5 runes — may need a store or exported function approach instead.

### Task 22: Create search results page

**Files:**
- Create: `packages/temper-ui/src/routes/(app)/vault/search/+page.server.ts`
- Create: `packages/temper-ui/src/routes/(app)/vault/search/+page.svelte`

- [ ] **Step 1: Create server load**

```ts
import type { PageServerLoad } from './$types';
import { apiGet } from '$lib/api/apiGet';

export const load: PageServerLoad = async ({ url, locals }) => {
  const q = url.searchParams.get('q') ?? '';
  const qs = new URLSearchParams({
    q,
    ...Object.fromEntries(url.searchParams),
  }).toString();

  const [resources, facets] = await Promise.all([
    apiGet(`/api/resources?${qs}`, locals.accessToken),
    apiGet(`/api/resources/facets?${qs}`, locals.accessToken).catch(() => null),
  ]);

  return { resources, facets, query: q };
};
```

- [ ] **Step 2: Create the page**

```svelte
<script lang="ts">
  import RuleHeading from '$lib/components/RuleHeading.svelte';
  import FacetChips from '$lib/components/FacetChips.svelte';
  import VaultGrid from '$lib/components/VaultGrid.svelte';
  import EmptyState from '$lib/components/EmptyState.svelte';
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();
</script>

<div class="flex flex-col gap-4 p-6 h-full">
  <div class="flex items-center gap-3">
    <RuleHeading
      title="Search: {data.query}"
      caption="{data.resources?.total ?? 0} results"
    />
    <a href="/vault/all" class="text-xs text-zinc-500 hover:text-zinc-300 ml-auto">&times; Clear</a>
  </div>

  <FacetChips facets={data.facets?.doc_type ?? null} />

  {#if data.resources?.rows?.length > 0}
    <VaultGrid rows={data.resources.rows} total={data.resources.total} />
  {:else}
    <EmptyState
      message="No results for &ldquo;{data.query}&rdquo;"
      action={{ label: 'Browse all', href: '/vault/all' }}
    />
  {/if}
</div>
```

- [ ] **Step 3: Verify and commit**

```bash
cd packages/temper-ui && bun run check
git add packages/temper-ui/src/
git commit -m "feat(temper-ui): ⌘K command palette + search results page"
```

---

## Phase 8: Stubs, Error Page, and Acceptance

### Task 23: Create placeholder stubs and error page

**Files:**
- Create: `packages/temper-ui/src/routes/(app)/teams/+page.svelte`
- Create: `packages/temper-ui/src/routes/(app)/settings/+page.svelte`
- Create: `packages/temper-ui/src/routes/(app)/+error.svelte`

- [ ] **Step 1: Create teams stub**

```svelte
<div class="flex flex-col items-center justify-center h-full gap-4 text-zinc-500">
  <h2 class="text-lg font-medium text-zinc-200">Teams</h2>
  <p class="text-sm">Team management is coming in a future release.</p>
  <a href="/vault/all" class="text-xs text-yellow-500 hover:text-yellow-400">&larr; Back to vault</a>
</div>
```

- [ ] **Step 2: Create settings stub**

```svelte
<div class="flex flex-col items-center justify-center h-full gap-4 text-zinc-500">
  <h2 class="text-lg font-medium text-zinc-200">Settings</h2>
  <p class="text-sm">Settings page coming soon.</p>
  <a href="/vault/all" class="text-xs text-yellow-500 hover:text-yellow-400">&larr; Back to vault</a>
</div>
```

- [ ] **Step 3: Create error page**

```svelte
<script lang="ts">
  import { page } from '$app/stores';
</script>

<div class="flex flex-col items-center justify-center h-full gap-4 text-zinc-500">
  <div class="text-6xl font-light text-zinc-700">{$page.status}</div>
  <h2 class="text-lg font-medium text-zinc-200">{$page.error?.message ?? 'Something went wrong'}</h2>
  <a href="/vault/all" class="text-xs text-yellow-500 hover:text-yellow-400">&larr; Back to vault</a>
</div>
```

- [ ] **Step 4: Commit**

```bash
git add packages/temper-ui/src/routes/
git commit -m "feat(temper-ui): teams/settings stubs and in-shell error page"
```

### Task 24: Full verification

- [ ] **Step 1: Rust quality + tests**

```bash
cargo make check
cargo make test
cargo make test-db
```

All must pass.

- [ ] **Step 2: TypeScript quality + build**

```bash
cd packages/temper-ui && bun run check && bun run build
```

Must pass.

- [ ] **Step 3: Manual smoke test**

Run through all 12 items from the spec §7.6:

1. Sign in → `/vault/all` → populated grid
2. Click sidebar context → filtered grid, URL updates
3. Click doc-type chip → narrows, URL has `?doc_type_name=`
4. Sort column → resorts, URL has `?sort=&order=`
5. Click row → `/vault/@me/<ctx>/<type>/<slug>`, rendered markdown
6. Back button → grid, scroll preserved
7. ⌘K → overlay, type query, preview results
8. "See all results" → `/vault/search?q=...`
9. Sidebar "Admin" → `/admin/access`
10. Sidebar "Teams" → stub
11. `/vault/@me/nonexistent/task/x` → 404 error page
12. Second-profile permission test

- [ ] **Step 4: Final commit if any fixes were needed**

```bash
git add -A
git commit -m "fix(temper-ui): smoke test fixes"
```

---

## Summary

| Phase | Tasks | Description |
|---|---|---|
| 0 | 1 | Database migration (JSONB indexes) |
| 1 | 2-4 | Shared types in temper-core |
| 2 | 5-8 | API service layer + handlers (TDD) |
| 3 | 9-10 | Cascade fixes + ts-rs/sqlx regen |
| 4 | 11-15 | SvelteKit shell (deps, components, sidebar layout) |
| 5 | 16-18 | Vault grid pages (/vault/all, /vault/[owner]/[context]) |
| 6 | 19-20 | Resource detail page (markdown rendering) |
| 7 | 21-22 | Search (⌘K + results page) |
| 8 | 23-24 | Stubs, error page, acceptance |

**Total: 24 tasks.** Estimated 8-12 commits. Each phase produces a working, testable increment.
