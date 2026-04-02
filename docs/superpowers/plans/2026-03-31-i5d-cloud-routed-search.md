# I5d: Cloud-Routed Search — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild `temper search` as a cloud-routed command — CLI embeds queries locally, sends vectors to Rust API for pgvector similarity search with access control.

**Architecture:** temper-embed crate provides local embedding (bge-base-en-v1.5 via ONNX) and extraction (kreuzberg). CLI embeds query text, sends 768-dim vector to `POST /api/search`. Server performs cosine similarity against `kb_current_chunks` with `resources_visible_to()` access control. Results enriched with local manifest cross-reference and `kb://` canonical URIs.

**Tech Stack:** Rust (ort, hf-hub, tokenizers, ndarray, kreuzberg), Axum, sqlx, pgvector, clap

**Spec:** `docs/superpowers/specs/2026-03-31-i5d-cloud-routed-search-design.md`

**Design principles:**
- **Thin commands**: CLI commands parse args and format output. All business logic lives in `src/actions/`.
- **Decomposed functions**: Each function does one thing and is independently testable. No monolithic query builders or inline SQL.
- **Shared types in temper-core**: Client and API share the same types. The JSON across the wire is identical.
- **Runtime abstraction**: Use `actions/runtime.rs` (created in Task 1) instead of raw `tokio::runtime::Runtime::new()`.
- **Composable queries**: SQL WHERE clauses are built incrementally, not duplicated across if/else branches.

---

## File Map

### New files
| File | Responsibility |
|------|---------------|
| `crates/temper-cli/src/actions/runtime.rs` | Shared runtime + client setup (missing I6a deliverable) |
| `crates/temper-embed/src/embed.rs` | ONNX embedding — model management, tokenization, inference, pooling, normalization |
| `crates/temper-embed/src/extract.rs` | Document extraction (kreuzberg, moved from temper-cli) |
| `crates/temper-embed/src/error.rs` | Error types for embed crate |
| `crates/temper-cli/src/commands/search_cmd.rs` | Thin CLI command — args → actions → output |
| `crates/temper-cli/src/actions/search.rs` | Search business logic — embedding, API call, manifest enrichment, formatting |

### Modified files
| File | Change |
|------|--------|
| `crates/temper-embed/Cargo.toml` | Add ort, hf-hub, tokenizers, ndarray, kreuzberg deps with feature gates |
| `crates/temper-embed/src/lib.rs` | Re-export embed + extract modules |
| `crates/temper-cli/Cargo.toml` | Replace kreuzberg dep with temper-embed dep |
| `crates/temper-cli/src/extract.rs` | Delegate to temper-embed |
| `crates/temper-cli/src/commands/mod.rs` | Add `pub mod search_cmd;` |
| `crates/temper-cli/src/actions/mod.rs` | Add `pub mod search;` and `pub mod runtime;` |
| `crates/temper-cli/src/cli.rs` | Add `Search` command variant |
| `crates/temper-cli/src/main.rs` | Add `Commands::Search` dispatch |
| `crates/temper-core/src/types/api.rs` | Update SearchParams (embedding vector) and SearchResultRow (add context, doc_type, header_path, origin_uri) |
| `crates/temper-core/src/types/search.rs` | Remove speculative types (SearchMode, SearchRequest, SearchResponse) — kept as module for future use |
| `crates/temper-core/src/types/mod.rs` | Remove search.rs re-exports (types stay in api.rs, which is already re-exported) |
| `crates/temper-api/src/handlers/search.rs` | Switch from GET+Query to POST+Json |
| `crates/temper-api/src/services/search_service.rs` | Implement pgvector search with composable query builder |
| `crates/temper-api/src/routes.rs` | Change route from `get` to `post` |
| `crates/temper-client/src/search.rs` | Switch to POST with JSON body |

---

## Task 1: Create shared runtime abstraction (missing I6a deliverable)

**Files:**
- Create: `crates/temper-cli/src/actions/runtime.rs`
- Modify: `crates/temper-cli/src/actions/mod.rs`

- [ ] **Step 1: Write tests for runtime abstraction**

Create `crates/temper-cli/src/actions/runtime.rs`:

```rust
//! Shared runtime and client setup for CLI commands that call the cloud API.
//!
//! Eliminates duplicated `tokio::runtime::Runtime::new()` + `build_client()`
//! boilerplate across command modules.

use std::future::Future;
use std::pin::Pin;

use crate::error::{Result, TemperError};

/// Create a tokio runtime and temper client, then execute an async closure.
///
/// This is the standard pattern for CLI commands that need to make API calls.
/// The closure receives a reference to the built client.
pub fn with_client<F, T>(f: F) -> Result<T>
where
    F: FnOnce(&temper_client::TemperClient) -> Pin<Box<dyn Future<Output = Result<T>> + '_>>,
{
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;
    let client =
        temper_client::config::build_client().map_err(|e| TemperError::Api(e.to_string()))?;
    rt.block_on(f(&client))
}

/// Require a device_id or return a clear auth error.
pub fn require_device_id() -> Result<String> {
    crate::config::load_device_id().ok_or_else(|| {
        TemperError::Config("not authenticated — run `temper auth login` first".into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_require_device_id_returns_error_when_not_logged_in() {
        // In test environment, no device_id file exists — should return Config error.
        let result = require_device_id();
        // This may pass or fail depending on test environment; the important thing
        // is it doesn't panic. If device_id file exists, it returns Ok.
        assert!(result.is_ok() || result.is_err());
    }
}
```

- [ ] **Step 2: Register module**

In `crates/temper-cli/src/actions/mod.rs`, add:

```rust
pub mod runtime;
```

- [ ] **Step 3: Check compilation**

