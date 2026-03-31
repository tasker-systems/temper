# I5d: Cloud-Routed Search — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild `temper search` as a cloud-routed command — CLI embeds queries locally, sends vectors to Rust API for pgvector similarity search with access control.

**Architecture:** temper-embed crate provides local embedding (bge-base-en-v1.5 via ONNX) and extraction (kreuzberg). CLI embeds query text, sends 768-dim vector to `POST /api/search`. Server performs cosine similarity against `kb_current_chunks` with `resources_visible_to()` access control. Results enriched with local manifest cross-reference.

**Tech Stack:** Rust (ort, hf-hub, tokenizers, ndarray, kreuzberg), Axum, sqlx, pgvector, clap

**Spec:** `docs/superpowers/specs/2026-03-31-i5d-cloud-routed-search-design.md`

---

## File Map

### New files
| File | Responsibility |
|------|---------------|
| `crates/temper-embed/src/embed.rs` | ONNX embedding (bge-base-en-v1.5 via ort + hf-hub) |
| `crates/temper-embed/src/extract.rs` | Extraction (kreuzberg, moved from temper-cli) |
| `crates/temper-cli/src/commands/search_cmd.rs` | CLI command — parse args, embed, call API, format output |
| `crates/temper-cli/src/actions/search.rs` | Business logic — manifest cross-reference, result formatting |

### Modified files
| File | Change |
|------|--------|
| `crates/temper-embed/Cargo.toml` | Add ort, hf-hub, tokenizers, ndarray, kreuzberg deps with feature gates |
| `crates/temper-embed/src/lib.rs` | Re-export embed + extract modules |
| `crates/temper-cli/Cargo.toml` | Replace kreuzberg dep with temper-embed dep |
| `crates/temper-cli/src/extract.rs` | Delegate to temper-embed instead of direct kreuzberg |
| `crates/temper-cli/src/commands/mod.rs` | Add `pub mod search_cmd;` |
| `crates/temper-cli/src/actions/mod.rs` | Add `pub mod search;` |
| `crates/temper-cli/src/cli.rs` | Add `Search` command variant |
| `crates/temper-cli/src/main.rs` | Add `Commands::Search` dispatch |
| `crates/temper-core/src/types/api.rs` | Update SearchParams (embedding vector) and SearchResultRow (richer fields) |
| `crates/temper-core/src/types/search.rs` | Remove speculative types (SearchMode, SearchRequest, etc.) |
| `crates/temper-core/src/types/mod.rs` | Update re-exports |
| `crates/temper-api/src/handlers/search.rs` | Switch from GET+Query to POST+Json |
| `crates/temper-api/src/services/search_service.rs` | Implement pgvector cosine similarity query |
| `crates/temper-client/src/search.rs` | Switch to POST with JSON body |

---

## Task 1: Update core search types

**Files:**
- Modify: `crates/temper-core/src/types/api.rs`
- Modify: `crates/temper-core/src/types/search.rs`
- Modify: `crates/temper-core/src/types/mod.rs`

- [ ] **Step 1: Update SearchParams in api.rs**

Replace the existing `SearchParams` and `SearchResultRow` in `crates/temper-core/src/types/api.rs`:

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
    pub context: Option<String>,
    pub doc_type: String,
    pub score: f32,
    pub snippet: String,
    pub header_path: Option<String>,
}
```

- [ ] **Step 2: Remove speculative search types**

Replace `crates/temper-core/src/types/search.rs` with an empty module (keep the file to avoid churn, just remove the types and tests):

```rust
//! Search types — vector search types live in `api.rs`.
//!
//! This module previously held speculative SearchMode/SearchRequest/SearchResponse
//! types. Those were removed in I5d; all search types now live in api.rs.
```

- [ ] **Step 3: Update mod.rs re-exports**

In `crates/temper-core/src/types/mod.rs`, remove the search re-exports:

```rust
// REMOVE this line:
pub use search::{SearchMode, SearchRequest, SearchResponse, SearchResult};
```

Keep `pub mod search;` — the module stays, just empty of public types.

- [ ] **Step 4: Check compilation**

Run: `cargo check -p temper-core 2>&1 | head -30`
Expected: Compilation errors in downstream crates referencing removed types — that's fine, we fix them in subsequent tasks.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/api.rs crates/temper-core/src/types/search.rs crates/temper-core/src/types/mod.rs
git commit -m "refactor: update search types for vector search, remove speculative types"
```

