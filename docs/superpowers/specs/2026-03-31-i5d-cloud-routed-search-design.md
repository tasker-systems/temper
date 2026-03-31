# I5d: Cloud-Routed Search — Design Spec

**Date:** 2026-03-31
**Task:** 2026-03-29-i5d-cloud-routed-search
**Mode:** build/medium
**Branch:** jcoletaylor/temper-cloud

## Overview

Rebuild `temper search` as a cloud-routed command. The CLI embeds the query locally via bge-base-en-v1.5 (ONNX), sends the 768-dim vector to the Rust API, which performs pgvector cosine similarity against `kb_current_chunks` with access control via `resources_visible_to()`. Results are cross-referenced against the local manifest for vault path enrichment.

## Architecture

```
temper search "query"
    │
    ├─ temper-embed: embed_text("query") → Vec<f32> (768-dim, bge-base-en-v1.5)
    │   └─ model cached at ~/.cache/huggingface/ via hf-hub
    │
    ├─ temper-client: SearchClient::query(embedding, filters) → POST /api/search
    │
    └─ temper-api: search_service::search()
        ├─ kb_current_chunks <=> query_vector (cosine similarity)
        ├─ JOIN resources_visible_to(profile_id) (access control)
        └─ return ranked results with snippets
```

Key design decision: **client-side embedding**. The server never loads the embedding model. This avoids Vercel cold-start issues (model download, /tmp not persisting) and keeps the API request-response cycle fast. The temper-embed crate is shared between temper-cli and future temper-mcp.

## Component Design

### 1. temper-embed crate

Two feature gates, both default-on but opt-out-able:

- **`extract`**: kreuzberg markdown extraction (moved from temper-cli)
- **`embed`**: bge-base-en-v1.5 via `ort` + `hf-hub` (768-dim normalized vectors)

```toml
[features]
default = ["extract", "embed"]
extract = ["dep:kreuzberg"]
embed = ["dep:ort", "dep:hf-hub", "dep:tokenizers", "dep:ndarray"]
```

Public API:

```rust
// embed feature
pub fn embed_text(text: &str) -> Result<Vec<f32>>;    // single text → 768-dim
pub fn embed_texts(texts: &[&str]) -> Result<Vec<Vec<f32>>>;  // batch

// extract feature (moved from temper-cli)
pub fn extract_to_markdown(path: &Path) -> Result<ExtractionResult>;
```

Model management: `hf-hub` downloads bge-base-en-v1.5 on first call, caches at `~/.cache/huggingface/`. ONNX session created once per process (lazy static or once_cell).

temper-cli's `extract.rs` becomes a thin re-export of temper-embed's extract module, or temper-cli simply depends on temper-embed with the `extract` feature and removes its own kreuzberg dependency.

### 2. Search service (`search_service.rs`)

Complete the existing stub. Core query:

```sql
SELECT
    r.id AS resource_id,
    r.title,
    r.kb_context_id,
    r.doc_type,
    c.content AS snippet,
    c.header_path,
    1 - (c.embedding <=> $1::vector) AS score
FROM kb_current_chunks c
JOIN kb_resources r ON c.resource_id = r.id
WHERE r.id IN (SELECT resource_id FROM resources_visible_to($2))
ORDER BY c.embedding <=> $1::vector
LIMIT $3
```

- `$1` = query embedding vector
- `$2` = authenticated profile_id
- `$3` = limit (default 10, max 50)
- Optional filters: `kb_context_id`, `doc_type` added as WHERE clauses when present

Access control is enforced by the `resources_visible_to()` SQL function, which returns only resources the authenticated user can see.

### 3. Type reconciliation

**Keep** the simpler types from `api.rs`, updated for vector search. **Remove** the speculative types from `search.rs` (SearchMode, SearchRequest, SearchResponse).

Updated types:

```rust
// SearchParams — POST body
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub embedding: Vec<f32>,           // 768-dim query vector
    pub context: Option<Uuid>,         // filter by kb_context
    pub doc_type: Option<String>,      // filter by doc_type
    pub limit: Option<i64>,            // default 10, max 50
}

// SearchResultRow — response item
#[derive(Debug, Serialize)]
pub struct SearchResultRow {
    pub resource_id: Uuid,
    pub title: String,
    pub context: Option<String>,       // kb_context name
    pub doc_type: String,
    pub score: f32,                    // cosine similarity (0-1)
    pub snippet: String,               // chunk content
    pub header_path: Option<String>,   // markdown header context
}
```

### 4. Handler update

Switch `/api/search` from GET with query extraction to POST with JSON body. Auth required (existing middleware). The handler deserializes `SearchParams` from the request body, validates embedding dimension (must be 768), and delegates to the service.

### 5. Client update

`SearchClient::query()` updated to send POST with JSON body. Method signature:

```rust
pub async fn query(
    &self,
    embedding: Vec<f32>,
    context: Option<Uuid>,
    doc_type: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<SearchResultRow>>;
```

### 6. CLI command (`commands/search_cmd.rs`)

Follows sync_cmd pattern (single tokio runtime, client reference):

```
temper search <query> [--context <ctx>] [--doc-type <dt>] [--limit N] [--pretty]
```

Flow:
1. Load config, build client (auth required — clear error if not logged in)
2. Embed query text locally via `temper_embed::embed_text()`
3. Send vector + filters to API via `SearchClient::query()`
4. Cross-reference results against local manifest for `local: bool` and `vault_path`
5. Output JSON (compact by default, `--pretty` for TTY)

Result schema per item:

```json
{
  "resource_id": "uuid",
  "title": "string",
  "context": "string | null",
  "doc_type": "string",
  "score": 0.85,
  "snippet": "relevant text...",
  "header_path": "## Section > ### Subsection",
  "local": true,
  "vault_path": "tasks/temper/some-task.md"
}
```

Actions layer (`actions/search.rs`) handles:
- Manifest cross-reference (resource_id → manifest entry → vault_path)
- Result formatting
- Snippet truncation if needed

### 7. Testing

| Layer | Test Type | What |
|-------|-----------|------|
| temper-embed | Unit | Embedding produces 768-dim normalized vector |
| temper-embed | Unit | Extract markdown/txt passthrough works |
| actions/search | Unit | Manifest cross-reference enrichment |
| actions/search | Unit | Result formatting (JSON output shape) |
| search_service | Integration (`test-db`) | Vector similarity query against seeded chunks |
| search handler | Integration (`test-db`) | POST /api/search returns ranked results with auth |

## Dependencies

- **Completed:** I5b (temper-client + SearchClient), I6-pre (schema + HNSW index), I6a (sync infra + patterns)
- **Not required:** I6 full sync (content can be seeded via `temper add`/`temper import` for testing)

## Out of scope

- Text-based search (tsvector) — future addition
- Graph search (knowledge graph traversal) — future addition
- Web UI search — future addition
- TypeScript search endpoint — not needed, Rust handles it
