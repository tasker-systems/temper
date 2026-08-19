# Knowledge Graph Phase 3: Combined Vector + Graph Search

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the search pipeline so that graph edges automatically boost search results — resources structurally connected to the top hits surface alongside semantically similar ones, with no caller changes required.

**Architecture:** A new `graph_search()` SQL function composes `unified_search()` as a CTE, graph-expands from its results via `graph_traverse()`, and blends scores (0.7 vector/FTS + 0.3 graph proximity). The Rust service routes to `graph_search()` by default, falling back to `unified_search()` when `graph_expand: false`. Same return type, same handler, same API shape — existing callers get graph-enhanced results transparently.

**Tech Stack:** Postgres 18, sqlx 0.8 (runtime queries for pgvector/enum casts), temper-core types, temper-client for e2e tests

**Source spec:** `docs/superpowers/specs/2026-04-11-knowledge-graph-phase3-combined-search.md`

**Important codebase patterns:**
- `unified_search()` SQL function is in `migrations/20260405000001_fts_search_index.sql` — it takes 10 params and returns 11 columns
- `graph_traverse()` SQL function is in `migrations/20260411000002_knowledge_graph_edges.sql` — takes `(profile_id, seed_ids UUID[], max_depth, edge_types TEXT[])`
- `search_service.rs` uses runtime `sqlx::query_as::<_, UnifiedSearchResultRow>()` (not macros) because of `::vector` casts
- `SearchParams` lives in `temper-core/src/types/api.rs` — shared by API handler, MCP tool, and client
- `SearchClient` in `temper-client/src/search.rs` constructs `SearchParams` and POSTs to `/api/search`
- E2e tests use `#[sqlx::test(migrator = "temper_api::MIGRATOR")]` with `common::setup(pool)` for in-process API + client
- All new `SearchParams` fields MUST have `#[serde(default)]` so existing callers don't break

---

## File Structure

### New Files
| File | Responsibility |
|------|---------------|
| `migrations/20260411000003_graph_search.sql` | `graph_search()` SQL function composing `unified_search()` + `graph_traverse()` |
| `tests/e2e/tests/graph_search_test.rs` | E2e test: ingest linked documents, search with graph expansion, verify connected docs surface |

### Modified Files
| File | Change |
|------|--------|
| `crates/temper-core/src/types/api.rs` | Add `seed_ids`, `edge_types`, `graph_depth`, `graph_expand` to `SearchParams`; add unit tests |
| `crates/temper-api/src/services/search_service.rs` | Add `graph_search()` Rust function, route based on `graph_expand` flag, relax validation for seed_ids-only |
| `crates/temper-client/src/search.rs` | Add `graph_search()` convenience method on `SearchClient` for e2e tests; update `search()` to pass new fields |

---

## Task 1: Extend SearchParams with Graph Fields

**Files:**
- Modify: `crates/temper-core/src/types/api.rs`

- [ ] **Step 1: Add the default function and new fields**

Add above `SearchParams`:
```rust
fn default_graph_expand() -> bool {
    true
}
```

Add these fields to `SearchParams` after `offset`:
```rust
    /// Explicit seed resource IDs for graph expansion. Omit or pass empty
    /// to auto-seed from vector/FTS results.
    #[serde(default)]
    pub seed_ids: Option<Vec<Uuid>>,
    /// Edge type filter for graph expansion (empty = all types).
    #[serde(default)]
    pub edge_types: Option<Vec<String>>,
    /// Max hops for graph traversal (default 2, max 10).
    #[serde(default)]
    pub graph_depth: Option<i32>,
    /// Whether to expand results via graph edges (default true).
    /// Set false for pure FTS/vector search without graph boosting.
    #[serde(default = "default_graph_expand")]
    pub graph_expand: bool,
```

- [ ] **Step 2: Fix existing tests and struct literals**