---

## Task 2: Update search handler and service

**Files:**
- Modify: `crates/temper-api/src/handlers/search.rs`
- Modify: `crates/temper-api/src/services/search_service.rs`
- Modify: `crates/temper-api/src/routes.rs`

- [ ] **Step 1: Update the search handler to POST with JSON body**

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

- [ ] **Step 2: Implement the search service with pgvector query**

Replace `crates/temper-api/src/services/search_service.rs`:

```rust
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

pub use temper_core::types::api::{SearchParams, SearchResultRow};

/// Maximum allowed search results.
const MAX_LIMIT: i64 = 50;
/// Default number of results.
const DEFAULT_LIMIT: i64 = 10;
/// Expected embedding dimension.
const EMBEDDING_DIM: usize = 768;

/// Search resources visible to the given profile using cosine similarity.
pub async fn search(
    pool: &PgPool,
    profile_id: Uuid,
    params: SearchParams,
) -> ApiResult<Vec<SearchResultRow>> {
    if params.embedding.len() != EMBEDDING_DIM {
        return Err(ApiError::BadRequest(format!(
            "embedding must be {EMBEDDING_DIM} dimensions, got {}",
            params.embedding.len()
        )));
    }

    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);

    // Format embedding as pgvector literal: [0.1,0.2,...]
    let embedding_str = format!(
        "[{}]",
        params
            .embedding
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );

    let rows = if let Some(context_id) = params.context {
        if let Some(ref doc_type) = params.doc_type {
            sqlx::query_as::<_, SearchResultRow>(
                "SELECT r.id AS resource_id, r.title, \
                 ctx.name AS context, r.doc_type, \
                 c.content AS snippet, c.header_path, \
                 (1 - (c.embedding <=> $1::vector))::real AS score \
                 FROM kb_current_chunks c \
                 JOIN kb_resources r ON c.resource_id = r.id \
                 LEFT JOIN kb_contexts ctx ON r.kb_context_id = ctx.id \
                 WHERE r.id IN (SELECT resource_id FROM resources_visible_to($2)) \
                 AND r.kb_context_id = $3 AND r.doc_type = $4 \
                 ORDER BY c.embedding <=> $1::vector LIMIT $5",
            )
            .bind(&embedding_str)
            .bind(profile_id)
            .bind(context_id)
            .bind(doc_type)
            .bind(limit)
            .fetch_all(pool)
            .await?
        } else {
            sqlx::query_as::<_, SearchResultRow>(
                "SELECT r.id AS resource_id, r.title, \
                 ctx.name AS context, r.doc_type, \
                 c.content AS snippet, c.header_path, \
                 (1 - (c.embedding <=> $1::vector))::real AS score \
                 FROM kb_current_chunks c \
                 JOIN kb_resources r ON c.resource_id = r.id \
                 LEFT JOIN kb_contexts ctx ON r.kb_context_id = ctx.id \
                 WHERE r.id IN (SELECT resource_id FROM resources_visible_to($2)) \
                 AND r.kb_context_id = $3 \
                 ORDER BY c.embedding <=> $1::vector LIMIT $4",
            )
            .bind(&embedding_str)
            .bind(profile_id)
            .bind(context_id)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
    } else if let Some(ref doc_type) = params.doc_type {
        sqlx::query_as::<_, SearchResultRow>(
            "SELECT r.id AS resource_id, r.title, \
             ctx.name AS context, r.doc_type, \
             c.content AS snippet, c.header_path, \
             (1 - (c.embedding <=> $1::vector))::real AS score \
             FROM kb_current_chunks c \
             JOIN kb_resources r ON c.resource_id = r.id \
             LEFT JOIN kb_contexts ctx ON r.kb_context_id = ctx.id \
             WHERE r.id IN (SELECT resource_id FROM resources_visible_to($2)) \
             AND r.doc_type = $3 \
             ORDER BY c.embedding <=> $1::vector LIMIT $4",
        )
        .bind(&embedding_str)
        .bind(profile_id)
        .bind(doc_type)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, SearchResultRow>(
            "SELECT r.id AS resource_id, r.title, \
             ctx.name AS context, r.doc_type, \
             c.content AS snippet, c.header_path, \
             (1 - (c.embedding <=> $1::vector))::real AS score \
             FROM kb_current_chunks c \
             JOIN kb_resources r ON c.resource_id = r.id \
             LEFT JOIN kb_contexts ctx ON r.kb_context_id = ctx.id \
             WHERE r.id IN (SELECT resource_id FROM resources_visible_to($2)) \
             ORDER BY c.embedding <=> $1::vector LIMIT $3",
        )
        .bind(&embedding_str)
        .bind(profile_id)
        .bind(limit)
        .fetch_all(pool)
        .await?
    };

    Ok(rows)
}
```

