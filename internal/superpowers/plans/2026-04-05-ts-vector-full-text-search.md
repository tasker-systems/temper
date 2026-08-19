# tsvector Full-Text Search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable MCP clients and any non-embedding client to search the knowledge base via plain text, by wiring up the existing tsvector SQL infrastructure to the Rust service layer and replacing the per-chunk insert loop with batch SQL functions.

**Architecture:** The SQL migrations (20260405000001/2) are already written — they define `kb_resource_search_index`, triggers, `fts_search()`, `unified_search()`, `persist_resource_chunks()`, and `replace_resource_chunks()`. This plan covers only the Rust-side changes: evolving `SearchParams` and `SearchResultRow`, rewriting `search_service.rs` to call `unified_search()`, adding `ChunkRowJsonb` for batch chunk persistence, replacing the `insert_chunks()` loop in `ingest_service.rs`, updating the search handler, MCP tool description, and testing the migrations against Docker Postgres.

**Tech Stack:** Rust (sqlx, serde, schemars, utoipa), PostgreSQL 18 (pgvector, tsvector), Docker

---

## File Structure

| Action | File | Responsibility |
|--------|------|---------------|
| Modify | `crates/temper-core/src/types/api.rs` | Evolve `SearchParams` (add `query`, `search_config`, `offset`; make `embedding` optional). Add `UnifiedSearchResultRow` for the unified_search return type. Keep old `SearchResultRow` for backward compat. |
| Modify | `crates/temper-core/src/types/ingest.rs` | Add `format_embedding()`, `ChunkRowJsonb`, `chunks_to_jsonb()` |
| Modify | `crates/temper-api/src/services/search_service.rs` | Rewrite `search()` to call `unified_search()` SQL function. Update `validate_params()`. Remove `format_embedding()` (moved to core). Remove `build_filter_clause()` (no longer needed — SQL function handles filters). |
| Modify | `crates/temper-api/src/services/ingest_service.rs` | Replace `insert_chunks()` with `persist_chunks()` and `replace_chunks()` that call the batch SQL functions. Remove `format_embedding` import. |
| Modify | `crates/temper-api/src/handlers/search.rs` | Update return type to `Vec<UnifiedSearchResultRow>`. |
| Modify | `crates/temper-mcp/src/service.rs` | Update `#[tool(description)]` for search to mention text search. |
| Modify | `crates/temper-mcp/src/tools/search.rs` | Update comment. Return type adapts automatically via `search_service::search()`. |

---

### Task 1: Validate Migrations Against Local Docker Postgres

**Files:**
- Read: `migrations/20260405000001_fts_search_index.sql`
- Read: `migrations/20260405000002_fts_backfill.sql`

- [ ] **Step 1: Ensure Docker Postgres is running**

```bash
cargo make docker-up
```

Expected: Container `temper-postgres` running on port 5437.