Every place that constructs `SearchParams` with struct literal syntax will need the new fields.
In the `tests` module at the bottom of `api.rs`, update each test's `SearchParams` to add:
```rust
    seed_ids: None,
    edge_types: None,
    graph_depth: None,
    graph_expand: true,
```

Also fix `search_service.rs` (the `search()` function and its tests) and `temper-client/src/search.rs` (the `search()` method) — anywhere `SearchParams { ... }` is constructed.

Search the workspace for `SearchParams {` to find all struct literals that need updating.

- [ ] **Step 3: Add unit tests for new field deserialization**

Append to the `tests` module in `api.rs`:
```rust
    #[test]
    fn search_params_graph_expand_defaults_true() {
        let json = r#"{"query": "hello"}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert!(params.graph_expand, "graph_expand should default to true");
        assert!(params.seed_ids.is_none());
        assert!(params.edge_types.is_none());
        assert!(params.graph_depth.is_none());
    }

    #[test]
    fn search_params_graph_expand_can_be_disabled() {
        let json = r#"{"query": "hello", "graph_expand": false}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert!(!params.graph_expand);
    }

    #[test]
    fn search_params_with_seed_ids() {
        let json = r#"{"seed_ids": ["019d1d24-2000-7379-8f26-ae4ae87bc5c6"]}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.seed_ids.unwrap().len(), 1);
        assert!(params.query.is_none());
        assert!(params.embedding.is_none());
    }

    #[test]
    fn search_params_with_edge_types_and_depth() {
        let json = r#"{"query": "test", "edge_types": ["extends", "depends_on"], "graph_depth": 4}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.edge_types.unwrap(), vec!["extends", "depends_on"]);
        assert_eq!(params.graph_depth.unwrap(), 4);
    }
```

- [ ] **Step 4: Verify compilation and tests**

Run: `cargo nextest run -p temper-core -E 'test(api::tests)' --no-fail-fast`

Expected: All tests pass including the 4 new ones.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/api.rs crates/temper-api/src/services/search_service.rs crates/temper-client/src/search.rs
git commit -m "feat(search): extend SearchParams with graph expansion fields

seed_ids, edge_types, graph_depth, graph_expand (default true).
All serde(default) so existing callers are unaffected."
```

---

## Task 2: Migration — graph_search() SQL Function

**Files:**
- Create: `migrations/20260411000003_graph_search.sql`

- [ ] **Step 1: Write the migration**

Create `migrations/20260411000003_graph_search.sql`:

```sql
-- =============================================================================
-- R7 Phase 3: Combined Vector + Graph Search
-- =============================================================================
-- Adds: graph_search() — composes unified_search() with graph_traverse()
-- to produce graph-enhanced search results.
--
-- When graph_expand is true (default), vector/FTS results are used as seeds
-- for graph traversal. Graph-connected resources are blended into results
-- with a configurable weight (default 0.3 graph, 0.7 vector/FTS).
--
-- When no edges exist, graph_traverse() returns empty and the function
-- degrades to unified_search() results at zero additional cost.