Note: `SearchResultRow` needs `sqlx::FromRow` derive. Go back to `api.rs` and add it:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
```

- [ ] **Step 3: Update route from GET to POST**

In `crates/temper-api/src/routes.rs`, change the search route:

```rust
// Change this line:
.route("/api/search", get(handlers::search::search))
// To:
.route("/api/search", post(handlers::search::search))
```

Add `post` to the `use axum::routing::get;` import:

```rust
use axum::routing::{get, post};
```

- [ ] **Step 4: Check compilation**

Run: `cargo check -p temper-api 2>&1 | head -30`
Expected: Clean compilation (or minor fixups needed).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/handlers/search.rs crates/temper-api/src/services/search_service.rs crates/temper-api/src/routes.rs crates/temper-core/src/types/api.rs
git commit -m "feat: implement pgvector search service with POST endpoint and access control"
```

---

## Task 3: Update search client

**Files:**
- Modify: `crates/temper-client/src/search.rs`

- [ ] **Step 1: Update SearchClient to use POST with JSON body**

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

Run: `cargo check -p temper-client 2>&1 | head -30`
Expected: Clean compilation. If `uuid` is not in temper-client's deps, add it.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-client/src/search.rs
git commit -m "feat: update SearchClient to POST with embedding vector"
```

---

## Task 4: Build temper-embed crate — extraction

**Files:**
- Modify: `crates/temper-embed/Cargo.toml`
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

- [ ] **Step 2: Create temper-embed extract module**

Create `crates/temper-embed/src/extract.rs`:

```rust
//! Document extraction module.
//!
//! Provides extraction of files to markdown/plain text. Markdown and plain text
//! files are read directly. All other formats require the `extract` feature,
//! which pulls in the `kreuzberg` crate.

use std::path::Path;

use crate::error::{EmbedError, Result};

/// The result of extracting a file to text.
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    /// The extracted text content (markdown or plain text).
    pub content: String,
    /// The detected or inferred MIME type of the source file.
    pub mime_type: String,
}

/// Extract a file to markdown text.
///
/// Markdown and plain text files are read directly without kreuzberg.
/// All other formats require the `extract` feature to be enabled.
pub fn extract_to_markdown(path: &Path) -> Result<ExtractionResult> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match extension.as_str() {
        "md" | "markdown" => {
            let content = std::fs::read_to_string(path)?;
            Ok(ExtractionResult {
                content,
                mime_type: "text/markdown".to_string(),
            })
        }
        "txt" | "text" => {
            let content = std::fs::read_to_string(path)?;
            Ok(ExtractionResult {
                content,
                mime_type: "text/plain".to_string(),
            })
        }
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
    fn test_extract_markdown_file() {
        let mut file = NamedTempFile::with_suffix(".md").unwrap();
        writeln!(file, "# Hello\n\nThis is markdown.").unwrap();
        let result = extract_to_markdown(file.path()).unwrap();
        assert!(result.content.contains("# Hello"));
        assert_eq!(result.mime_type, "text/markdown");
    }

    #[test]
    fn test_extract_plain_text_file() {
        let mut file = NamedTempFile::with_suffix(".txt").unwrap();
        writeln!(file, "Hello, world!").unwrap();
        let result = extract_to_markdown(file.path()).unwrap();
        assert!(result.content.contains("Hello, world!"));
        assert_eq!(result.mime_type, "text/plain");
    }

    #[test]
    #[cfg(not(feature = "extract"))]
    fn test_non_text_without_extract_feature_returns_error() {
        let file = NamedTempFile::with_suffix(".pdf").unwrap();
        let result = extract_to_markdown(file.path());
        assert!(result.is_err());
    }
}
```

- [ ] **Step 3: Create error module for temper-embed**

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

- [ ] **Step 4: Update temper-embed lib.rs**

Replace `crates/temper-embed/src/lib.rs`:

```rust
//! temper-embed — Embedding and extraction pipeline.
//!
//! Feature-gated:
//! - `extract`: kreuzberg-based document extraction (markdown, PDF, Office, etc.)
//! - `embed`: bge-base-en-v1.5 text embedding via ONNX Runtime (added in Task 5)