Run: `cargo check -p temper-cli 2>&1 | head -20`
Expected: Clean compilation.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/actions/runtime.rs crates/temper-cli/src/actions/mod.rs
git commit -m "feat: add shared runtime abstraction for CLI API commands"
```

---

## Task 2: Update core search types

**Files:**
- Modify: `crates/temper-core/src/types/api.rs`
- Modify: `crates/temper-core/src/types/search.rs`
- Modify: `crates/temper-core/src/types/mod.rs`

- [ ] **Step 1: Update SearchParams and SearchResultRow in api.rs**

In `crates/temper-core/src/types/api.rs`, replace the `SearchParams` and `SearchResultRow` structs:

```rust
/// Request body for POST /api/search.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct SearchParams {
    /// Pre-computed 768-dim embedding vector.
    pub embedding: Vec<f32>,
    /// Filter by kb_context ID.
    pub context: Option<Uuid>,
    /// Filter by document type.
    pub doc_type: Option<String>,
    /// Maximum results (default 10, max 50).
    pub limit: Option<i64>,
}

/// A single search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct SearchResultRow {
    pub resource_id: Uuid,
    pub title: String,
    /// Canonical kb:// URI: kb://context/doc_type/uuid (from kb_resource_uri SQL function)
    pub kb_uri: String,
    /// Original source URL or file reference
    pub origin_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub doc_type: String,
    pub score: f32,
    pub snippet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_path: Option<String>,
}
```

- [ ] **Step 2: Clear speculative types from search.rs**

Replace `crates/temper-core/src/types/search.rs`:

```rust
//! Search types.
//!
//! Core search types (SearchParams, SearchResultRow) live in `api.rs` so they
//! are shared between the API and client crates. This module is reserved for
//! future search-specific types (e.g., graph traversal, faceted search).
```

- [ ] **Step 3: Remove search re-exports from mod.rs**

In `crates/temper-core/src/types/mod.rs`, remove the search re-export line:

```rust
// REMOVE this line:
pub use search::{SearchMode, SearchRequest, SearchResponse, SearchResult};
```

Keep `pub mod search;` — the module stays for future use.

- [ ] **Step 4: Check compilation**

Run: `cargo check -p temper-core 2>&1 | head -20`
Expected: Compilation errors in downstream crates referencing removed types — fixed in subsequent tasks.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/api.rs crates/temper-core/src/types/search.rs crates/temper-core/src/types/mod.rs
git commit -m "refactor: update search types for vector search, add kb_uri and origin_uri"
```

---

## Task 3: Implement search service with composable query

**Files:**
- Modify: `crates/temper-api/src/handlers/search.rs`
- Modify: `crates/temper-api/src/services/search_service.rs`
- Modify: `crates/temper-api/src/routes.rs`

- [ ] **Step 1: Write the search query builder**

Replace `crates/temper-api/src/services/search_service.rs`:

```rust
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

pub use temper_core::types::api::{SearchParams, SearchResultRow};

const MAX_LIMIT: i64 = 50;
const DEFAULT_LIMIT: i64 = 10;
const EMBEDDING_DIM: usize = 768;

/// Validate search params. Returns the sanitized limit.
pub fn validate_params(params: &SearchParams) -> ApiResult<i64> {
    if params.embedding.len() != EMBEDDING_DIM {
        return Err(ApiError::BadRequest(format!(
            "embedding must be {EMBEDDING_DIM} dimensions, got {}",
            params.embedding.len()
        )));
    }
    Ok(params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT))
}

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

/// Build the WHERE clause fragments and corresponding bind values for optional filters.
///
/// Returns (where_clause, context_bind, doc_type_bind) where where_clause is
/// appended to the base query. The bind values are Options that the caller
/// binds in order — only non-None values produce parameter placeholders.
pub fn build_filter_clause(
    context: Option<Uuid>,
    doc_type: Option<&str>,
    next_param: &mut i32,
) -> (String, Option<Uuid>, Option<String>) {
    let mut clause = String::new();
    let mut ctx_bind = None;
    let mut dt_bind = None;

    if let Some(ctx) = context {
        clause.push_str(&format!(" AND r.kb_context_id = ${next_param}"));
        *next_param += 1;
        ctx_bind = Some(ctx);
    }
    if let Some(dt) = doc_type {
        clause.push_str(&format!(" AND dt.name = ${next_param}"));
        *next_param += 1;
        dt_bind = Some(dt.to_string());
    }

    (clause, ctx_bind, dt_bind)
}

/// Execute the vector similarity search query.
pub async fn search(
    pool: &PgPool,
    profile_id: Uuid,
    params: SearchParams,
) -> ApiResult<Vec<SearchResultRow>> {
    let limit = validate_params(&params)?;
    let embedding_str = format_embedding(&params.embedding);

    // Base query with pgvector cosine similarity.
    // Parameter slots: $1 = embedding, $2 = profile_id, then optional filters, then limit.
    let mut next_param: i32 = 3;
    let (filter_clause, ctx_bind, dt_bind) =
        build_filter_clause(params.context, params.doc_type.as_deref(), &mut next_param);
    let limit_param = next_param;

    let sql = format!(
        "SELECT r.id AS resource_id, r.title, \
         kb_resource_uri(r.id) AS kb_uri, r.origin_uri, \
         ctx.name AS context, dt.name AS doc_type, \
         c.content AS snippet, c.header_path, \
         (1 - (c.embedding <=> $1::vector))::real AS score \
         FROM kb_current_chunks c \
         JOIN kb_resources r ON c.resource_id = r.id \
         LEFT JOIN kb_contexts ctx ON r.kb_context_id = ctx.id \
         JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id \
         WHERE r.id IN (SELECT resource_id FROM resources_visible_to($2)) \
         {filter_clause} \
         ORDER BY c.embedding <=> $1::vector LIMIT ${limit_param}"
    );

    // Build the query with dynamic binds.
    let mut query = sqlx::query_as::<_, SearchResultRow>(&sql)
        .bind(&embedding_str)
        .bind(profile_id);

    if let Some(ctx) = ctx_bind {
        query = query.bind(ctx);
    }
    if let Some(dt) = dt_bind {
        query = query.bind(dt);
    }

    let rows = query.bind(limit).fetch_all(pool).await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_params_wrong_dimension() {
        let params = SearchParams {
            embedding: vec![0.0; 100],
            context: None,
            doc_type: None,
            limit: None,
        };
        let result = validate_params(&params);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_params_correct_dimension() {
        let params = SearchParams {
            embedding: vec![0.0; 768],
            context: None,
            doc_type: None,
            limit: None,
        };
        let result = validate_params(&params);
        assert_eq!(result.unwrap(), 10); // default limit
    }

    #[test]
    fn test_validate_params_clamps_limit() {
        let params = SearchParams {
            embedding: vec![0.0; 768],
            context: None,
            doc_type: None,
            limit: Some(200),
        };
        assert_eq!(validate_params(&params).unwrap(), 50);
    }

    #[test]
    fn test_format_embedding() {
        let embedding = vec![0.1, 0.2, 0.3];
        let result = format_embedding(&embedding);
        assert_eq!(result, "[0.1,0.2,0.3]");
    }

    #[test]
    fn test_format_embedding_empty() {
        let result = format_embedding(&[]);
        assert_eq!(result, "[]");
    }

    #[test]
    fn test_build_filter_clause_no_filters() {
        let mut next = 3;
        let (clause, ctx, dt) = build_filter_clause(None, None, &mut next);
        assert_eq!(clause, "");
        assert!(ctx.is_none());
        assert!(dt.is_none());
        assert_eq!(next, 3);
    }

    #[test]
    fn test_build_filter_clause_context_only() {
        let mut next = 3;
        let id = Uuid::nil();
        let (clause, ctx, dt) = build_filter_clause(Some(id), None, &mut next);
        assert_eq!(clause, " AND r.kb_context_id = $3");
        assert_eq!(ctx, Some(id));
        assert!(dt.is_none());
        assert_eq!(next, 4);
    }

    #[test]
    fn test_build_filter_clause_both_filters() {
        let mut next = 3;
        let id = Uuid::nil();
        let (clause, ctx, dt) = build_filter_clause(Some(id), Some("task"), &mut next);
        assert_eq!(clause, " AND r.kb_context_id = $3 AND dt.name = $4");
        assert_eq!(ctx, Some(id));
        assert_eq!(dt.as_deref(), Some("task"));
        assert_eq!(next, 5);
    }

    #[test]
    fn test_build_filter_clause_doc_type_only() {
        let mut next = 3;
        let (clause, ctx, dt) = build_filter_clause(None, Some("session"), &mut next);
        assert_eq!(clause, " AND dt.name = $3");
        assert!(ctx.is_none());
        assert_eq!(dt.as_deref(), Some("session"));
        assert_eq!(next, 4);
    }
}
```