CREATE FUNCTION graph_search(
    p_profile_id      UUID,
    p_query           TEXT DEFAULT '',
    p_embedding       vector(768) DEFAULT NULL,
    p_search_config   VARCHAR DEFAULT 'english',
    p_context_name    VARCHAR DEFAULT NULL,
    p_doc_type        VARCHAR DEFAULT NULL,
    p_fts_weight      FLOAT DEFAULT 0.5,
    p_vec_weight      FLOAT DEFAULT 0.5,
    p_seed_ids        UUID[] DEFAULT '{}',
    p_edge_types      TEXT[] DEFAULT '{}',
    p_graph_depth     INT DEFAULT 2,
    p_graph_weight    FLOAT DEFAULT 0.3,
    p_limit           INT DEFAULT 10,
    p_offset          INT DEFAULT 0
) RETURNS TABLE (
    resource_id    UUID,
    title          TEXT,
    slug           VARCHAR(256),
    kb_uri         TEXT,
    origin_uri     TEXT,
    context        VARCHAR(128),
    doc_type       VARCHAR(64),
    fts_score      REAL,
    vector_score   REAL,
    combined_score REAL,
    origin         VARCHAR(16)
)
LANGUAGE SQL STABLE AS $$
    WITH
    -- Stage 1: Run unified_search to get FTS + vector results
    base_results AS (
        SELECT us.resource_id, us.title, us.slug, us.kb_uri, us.origin_uri,
               us.context, us.doc_type, us.fts_score, us.vector_score,
               us.combined_score, us.origin
          FROM unified_search(
            p_profile_id, p_query, p_embedding, p_search_config,
            p_context_name, p_doc_type, p_fts_weight, p_vec_weight,
            p_limit, p_offset
          ) us
    ),

    -- Stage 2: Collect seeds = base result IDs ∪ explicit seed_ids
    all_seeds AS (
        SELECT resource_id FROM base_results
        UNION
        SELECT unnest(p_seed_ids)
    ),

    -- Stage 3: Graph expand from seeds
    graph_hits AS (
        SELECT gt.resource_id, gt.depth, gt.path_weight
          FROM graph_traverse(
            p_profile_id,
            ARRAY(SELECT resource_id FROM all_seeds),
            p_graph_depth,
            p_edge_types
          ) gt
         WHERE gt.depth > 0  -- exclude seeds themselves
    ),

    -- Stage 4: Best graph proximity score per resource
    graph_scores AS (
        SELECT resource_id,
               MAX(path_weight / (depth + 1)::FLOAT)::REAL AS graph_proximity
          FROM graph_hits
         GROUP BY resource_id
    ),

    -- Stage 5: Combine base results with graph-expanded resources
    combined AS (
        SELECT
            COALESCE(br.resource_id, gs.resource_id) AS resource_id,
            COALESCE(br.fts_score, 0.0::REAL) AS fts_score,
            COALESCE(br.vector_score, 0.0::REAL) AS vector_score,
            COALESCE(gs.graph_proximity, 0.0::REAL) AS graph_score,
            -- Blend: (1 - graph_weight) * max(fts, vector) + graph_weight * graph_proximity
            ((1.0 - p_graph_weight) * GREATEST(COALESCE(br.fts_score, 0.0), COALESCE(br.vector_score, 0.0))
             + p_graph_weight * COALESCE(gs.graph_proximity, 0.0))::REAL AS combined_score,
            CASE
                WHEN br.resource_id IS NOT NULL AND gs.resource_id IS NOT NULL THEN 'both'
                WHEN br.resource_id IS NOT NULL THEN COALESCE(br.origin, 'fts')
                ELSE 'graph'
            END AS origin
        FROM base_results br
        FULL OUTER JOIN graph_scores gs ON gs.resource_id = br.resource_id
    )

    SELECT
        c.resource_id,
        r.title,
        r.slug,
        kb_resource_uri(r.id) AS kb_uri,
        r.origin_uri,
        ctx.name AS context,
        dt.name AS doc_type,
        c.fts_score,
        c.vector_score,
        c.combined_score,
        c.origin::VARCHAR(16)
    FROM combined c
    JOIN kb_resources r ON r.id = c.resource_id
    LEFT JOIN kb_contexts ctx ON r.kb_context_id = ctx.id
    JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
    ORDER BY c.combined_score DESC
    LIMIT p_limit
    OFFSET p_offset
$$;
```

- [ ] **Step 2: Apply the migration**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development sqlx migrate run`

Expected: `Applied 20260411000003/migrate graph search`

- [ ] **Step 3: Verify the function exists**

Run: `psql postgresql://temper:temper@localhost:5437/temper_development -c "\df graph_search"`

Expected: Shows the function with its parameter types.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260411000003_graph_search.sql
git commit -m "feat(schema): add graph_search() SQL function