pub mod error;

#[cfg(feature = "extract")]
pub mod extract;

#[cfg(not(feature = "extract"))]
pub mod extract;
```

Simplify — extract is always compiled (it has its own internal feature gate for kreuzberg):

```rust
//! temper-embed — Embedding and extraction pipeline.
//!
//! Feature-gated:
//! - `extract`: kreuzberg-based document extraction (markdown, PDF, Office, etc.)
//! - `embed`: bge-base-en-v1.5 text embedding via ONNX Runtime

pub mod error;
pub mod extract;
```

- [ ] **Step 5: Update temper-cli to depend on temper-embed for extraction**

In `crates/temper-cli/Cargo.toml`, replace the kreuzberg dependency:

```toml
# Remove:
kreuzberg = { version = "4.6.3", optional = true, features = ["tokio-runtime"] }

# Add:
temper-embed = { path = "../temper-embed" }
```

Update the `extract` feature:

```toml
[features]
default = ["extract"]
extract = ["temper-embed/extract"]
```

- [ ] **Step 6: Update temper-cli extract.rs to delegate to temper-embed**

Replace `crates/temper-cli/src/extract.rs`:

```rust
//! Document extraction — delegates to temper-embed.

use std::path::Path;

use crate::error::{Result, TemperError};

pub use temper_embed::extract::ExtractionResult;

/// Extract a file to markdown text.
pub fn extract_to_markdown(path: &Path) -> Result<ExtractionResult> {
    temper_embed::extract::extract_to_markdown(path)
        .map_err(|e| TemperError::Extraction(e.to_string()))
}
```

- [ ] **Step 7: Check compilation**

Run: `cargo check -p temper-embed -p temper-cli 2>&1 | head -30`
Expected: Clean compilation.

- [ ] **Step 8: Run temper-cli extract tests**

Run: `cargo test -p temper-embed -- extract 2>&1`
Expected: All extract tests pass.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-embed/ crates/temper-cli/Cargo.toml crates/temper-cli/src/extract.rs
git commit -m "refactor: move extraction to temper-embed crate, temper-cli delegates"
```

---

## Task 5: Build temper-embed crate — embedding

**Files:**
- Modify: `crates/temper-embed/Cargo.toml`
- Create: `crates/temper-embed/src/embed.rs`
- Modify: `crates/temper-embed/src/lib.rs`
- Modify: `crates/temper-embed/src/error.rs`

- [ ] **Step 1: Add embed dependencies to Cargo.toml**

In `crates/temper-embed/Cargo.toml`, add the embed dependencies:

```toml
[dependencies]
temper-core = { path = "../temper-core" }
thiserror = "2"
kreuzberg = { version = "4.6.3", optional = true, features = ["tokio-runtime"] }
ort = { version = "2", optional = true }
hf-hub = { version = "0.4", optional = true }
tokenizers = { version = "0.21", optional = true }
ndarray = { version = "0.16", optional = true }

[features]
default = ["extract", "embed"]
extract = ["dep:kreuzberg"]
embed = ["dep:ort", "dep:hf-hub", "dep:tokenizers", "dep:ndarray"]
```

Note: Check the latest versions on crates.io before implementing. The versions above are approximate — the implementer should use `cargo add` to get current versions, or check what tasker-core uses for `hf-hub` on main branch.

- [ ] **Step 2: Create the embed module**

Create `crates/temper-embed/src/embed.rs`:

```rust
//! Text embedding using BAAI/bge-base-en-v1.5 via ONNX Runtime.
//!
//! The model is downloaded on first use via hf-hub and cached at
//! `~/.cache/huggingface/`. ONNX session is created once per process.

use std::sync::OnceLock;

use ndarray::{Array2, Axis};
use ort::session::Session;
use tokenizers::Tokenizer;

use crate::error::{EmbedError, Result};

/// Embedding dimension for bge-base-en-v1.5.
pub const EMBEDDING_DIM: usize = 768;

/// Model repository on Hugging Face.
const MODEL_REPO: &str = "BAAI/bge-base-en-v1.5";

/// Global session + tokenizer (initialized once).
static MODEL: OnceLock<Result<(Session, Tokenizer)>> = OnceLock::new();

fn load_model() -> Result<&'static (Session, Tokenizer)> {
    let result = MODEL.get_or_init(|| {
        let api = hf_hub::api::sync::Api::new()
            .map_err(|e| EmbedError::Embedding(format!("hf-hub init: {e}")))?;

        let repo = api.model(MODEL_REPO.to_string());

        let model_path = repo
            .get("onnx/model.onnx")
            .map_err(|e| EmbedError::Embedding(format!("download model: {e}")))?;

        let tokenizer_path = repo
            .get("tokenizer.json")
            .map_err(|e| EmbedError::Embedding(format!("download tokenizer: {e}")))?;

        let session = Session::builder()
            .map_err(|e| EmbedError::Embedding(format!("ort session builder: {e}")))?
            .with_intra_threads(1)
            .map_err(|e| EmbedError::Embedding(format!("ort threads: {e}")))?
            .commit_from_file(&model_path)
            .map_err(|e| EmbedError::Embedding(format!("ort load model: {e}")))?;

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| EmbedError::Embedding(format!("load tokenizer: {e}")))?;

        Ok((session, tokenizer))
    });

    match result {
        Ok(model) => Ok(model),
        Err(e) => Err(EmbedError::Embedding(format!("model init failed: {e}"))),
    }
}

/// Embed a single text string into a 768-dim normalized vector.
pub fn embed_text(text: &str) -> Result<Vec<f32>> {
    let results = embed_texts(&[text])?;
    Ok(results.into_iter().next().expect("single input produces single output"))
}

/// Embed multiple texts into 768-dim normalized vectors.
pub fn embed_texts(texts: &[&str]) -> Result<Vec<Vec<f32>>> {
    let (session, tokenizer) = load_model()?;

    let encodings = tokenizer
        .encode_batch(texts.to_vec(), true)
        .map_err(|e| EmbedError::Embedding(format!("tokenize: {e}")))?;

    let batch_size = encodings.len();
    let max_len = encodings.iter().map(|e| e.get_ids().len()).max().unwrap_or(0);

    // Build input tensors
    let mut input_ids = Array2::<i64>::zeros((batch_size, max_len));
    let mut attention_mask = Array2::<i64>::zeros((batch_size, max_len));
    let mut token_type_ids = Array2::<i64>::zeros((batch_size, max_len));

    for (i, encoding) in encodings.iter().enumerate() {
        for (j, &id) in encoding.get_ids().iter().enumerate() {
            input_ids[[i, j]] = id as i64;
        }
        for (j, &mask) in encoding.get_attention_mask().iter().enumerate() {
            attention_mask[[i, j]] = mask as i64;
        }
        for (j, &type_id) in encoding.get_type_ids().iter().enumerate() {
            token_type_ids[[i, j]] = type_id as i64;
        }
    }

    let outputs = session
        .run(ort::inputs![input_ids, attention_mask.clone(), token_type_ids]
            .map_err(|e| EmbedError::Embedding(format!("ort inputs: {e}")))?)
        .map_err(|e| EmbedError::Embedding(format!("ort run: {e}")))?;

    // Extract last_hidden_state (batch_size, seq_len, 768)
    let hidden_state = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|e| EmbedError::Embedding(format!("extract tensor: {e}")))?;

    let hidden_view = hidden_state.view();

    // Mean pooling with attention mask
    let mask_f32 = attention_mask.mapv(|v| v as f32);
    let mut results = Vec::with_capacity(batch_size);

    for i in 0..batch_size {
        let mut embedding = vec![0f32; EMBEDDING_DIM];
        let mut mask_sum = 0f32;

        for j in 0..max_len {
            let m = mask_f32[[i, j]];
            mask_sum += m;
            for k in 0..EMBEDDING_DIM {
                embedding[k] += hidden_view[[i, j, k]] * m;
            }
        }

        if mask_sum > 0.0 {
            for v in &mut embedding {
                *v /= mask_sum;
            }
        }

        // L2 normalize
        let norm: f32 = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut embedding {
                *v /= norm;
            }
        }

        results.push(embedding);
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embed_text_produces_correct_dimension() {
        let vec = embed_text("hello world").unwrap();
        assert_eq!(vec.len(), EMBEDDING_DIM);
    }

    #[test]
    fn test_embed_text_is_normalized() {
        let vec = embed_text("hello world").unwrap();
        let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "norm was {norm}, expected ~1.0");
    }

    #[test]
    fn test_embed_texts_batch() {
        let vecs = embed_texts(&["hello", "world"]).unwrap();
        assert_eq!(vecs.len(), 2);
        assert_eq!(vecs[0].len(), EMBEDDING_DIM);
        assert_eq!(vecs[1].len(), EMBEDDING_DIM);
    }

    #[test]
    fn test_similar_texts_have_higher_similarity() {
        let v1 = embed_text("rust programming language").unwrap();
        let v2 = embed_text("rust cargo build system").unwrap();
        let v3 = embed_text("chocolate cake recipe").unwrap();

        let sim_related: f32 = v1.iter().zip(&v2).map(|(a, b)| a * b).sum();
        let sim_unrelated: f32 = v1.iter().zip(&v3).map(|(a, b)| a * b).sum();

        assert!(
            sim_related > sim_unrelated,
            "related: {sim_related}, unrelated: {sim_unrelated}"
        );
    }
}
```