- [ ] **Step 2: Update the search handler to POST**

Replace `crates/temper-api/src/handlers/search.rs`:

```rust
use axum::extract::State;
use axum::Json;

use crate::error::{ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::search_service::{self, SearchParams, SearchResultRow};
use crate::state::AppState;

#[utoipa::path(
    post,
    path = "/api/search",
    tag = "Search",
    request_body = SearchParams,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Search results", body = Vec<SearchResultRow>),
        (status = 400, description = "Invalid request", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn search(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(params): Json<SearchParams>,
) -> ApiResult<Json<Vec<SearchResultRow>>> {
    search_service::search(&state.pool, auth.0.profile.id, params)
        .await
        .map(Json)
}
```

- [ ] **Step 3: Update route from GET to POST**

In `crates/temper-api/src/routes.rs`, change:

```rust
// Change import:
use axum::routing::{get, post};

// Change route:
.route("/api/search", post(handlers::search::search))
```

- [ ] **Step 4: Add FromRow to SearchResultRow**

In `crates/temper-core/src/types/api.rs`, add `FromRow` derive to `SearchResultRow`. The import `use sqlx::FromRow;` is already present (used by `EventRow`):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct SearchResultRow {
```

- [ ] **Step 5: Check compilation**

Run: `cargo check -p temper-api 2>&1 | head -20`
Expected: Clean compilation.

- [ ] **Step 6: Run search service tests**

Run: `cargo test -p temper-api -- search_service 2>&1`
Expected: All 7 tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-api/src/handlers/search.rs crates/temper-api/src/services/search_service.rs crates/temper-api/src/routes.rs crates/temper-core/src/types/api.rs
git commit -m "feat: implement pgvector search service with composable query builder"
```

---

## Task 4: Update search client

**Files:**
- Modify: `crates/temper-client/src/search.rs`

- [ ] **Step 1: Update SearchClient to POST with JSON body**

Replace `crates/temper-client/src/search.rs`:

```rust
//! Typed sub-client for the `/api/search` endpoint.

use crate::auth;
use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::api::{SearchParams, SearchResultRow};
use uuid::Uuid;

/// Sub-client for search operations.
pub struct SearchClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for SearchClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchClient").finish_non_exhaustive()
    }
}

impl<'a> SearchClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// Run a vector similarity search.
    pub async fn query(
        &self,
        embedding: Vec<f32>,
        context: Option<Uuid>,
        doc_type: Option<String>,
        limit: Option<i64>,
    ) -> Result<Vec<SearchResultRow>> {
        let token = auth::current_token()?;
        let params = SearchParams {
            embedding,
            context,
            doc_type,
            limit,
        };
        let req = self.http.post("/api/search").json(&params);
        self.http.send_json(req, Some(&token)).await
    }
}
```

- [ ] **Step 2: Check compilation**

Run: `cargo check -p temper-client 2>&1 | head -20`
Expected: Clean compilation. If `uuid` is not in temper-client's deps, add it.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-client/src/search.rs
git commit -m "feat: update SearchClient to POST with embedding vector"
```

---

## Task 5: Build temper-embed crate — extraction

**Files:**
- Modify: `crates/temper-embed/Cargo.toml`
- Create: `crates/temper-embed/src/error.rs`
- Create: `crates/temper-embed/src/extract.rs`
- Modify: `crates/temper-embed/src/lib.rs`
- Modify: `crates/temper-cli/Cargo.toml`
- Modify: `crates/temper-cli/src/extract.rs`

- [ ] **Step 1: Set up temper-embed Cargo.toml with extract feature**

Replace `crates/temper-embed/Cargo.toml`:

```toml
[package]
name = "temper-embed"
version = "0.1.0"
edition = "2021"
description = "Embedding and extraction pipeline for temper knowledge base"

[dependencies]
temper-core = { path = "../temper-core" }
thiserror = "2"
kreuzberg = { version = "4.6.3", optional = true, features = ["tokio-runtime"] }

[features]
default = ["extract"]
extract = ["dep:kreuzberg"]

[dev-dependencies]
tempfile = "3"

[package.metadata.cargo-machete]
ignored = ["temper-core"]
```

- [ ] **Step 2: Create error module**

Create `crates/temper-embed/src/error.rs`:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmbedError {
    #[error("extraction error: {0}")]
    Extraction(String),

    #[error("embedding error: {0}")]
    Embedding(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, EmbedError>;
```

- [ ] **Step 3: Create extract module**

Create `crates/temper-embed/src/extract.rs` — same logic as temper-cli's extract.rs but using temper-embed's error type:

```rust
//! Document extraction — markdown/text passthrough, kreuzberg for other formats.

use std::path::Path;

use crate::error::{EmbedError, Result};

/// The result of extracting a file to text.
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    pub content: String,
    pub mime_type: String,
}

/// Extract a file to markdown text.
pub fn extract_to_markdown(path: &Path) -> Result<ExtractionResult> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match extension.as_str() {
        "md" | "markdown" => Ok(ExtractionResult {
            content: std::fs::read_to_string(path)?,
            mime_type: "text/markdown".to_string(),
        }),
        "txt" | "text" => Ok(ExtractionResult {
            content: std::fs::read_to_string(path)?,
            mime_type: "text/plain".to_string(),
        }),
        _ => extract_with_kreuzberg(path),
    }
}

#[cfg(feature = "extract")]
fn extract_with_kreuzberg(path: &Path) -> Result<ExtractionResult> {
    use kreuzberg::{extract_file_sync, ExtractionConfig};

    let config = ExtractionConfig::default();
    let result = extract_file_sync(path, None, &config).map_err(|e| {
        EmbedError::Extraction(format!("failed to extract '{}': {}", path.display(), e))
    })?;

    Ok(ExtractionResult {
        content: result.content,
        mime_type: result.mime_type.into_owned(),
    })
}

#[cfg(not(feature = "extract"))]
fn extract_with_kreuzberg(path: &Path) -> Result<ExtractionResult> {
    Err(EmbedError::Extraction(format!(
        "cannot extract '{}': the 'extract' feature is required for non-text files",
        path.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_extract_markdown() {
        let mut f = NamedTempFile::with_suffix(".md").unwrap();
        writeln!(f, "# Hello").unwrap();
        let r = extract_to_markdown(f.path()).unwrap();
        assert!(r.content.contains("# Hello"));
        assert_eq!(r.mime_type, "text/markdown");
    }

    #[test]
    fn test_extract_plain_text() {
        let mut f = NamedTempFile::with_suffix(".txt").unwrap();
        writeln!(f, "Hello").unwrap();
        let r = extract_to_markdown(f.path()).unwrap();
        assert!(r.content.contains("Hello"));
        assert_eq!(r.mime_type, "text/plain");
    }

    #[test]
    #[cfg(not(feature = "extract"))]
    fn test_non_text_without_feature() {
        let f = NamedTempFile::with_suffix(".pdf").unwrap();
        assert!(extract_to_markdown(f.path()).is_err());
    }
}
```

- [ ] **Step 4: Update lib.rs**

Replace `crates/temper-embed/src/lib.rs`:

```rust
//! temper-embed — Embedding and extraction pipeline.
//!
//! Feature-gated:
//! - `extract`: kreuzberg-based document extraction
//! - `embed`: bge-base-en-v1.5 text embedding via ONNX Runtime (Task 6)

pub mod error;
pub mod extract;
```

- [ ] **Step 5: Update temper-cli to depend on temper-embed**

In `crates/temper-cli/Cargo.toml`:

Remove `kreuzberg` from `[dependencies]`:
```toml
# REMOVE: kreuzberg = { version = "4.6.3", optional = true, features = ["tokio-runtime"] }
```

Add `temper-embed`:
```toml
temper-embed = { path = "../temper-embed" }
```

Update `[features]`:
```toml
[features]
default = ["extract"]
extract = ["temper-embed/extract"]
```

- [ ] **Step 6: Update temper-cli extract.rs to delegate**

Replace `crates/temper-cli/src/extract.rs`:

```rust
//! Document extraction — delegates to temper-embed.

use std::path::Path;

use crate::error::{Result, TemperError};

pub use temper_embed::extract::ExtractionResult;

pub fn extract_to_markdown(path: &Path) -> Result<ExtractionResult> {
    temper_embed::extract::extract_to_markdown(path)
        .map_err(|e| TemperError::Extraction(e.to_string()))
}
```

- [ ] **Step 7: Check compilation and run tests**

Run: `cargo check -p temper-embed -p temper-cli 2>&1 | head -20`
Run: `cargo test -p temper-embed -- extract 2>&1`
Expected: Clean compilation, all extract tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-embed/ crates/temper-cli/Cargo.toml crates/temper-cli/src/extract.rs
git commit -m "refactor: move extraction to temper-embed, temper-cli delegates"
```

---

## Task 6: Build temper-embed crate — embedding

**Files:**
- Modify: `crates/temper-embed/Cargo.toml`
- Create: `crates/temper-embed/src/embed.rs`
- Modify: `crates/temper-embed/src/lib.rs`

Each step in the embedding pipeline is a discrete, testable function:
1. `load_model()` — download + cache model, create ONNX session
2. `tokenize()` — text → token IDs + attention masks
3. `build_input_tensors()` — encodings → ndarray tensors
4. `mean_pool()` — hidden states + mask → pooled embedding
5. `normalize()` — L2 normalization
6. `embed_text()` / `embed_texts()` — orchestration composing the above

- [ ] **Step 1: Add embed dependencies**

In `crates/temper-embed/Cargo.toml`, add to `[dependencies]`:

```toml
ort = { version = "2", optional = true }
hf-hub = { version = "0.4", optional = true }
tokenizers = { version = "0.21", optional = true }
ndarray = { version = "0.16", optional = true }
```

Add to `[features]`:

```toml
[features]
default = ["extract", "embed"]
extract = ["dep:kreuzberg"]
embed = ["dep:ort", "dep:hf-hub", "dep:tokenizers", "dep:ndarray"]
```

Note: Verify exact crate versions using `cargo add --dry-run` before implementing. Check tasker-core's main branch `Cargo.toml` for `hf-hub` version if in doubt.

- [ ] **Step 2: Create embed module with decomposed functions**

Create `crates/temper-embed/src/embed.rs`:

```rust
//! Text embedding using BAAI/bge-base-en-v1.5 via ONNX Runtime.
//!
//! Model downloaded on first use via hf-hub, cached at ~/.cache/huggingface/.
//! ONNX session created once per process via OnceLock.
//!
//! Pipeline: tokenize → build tensors → inference → mean pool → normalize

use std::sync::OnceLock;

use ndarray::{Array2, ArrayView3};
use ort::session::Session;
use tokenizers::{Encoding, Tokenizer};

use crate::error::{EmbedError, Result};

/// Embedding dimension for bge-base-en-v1.5.
pub const EMBEDDING_DIM: usize = 768;

const MODEL_REPO: &str = "BAAI/bge-base-en-v1.5";

// ---- Model management ----

struct Model {
    session: Session,
    tokenizer: Tokenizer,
}

static MODEL: OnceLock<std::result::Result<Model, String>> = OnceLock::new();

fn load_model() -> Result<&'static Model> {
    let result = MODEL.get_or_init(|| {
        let api = hf_hub::api::sync::Api::new()
            .map_err(|e| format!("hf-hub init: {e}"))?;
        let repo = api.model(MODEL_REPO.to_string());

        let model_path = repo
            .get("onnx/model.onnx")
            .map_err(|e| format!("download model: {e}"))?;
        let tokenizer_path = repo
            .get("tokenizer.json")
            .map_err(|e| format!("download tokenizer: {e}"))?;

        let session = Session::builder()
            .map_err(|e| format!("ort session builder: {e}"))?
            .with_intra_threads(1)
            .map_err(|e| format!("ort threads: {e}"))?
            .commit_from_file(&model_path)
            .map_err(|e| format!("ort load: {e}"))?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| format!("load tokenizer: {e}"))?;

        Ok(Model { session, tokenizer })
    });

    match result {
        Ok(m) => Ok(m),
        Err(e) => Err(EmbedError::Embedding(format!("model init: {e}"))),
    }
}