- [ ] **Step 2: Run all migrations**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo sqlx migrate run --source migrations
```

Expected: All migrations apply successfully, including `20260405000001_fts_search_index` and `20260405000002_fts_backfill`.

- [ ] **Step 3: Verify the search index table exists and has data**

```bash
psql postgresql://temper:temper@localhost:5437/temper_development -c "SELECT COUNT(*) FROM kb_resource_search_index;"
```

Expected: Row count > 0 (from the backfill migration).

- [ ] **Step 4: Verify the SQL functions exist**

```bash
psql postgresql://temper:temper@localhost:5437/temper_development -c "\df fts_search"
psql postgresql://temper:temper@localhost:5437/temper_development -c "\df unified_search"
psql postgresql://temper:temper@localhost:5437/temper_development -c "\df persist_resource_chunks"
psql postgresql://temper:temper@localhost:5437/temper_development -c "\df replace_resource_chunks"
psql postgresql://temper:temper@localhost:5437/temper_development -c "\df rebuild_resource_search_vector"
```

Expected: Each function listed with correct parameter signatures.

- [ ] **Step 5: Smoke-test fts_search with a real query**

```bash
psql postgresql://temper:temper@localhost:5437/temper_development -c "
SELECT resource_id, title, fts_rank
FROM fts_search(
    (SELECT id FROM kb_profiles LIMIT 1),
    'deployment',
    'english', NULL, NULL, 5, 0
);"
```

Expected: Results returned (or empty set if no matching content — not an error).

- [ ] **Step 6: Commit — no code changes, just validation**

No commit needed. This task is verification only.

---

### Task 2: Evolve `SearchParams` and Add `UnifiedSearchResultRow` in temper-core

**Files:**
- Modify: `crates/temper-core/src/types/api.rs:49-84`

- [ ] **Step 1: Write the failing test for new SearchParams fields**

Add to the bottom of `crates/temper-core/src/types/api.rs` (or create a test module if none exists). Since this file doesn't have tests currently, add a `#[cfg(test)] mod tests` block:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_params_deserializes_query_only() {
        let json = r#"{"query": "deployment config"}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.query.as_deref(), Some("deployment config"));
        assert!(params.embedding.is_none());
        assert_eq!(params.search_config, "english");
    }

    #[test]
    fn search_params_deserializes_embedding_only() {
        let json = r#"{"embedding": [0.1, 0.2, 0.3]}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert!(params.query.is_none());
        assert_eq!(params.embedding.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn search_params_deserializes_both() {
        let json = r#"{"query": "test", "embedding": [0.1], "search_config": "simple"}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.query.as_deref(), Some("test"));
        assert!(params.embedding.is_some());
        assert_eq!(params.search_config, "simple");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo nextest run -p temper-core -E 'test(search_params_deserializes)'
```

Expected: FAIL — `SearchParams` doesn't have `query` or `search_config` fields yet.

- [ ] **Step 3: Update SearchParams**

Replace the `SearchParams` struct in `crates/temper-core/src/types/api.rs:49-62` with:

```rust
/// Request body for POST /api/search.
///
/// At least one of `query` (for full-text search) or `embedding` (for vector
/// search) must be provided. When both are present, results are merged with
/// configurable weights.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct SearchParams {
    /// Free-text search query (for tsvector full-text search).
    #[serde(default)]
    pub query: Option<String>,

    /// Pre-computed 768-dim embedding vector (optional).
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,

    /// Text search configuration / language (default: "english").
    #[serde(default = "default_search_config")]
    pub search_config: String,

    /// Filter by context name (resolved to UUID server-side).
    pub context_name: Option<String>,

    /// Filter by document type.
    pub doc_type: Option<String>,

    /// Maximum results (default 10, max 50).
    pub limit: Option<i64>,

    /// Offset for pagination.
    #[serde(default)]
    pub offset: Option<i64>,
}

fn default_search_config() -> String {
    "english".to_string()
}
```

- [ ] **Step 4: Add UnifiedSearchResultRow**

Add after the existing `SearchResultRow` struct (keep `SearchResultRow` — it's used for backward compat and TypeScript export):

```rust
/// A single result from the unified search function (FTS + vector).
///
/// Different from `SearchResultRow` because unified_search returns
/// `fts_score`, `vector_score`, `combined_score`, and `origin` instead
/// of a single `score` + `snippet`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "search.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct UnifiedSearchResultRow {
    pub resource_id: Uuid,
    pub title: String,
    pub slug: String,
    /// Canonical kb:// URI
    pub kb_uri: String,
    /// Original source URL or file reference
    pub origin_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub doc_type: String,
    pub fts_score: f32,
    pub vector_score: f32,
    pub combined_score: f32,
    /// Which search mode(s) found this result: "fts", "vector", or "both"
    pub origin: String,
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo nextest run -p temper-core -E 'test(search_params_deserializes)'
```

Expected: All 3 tests PASS.

- [ ] **Step 6: Fix compilation across workspace**

The `SearchParams` change (`embedding: Vec<f32>` → `embedding: Option<Vec<f32>>`) breaks callers. Before fixing the callers in later tasks, verify the core crate builds:

```bash
cargo check -p temper-core
```

Expected: PASS (temper-core itself compiles — the callers in temper-api and temper-mcp will fail, which is expected and fixed in Tasks 4 and 6).

- [ ] **Step 7: Commit**

```bash
git add crates/temper-core/src/types/api.rs
git commit -m "feat: evolve SearchParams for FTS, add UnifiedSearchResultRow

Make embedding optional, add query/search_config/offset fields.
Add UnifiedSearchResultRow for unified_search SQL function."
```

---

### Task 3: Add `ChunkRowJsonb` and Move `format_embedding` to temper-core

**Files:**
- Modify: `crates/temper-core/src/types/ingest.rs`

- [ ] **Step 1: Write the failing tests**

Add to the existing `#[cfg(test)] mod tests` block in `crates/temper-core/src/types/ingest.rs`:

```rust
    #[test]
    fn format_embedding_basic() {
        let result = format_embedding(&[0.1, 0.2, 0.3]);
        assert_eq!(result, "[0.1,0.2,0.3]");
    }

    #[test]
    fn format_embedding_empty() {
        assert_eq!(format_embedding(&[]), "[]");
    }

    #[test]
    fn chunk_row_jsonb_from_packed() {
        let packed = PackedChunk {
            chunk_index: 0,
            header_path: "Title > Section".to_owned(),
            content: "Hello world".to_owned(),
            content_hash: "abc123".to_owned(),
            embedding: vec![0.1, 0.2, 0.3],
        };

        let row = ChunkRowJsonb::from_packed(&packed);
        assert_eq!(row.chunk_index, 0);
        assert_eq!(row.embedding, "[0.1,0.2,0.3]");
        assert_eq!(row.content, "Hello world");
    }

    #[test]
    fn chunks_to_jsonb_produces_array() {
        let chunks = vec![
            PackedChunk {
                chunk_index: 0,
                header_path: "A".into(),
                content: "first".into(),
                content_hash: "h1".into(),
                embedding: vec![0.1, 0.2],
            },
            PackedChunk {
                chunk_index: 1,
                header_path: "B".into(),
                content: "second".into(),
                content_hash: "h2".into(),
                embedding: vec![0.3, 0.4],
            },
        ];

        let json = chunks_to_jsonb(&chunks);
        assert!(json.is_array());
        assert_eq!(json.as_array().unwrap().len(), 2);
        assert_eq!(json[0]["chunk_index"], 0);
        assert_eq!(json[0]["embedding"], "[0.1,0.2]"); // string, not array
        assert!(json[0]["embedding"].is_string());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo nextest run -p temper-core -E 'test(format_embedding|chunk_row_jsonb|chunks_to_jsonb)'
```

Expected: FAIL — functions/types don't exist yet.

- [ ] **Step 3: Add format_embedding, ChunkRowJsonb, chunks_to_jsonb**

Add to `crates/temper-core/src/types/ingest.rs` after the `PackedChunk` struct (before `pack_chunks`):

```rust
/// Format an embedding vector as a pgvector literal string: `[0.1,0.2,...]`
pub fn format_embedding(embedding: &[f32]) -> String {
    format!(
        "[{}]",
        embedding
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// JSONB-serializable chunk row for the `persist_resource_chunks()` and
/// `replace_resource_chunks()` SQL functions.
///
/// `embedding` is a pre-formatted pgvector literal string (`"[0.1,0.2,...]"`)
/// rather than a `Vec<f32>`. The SQL function casts it to `vector` via `::vector`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRowJsonb {
    pub chunk_index: u32,
    pub header_path: String,
    pub content: String,
    pub content_hash: String,
    /// Pre-formatted pgvector literal: `[0.1,0.2,...]`
    pub embedding: String,
}

impl ChunkRowJsonb {
    pub fn from_packed(chunk: &PackedChunk) -> Self {
        Self {
            chunk_index: chunk.chunk_index,
            header_path: chunk.header_path.clone(),
            content: chunk.content.clone(),
            content_hash: chunk.content_hash.clone(),
            embedding: format_embedding(&chunk.embedding),
        }
    }
}

/// Convert a slice of `PackedChunk` into a JSONB-ready `serde_json::Value`
/// array suitable for the batch chunk SQL functions.
pub fn chunks_to_jsonb(chunks: &[PackedChunk]) -> serde_json::Value {
    let rows: Vec<ChunkRowJsonb> = chunks.iter().map(ChunkRowJsonb::from_packed).collect();
    serde_json::to_value(&rows).expect("ChunkRowJsonb is always serializable")
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo nextest run -p temper-core -E 'test(format_embedding|chunk_row_jsonb|chunks_to_jsonb)'
```

Expected: All 4 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/ingest.rs
git commit -m "feat: add ChunkRowJsonb and move format_embedding to temper-core

ChunkRowJsonb serializes chunks as JSONB for the batch SQL functions
persist_resource_chunks() and replace_resource_chunks()."
```

---

### Task 4: Rewrite search_service.rs to Use unified_search()

**Files:**
- Modify: `crates/temper-api/src/services/search_service.rs`

- [ ] **Step 1: Write the failing tests for new validation logic**

Replace the existing test module in `crates/temper-api/src/services/search_service.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_neither_query_nor_embedding() {
        let params = SearchParams {
            query: None,
            embedding: None,
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
        };
        assert!(validate_params(&params).is_err());
    }

    #[test]
    fn validate_accepts_query_only() {
        let params = SearchParams {
            query: Some("test".into()),
            embedding: None,
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
        };
        let result = validate_params(&params).unwrap();
        assert_eq!(result, DEFAULT_LIMIT);
    }

    #[test]
    fn validate_accepts_embedding_only() {
        let params = SearchParams {
            query: None,
            embedding: Some(vec![0.0; 768]),
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
        };
        assert!(validate_params(&params).is_ok());
    }

    #[test]
    fn validate_rejects_wrong_dimension() {
        let params = SearchParams {
            query: None,
            embedding: Some(vec![0.0; 100]),
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
        };
        assert!(validate_params(&params).is_err());
    }

    #[test]
    fn validate_clamps_limit() {
        let params = SearchParams {
            query: Some("test".into()),
            embedding: None,
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: Some(200),
            offset: None,
        };
        assert_eq!(validate_params(&params).unwrap(), MAX_LIMIT);
    }

    #[test]
    fn validate_rejects_empty_query_with_no_embedding() {
        let params = SearchParams {
            query: Some("".into()),
            embedding: None,
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
        };
        assert!(validate_params(&params).is_err());
    }

    #[test]
    fn compute_weights_query_only() {
        let (fts, vec) = compute_weights(&Some("test".into()), &None);
        assert_eq!(fts, 1.0);
        assert_eq!(vec, 0.0);
    }

    #[test]
    fn compute_weights_embedding_only() {
        let (fts, vec) = compute_weights(&None, &Some(vec![0.0; 768]));
        assert_eq!(fts, 0.0);
        assert_eq!(vec, 1.0);
    }

    #[test]
    fn compute_weights_both() {
        let (fts, vec) = compute_weights(&Some("q".into()), &Some(vec![0.0; 768]));
        assert_eq!(fts, 0.5);
        assert_eq!(vec, 0.5);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo nextest run -p temper-api -E 'test(validate_|compute_weights)'
```

Expected: FAIL — new functions/signatures don't exist yet.

- [ ] **Step 3: Rewrite search_service.rs**

Replace the entire contents of `crates/temper-api/src/services/search_service.rs` with:

```rust
//! Search service — routes queries to the `unified_search()` SQL function,
//! combining full-text (tsvector) and vector (pgvector) search.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

pub use temper_core::types::api::{SearchParams, UnifiedSearchResultRow};

const MAX_LIMIT: i64 = 50;
const DEFAULT_LIMIT: i64 = 10;
const EMBEDDING_DIM: usize = 768;

/// Validate search params. Returns the sanitized limit.
///
/// Rules:
/// - At least one of `query` or `embedding` must be provided
/// - If `query` is provided, it must be non-empty
/// - If `embedding` is provided, it must be 768 dimensions
/// - Limit is clamped to MAX_LIMIT
pub fn validate_params(params: &SearchParams) -> ApiResult<i64> {
    let has_query = params
        .query
        .as_ref()
        .is_some_and(|q| !q.trim().is_empty());
    let has_embedding = params.embedding.is_some();

    if !has_query && !has_embedding {
        return Err(ApiError::BadRequest(
            "at least one of 'query' or 'embedding' must be provided".into(),
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

/// Compute FTS/vector weights based on which inputs are provided.
pub fn compute_weights(
    query: &Option<String>,
    embedding: &Option<Vec<f32>>,
) -> (f64, f64) {
    let has_query = query.as_ref().is_some_and(|q| !q.trim().is_empty());
    match (has_query, embedding.is_some()) {
        (true, true) => (0.5, 0.5),
        (true, false) => (1.0, 0.0),
        (false, true) => (0.0, 1.0),
        (false, false) => (0.0, 0.0), // unreachable after validation
    }
}

/// Execute the unified search (FTS + optional vector).
pub async fn search(
    pool: &PgPool,
    profile_id: Uuid,
    params: SearchParams,
) -> ApiResult<Vec<UnifiedSearchResultRow>> {
    let limit = validate_params(&params)?;
    let offset = params.offset.unwrap_or(0);

    let (fts_weight, vec_weight) = compute_weights(&params.query, &params.embedding);

    // Format embedding as pgvector literal if provided
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_neither_query_nor_embedding() {
        let params = SearchParams {
            query: None,
            embedding: None,
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
        };
        assert!(validate_params(&params).is_err());
    }

    #[test]
    fn validate_accepts_query_only() {
        let params = SearchParams {
            query: Some("test".into()),
            embedding: None,
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
        };
        let result = validate_params(&params).unwrap();
        assert_eq!(result, DEFAULT_LIMIT);
    }

    #[test]
    fn validate_accepts_embedding_only() {
        let params = SearchParams {
            query: None,
            embedding: Some(vec![0.0; 768]),
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
        };
        assert!(validate_params(&params).is_ok());
    }

    #[test]
    fn validate_rejects_wrong_dimension() {
        let params = SearchParams {
            query: None,
            embedding: Some(vec![0.0; 100]),
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
        };
        assert!(validate_params(&params).is_err());
    }

    #[test]
    fn validate_clamps_limit() {
        let params = SearchParams {
            query: Some("test".into()),
            embedding: None,
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: Some(200),
            offset: None,
        };
        assert_eq!(validate_params(&params).unwrap(), MAX_LIMIT);
    }

    #[test]
    fn validate_rejects_empty_query_with_no_embedding() {
        let params = SearchParams {
            query: Some("".into()),
            embedding: None,
            search_config: "english".into(),
            context_name: None,
            doc_type: None,
            limit: None,
            offset: None,
        };
        assert!(validate_params(&params).is_err());
    }

    #[test]
    fn compute_weights_query_only() {
        let (fts, vec) = compute_weights(&Some("test".into()), &None);
        assert_eq!(fts, 1.0);
        assert_eq!(vec, 0.0);
    }

    #[test]
    fn compute_weights_embedding_only() {
        let (fts, vec) = compute_weights(&None, &Some(vec![0.0; 768]));
        assert_eq!(fts, 0.0);
        assert_eq!(vec, 1.0);
    }

    #[test]
    fn compute_weights_both() {
        let (fts, vec) = compute_weights(&Some("q".into()), &Some(vec![0.0; 768]));
        assert_eq!(fts, 0.5);
        assert_eq!(vec, 0.5);
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo nextest run -p temper-api -E 'test(validate_|compute_weights)'
```

Expected: All 9 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/services/search_service.rs
git commit -m "feat: rewrite search service to call unified_search() SQL function

Supports text-only, embedding-only, or combined search.
Removes build_filter_clause() — SQL function handles filters."
```

---

### Task 5: Update search handler and re-exports

**Files:**
- Modify: `crates/temper-api/src/handlers/search.rs`

- [ ] **Step 1: Update the search handler return type**

Replace the contents of `crates/temper-api/src/handlers/search.rs` with:

```rust
use axum::extract::State;
use axum::Json;

use crate::error::{ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::search_service::{self, SearchParams, UnifiedSearchResultRow};
use crate::state::AppState;

#[utoipa::path(
    post,
    path = "/api/search",
    tag = "Search",
    request_body = SearchParams,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Search results", body = Vec<UnifiedSearchResultRow>),
        (status = 400, description = "Invalid request", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn search(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(params): Json<SearchParams>,
) -> ApiResult<Json<Vec<UnifiedSearchResultRow>>> {
    search_service::search(&state.pool, auth.0.profile.id, params)
        .await
        .map(Json)
}
```

- [ ] **Step 2: Check if SearchResultRow is re-exported or used elsewhere**

Search for other uses of `SearchResultRow` across the codebase. If it's used in temper-mcp, temper-client, or TypeScript bindings, keep it in `api.rs` but it's no longer used by the search handler. The MCP tool will now return `UnifiedSearchResultRow` via `search_service::search()`.

```bash
cargo check -p temper-api
```

Expected: May have warnings about unused `SearchResultRow` import — that's fine. Errors about `format_embedding` import in `ingest_service.rs` are expected (fixed in Task 6).

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/src/handlers/search.rs
git commit -m "feat: update search handler to return UnifiedSearchResultRow"
```

---

### Task 6: Replace insert_chunks Loop with Batch SQL Functions in ingest_service.rs

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs`

- [ ] **Step 1: Replace format_embedding import and add batch helpers**

In `crates/temper-api/src/services/ingest_service.rs`, change the imports at the top:

Replace:
```rust
use crate::services::search_service::format_embedding;
```

With:
```rust
use temper_core::types::ingest::chunks_to_jsonb;
```

- [ ] **Step 2: Replace insert_chunks with persist_chunks and replace_chunks**

Replace the `insert_chunks` function (lines 113-147) with:

```rust
/// Batch-insert chunks for a new resource via SQL function.
/// Gates search triggers, does bulk INSERT, rebuilds search index once.
async fn persist_chunks(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    resource_id: Uuid,
    chunks: &[PackedChunk],
) -> ApiResult<i32> {
    let chunks_json = chunks_to_jsonb(chunks);

    let (count,): (i32,) = sqlx::query_as("SELECT persist_resource_chunks($1, $2)")
        .bind(resource_id)
        .bind(&chunks_json)
        .fetch_one(&mut **tx)
        .await?;

    Ok(count)
}

/// Version-bump old chunks and batch-insert new ones via SQL function.
/// Gates search triggers, does bulk version-bump + INSERT, rebuilds once.
async fn replace_chunks(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    resource_id: Uuid,
    chunks: &[PackedChunk],
) -> ApiResult<i32> {
    let chunks_json = chunks_to_jsonb(chunks);

    let (count,): (i32,) = sqlx::query_as("SELECT replace_resource_chunks($1, $2)")
        .bind(resource_id)
        .bind(&chunks_json)
        .fetch_one(&mut **tx)
        .await?;

    Ok(count)
}
```

- [ ] **Step 3: Update ingest() to use persist_chunks**

In the `ingest()` function, replace line 228:

```rust
    insert_chunks(&mut tx, resource_id, &chunks).await?;
```

With:

```rust
    persist_chunks(&mut tx, resource_id, &chunks).await?;
```

- [ ] **Step 4: Update update() to use replace_chunks**

In the `update()` function, replace lines 319-357 (the version-bump + insert loop):

```rust
    // Version-bump old chunks
    sqlx::query(
        "UPDATE kb_chunks SET is_current = false WHERE resource_id = $1 AND is_current = true",
    )
    .bind(resource_id)
    .execute(&mut *tx)
    .await?;

    // Insert new chunks (version auto-computed)
    for chunk in &chunks {
        let chunk_id = Uuid::now_v7();
        let embedding_str = format_embedding(&chunk.embedding);
        sqlx::query(
            r#"
            INSERT INTO kb_chunks (
                id, resource_id, chunk_index, version, header_path,
                content_hash, embedding, is_current
            )
            VALUES ($1, $2, $3,
                    COALESCE((SELECT MAX(version) FROM kb_chunks
                              WHERE resource_id = $2 AND chunk_index = $3), 0) + 1,
                    $4, $5, $6::vector, true)
            "#,
        )
        .bind(chunk_id)
        .bind(resource_id)
        .bind(chunk.chunk_index as i32)
        .bind(&chunk.header_path)
        .bind(&chunk.content_hash)
        .bind(&embedding_str)
        .execute(&mut *tx)
        .await?;

        sqlx::query("INSERT INTO kb_chunk_content (chunk_id, content) VALUES ($1, $2)")
            .bind(chunk_id)
            .bind(&chunk.content)
            .execute(&mut *tx)
            .await?;
    }
```

With:

```rust
    // Replace chunks — version-bump + batch insert + search rebuild in one call
    replace_chunks(&mut tx, resource_id, &chunks).await?;
```

- [ ] **Step 5: Verify compilation**

```bash
cargo check -p temper-api
```

Expected: PASS (no more `format_embedding` import errors).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs
git commit -m "feat: replace insert_chunks loop with batch SQL functions

persist_chunks() for new resources, replace_chunks() for updates.
Single search index rebuild per ingest instead of O(n) per chunk."
```

---

### Task 7: Update MCP Tool Description and Verify Workspace Compilation

**Files:**
- Modify: `crates/temper-mcp/src/service.rs:98`
- Modify: `crates/temper-mcp/src/tools/search.rs:1`

- [ ] **Step 1: Update MCP tool description**

In `crates/temper-mcp/src/service.rs`, replace line 98:

```rust
    #[tool(description = "Semantic search across resources using an embedding vector.")]
```

With:

```rust
    #[tool(description = "Search resources using text queries, embedding vectors, or both. Send a plain text 'query' for full-text search — no embedding required.")]
```

- [ ] **Step 2: Update MCP search tool comment**

In `crates/temper-mcp/src/tools/search.rs`, replace line 1:

```rust
//! Search tool — vector similarity search across resources.
```

With:

```rust
//! Search tool — full-text and/or vector similarity search across resources.
```

- [ ] **Step 3: Verify full workspace compilation**

```bash
cargo check --workspace
```

Expected: PASS — all crates compile. If there are compilation errors from other callers of the old `SearchParams` (e.g. temper-client), fix them by wrapping embedding in `Some(...)` or adapting to the new field names.

- [ ] **Step 4: Run all unit tests**

```bash
cargo make test
```

Expected: All unit tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-mcp/src/service.rs crates/temper-mcp/src/tools/search.rs
git commit -m "feat: update MCP search tool for text-based queries

MCP clients can now send plain text queries without embeddings."
```

---

### Task 8: Fix All Remaining Callers of Old SearchParams

**Files:**
- Varies — depends on what `cargo check --workspace` reveals

This task handles any remaining compilation errors from the `SearchParams` change. The most likely callers are:

1. **temper-client** (CLI search command) — currently constructs `SearchParams { embedding: vec![...], ... }`. Must become `SearchParams { embedding: Some(vec![...]), query: None, search_config: "english".into(), offset: None, ... }`.
2. **temper-cli** search command — if it constructs `SearchParams` directly.
3. **Any integration tests** that construct `SearchParams`.

- [ ] **Step 1: Find all callers**

```bash
cargo check --workspace 2>&1 | head -60
```

Identify each file/line with a compilation error related to `SearchParams`.

- [ ] **Step 2: Fix each caller**

For each caller that constructs a `SearchParams`:
- Wrap `embedding` value in `Some(...)`
- Add `query: None` (or `query: Some(...)` if the caller has a text query)
- Add `search_config: "english".into()`
- Add `offset: None`

For each caller that reads `params.embedding` directly (not through an Option):
- Change to `params.embedding.as_ref()` or `.unwrap()` as appropriate

- [ ] **Step 3: Verify full workspace compiles**

```bash
cargo check --workspace
```

Expected: PASS.

- [ ] **Step 4: Run all unit tests**

```bash
cargo make test
```

Expected: All unit tests pass.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "fix: update all SearchParams callers for optional embedding"
```

---

### Task 9: Run Integration Tests Against Docker Postgres

**Files:**
- No new files — validation only

- [ ] **Step 1: Ensure Docker Postgres is running**

```bash
cargo make docker-up
```

- [ ] **Step 2: Run integration tests**

```bash
cargo make test-db
```

Expected: All integration tests pass. The `#[sqlx::test(migrator = ...)]` tests will automatically apply all migrations including the new FTS ones.

- [ ] **Step 3: Run E2E tests if available**

```bash
cargo make test-e2e
```

Expected: Pass (or skip if E2E tests don't exist for search yet).

- [ ] **Step 4: Run quality checks**

```bash
cargo make check
```

Expected: No clippy warnings, formatting is clean.

- [ ] **Step 5: Fix any issues found**

Address any test failures or clippy warnings. Common issues:
- Unused imports after removing `build_filter_clause` / `format_embedding`
- Dead code warnings for `SearchResultRow` if nothing uses it anymore

- [ ] **Step 6: Commit any fixes**

```bash
git add -u
git commit -m "fix: address clippy warnings and test failures from FTS migration"
```

---

### Task 10: Verify FTS Search End-to-End via psql

**Files:**
- No code changes — manual verification

- [ ] **Step 1: Run a text-only search via psql**

```bash
psql postgresql://temper:temper@localhost:5437/temper_development -c "
SELECT resource_id, title, fts_score, vector_score, combined_score, origin
FROM unified_search(
    (SELECT id FROM kb_profiles LIMIT 1),
    'deployment',         -- text query
    NULL,                 -- no embedding
    'english',
    NULL,                 -- no context filter
    NULL,                 -- no doc_type filter
    1.0,                  -- fts_weight
    0.0,                  -- vec_weight
    10,                   -- limit
    0                     -- offset
);"
```

Expected: Results with `origin = 'fts'`, non-zero `fts_score`, zero `vector_score`.

- [ ] **Step 2: Verify trigger works — create a resource and search for it**

```bash
psql postgresql://temper:temper@localhost:5437/temper_development -c "
-- Insert a test resource
INSERT INTO kb_resources (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug, originator_profile_id, owner_profile_id)
SELECT gen_random_uuid(),
       (SELECT id FROM kb_contexts LIMIT 1),
       (SELECT id FROM kb_doc_types WHERE name = 'task'),
       'test://fts-verify',
       'Unique FTS Verification Resource XYZ789',
       'fts-verify-xyz789',
       (SELECT id FROM kb_profiles LIMIT 1),
       (SELECT id FROM kb_profiles LIMIT 1)
RETURNING id, title;
"

psql postgresql://temper:temper@localhost:5437/temper_development -c "
-- Search should find it via title match
SELECT title, fts_score
FROM fts_search(
    (SELECT id FROM kb_profiles LIMIT 1),
    'Unique FTS Verification XYZ789',
    'english', NULL, NULL, 5, 0
);"
```

Expected: The newly inserted resource appears in search results.

- [ ] **Step 3: Clean up test data**

```bash
psql postgresql://temper:temper@localhost:5437/temper_development -c "
DELETE FROM kb_resources WHERE slug = 'fts-verify-xyz789';"
```

---

## Summary of Changes by Session

### This Session (Tasks 1-10)
- Validate SQL migrations against Docker Postgres
- Evolve `SearchParams` (query, search_config, offset; optional embedding)
- Add `UnifiedSearchResultRow`
- Add `ChunkRowJsonb`, `chunks_to_jsonb()`, move `format_embedding` to core
- Rewrite search service to call `unified_search()` SQL function
- Replace per-chunk insert loop with batch SQL functions
- Update handler, MCP tool, all callers
- Full test pass + manual verification

### Future Sessions
- **Session 2:** Integration tests for FTS search (create resource → search by text → verify results)
- **Session 3:** CLI `temper search` command to support `--query` flag alongside existing embedding search
- **Session 4:** MCP end-to-end testing with Claude Desktop