- [ ] **Step 3: Update lib.rs to include embed module**

Update `crates/temper-embed/src/lib.rs`:

```rust
//! temper-embed — Embedding and extraction pipeline.
//!
//! Feature-gated:
//! - `extract`: kreuzberg-based document extraction (markdown, PDF, Office, etc.)
//! - `embed`: bge-base-en-v1.5 text embedding via ONNX Runtime

pub mod error;
pub mod extract;

#[cfg(feature = "embed")]
pub mod embed;
```

- [ ] **Step 4: Check compilation**

Run: `cargo check -p temper-embed --all-features 2>&1 | head -30`
Expected: Clean compilation. Fix any version mismatches.

- [ ] **Step 5: Run embed tests**

Run: `cargo test -p temper-embed -- embed 2>&1`
Expected: All 4 embed tests pass (first run will download the model — may take a minute).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-embed/
git commit -m "feat: add bge-base-en-v1.5 embedding to temper-embed crate"
```

---

## Task 6: CLI search actions layer

**Files:**
- Create: `crates/temper-cli/src/actions/search.rs`
- Modify: `crates/temper-cli/src/actions/mod.rs`

- [ ] **Step 1: Write tests for manifest cross-reference and result formatting**

Create `crates/temper-cli/src/actions/search.rs`:

```rust
//! Search business logic — manifest cross-reference and result formatting.

use std::collections::HashMap;

use serde::Serialize;
use temper_core::types::api::SearchResultRow;
use temper_core::types::Manifest;
use uuid::Uuid;