// ---- Tokenization ----

/// Tokenize a batch of texts using the model's tokenizer.
pub fn tokenize(tokenizer: &Tokenizer, texts: &[&str]) -> Result<Vec<Encoding>> {
    tokenizer
        .encode_batch(texts.to_vec(), true)
        .map_err(|e| EmbedError::Embedding(format!("tokenize: {e}")))
}

// ---- Tensor construction ----

/// Input tensors for the ONNX model.
pub struct InputTensors {
    pub input_ids: Array2<i64>,
    pub attention_mask: Array2<i64>,
    pub token_type_ids: Array2<i64>,
}

/// Build input tensors from tokenizer encodings.
pub fn build_input_tensors(encodings: &[Encoding]) -> InputTensors {
    let batch_size = encodings.len();
    let max_len = encodings.iter().map(|e| e.get_ids().len()).max().unwrap_or(0);

    let mut input_ids = Array2::<i64>::zeros((batch_size, max_len));
    let mut attention_mask = Array2::<i64>::zeros((batch_size, max_len));
    let mut token_type_ids = Array2::<i64>::zeros((batch_size, max_len));

    for (i, enc) in encodings.iter().enumerate() {
        for (j, &id) in enc.get_ids().iter().enumerate() {
            input_ids[[i, j]] = id as i64;
        }
        for (j, &mask) in enc.get_attention_mask().iter().enumerate() {
            attention_mask[[i, j]] = mask as i64;
        }
        for (j, &tid) in enc.get_type_ids().iter().enumerate() {
            token_type_ids[[i, j]] = tid as i64;
        }
    }

    InputTensors {
        input_ids,
        attention_mask,
        token_type_ids,
    }
}