Composes unified_search() with graph_traverse() — FTS/vector
results auto-seed graph expansion, blended with 0.7/0.3 scoring.
Degrades to unified_search() when no edges exist."
```

---

## Task 3: Search Service — Route to graph_search()

**Files:**
- Modify: `crates/temper-api/src/services/search_service.rs`

- [ ] **Step 1: Relax validation to accept seed_ids**

In `validate_params()`, change the validation to accept seed_ids as a standalone input:

```rust
pub fn validate_params(params: &SearchParams) -> ApiResult<i64> {
    let has_query = params.query.as_ref().is_some_and(|q| !q.trim().is_empty());
    let has_embedding = params.embedding.is_some();
    let has_seeds = params.seed_ids.as_ref().is_some_and(|s| !s.is_empty());

    if !has_query && !has_embedding && !has_seeds {
        return Err(ApiError::BadRequest(
            "at least one of 'query', 'embedding', or 'seed_ids' must be provided".into(),
        ));
    }

    if let Some(ref emb) = params.embedding {
        if emb.len() != EMBEDDING_DIM {
            return Err(ApiError::BadRequest(format!(
                "embedding must be {EMBEDDING_DIM} dimensions, got {}",
                emb.len()
            )));
        }
    }

    Ok(params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT))
}
```

- [ ] **Step 2: Add the graph_search function**

Add after the existing `search()` function:

```rust
const DEFAULT_GRAPH_DEPTH: i32 = 2;
const MAX_GRAPH_DEPTH: i32 = 10;