/// A search result enriched with local vault information.
#[derive(Debug, Clone, Serialize)]
pub struct EnrichedSearchResult {
    pub resource_id: Uuid,
    pub title: String,
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

/// Enrich API results with local manifest data.
pub fn enrich_results(
    results: Vec<SearchResultRow>,
    manifest: &Manifest,
) -> Vec<EnrichedSearchResult> {
    results
        .into_iter()
        .map(|row| {
            let manifest_entry = manifest.entries.get(&row.resource_id);
            EnrichedSearchResult {
                resource_id: row.resource_id,
                title: row.title,
                context: row.context,
                doc_type: row.doc_type,
                score: row.score,
                snippet: truncate_snippet(&row.snippet, 200),
                header_path: row.header_path,
                local: manifest_entry.is_some(),
                vault_path: manifest_entry.map(|e| e.path.clone()),
            }
        })
        .collect()
}

/// Truncate a snippet to a maximum character length, breaking at word boundaries.
pub fn truncate_snippet(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    // Find last space before max_chars
    let truncated = &text[..max_chars];
    match truncated.rfind(' ') {
        Some(pos) => format!("{}...", &text[..pos]),
        None => format!("{truncated}..."),
    }
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
            context: Some("temper".to_string()),
            doc_type: "task".to_string(),
            score: 0.85,
            snippet: "Some relevant content".to_string(),
            header_path: Some("## Section".to_string()),
        }
    }

    fn sample_manifest() -> Manifest {
        let mut manifest = Manifest::new("device-test".to_string());
        let id = Uuid::nil();
        manifest.entries.insert(
            id,
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
    fn test_enrich_results_marks_local_resources() {
        let local_id = Uuid::nil();
        let remote_id = Uuid::from_u128(1);
        let results = vec![
            sample_result(local_id, "Local Task"),
            sample_result(remote_id, "Remote Task"),
        ];
        let manifest = sample_manifest();

        let enriched = enrich_results(results, &manifest);

        assert_eq!(enriched.len(), 2);
        assert!(enriched[0].local);
        assert_eq!(
            enriched[0].vault_path.as_deref(),
            Some("temper/tasks/test-task.md")
        );
        assert!(!enriched[1].local);
        assert!(enriched[1].vault_path.is_none());
    }

    #[test]
    fn test_enrich_results_empty_manifest() {
        let results = vec![sample_result(Uuid::from_u128(1), "Task")];
        let manifest = Manifest::new("device-test".to_string());

        let enriched = enrich_results(results, &manifest);

        assert_eq!(enriched.len(), 1);
        assert!(!enriched[0].local);
    }

    #[test]
    fn test_enrich_results_empty_results() {
        let manifest = sample_manifest();
        let enriched = enrich_results(vec![], &manifest);
        assert!(enriched.is_empty());
    }

    #[test]
    fn test_truncate_snippet_short_text() {
        let result = truncate_snippet("short text", 200);
        assert_eq!(result, "short text");
    }

    #[test]
    fn test_truncate_snippet_long_text() {
        let long = "word ".repeat(100); // 500 chars
        let result = truncate_snippet(&long, 20);
        assert!(result.len() < 30);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_snippet_no_space() {
        let result = truncate_snippet("aaaaaaaaaaaaaaaaaaaaa", 10);
        assert_eq!(result, "aaaaaaaaaa...");
    }

    #[test]
    fn test_enriched_result_json_shape() {
        let results = vec![sample_result(Uuid::nil(), "Test")];
        let manifest = sample_manifest();
        let enriched = enrich_results(results, &manifest);
        let json = serde_json::to_value(&enriched[0]).unwrap();

        assert!(json.get("resource_id").is_some());
        assert!(json.get("title").is_some());
        assert!(json.get("score").is_some());
        assert!(json.get("local").is_some());
        assert!(json.get("vault_path").is_some());
    }
}
```

- [ ] **Step 2: Register module**

In `crates/temper-cli/src/actions/mod.rs`, add:

```rust
pub mod search;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p temper-cli -- actions::search 2>&1`
Expected: All 6 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/actions/search.rs crates/temper-cli/src/actions/mod.rs
git commit -m "feat: add search actions layer with manifest cross-reference"
```

---

## Task 7: CLI search command

**Files:**
- Create: `crates/temper-cli/src/commands/search_cmd.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs`
- Modify: `crates/temper-cli/src/cli.rs`
- Modify: `crates/temper-cli/src/main.rs`

- [ ] **Step 1: Create the search command module**

Create `crates/temper-cli/src/commands/search_cmd.rs`:

```rust
//! `temper search` — cloud-routed semantic search.
//!
//! Embeds the query locally via temper-embed, sends the vector to the
//! cloud API, and enriches results with local manifest data.

use crate::actions::search as search_actions;
use crate::error::{Result, TemperError};
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
    let device_id = crate::config::load_device_id().ok_or_else(|| {
        TemperError::Config("not authenticated — run `temper auth login` first".into())
    })?;

    let manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

    // Embed the query locally
    #[cfg(feature = "embed")]
    let embedding = temper_embed::embed::embed_text(query)
        .map_err(|e| TemperError::Extraction(format!("embedding failed: {e}")))?;

    #[cfg(not(feature = "embed"))]
    return Err(TemperError::Config(
        "search requires the 'embed' feature — rebuild with --features embed".into(),
    ));

    // Context-to-UUID resolution is not yet wired (no cloud_id on context config).
    // Pass None for now — search returns all accessible results.
    // TODO: Wire context filtering once context registration includes cloud UUIDs.
    let context_id: Option<uuid::Uuid> = None;
    if context.is_some() {
        crate::output::warning("--context filtering is not yet implemented; showing all results");
    }

    // Call the API
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;
    let client =
        temper_client::config::build_client().map_err(|e| TemperError::Api(e.to_string()))?;

    let results = rt.block_on(async {
        client
            .search()
            .query(
                embedding,
                context_id,
                doc_type.map(String::from),
                limit,
            )
            .await
    })
    .map_err(|e| TemperError::Api(e.to_string()))?;

    // Enrich with manifest data
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
        let json = serde_json::to_string_pretty(&enriched)
            .map_err(|e| TemperError::Api(format!("json serialize: {e}")))?;
        crate::output::plain(json);
    } else {
        for (i, result) in enriched.iter().enumerate() {
            let local_marker = if result.local { " [local]" } else { "" };
            crate::output::plain(format!(
                "{}. {} (score: {:.2}){local_marker}",
                i + 1,
                result.title,
                result.score
            ));
            if let Some(ref header) = result.header_path {
                crate::output::plain(format!("   {header}"));
            }
            crate::output::plain(format!("   {}", result.snippet));
            if let Some(ref path) = result.vault_path {
                crate::output::plain(format!("   vault: {path}"));
            }
            crate::output::plain("");
        }
    }

    Ok(())
}
```

- [ ] **Step 2: Register the command module**

In `crates/temper-cli/src/commands/mod.rs`, add:

```rust
pub mod search_cmd;
```

- [ ] **Step 3: Add Search variant to CLI**

In `crates/temper-cli/src/cli.rs`, add the `Search` variant to `Commands` enum (add it after the `Sync` variant):

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

- [ ] **Step 4: Add Search dispatch to main.rs**

In `crates/temper-cli/src/main.rs`, add the dispatch case (add it after the `Commands::Sync` match arm):

```rust
        Commands::Search {
            query,
            context,
            doc_type,
            limit,
            format,
        } => commands::search_cmd::run(&query, context.as_deref(), doc_type.as_deref(), limit, &format),
```

- [ ] **Step 5: Update temper-cli Cargo.toml with embed feature**

In `crates/temper-cli/Cargo.toml`, update features:

```toml
[features]
default = ["extract", "embed"]
extract = ["temper-embed/extract"]
embed = ["temper-embed/embed"]
```

- [ ] **Step 6: Check compilation**

Run: `cargo check -p temper-cli --all-features 2>&1 | head -30`
Expected: Clean compilation.

Note: The `context_id` resolution depends on `load_temper_config` returning contexts with a `cloud_id` field. If this doesn't exist yet, use `None` as a placeholder and note it for the implementer. The search will work without context filtering — it just won't filter by context until that mapping exists.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/commands/search_cmd.rs crates/temper-cli/src/commands/mod.rs crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs crates/temper-cli/Cargo.toml
git commit -m "feat: add temper search CLI command with local embedding"
```

---

## Task 8: Full workspace verification

**Files:** None (verification only)

- [ ] **Step 1: Workspace compilation**

Run: `cargo check --workspace 2>&1 | tail -5`
Expected: Clean compilation across all crates.

- [ ] **Step 2: Clippy**

Run: `cargo clippy --workspace --all-features 2>&1 | tail -10`
Expected: No warnings.

- [ ] **Step 3: All Rust tests**

Run: `cargo test --workspace 2>&1 | tail -20`
Expected: All tests pass (embed tests may be slow on first run due to model download).

- [ ] **Step 4: TypeScript checks**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && tsc --noEmit && tsc --noEmit --project tsconfig.api.json`
Expected: No errors.

- [ ] **Step 5: Fix any issues found**

Address compilation errors, test failures, or clippy warnings. Common issues:
- Missing `uuid` in temper-client deps
- `FromRow` derive needs `sqlx` feature in temper-core
- Feature gate interactions between extract/embed

- [ ] **Step 6: Final commit (if fixes needed)**

```bash
git add -A
git commit -m "fix: resolve workspace-level compilation issues for I5d"
```