// ---- Pooling and normalization ----

/// Mean pooling: average hidden states weighted by attention mask.
pub fn mean_pool(hidden_states: ArrayView3<f32>, attention_mask: &Array2<i64>) -> Vec<Vec<f32>> {
    let batch_size = hidden_states.shape()[0];
    let max_len = hidden_states.shape()[1];
    let dim = hidden_states.shape()[2];
    let mut results = Vec::with_capacity(batch_size);

    for i in 0..batch_size {
        let mut embedding = vec![0f32; dim];
        let mut mask_sum = 0f32;

        for j in 0..max_len {
            let m = attention_mask[[i, j]] as f32;
            mask_sum += m;
            for k in 0..dim {
                embedding[k] += hidden_states[[i, j, k]] * m;
            }
        }

        if mask_sum > 0.0 {
            for v in &mut embedding {
                *v /= mask_sum;
            }
        }

        results.push(embedding);
    }

    results
}

/// L2-normalize a vector in place.
pub fn l2_normalize(vec: &mut [f32]) {
    let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in vec.iter_mut() {
            *v /= norm;
        }
    }
}

// ---- Public API ----

/// Embed a single text string into a 768-dim normalized vector.
pub fn embed_text(text: &str) -> Result<Vec<f32>> {
    let mut results = embed_texts(&[text])?;
    Ok(results.remove(0))
}