/// Execute graph-enhanced search: unified_search() + graph_traverse() composition.
async fn graph_search(
    pool: &PgPool,
    profile_id: Uuid,
    params: SearchParams,
) -> ApiResult<Vec<UnifiedSearchResultRow>> {
    let limit = validate_params(&params)?;
    let offset = params.offset.unwrap_or(0);
    let (fts_weight, vec_weight) = compute_weights(&params.query, &params.embedding);

    let embedding_str = params
        .embedding
        .as_ref()
        .map(|e| temper_core::types::ingest::format_embedding(e));

    let seed_ids: Vec<Uuid> = params.seed_ids.unwrap_or_default();
    let edge_types: Vec<String> = params.edge_types.unwrap_or_default();
    let graph_depth = params
        .graph_depth
        .unwrap_or(DEFAULT_GRAPH_DEPTH)
        .min(MAX_GRAPH_DEPTH);

    let rows = sqlx::query_as::<_, UnifiedSearchResultRow>(
        r#"
        SELECT resource_id, title, slug, kb_uri, origin_uri,
               context, doc_type, fts_score, vector_score,
               combined_score, origin
          FROM graph_search($1, $2, $3::vector, $4, $5, $6, $7, $8, $9, $10, $11, 0.3, $12, $13)
        "#,
    )
    .bind(profile_id)
    .bind(params.query.as_deref().unwrap_or(""))
    .bind(embedding_str.as_deref())
    .bind(&params.search_config)
    .bind(params.context_name.as_deref())
    .bind(params.doc_type.as_deref())
    .bind(fts_weight)
    .bind(vec_weight)
    .bind(&seed_ids)
    .bind(&edge_types)
    .bind(graph_depth)
    .bind(limit as i32)
    .bind(offset as i32)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
```

- [ ] **Step 3: Update the public search() to route**

Replace the existing `search()` body:

```rust
pub async fn search(
    pool: &PgPool,
    profile_id: Uuid,
    params: SearchParams,
) -> ApiResult<Vec<UnifiedSearchResultRow>> {
    if params.graph_expand {
        graph_search(pool, profile_id, params).await
    } else {
        unified_search(pool, profile_id, params).await
    }
}
```

Rename the current `search()` body to `unified_search()` (private):

```rust
/// Execute unified search (FTS + optional vector) without graph expansion.
async fn unified_search(
    pool: &PgPool,
    profile_id: Uuid,
    params: SearchParams,
) -> ApiResult<Vec<UnifiedSearchResultRow>> {
    let limit = validate_params(&params)?;
    let offset = params.offset.unwrap_or(0);
    let (fts_weight, vec_weight) = compute_weights(&params.query, &params.embedding);

    let embedding_str = params
        .embedding
        .as_ref()
        .map(|e| temper_core::types::ingest::format_embedding(e));

    let rows = sqlx::query_as::<_, UnifiedSearchResultRow>(
        r#"
        SELECT resource_id, title, slug, kb_uri, origin_uri,
               context, doc_type, fts_score, vector_score,
               combined_score, origin
          FROM unified_search($1, $2, $3::vector, $4, $5, $6, $7, $8, $9, $10)
        "#,
    )
    .bind(profile_id)
    .bind(params.query.as_deref().unwrap_or(""))
    .bind(embedding_str.as_deref())
    .bind(&params.search_config)
    .bind(params.context_name.as_deref())
    .bind(params.doc_type.as_deref())
    .bind(fts_weight)
    .bind(vec_weight)
    .bind(limit as i32)
    .bind(offset as i32)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
```

- [ ] **Step 4: Update validation unit tests**

Update the existing `validate_rejects_neither_query_nor_embedding` test to also set the new fields:
```rust
    #[test]
    fn validate_rejects_no_inputs() {
        let params = SearchParams {
            query: None,
            embedding: None,
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
            seed_ids: None,
            edge_types: None,
            graph_depth: None,
            graph_expand: true,
        };
        assert!(validate_params(&params).is_err());
    }
```

Add a new test:
```rust
    #[test]
    fn validate_accepts_seed_ids_only() {
        let params = SearchParams {
            query: None,
            embedding: None,
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
            seed_ids: Some(vec![uuid::Uuid::nil()]),
            edge_types: None,
            graph_depth: None,
            graph_expand: true,
        };
        assert!(validate_params(&params).is_ok());
    }
```

Update all other test struct literals in `search_service.rs` to include the new fields.

- [ ] **Step 5: Verify compilation and tests**

Run: `cargo check -p temper-api && cargo nextest run -p temper-api -E 'test(search_service)' --no-fail-fast`

Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/services/search_service.rs
git commit -m "feat(search): route to graph_search() when graph_expand is true

graph_search() composes unified_search + graph_traverse in SQL.
Validation now accepts seed_ids as standalone input for pure
graph traversal."
```

---

## Task 4: Update SearchClient for Graph Params

**Files:**
- Modify: `crates/temper-client/src/search.rs`

- [ ] **Step 1: Add a search_with_params method**

The existing `search()` method constructs `SearchParams` from individual args. Rather than
adding 4 more params to that signature, add a method that accepts a pre-built `SearchParams`:

```rust
    /// Run a search with full control over all parameters including graph expansion.
    pub async fn search_with_params(
        &self,
        params: &SearchParams,
    ) -> Result<Vec<UnifiedSearchResultRow>> {
        let token = self.http.resolve_token()?;
        let req = self.http.post("/api/search").json(params);
        self.http
            .send_json(&Method::POST, "/api/search", req, Some(&token))
            .await
    }
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check -p temper-client`

Expected: Compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-client/src/search.rs
git commit -m "feat(client): add search_with_params() for graph search fields

Accepts a full SearchParams struct, enabling e2e tests and callers
to pass seed_ids, edge_types, graph_depth, graph_expand."
```

---

## Task 5: Integration Test — graph_search() SQL Function

**Files:**
- Create: `crates/temper-api/tests/graph_search_test.rs`

- [ ] **Step 1: Write integration test for graph_search**

Create `crates/temper-api/tests/graph_search_test.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

use sqlx::PgPool;
use temper_core::types::api::UnifiedSearchResultRow;

/// graph_search returns graph-connected resources even when they have no
/// vector/FTS match, by expanding from seed results.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_graph_search_expands_from_seeds(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "gsearch@test.com").await;

    // Create three resources: A extends B, B depends_on C
    let a = common::fixtures::create_test_resource(&pool, profile, "Doc Alpha", "doc-alpha").await;
    let b = common::fixtures::create_test_resource(&pool, profile, "Doc Beta", "doc-beta").await;
    let c = common::fixtures::create_test_resource(&pool, profile, "Doc Gamma", "doc-gamma").await;

    // Create edges: A→B (extends), B→C (depends_on)
    common::fixtures::create_test_edge(&pool, a, b, "extends", profile).await;
    common::fixtures::create_test_edge(&pool, b, c, "depends_on", profile).await;

    // Insert a dummy chunk for Doc Alpha so vector search can find it
    let chunk_json = serde_json::json!([{
        "chunk_index": 0,
        "header_path": "Doc Alpha",
        "heading_depth": 1,
        "content": "Alpha content about deployment configuration",
        "content_hash": "alpha-hash",
        "embedding": format!("[{}]", vec!["0.1"; 768].join(","))
    }]);
    sqlx::query("SELECT persist_resource_chunks($1, $2)")
        .bind(a)
        .bind(&chunk_json)
        .execute(&pool)
        .await
        .expect("persist chunks for alpha");

    // Call graph_search with explicit seed = A
    let results: Vec<UnifiedSearchResultRow> = sqlx::query_as(
        r#"
        SELECT resource_id, title, slug, kb_uri, origin_uri,
               context, doc_type, fts_score, vector_score,
               combined_score, origin
          FROM graph_search($1, '', NULL, 'english', NULL, NULL, 0.5, 0.5,
                           $2, '{}', 2, 0.3, 10, 0)
        "#,
    )
    .bind(profile)
    .bind(&vec![a])
    .fetch_all(&pool)
    .await
    .expect("graph_search query");

    let result_ids: Vec<uuid::Uuid> = results.iter().map(|r| r.resource_id).collect();

    // A is the seed (depth 0, excluded from graph_hits but present as base result if it had a match)
    // B should appear via graph (1 hop from A via extends)
    // C should appear via graph (2 hops from A via extends→depends_on)
    assert!(
        result_ids.contains(&b),
        "Doc Beta should appear via graph expansion (1 hop from seed A)"
    );
    assert!(
        result_ids.contains(&c),
        "Doc Gamma should appear via graph expansion (2 hops from seed A)"
    );

    // Verify graph-only results have origin = 'graph'
    let beta_result = results.iter().find(|r| r.resource_id == b).unwrap();
    assert_eq!(beta_result.origin, "graph", "Beta should have origin=graph");
}

/// graph_search with no edges degrades to unified_search results.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_graph_search_no_edges_degrades(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "degrade@test.com").await;
    let a = common::fixtures::create_test_resource(&pool, profile, "Isolated Doc", "isolated-doc").await;

    // Insert a chunk so it's findable
    let chunk_json = serde_json::json!([{
        "chunk_index": 0,
        "header_path": "Isolated",
        "heading_depth": 1,
        "content": "Isolated content",
        "content_hash": "iso-hash",
        "embedding": format!("[{}]", vec!["0.1"; 768].join(","))
    }]);
    sqlx::query("SELECT persist_resource_chunks($1, $2)")
        .bind(a)
        .bind(&chunk_json)
        .execute(&pool)
        .await
        .expect("persist chunks");

    // Search with a matching embedding — should work just like unified_search
    let embedding_str = format!("[{}]", vec!["0.1"; 768].join(","));
    let results: Vec<UnifiedSearchResultRow> = sqlx::query_as(
        r#"
        SELECT resource_id, title, slug, kb_uri, origin_uri,
               context, doc_type, fts_score, vector_score,
               combined_score, origin
          FROM graph_search($1, '', $2::vector, 'english', NULL, NULL, 0.0, 1.0,
                           '{}', '{}', 2, 0.3, 10, 0)
        "#,
    )
    .bind(profile)
    .bind(&embedding_str)
    .fetch_all(&pool)
    .await
    .expect("graph_search with no edges");

    assert!(
        !results.is_empty(),
        "should find the isolated doc via vector search"
    );
    assert!(
        results.iter().any(|r| r.resource_id == a),
        "isolated doc should appear in results"
    );
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo nextest run -p temper-api --features test-db -E 'test(graph_search_test)' --run-ignored all`

Expected: Both tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/tests/graph_search_test.rs
git commit -m "test(graph): add graph_search SQL function integration tests

Tests: seed-based expansion finds 2-hop connected resources,
no-edge degradation to unified_search behavior."
```

---

## Task 6: E2E Test — Full Pipeline Graph Search

**Files:**
- Create: `tests/e2e/tests/graph_search_test.rs`

- [ ] **Step 1: Write the e2e test**

Create `tests/e2e/tests/graph_search_test.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

use serde_json::json;
use temper_core::types::api::SearchParams;
use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};

/// Helper: build an IngestPayload with a dummy embedding and open_meta.
fn test_payload(
    title: &str,
    slug: &str,
    context: &str,
    open_meta: Option<serde_json::Value>,
) -> IngestPayload {
    let dummy_embedding = vec![0.1_f32; 768];
    let chunks = vec![PackedChunk {
        chunk_index: 0,
        header_path: title.to_string(),
        heading_depth: 1,
        content: format!("{title} content for testing"),
        content_hash: format!("{slug}-hash"),
        embedding: dummy_embedding,
    }];

    IngestPayload {
        title: title.to_string(),
        origin_uri: format!("test://e2e/{slug}"),
        context_name: context.to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(format!("{slug}-body-hash-000000000000000000000000000000000000000000")),
        slug: slug.to_string(),
        content: format!("# {title}\n\n{title} content for testing."),
        metadata: None,
        managed_meta: Some(json!({"date": "2026-04-11"})),
        open_meta,
        chunks_packed: Some(pack_chunks(&chunks).expect("pack")),
    }
}

/// Ingest linked documents, search, verify graph expansion surfaces connected docs.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn graph_search_e2e_expands_connected_documents(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    // Ensure profile + context exist
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("graph-e2e")
        .await
        .expect("create context");

    // Ingest Doc C first (no relationships — it's the leaf)
    let payload_c = test_payload("Data Model", "data-model", "graph-e2e", None);
    let resource_c = app.client.ingest().create(&payload_c).await.expect("ingest C");

    // Ingest Doc B (depends_on C)
    let payload_b = test_payload(
        "Architecture Design",
        "architecture-design",
        "graph-e2e",
        Some(json!({"depends_on": ["data-model"]})),
    );
    let resource_b = app.client.ingest().create(&payload_b).await.expect("ingest B");

    // Ingest Doc A (extends B)
    let payload_a = test_payload(
        "Deployment Config",
        "deployment-config",
        "graph-e2e",
        Some(json!({"extends": ["architecture-design"]})),
    );
    let resource_a = app.client.ingest().create(&payload_a).await.expect("ingest A");

    // Search with graph_expand: true (default) using Doc A as explicit seed
    let params_with_graph = SearchParams {
        query: None,
        embedding: None,
        search_config: "english".into(),
        context_name: Some("graph-e2e".into()),
        doc_type: None,
        limit: Some(10),
        offset: None,
        seed_ids: Some(vec![resource_a.id.into()]),
        edge_types: None,
        graph_depth: Some(3),
        graph_expand: true,
    };

    let results = app
        .client
        .search()
        .search_with_params(&params_with_graph)
        .await
        .expect("graph search");

    let result_ids: Vec<uuid::Uuid> = results.iter().map(|r| r.resource_id).collect();

    assert!(
        result_ids.contains(&resource_b.id.into()),
        "Architecture Design should appear via graph (1 hop extends from A). Got: {result_ids:?}"
    );
    assert!(
        result_ids.contains(&resource_c.id.into()),
        "Data Model should appear via graph (2 hops extends→depends_on from A). Got: {result_ids:?}"
    );

    // Search with graph_expand: false — only seed itself (if it has vector match)
    let params_no_graph = SearchParams {
        graph_expand: false,
        seed_ids: Some(vec![resource_a.id.into()]),
        ..params_with_graph.clone()
    };

    let results_no_graph = app
        .client
        .search()
        .search_with_params(&params_no_graph)
        .await
        .expect("non-graph search");

    let no_graph_ids: Vec<uuid::Uuid> = results_no_graph.iter().map(|r| r.resource_id).collect();

    // Without graph expansion and with no query/embedding, seed_ids alone shouldn't
    // produce results via unified_search (no FTS/vector match)
    assert!(
        !no_graph_ids.contains(&resource_b.id.into()),
        "Architecture Design should NOT appear without graph expansion"
    );
}
```

- [ ] **Step 2: Make SearchParams Clone**

The e2e test uses `..params_with_graph.clone()`. Check if `SearchParams` already derives `Clone` — it should (the struct has `#[derive(Debug, Clone, Deserialize, Serialize)]`). If not, add `Clone` to the derive list.

- [ ] **Step 3: Run the e2e test**

Run: `cargo nextest run -p temper-e2e --features test-db -E 'test(graph_search_e2e)' --run-ignored all`

Expected: Test passes.

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/tests/graph_search_test.rs
git commit -m "test(e2e): add graph search end-to-end test

Ingests 3 linked documents (A extends B depends_on C), verifies
graph expansion surfaces B and C from seed A, and opt-out with
graph_expand: false excludes graph-connected results."
```

---

## Task 7: Regenerate sqlx Cache and Run Full Test Suite

**Files:**
- Modify: `.sqlx/` (regenerated)

- [ ] **Step 1: Regenerate the sqlx offline cache**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo sqlx prepare --workspace -- --all-features`

Expected: `query data written to .sqlx`

- [ ] **Step 2: Run the full check suite**

Run: `cargo make check`

Expected: All checks pass.

- [ ] **Step 3: Run all integration tests**

Run: `cargo nextest run -p temper-api --features test-db --run-ignored all --no-fail-fast`

Expected: All tests pass (85 existing + new graph_search tests).

- [ ] **Step 4: Run all e2e tests**

Run: `cargo nextest run -p temper-e2e --features test-db --run-ignored all --no-fail-fast`

Expected: All e2e tests pass.

- [ ] **Step 5: Commit the sqlx cache**

```bash
git add .sqlx/
git commit -m "chore: regenerate sqlx cache for graph_search queries"
```

---

## Implementation Notes for Agents

### SQL function composition

`graph_search()` calls `unified_search()` and `graph_traverse()` as CTEs — this is standard Postgres. Both are `LANGUAGE SQL STABLE` functions that compose cleanly. The key is that `unified_search()` returns a table, so `SELECT ... FROM unified_search(...)` works as a CTE source.

### The `::vector` cast issue

Both `search_service.rs` and the SQL function use pgvector's `::vector` cast. The sqlx compile-time macro doesn't support this, so all search queries use runtime `sqlx::query_as::<_, UnifiedSearchResultRow>()`.

### Binding UUID arrays

The `seed_ids` parameter is `UUID[]` in SQL. From Rust, bind as `&Vec<Uuid>` — sqlx handles the array serialization. Empty vec maps to `'{}'` in SQL.

### The `graph_weight` constant

The SQL function takes `p_graph_weight` (default 0.3) but the Rust caller always passes `0.3` as a literal. This is intentional — it's a server-side constant, not a user param. If we later want to expose it, add a field to `SearchParams` and pass it through.

### ResourceId vs Uuid in e2e tests

`ResourceRow.id` is `ResourceId` (newtype). When comparing with search results (`UnifiedSearchResultRow.resource_id` which is `Uuid`), use `.into()` to convert: `resource_a.id.into()`.