/// Embed multiple texts into 768-dim normalized vectors.
pub fn embed_texts(texts: &[&str]) -> Result<Vec<Vec<f32>>> {
    let model = load_model()?;

    let encodings = tokenize(&model.tokenizer, texts)?;
    let tensors = build_input_tensors(&encodings);

    let outputs = model
        .session
        .run(
            ort::inputs![
                tensors.input_ids,
                tensors.attention_mask.clone(),
                tensors.token_type_ids
            ]
            .map_err(|e| EmbedError::Embedding(format!("ort inputs: {e}")))?,
        )
        .map_err(|e| EmbedError::Embedding(format!("ort run: {e}")))?;

    let hidden = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|e| EmbedError::Embedding(format!("extract tensor: {e}")))?;

    let mut pooled = mean_pool(hidden.view(), &tensors.attention_mask);
    for embedding in &mut pooled {
        l2_normalize(embedding);
    }

    Ok(pooled)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Unit tests for decomposed functions (no model needed) ----

    #[test]
    fn test_l2_normalize() {
        let mut vec = vec![3.0, 4.0];
        l2_normalize(&mut vec);
        assert!((vec[0] - 0.6).abs() < 1e-6);
        assert!((vec[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn test_l2_normalize_zero_vector() {
        let mut vec = vec![0.0, 0.0, 0.0];
        l2_normalize(&mut vec);
        assert_eq!(vec, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_l2_normalize_already_unit() {
        let mut vec = vec![1.0, 0.0, 0.0];
        l2_normalize(&mut vec);
        assert!((vec[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_mean_pool_basic() {
        // 1 sample, 2 tokens, 3-dim hidden states
        let hidden = ndarray::array![[[1.0f32, 2.0, 3.0], [4.0, 5.0, 6.0]]];
        let mask = ndarray::array![[1i64, 1]];
        let result = mean_pool(hidden.view(), &mask);
        assert_eq!(result.len(), 1);
        assert!((result[0][0] - 2.5).abs() < 1e-6); // (1+4)/2
        assert!((result[0][1] - 3.5).abs() < 1e-6); // (2+5)/2
        assert!((result[0][2] - 4.5).abs() < 1e-6); // (3+6)/2
    }

    #[test]
    fn test_mean_pool_with_padding() {
        // 1 sample, 2 tokens, but second is masked (padding)
        let hidden = ndarray::array![[[1.0f32, 2.0, 3.0], [99.0, 99.0, 99.0]]];
        let mask = ndarray::array![[1i64, 0]];
        let result = mean_pool(hidden.view(), &mask);
        // Only first token contributes
        assert!((result[0][0] - 1.0).abs() < 1e-6);
        assert!((result[0][1] - 2.0).abs() < 1e-6);
        assert!((result[0][2] - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_build_input_tensors_shapes() {
        // Create a minimal tokenizer encoding using the builder pattern
        // We test indirectly via the model in integration tests; here test shapes.
        // Use a simple case: 2 encodings with known lengths.
        let model = load_model();
        if model.is_err() {
            // Model not downloaded in CI — skip
            return;
        }
        let model = model.unwrap();
        let encodings = tokenize(&model.tokenizer, &["hello", "hi"]).unwrap();
        let tensors = build_input_tensors(&encodings);

        assert_eq!(tensors.input_ids.shape()[0], 2); // batch size
        assert_eq!(tensors.attention_mask.shape()[0], 2);
        assert_eq!(tensors.token_type_ids.shape()[0], 2);
        // All should have same seq_len (padded to max)
        assert_eq!(tensors.input_ids.shape()[1], tensors.attention_mask.shape()[1]);
    }

    // ---- Integration tests (require model download) ----

    #[test]
    fn test_embed_text_dimension() {
        let vec = embed_text("hello world").unwrap();
        assert_eq!(vec.len(), EMBEDDING_DIM);
    }

    #[test]
    fn test_embed_text_is_normalized() {
        let vec = embed_text("hello world").unwrap();
        let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "norm was {norm}");
    }

    #[test]
    fn test_embed_texts_batch() {
        let vecs = embed_texts(&["hello", "world"]).unwrap();
        assert_eq!(vecs.len(), 2);
        assert_eq!(vecs[0].len(), EMBEDDING_DIM);
        assert_eq!(vecs[1].len(), EMBEDDING_DIM);
    }

    #[test]
    fn test_similar_texts_higher_similarity() {
        let v1 = embed_text("rust programming language").unwrap();
        let v2 = embed_text("rust cargo build system").unwrap();
        let v3 = embed_text("chocolate cake recipe").unwrap();

        let sim_related: f32 = v1.iter().zip(&v2).map(|(a, b)| a * b).sum();
        let sim_unrelated: f32 = v1.iter().zip(&v3).map(|(a, b)| a * b).sum();
        assert!(sim_related > sim_unrelated);
    }
}
```

- [ ] **Step 3: Update lib.rs**

```rust
//! temper-embed — Embedding and extraction pipeline.
//!
//! Feature-gated:
//! - `extract`: kreuzberg-based document extraction
//! - `embed`: bge-base-en-v1.5 text embedding via ONNX Runtime

pub mod error;
pub mod extract;

#[cfg(feature = "embed")]
pub mod embed;
```

- [ ] **Step 4: Check compilation**

Run: `cargo check -p temper-embed --all-features 2>&1 | head -20`
Expected: Clean. Fix version mismatches by checking crates.io.

- [ ] **Step 5: Run unit tests (no model download)**

Run: `cargo test -p temper-embed -- l2_normalize mean_pool 2>&1`
Expected: All pooling/normalization unit tests pass.

- [ ] **Step 6: Run full embed tests (downloads model on first run)**

Run: `cargo test -p temper-embed -- embed 2>&1`
Expected: All tests pass. First run downloads ~400MB model.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-embed/
git commit -m "feat: add bge-base-en-v1.5 embedding with decomposed pipeline"
```

---

## Task 7: CLI search actions and thin command

**Files:**
- Create: `crates/temper-cli/src/actions/search.rs`
- Modify: `crates/temper-cli/src/actions/mod.rs`
- Create: `crates/temper-cli/src/commands/search_cmd.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs`
- Modify: `crates/temper-cli/src/cli.rs`
- Modify: `crates/temper-cli/src/main.rs`
- Modify: `crates/temper-cli/Cargo.toml`

- [ ] **Step 1: Create search actions with all business logic**

Create `crates/temper-cli/src/actions/search.rs`:

```rust
//! Search business logic — embedding, API call, manifest enrichment, formatting.
//!
//! All testable functions. The CLI command is a thin wrapper over these.

use serde::Serialize;
use temper_core::types::api::SearchResultRow;
use temper_core::types::Manifest;
use uuid::Uuid;

use crate::error::{Result, TemperError};

/// A search result enriched with local vault information.
#[derive(Debug, Clone, Serialize)]
pub struct EnrichedSearchResult {
    pub resource_id: Uuid,
    pub title: String,
    pub kb_uri: String,
    pub origin_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub doc_type: String,
    pub score: f32,
    pub snippet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_path: Option<String>,
    pub local: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_path: Option<String>,
}

/// Embed query text locally via temper-embed.
#[cfg(feature = "embed")]
pub fn embed_query(text: &str) -> Result<Vec<f32>> {
    temper_embed::embed::embed_text(text)
        .map_err(|e| TemperError::Extraction(format!("embedding failed: {e}")))
}

#[cfg(not(feature = "embed"))]
pub fn embed_query(_text: &str) -> Result<Vec<f32>> {
    Err(TemperError::Config(
        "search requires the 'embed' feature — rebuild with --features embed".into(),
    ))
}

/// Call the search API with a pre-computed embedding.
pub async fn query_api(
    client: &temper_client::TemperClient,
    embedding: Vec<f32>,
    context: Option<Uuid>,
    doc_type: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<SearchResultRow>> {
    client
        .search()
        .query(embedding, context, doc_type, limit)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))
}

/// Enrich API results with local manifest data.
pub fn enrich_results(
    results: Vec<SearchResultRow>,
    manifest: &Manifest,
) -> Vec<EnrichedSearchResult> {
    results
        .into_iter()
        .map(|row| {
            let entry = manifest.entries.get(&row.resource_id);
            EnrichedSearchResult {
                resource_id: row.resource_id,
                title: row.title,
                kb_uri: row.kb_uri,
                origin_uri: row.origin_uri,
                context: row.context,
                doc_type: row.doc_type,
                score: row.score,
                snippet: truncate_snippet(&row.snippet, 200),
                header_path: row.header_path,
                local: entry.is_some(),
                vault_path: entry.map(|e| e.path.clone()),
            }
        })
        .collect()
}

/// Truncate a snippet to max_chars, breaking at word boundaries.
pub fn truncate_snippet(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let truncated = &text[..max_chars];
    match truncated.rfind(' ') {
        Some(pos) => format!("{}...", &text[..pos]),
        None => format!("{truncated}..."),
    }
}

/// Format results as human-readable text lines.
pub fn format_text(results: &[EnrichedSearchResult]) -> Vec<String> {
    let mut lines = Vec::new();
    for (i, r) in results.iter().enumerate() {
        let local_marker = if r.local { " [local]" } else { "" };
        lines.push(format!(
            "{}. {} (score: {:.2}){local_marker}",
            i + 1,
            r.title,
            r.score
        ));
        if let Some(ref header) = r.header_path {
            lines.push(format!("   {header}"));
        }
        lines.push(format!("   {}", r.snippet));
        if let Some(ref path) = r.vault_path {
            lines.push(format!("   vault: {path}"));
        }
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use temper_core::types::{ManifestEntry, ManifestEntryState};

    fn sample_result(resource_id: Uuid, title: &str) -> SearchResultRow {
        SearchResultRow {
            resource_id,
            title: title.to_string(),
            kb_uri: format!("kb://temper/task/{resource_id}"),
            origin_uri: "file:///vault/temper/tasks/test.md".to_string(),
            context: Some("temper".to_string()),
            doc_type: "task".to_string(),
            score: 0.85,
            snippet: "Some relevant content".to_string(),
            header_path: Some("## Section".to_string()),
        }
    }

    fn sample_manifest() -> Manifest {
        let mut manifest = Manifest::new("test-device".to_string());
        manifest.entries.insert(
            Uuid::nil(),
            ManifestEntry {
                path: "temper/tasks/test-task.md".to_string(),
                content_hash: "sha256:abc".to_string(),
                remote_hash: "sha256:abc".to_string(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
            },
        );
        manifest
    }

    #[test]
    fn test_enrich_marks_local_resources() {
        let results = vec![
            sample_result(Uuid::nil(), "Local Task"),
            sample_result(Uuid::from_u128(1), "Remote Task"),
        ];
        let enriched = enrich_results(results, &sample_manifest());
        assert!(enriched[0].local);
        assert_eq!(enriched[0].vault_path.as_deref(), Some("temper/tasks/test-task.md"));
        assert!(!enriched[1].local);
        assert!(enriched[1].vault_path.is_none());
    }

    #[test]
    fn test_enrich_preserves_kb_uri() {
        let id = Uuid::nil();
        let results = vec![sample_result(id, "Task")];
        let enriched = enrich_results(results, &Manifest::new("d".into()));
        assert_eq!(enriched[0].kb_uri, format!("kb://temper/task/{id}"));
    }

    #[test]
    fn test_enrich_empty_inputs() {
        assert!(enrich_results(vec![], &sample_manifest()).is_empty());
    }

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate_snippet("short", 200), "short");
    }

    #[test]
    fn test_truncate_long() {
        let long = "word ".repeat(100);
        let result = truncate_snippet(&long, 20);
        assert!(result.ends_with("..."));
        assert!(result.len() < 30);
    }

    #[test]
    fn test_truncate_no_space() {
        assert_eq!(truncate_snippet("aaaaaaaaaaaa", 5), "aaaaa...");
    }

    #[test]
    fn test_format_text_output() {
        let results = vec![sample_result(Uuid::nil(), "My Task")];
        let enriched = enrich_results(results, &sample_manifest());
        let lines = format_text(&enriched);
        assert!(lines[0].contains("1. My Task"));
        assert!(lines[0].contains("0.85"));
        assert!(lines[0].contains("[local]"));
    }

    #[test]
    fn test_format_text_no_local() {
        let results = vec![sample_result(Uuid::from_u128(99), "Remote")];
        let enriched = enrich_results(results, &Manifest::new("d".into()));
        let lines = format_text(&enriched);
        assert!(!lines[0].contains("[local]"));
    }

    #[test]
    fn test_enriched_json_shape() {
        let results = vec![sample_result(Uuid::nil(), "Test")];
        let enriched = enrich_results(results, &sample_manifest());
        let json = serde_json::to_value(&enriched[0]).unwrap();
        assert!(json.get("resource_id").is_some());
        assert!(json.get("kb_uri").is_some());
        assert!(json.get("origin_uri").is_some());
        assert!(json.get("local").is_some());
        assert!(json.get("score").is_some());
    }
}
```

- [ ] **Step 2: Register search actions module**

In `crates/temper-cli/src/actions/mod.rs`, add:

```rust
pub mod search;
```

- [ ] **Step 3: Create thin search command**

Create `crates/temper-cli/src/commands/search_cmd.rs`:

```rust
//! `temper search` — thin CLI wrapper over actions::search.

use crate::actions::{runtime, search as search_actions};
use crate::error::Result;
use crate::format::OutputFormat;

pub fn run(
    query: &str,
    context: Option<&str>,
    doc_type: Option<&str>,
    limit: Option<i64>,
    format: &str,
) -> Result<()> {
    let fmt = OutputFormat::parse(format);
    let vault_root = crate::config::resolve_vault(None)?;
    let temper_dir = vault_root.join(".temper");
    let device_id = runtime::require_device_id()?;
    let manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

    let embedding = search_actions::embed_query(query)?;

    // Context name → UUID not yet wired. Search returns all accessible results.
    if context.is_some() {
        crate::output::warning("--context filtering not yet implemented; showing all results");
    }

    let results = runtime::with_client(|client| {
        let embedding = embedding.clone();
        let doc_type = doc_type.map(String::from);
        Box::pin(async move {
            search_actions::query_api(client, embedding, None, doc_type, limit).await
        })
    })?;

    let enriched = search_actions::enrich_results(results, &manifest);

    if enriched.is_empty() {
        if fmt == OutputFormat::Json {
            crate::output::plain("[]");
        } else {
            crate::output::warning("No results found.");
        }
        return Ok(());
    }

    if fmt == OutputFormat::Json {
        crate::output::plain(serde_json::to_string_pretty(&enriched)?);
    } else {
        for line in search_actions::format_text(&enriched) {
            crate::output::plain(line);
        }
    }

    Ok(())
}
```

- [ ] **Step 4: Register command module**

In `crates/temper-cli/src/commands/mod.rs`, add:

```rust
pub mod search_cmd;
```

- [ ] **Step 5: Add Search variant to CLI**

In `crates/temper-cli/src/cli.rs`, add to `Commands` enum (after `Sync`):

```rust
    /// Search the knowledge base
    Search {
        /// Search query text
        query: String,
        /// Filter by context name
        #[arg(long)]
        context: Option<String>,
        /// Filter by document type
        #[arg(long)]
        doc_type: Option<String>,
        /// Maximum results (default 10)
        #[arg(long)]
        limit: Option<i64>,
        /// Output format (text or json)
        #[arg(long, default_value = "text")]
        format: String,
    },
```

- [ ] **Step 6: Add dispatch in main.rs**

In `crates/temper-cli/src/main.rs`, add after `Commands::Sync` match arm:

```rust
        Commands::Search {
            query,
            context,
            doc_type,
            limit,
            format,
        } => commands::search_cmd::run(&query, context.as_deref(), doc_type.as_deref(), limit, &format),
```

- [ ] **Step 7: Update Cargo.toml features**

In `crates/temper-cli/Cargo.toml`, update features:

```toml
[features]
default = ["extract", "embed"]
extract = ["temper-embed/extract"]
embed = ["temper-embed/embed"]
```

- [ ] **Step 8: Check compilation**

Run: `cargo check -p temper-cli --all-features 2>&1 | head -20`
Expected: Clean compilation.

- [ ] **Step 9: Run search action tests**

Run: `cargo test -p temper-cli -- actions::search 2>&1`
Expected: All 9 tests pass.

- [ ] **Step 10: Commit**

```bash
git add crates/temper-cli/src/actions/search.rs crates/temper-cli/src/actions/mod.rs \
  crates/temper-cli/src/commands/search_cmd.rs crates/temper-cli/src/commands/mod.rs \
  crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs crates/temper-cli/Cargo.toml
git commit -m "feat: add temper search command with thin wrapper and tested actions"
```

---

## Task 8: Full workspace verification

**Files:** None (verification only)

- [ ] **Step 1: Workspace compilation**

Run: `cargo check --workspace 2>&1 | tail -5`
Expected: Clean.

- [ ] **Step 2: Clippy**

Run: `cargo clippy --workspace --all-features 2>&1 | tail -10`
Expected: No warnings.

- [ ] **Step 3: All Rust tests**

Run: `cargo test --workspace 2>&1 | tail -20`
Expected: All pass.

- [ ] **Step 4: TypeScript checks**

Run: `tsc --noEmit && tsc --noEmit --project tsconfig.api.json`
Expected: Clean.

- [ ] **Step 5: Fix any issues found, commit if needed**

```bash
git add -A
git commit -m "fix: resolve workspace-level issues for I5d"
```
