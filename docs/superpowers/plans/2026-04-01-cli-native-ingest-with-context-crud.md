# CLI-Native Ingest with Context CRUD — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the entire ingest pipeline to Rust/Axum, add context CRUD, migrate sync endpoints, and validate I5e end-to-end.

**Architecture:** Bottom-up pipeline build. Rename `temper-embed` → `temper-ingest` and add chunking. Add shared types to `temper-core`. Build context + ingest + sync services/handlers in `temper-api`. Update `temper-client` and CLI. Delete TypeScript ingest/sync code.

**Tech Stack:** Rust (Axum, sqlx, serde, rmp-serde, base64, sha2), existing temper workspace crates.

**Spec:** `docs/superpowers/specs/2026-04-01-cli-native-ingest-with-context-crud-design.md`

---

## File Structure

### New files
| File | Responsibility |
|------|---------------|
| `crates/temper-ingest/src/chunk.rs` | Markdown chunker — pure function, no deps |
| `crates/temper-core/src/types/context.rs` | `ContextRow` type |
| `crates/temper-core/src/types/ingest.rs` | Replace `IngestRequest` with `IngestPayload` + `PackedChunk` |
| `crates/temper-api/src/handlers/contexts.rs` | Context CRUD handlers |
| `crates/temper-api/src/services/context_service.rs` | Context queries using `contexts_visible_to()` |
| `crates/temper-api/src/handlers/ingest.rs` | Ingest handler — accept full payload |
| `crates/temper-api/src/services/ingest_service.rs` | Ingest pipeline — resolve, dedup, insert resource + chunks |
| `crates/temper-api/src/handlers/sync.rs` | Sync status + complete handlers |
| `crates/temper-api/src/services/sync_service.rs` | Sync diff computation + round completion |
| `crates/temper-client/src/contexts.rs` | Context sub-client |

### Modified files
| File | Change |
|------|--------|
| `crates/temper-ingest/Cargo.toml` | Renamed from temper-embed, add `sha2` dep |
| `crates/temper-ingest/src/lib.rs` | Add `pub mod chunk;` |
| `crates/temper-core/src/types/mod.rs` | Add `pub mod context;` + re-exports |
| `crates/temper-core/Cargo.toml` | Add `rmp-serde`, `base64` deps |
| `crates/temper-api/src/handlers/mod.rs` | Add `pub mod contexts;`, `pub mod ingest;`, `pub mod sync;` |
| `crates/temper-api/src/services/mod.rs` | Add `pub mod context_service;`, `pub mod ingest_service;`, `pub mod sync_service;` |
| `crates/temper-api/src/routes.rs` | Register new routes |
| `crates/temper-api/Cargo.toml` | Add `rmp-serde`, `base64` deps |
| `crates/temper-client/src/lib.rs` | Add `pub mod contexts;` + accessor |
| `crates/temper-client/src/ingest.rs` | Replace multipart with JSON POST |
| `crates/temper-client/Cargo.toml` | Add `rmp-serde`, `base64` deps |
| `crates/temper-cli/src/cli.rs` | Add `Create` variant to `ContextAction` |
| `crates/temper-cli/src/commands/context_cmd.rs` | Add `create()` function |
| `crates/temper-cli/src/actions/ingest.rs` | New pipeline: extract→chunk→embed→pack→upload |
| `crates/temper-cli/Cargo.toml` | Update dep name `temper-embed` → `temper-ingest`, add `rmp-serde`, `base64` |
| `Cargo.toml` (workspace root) | Update member path |

### Deleted files
| File | Reason |
|------|--------|
| `api/ingest.ts` | Replaced by Axum endpoint |
| `api/ingest/[id].ts` (if exists) | Replaced by Axum endpoint |
| `api/sync/status.ts` | Replaced by Axum endpoint |
| `api/sync/complete.ts` | Replaced by Axum endpoint |

---

## Task 1: Rename `temper-embed` → `temper-ingest`

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Rename: `crates/temper-embed/` → `crates/temper-ingest/`
- Modify: `crates/temper-ingest/Cargo.toml`
- Modify: `crates/temper-cli/Cargo.toml`
- Modify: all files importing `temper_embed`

- [ ] **Step 1: Rename the crate directory**

```bash
mv crates/temper-embed crates/temper-ingest
```

- [ ] **Step 2: Update the crate's own Cargo.toml**

In `crates/temper-ingest/Cargo.toml`, change:
```toml
[package]
name = "temper-ingest"
```

- [ ] **Step 3: Update workspace root Cargo.toml**

The workspace uses `members = ["crates/*"]` so no member path change needed. But the root `[dependencies]` section references `temper-embed` — update if present. Check for any workspace dependency alias.

- [ ] **Step 4: Update temper-cli Cargo.toml dependency**

In `crates/temper-cli/Cargo.toml`, change:
```toml
temper-ingest = { path = "../temper-ingest", features = ["extract", "embed"] }
```
(was `temper-embed`)

- [ ] **Step 5: Update all Rust imports**

Find and replace across the workspace:
```bash
# Find all files referencing temper_embed
grep -r "temper_embed" crates/ --include="*.rs" -l
```

In each file, replace `temper_embed` → `temper_ingest` (both `use` statements and any doc comments).

Key files likely affected:
- `crates/temper-cli/src/actions/ingest.rs` — `use temper_ingest::extract::extract_to_markdown;`
- `crates/temper-cli/src/commands/search_cmd.rs` — `use temper_ingest::embed::embed_text;`
- Any test files referencing the crate

- [ ] **Step 6: Verify the build compiles**

```bash
cargo check --workspace --all-features
```
Expected: clean compile, zero errors.

- [ ] **Step 7: Run existing tests**

```bash
cargo test -p temper-ingest
```
Expected: all existing extract/embed tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-ingest/ Cargo.toml Cargo.lock crates/temper-cli/
git commit -m "refactor: rename temper-embed → temper-ingest

The crate handles the full content ingestion lifecycle (extract → chunk →
embed), not just embedding. Rename while the surface area is small."
```

---

## Task 2: Add markdown chunker to `temper-ingest`

**Files:**
- Create: `crates/temper-ingest/src/chunk.rs`
- Modify: `crates/temper-ingest/src/lib.rs`
- Modify: `crates/temper-ingest/Cargo.toml` (add `sha2` dep)

- [ ] **Step 1: Add sha2 dependency**

In `crates/temper-ingest/Cargo.toml`, add to `[dependencies]`:
```toml
sha2 = "0.10"
```

- [ ] **Step 2: Write the failing tests**

Create `crates/temper-ingest/src/chunk.rs`:

```rust
//! Markdown-aware text chunker.
//!
//! Splits markdown content on headings (`# … ######`), maintains a header
//! breadcrumb stack, and computes SHA-256 content hashes per chunk.
//! Port of `packages/temper-cloud/src/processing/chunk.ts`.

use sha2::{Digest, Sha256};

/// A single chunk of markdown content with its heading context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkData {
    /// Sequential index (0-based).
    pub chunk_index: u32,
    /// Heading breadcrumb, e.g. "Design > API > Auth". Empty if no headings.
    pub header_path: String,
    /// The chunk's text content (trimmed).
    pub content: String,
    /// SHA-256 hex digest of `content`.
    pub content_hash: String,
}

/// Split markdown text into chunks by headings.
///
/// Each heading starts a new chunk. Content before the first heading (if any)
/// becomes chunk 0 with an empty `header_path`. Chunks with empty content
/// after trimming are skipped.
pub fn chunk_markdown(text: &str) -> Vec<ChunkData> {
    todo!()
}

fn sha256_hex(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn expected_hash(content: &str) -> String {
        let mut h = Sha256::new();
        h.update(content.as_bytes());
        format!("{:x}", h.finalize())
    }

    #[test]
    fn chunks_simple_document_by_headers() {
        let text = "# Title\n\nIntroduction paragraph.\n\n## Section One\n\nContent of section one.\n\n## Section Two\n\nContent of section two.\n";
        let chunks = chunk_markdown(text);

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].header_path, "Title");
        assert!(chunks[0].content.contains("Introduction paragraph."));
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!(chunks[1].header_path, "Title > Section One");
        assert!(chunks[1].content.contains("Content of section one."));
        assert_eq!(chunks[1].chunk_index, 1);
        assert_eq!(chunks[2].header_path, "Title > Section Two");
        assert_eq!(chunks[2].chunk_index, 2);
    }

    #[test]
    fn produces_deterministic_content_hash() {
        let text = "# Hello\n\nWorld";
        let chunks1 = chunk_markdown(text);
        let chunks2 = chunk_markdown(text);

        assert_eq!(chunks1[0].content_hash, chunks2[0].content_hash);
        assert_eq!(chunks1[0].content_hash, expected_hash(&chunks1[0].content));
    }

    #[test]
    fn handles_text_with_no_headers_as_single_chunk() {
        let text = "Just plain text without any headers.";
        let chunks = chunk_markdown(text);

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].header_path, "");
        assert_eq!(chunks[0].chunk_index, 0);
    }

    #[test]
    fn handles_empty_text() {
        let chunks = chunk_markdown("");
        assert_eq!(chunks.len(), 0);
    }

    #[test]
    fn handles_nested_headers() {
        let text = "# Top\n## Mid\n### Deep\n\nDeep content.\n";
        let chunks = chunk_markdown(text);
        let deep_chunk = chunks.iter().find(|c| c.content.contains("Deep content"));
        assert_eq!(deep_chunk.unwrap().header_path, "Top > Mid > Deep");
    }

    #[test]
    fn skips_empty_chunks() {
        let text = "# First\n\n# Second\n\nActual content.\n";
        let chunks = chunk_markdown(text);
        // First heading has no content, should be skipped
        // Only "Second" chunk with content should remain
        assert!(chunks.iter().all(|c| !c.content.trim().is_empty()));
    }

    #[test]
    fn header_stack_pops_on_same_or_higher_level() {
        let text = "# A\n## B\n### C\n\nC content.\n\n## D\n\nD content.\n";
        let chunks = chunk_markdown(text);
        let d_chunk = chunks.iter().find(|c| c.content.contains("D content"));
        // When ## D appears, ### C and ## B should be popped, leaving just # A
        assert_eq!(d_chunk.unwrap().header_path, "A > D");
    }
}
```

- [ ] **Step 3: Register the module**

In `crates/temper-ingest/src/lib.rs`, add:
```rust
pub mod chunk;
```

- [ ] **Step 4: Run tests to verify they fail**

```bash
cargo test -p temper-ingest chunk
```
Expected: FAIL — `todo!()` panics.

- [ ] **Step 5: Implement chunk_markdown**

Replace the `todo!()` in `chunk_markdown` with:

```rust
pub fn chunk_markdown(text: &str) -> Vec<ChunkData> {
    let heading_re = regex::Regex::new(r"^(#{1,6})\s+(.+)$").unwrap();
    let mut chunks = Vec::new();
    let mut header_stack: Vec<(usize, String)> = Vec::new(); // (level, text)
    let mut current_content = String::new();
    let mut chunk_index: u32 = 0;

    let flush = |content: &mut String,
                 header_stack: &[(usize, String)],
                 chunks: &mut Vec<ChunkData>,
                 chunk_index: &mut u32| {
        let trimmed = content.trim().to_owned();
        if !trimmed.is_empty() {
            let header_path = header_stack
                .iter()
                .map(|(_, t)| t.as_str())
                .collect::<Vec<_>>()
                .join(" > ");
            let content_hash = sha256_hex(&trimmed);
            chunks.push(ChunkData {
                chunk_index: *chunk_index,
                header_path,
                content: trimmed,
                content_hash,
            });
            *chunk_index += 1;
        }
        content.clear();
    };

    for line in text.lines() {
        if let Some(caps) = heading_re.captures(line) {
            // Flush current content before starting new section
            flush(
                &mut current_content,
                &header_stack,
                &mut chunks,
                &mut chunk_index,
            );

            let level = caps[1].len();
            let heading_text = caps[2].trim().to_owned();

            // Pop headers at same or deeper level
            while header_stack
                .last()
                .map_or(false, |(l, _)| *l >= level)
            {
                header_stack.pop();
            }
            header_stack.push((level, heading_text));
        } else {
            current_content.push_str(line);
            current_content.push('\n');
        }
    }

    // Flush remaining content
    flush(
        &mut current_content,
        &header_stack,
        &mut chunks,
        &mut chunk_index,
    );

    chunks
}
```

Add `regex` to `crates/temper-ingest/Cargo.toml`:
```toml
regex = "1"
```

- [ ] **Step 6: Run tests to verify they pass**

```bash
cargo test -p temper-ingest chunk
```
Expected: all 7 tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-ingest/
git commit -m "feat(ingest): add markdown chunker with header-stack breadcrumbs

Port of processing/chunk.ts. Pure function that splits markdown on
headings, maintains hierarchical header context, and SHA-256 hashes
each chunk's content. Test parity with TypeScript chunk.test.ts."
```

---

## Task 3: Add shared types to `temper-core`

**Files:**
- Create: `crates/temper-core/src/types/context.rs`
- Modify: `crates/temper-core/src/types/ingest.rs`
- Modify: `crates/temper-core/src/types/mod.rs`
- Modify: `crates/temper-core/Cargo.toml`

- [ ] **Step 1: Add dependencies to temper-core**

In `crates/temper-core/Cargo.toml`, add:
```toml
rmp-serde = "1"
base64 = "0.22"
```

- [ ] **Step 2: Create ContextRow type**

Create `crates/temper-core/src/types/context.rs`:

```rust
//! Context types — API request/response types for context CRUD.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Response row for context endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ContextRow {
    pub id: Uuid,
    pub name: String,
    pub kb_owner_table: String,
    pub kb_owner_id: Uuid,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

#[cfg(feature = "db")]
impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for ContextRow {
    fn from_row(row: &sqlx::postgres::PgRow) -> sqlx::Result<Self> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            kb_owner_table: row.try_get("kb_owner_table")?,
            kb_owner_id: row.try_get("kb_owner_id")?,
            created: row.try_get("created")?,
            updated: row.try_get("updated")?,
        })
    }
}

/// Request body for POST /api/contexts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ContextCreateRequest {
    pub name: String,
}
```

- [ ] **Step 3: Replace IngestRequest with IngestPayload + PackedChunk**

Replace the contents of `crates/temper-core/src/types/ingest.rs` with:

```rust
//! Ingest API types — wire format for CLI → Axum ingest pipeline.

use serde::{Deserialize, Serialize};

/// Wire payload for POST /api/ingest — resource + pre-processed chunks.
///
/// The CLI performs extract → chunk → embed locally and sends everything
/// in a single request. `chunks_packed` is a base64-encoded MessagePack
/// blob containing `Vec<PackedChunk>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct IngestPayload {
    pub title: String,
    pub origin_uri: String,
    pub context_name: String,
    pub doc_type_name: String,
    /// "added" or "imported"
    pub resource_mode: String,
    /// "sha256:<hex>"
    pub content_hash: String,
    pub slug: String,
    pub mimetype: String,
    /// Full extracted markdown content.
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    /// Base64-encoded MessagePack of `Vec<PackedChunk>`.
    pub chunks_packed: String,
}

/// A single chunk with its embedding, serialized via MessagePack inside
/// `IngestPayload::chunks_packed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackedChunk {
    pub chunk_index: u32,
    pub header_path: String,
    pub content: String,
    pub content_hash: String,
    /// 768-dimensional embedding vector.
    pub embedding: Vec<f32>,
}

/// Encode chunks into the `chunks_packed` wire format (MessagePack → base64).
pub fn pack_chunks(chunks: &[PackedChunk]) -> Result<String, PackError> {
    use base64::Engine;
    let bytes = rmp_serde::to_vec(chunks).map_err(PackError::Serialize)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

/// Decode `chunks_packed` from wire format (base64 → MessagePack).
pub fn unpack_chunks(packed: &str) -> Result<Vec<PackedChunk>, PackError> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(packed)
        .map_err(PackError::Base64)?;
    rmp_serde::from_slice(&bytes).map_err(PackError::Deserialize)
}

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("MessagePack serialization failed: {0}")]
    Serialize(rmp_serde::encode::Error),
    #[error("MessagePack deserialization failed: {0}")]
    Deserialize(rmp_serde::decode::Error),
    #[error("Base64 decode failed: {0}")]
    Base64(base64::DecodeError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_chunks() -> Vec<PackedChunk> {
        vec![
            PackedChunk {
                chunk_index: 0,
                header_path: "Title".to_owned(),
                content: "Hello world".to_owned(),
                content_hash: "abc123".to_owned(),
                embedding: vec![0.1; 768],
            },
            PackedChunk {
                chunk_index: 1,
                header_path: "Title > Section".to_owned(),
                content: "Section content".to_owned(),
                content_hash: "def456".to_owned(),
                embedding: vec![0.2; 768],
            },
        ]
    }

    #[test]
    fn pack_unpack_roundtrip() {
        let chunks = sample_chunks();
        let packed = pack_chunks(&chunks).unwrap();
        let unpacked = unpack_chunks(&packed).unwrap();

        assert_eq!(unpacked.len(), 2);
        assert_eq!(unpacked[0].chunk_index, 0);
        assert_eq!(unpacked[0].header_path, "Title");
        assert_eq!(unpacked[0].content, "Hello world");
        assert_eq!(unpacked[0].embedding.len(), 768);
        assert_eq!(unpacked[1].chunk_index, 1);
        assert_eq!(unpacked[1].header_path, "Title > Section");
    }

    #[test]
    fn pack_produces_valid_base64() {
        let packed = pack_chunks(&sample_chunks()).unwrap();
        // Should be valid base64 — no error on decode
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(&packed)
            .unwrap();
    }

    #[test]
    fn unpack_invalid_base64_errors() {
        let result = unpack_chunks("not-valid-base64!!!");
        assert!(result.is_err());
    }

    #[test]
    fn payload_serialization_roundtrip() {
        let payload = IngestPayload {
            title: "Test".to_owned(),
            origin_uri: "kb://ctx/task/test".to_owned(),
            context_name: "ctx".to_owned(),
            doc_type_name: "task".to_owned(),
            resource_mode: "imported".to_owned(),
            content_hash: "sha256:abc".to_owned(),
            slug: "test".to_owned(),
            mimetype: "text/markdown".to_owned(),
            content: "# Test".to_owned(),
            metadata: None,
            chunks_packed: pack_chunks(&sample_chunks()).unwrap(),
        };

        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: IngestPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.title, "Test");
        assert_eq!(deserialized.context_name, "ctx");

        // Verify embedded chunks survive the full roundtrip
        let chunks = unpack_chunks(&deserialized.chunks_packed).unwrap();
        assert_eq!(chunks.len(), 2);
    }
}
```

- [ ] **Step 4: Update mod.rs re-exports**

In `crates/temper-core/src/types/mod.rs`, add:

```rust
pub mod context;
```

And add to the re-exports:
```rust
pub use context::{ContextCreateRequest, ContextRow};
pub use ingest::{pack_chunks, unpack_chunks, IngestPayload, PackError, PackedChunk};
```

Remove the old `pub use ingest::IngestRequest;` line.

- [ ] **Step 5: Fix compilation errors from IngestRequest removal**

The old `IngestRequest` is used in:
- `crates/temper-client/src/ingest.rs` — will be replaced in Task 8
- `crates/temper-cli/src/actions/ingest.rs` — will be replaced in Task 9

For now, keep the old `IngestRequest` struct in `ingest.rs` alongside the new types, marked as deprecated:

```rust
#[deprecated(note = "Use IngestPayload — the multipart ingest path is removed")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestRequest {
    // ... keep existing fields unchanged
}
```

Also keep the old re-export in `mod.rs`:
```rust
#[allow(deprecated)]
pub use ingest::IngestRequest;
```

- [ ] **Step 6: Verify compilation and tests**

```bash
cargo check --workspace --all-features
cargo test -p temper-core
```
Expected: compiles with deprecation warnings, all tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-core/
git commit -m "feat(core): add IngestPayload, PackedChunk, ContextRow types

New wire format for CLI-native ingest: IngestPayload with MessagePack-
encoded chunks (pack_chunks/unpack_chunks). ContextRow for context CRUD.
Old IngestRequest deprecated — removed when client/CLI are updated."
```

---

## Task 4: Context service in `temper-api`

**Files:**
- Create: `crates/temper-api/src/services/context_service.rs`
- Modify: `crates/temper-api/src/services/mod.rs`

- [ ] **Step 1: Create context_service.rs**

```rust
//! Context CRUD service — queries scoped through `contexts_visible_to()`.
//!
//! Future scope (I5h): rename, delete (zero-resource guard), resource
//! relocation. See tasks/temper/2026-04-01-i5h-context-crud-lifecycle-
//! rename-delete-relocate.md.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

pub use temper_core::types::context::{ContextCreateRequest, ContextRow};

/// List all contexts visible to the profile (owned + team-shared).
pub async fn list_visible(pool: &PgPool, profile_id: Uuid) -> ApiResult<Vec<ContextRow>> {
    let rows = sqlx::query_as::<_, ContextRow>(
        r#"
        SELECT id, name, kb_owner_table, kb_owner_id, created, updated
          FROM contexts_visible_to($1)
         ORDER BY name
        "#,
    )
    .bind(profile_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Get a single context by ID, scoped to profile visibility.
pub async fn get_visible(
    pool: &PgPool,
    profile_id: Uuid,
    context_id: Uuid,
) -> ApiResult<ContextRow> {
    sqlx::query_as::<_, ContextRow>(
        r#"
        SELECT id, name, kb_owner_table, kb_owner_id, created, updated
          FROM contexts_visible_to($1)
         WHERE id = $2
        "#,
    )
    .bind(profile_id)
    .bind(context_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

/// Resolve a context by name within the profile's visible contexts.
pub async fn resolve_by_name(
    pool: &PgPool,
    profile_id: Uuid,
    name: &str,
) -> ApiResult<ContextRow> {
    sqlx::query_as::<_, ContextRow>(
        r#"
        SELECT id, name, kb_owner_table, kb_owner_id, created, updated
          FROM contexts_visible_to($1)
         WHERE name = $2
        "#,
    )
    .bind(profile_id)
    .bind(name)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

/// Create a new context owned by the profile.
///
/// Returns 409 Conflict if a context with the same name already exists
/// for this owner (enforced by `kb_contexts_owner_name_unique` constraint).
pub async fn create(
    pool: &PgPool,
    profile_id: Uuid,
    name: &str,
) -> ApiResult<ContextRow> {
    let id = Uuid::now_v7();
    let row = sqlx::query_as::<_, ContextRow>(
        r#"
        INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id)
        VALUES ($1, $2, 'kb_profiles', $3)
        RETURNING id, name, kb_owner_table, kb_owner_id, created, updated
        "#,
    )
    .bind(id)
    .bind(name)
    .bind(profile_id)
    .fetch_one(pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
            ApiError::Conflict(format!("context '{name}' already exists"))
        }
        _ => e.into(),
    })?;

    Ok(row)
}
```

- [ ] **Step 2: Register in services/mod.rs**

Add to `crates/temper-api/src/services/mod.rs`:
```rust
pub mod context_service;
```

- [ ] **Step 3: Verify compilation**

```bash
cargo check -p temper-api
```

Check that `ApiError::Conflict` exists. If not, add the variant to `crates/temper-api/src/error.rs`:
```rust
Conflict(String),
```
with the HTTP response mapping:
```rust
ApiError::Conflict(msg) => (StatusCode::CONFLICT, msg),
```

Expected: compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/services/
git commit -m "feat(api): add context_service with list, get, create, resolve_by_name

Uses contexts_visible_to() SQL function for all queries. Create uses
Uuid::now_v7() and returns 409 on duplicate name per owner."
```

---

## Task 5: Context handlers and routes

**Files:**
- Create: `crates/temper-api/src/handlers/contexts.rs`
- Modify: `crates/temper-api/src/handlers/mod.rs`
- Modify: `crates/temper-api/src/routes.rs`

- [ ] **Step 1: Create context handler**

Create `crates/temper-api/src/handlers/contexts.rs`:

```rust
use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::error::{ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::context_service::{self, ContextCreateRequest, ContextRow};
use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/api/contexts",
    tag = "Contexts",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "List of visible contexts", body = Vec<ContextRow>),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Vec<ContextRow>>> {
    context_service::list_visible(&state.pool, auth.0.profile.id)
        .await
        .map(Json)
}

#[utoipa::path(
    post,
    path = "/api/contexts",
    tag = "Contexts",
    security(("bearer_auth" = [])),
    request_body = ContextCreateRequest,
    responses(
        (status = 201, description = "Context created", body = ContextRow),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 409, description = "Context name already exists", body = ErrorBody),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ContextCreateRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<ContextRow>)> {
    let row = context_service::create(&state.pool, auth.0.profile.id, &body.name).await?;
    Ok((axum::http::StatusCode::CREATED, Json(row)))
}

#[utoipa::path(
    get,
    path = "/api/contexts/{id}",
    tag = "Contexts",
    params(("id" = Uuid, Path, description = "Context ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Context details", body = ContextRow),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
    )
)]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(context_id): Path<Uuid>,
) -> ApiResult<Json<ContextRow>> {
    context_service::get_visible(&state.pool, auth.0.profile.id, context_id)
        .await
        .map(Json)
}
```

- [ ] **Step 2: Register handler module**

In `crates/temper-api/src/handlers/mod.rs`, add:
```rust
pub mod contexts;
```

- [ ] **Step 3: Add routes**

In `crates/temper-api/src/routes.rs`, add to the `protected` router:
```rust
.route(
    "/api/contexts",
    get(handlers::contexts::list).post(handlers::contexts::create),
)
.route(
    "/api/contexts/{id}",
    get(handlers::contexts::get),
)
```

- [ ] **Step 4: Verify compilation**

```bash
cargo check -p temper-api
```
Expected: compiles cleanly.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/handlers/ crates/temper-api/src/routes.rs
git commit -m "feat(api): add context CRUD handlers (list, create, get)

GET /api/contexts, POST /api/contexts, GET /api/contexts/:id.
All scoped through contexts_visible_to() SQL function."
```

---

## Task 6: Ingest service in `temper-api`

**Files:**
- Create: `crates/temper-api/src/services/ingest_service.rs`
- Modify: `crates/temper-api/src/services/mod.rs`
- Modify: `crates/temper-api/Cargo.toml`

- [ ] **Step 1: Add dependencies to temper-api**

In `crates/temper-api/Cargo.toml`, add:
```toml
rmp-serde = "1"
base64 = "0.22"
```

Verify `temper-core` is already a dependency (it should be).

- [ ] **Step 2: Create ingest_service.rs**

```rust
//! Ingest service — accepts a fully-processed payload (content + chunks +
//! embeddings) and writes resource + chunks to the database in a single
//! transaction.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::context_service;

use temper_core::types::ingest::{unpack_chunks, IngestPayload};
use temper_core::types::resource::ResourceRow;

/// Resolve doc_type name to UUID from kb_doc_types.
async fn resolve_doc_type(pool: &PgPool, name: &str) -> ApiResult<Uuid> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM kb_doc_types WHERE name = $1",
    )
    .bind(name)
    .fetch_optional(pool)
    .await?;

    row.map(|(id,)| id)
        .ok_or_else(|| ApiError::BadRequest(format!("unknown doc_type: '{name}'")))
}

/// Check for content-hash dedup — returns existing resource if hash matches.
async fn find_by_content_hash(
    pool: &PgPool,
    profile_id: Uuid,
    content_hash: &str,
) -> ApiResult<Option<ResourceRow>> {
    let row = sqlx::query_as::<_, ResourceRow>(
        r#"
        WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
        SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.origin_uri, r.title,
               r.slug, r.content_hash, r.mimetype,
               r.originator_profile_id, r.owner_profile_id, r.is_active,
               r.created, r.updated
          FROM kb_resources r
          JOIN visible v ON v.resource_id = r.id
         WHERE r.content_hash = $2
           AND r.is_active = true
         LIMIT 1
        "#,
    )
    .bind(profile_id)
    .bind(content_hash)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Process a full ingest payload: resolve names, dedup, insert resource + chunks.
pub async fn ingest(
    pool: &PgPool,
    profile_id: Uuid,
    payload: IngestPayload,
) -> ApiResult<ResourceRow> {
    // 1. Resolve context
    let context = context_service::resolve_by_name(pool, profile_id, &payload.context_name)
        .await
        .map_err(|_| {
            ApiError::NotFound
        })?;

    // 2. Resolve doc_type
    let doc_type_id = resolve_doc_type(pool, &payload.doc_type_name).await?;

    // 3. Content-hash dedup
    if let Some(existing) = find_by_content_hash(pool, profile_id, &payload.content_hash).await? {
        return Ok(existing);
    }

    // 4. Decode chunks
    let chunks = unpack_chunks(&payload.chunks_packed)
        .map_err(|e| ApiError::BadRequest(format!("invalid chunks_packed: {e}")))?;

    // 5. Insert resource + chunks in a transaction
    let mut tx = pool.begin().await?;

    let resource_id = Uuid::now_v7();
    let resource = sqlx::query_as::<_, ResourceRow>(
        r#"
        INSERT INTO kb_resources (
            id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
            content_hash, mimetype, resource_mode,
            originator_profile_id, owner_profile_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10)
        RETURNING id, kb_context_id, kb_doc_type_id, origin_uri, title,
                  slug, content_hash, mimetype,
                  originator_profile_id, owner_profile_id, is_active,
                  created, updated
        "#,
    )
    .bind(resource_id)
    .bind(context.id)
    .bind(doc_type_id)
    .bind(&payload.origin_uri)
    .bind(&payload.title)
    .bind(&payload.slug)
    .bind(&payload.content_hash)
    .bind(&payload.mimetype)
    .bind(&payload.resource_mode)
    .bind(profile_id)
    .fetch_one(&mut *tx)
    .await?;

    // 6. Insert chunks with embeddings
    for chunk in &chunks {
        let chunk_id = Uuid::now_v7();
        sqlx::query(
            r#"
            INSERT INTO kb_chunks (
                id, resource_id, chunk_index, version, header_path,
                content, content_hash, embedding, is_current
            )
            VALUES ($1, $2, $3, 1, $4, $5, $6, $7::vector, true)
            "#,
        )
        .bind(chunk_id)
        .bind(resource_id)
        .bind(chunk.chunk_index as i32)
        .bind(&chunk.header_path)
        .bind(&chunk.content)
        .bind(&chunk.content_hash)
        .bind(&chunk.embedding as &[f32])
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(resource)
}

/// Update an existing resource's content — re-chunk and re-embed.
pub async fn update(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
    payload: IngestPayload,
) -> ApiResult<ResourceRow> {
    // Verify the profile can modify this resource
    let can_modify: Option<(bool,)> = sqlx::query_as(
        "SELECT true FROM can_modify_resource($1, $2)",
    )
    .bind(profile_id)
    .bind(resource_id)
    .fetch_optional(pool)
    .await?;

    if can_modify.is_none() {
        return Err(ApiError::NotFound);
    }

    let chunks = unpack_chunks(&payload.chunks_packed)
        .map_err(|e| ApiError::BadRequest(format!("invalid chunks_packed: {e}")))?;

    let mut tx = pool.begin().await?;

    // Update resource metadata
    let resource = sqlx::query_as::<_, ResourceRow>(
        r#"
        UPDATE kb_resources
        SET content_hash = $1, updated = now()
        WHERE id = $2
        RETURNING id, kb_context_id, kb_doc_type_id, origin_uri, title,
                  slug, content_hash, mimetype,
                  originator_profile_id, owner_profile_id, is_active,
                  created, updated
        "#,
    )
    .bind(&payload.content_hash)
    .bind(resource_id)
    .fetch_one(&mut *tx)
    .await?;

    // Version-bump old chunks
    sqlx::query(
        "UPDATE kb_chunks SET is_current = false WHERE resource_id = $1 AND is_current = true",
    )
    .bind(resource_id)
    .execute(&mut *tx)
    .await?;

    // Insert new chunks
    for chunk in &chunks {
        let chunk_id = Uuid::now_v7();
        sqlx::query(
            r#"
            INSERT INTO kb_chunks (
                id, resource_id, chunk_index, version, header_path,
                content, content_hash, embedding, is_current
            )
            VALUES ($1, $2, $3,
                    COALESCE((SELECT MAX(version) FROM kb_chunks
                              WHERE resource_id = $2 AND chunk_index = $3), 0) + 1,
                    $4, $5, $6, $7::vector, true)
            "#,
        )
        .bind(chunk_id)
        .bind(resource_id)
        .bind(chunk.chunk_index as i32)
        .bind(&chunk.header_path)
        .bind(&chunk.content)
        .bind(&chunk.content_hash)
        .bind(&chunk.embedding as &[f32])
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(resource)
}
```

- [ ] **Step 3: Register in services/mod.rs**

Add to `crates/temper-api/src/services/mod.rs`:
```rust
pub mod ingest_service;
```

- [ ] **Step 4: Verify compilation**

```bash
cargo check -p temper-api
```

If `ApiError::BadRequest` doesn't exist, add it to `crates/temper-api/src/error.rs`:
```rust
BadRequest(String),
```
with mapping:
```rust
ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
```

Also verify the `pgvector` cast works — `&chunk.embedding as &[f32]` may need the `pgvector` sqlx extension. Check how `search_service.rs` binds embeddings and follow the same pattern.

Expected: compiles cleanly.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/
git commit -m "feat(api): add ingest_service with create and update pipelines

Single-transaction ingest: resolve context/doc_type by name, dedup by
content_hash, insert resource + chunks with embeddings. Update path
version-bumps old chunks before inserting new ones."
```

---

## Task 7: Ingest handler and routes

**Files:**
- Create: `crates/temper-api/src/handlers/ingest.rs`
- Modify: `crates/temper-api/src/handlers/mod.rs`
- Modify: `crates/temper-api/src/routes.rs`

- [ ] **Step 1: Create ingest handler**

Create `crates/temper-api/src/handlers/ingest.rs`:

```rust
use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::error::{ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::ingest_service;
use crate::state::AppState;

use temper_core::types::ingest::IngestPayload;
use temper_core::types::resource::ResourceRow;

#[utoipa::path(
    post,
    path = "/api/ingest",
    tag = "Ingest",
    security(("bearer_auth" = [])),
    request_body = IngestPayload,
    responses(
        (status = 200, description = "Resource created (or existing on dedup)", body = ResourceRow),
        (status = 400, description = "Invalid payload", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Context not found", body = ErrorBody),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<IngestPayload>,
) -> ApiResult<Json<ResourceRow>> {
    ingest_service::ingest(&state.pool, auth.0.profile.id, payload)
        .await
        .map(Json)
}

#[utoipa::path(
    put,
    path = "/api/ingest/{id}",
    tag = "Ingest",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    request_body = IngestPayload,
    responses(
        (status = 200, description = "Resource updated", body = ResourceRow),
        (status = 400, description = "Invalid payload", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Resource not found", body = ErrorBody),
    )
)]
pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
    Json(payload): Json<IngestPayload>,
) -> ApiResult<Json<ResourceRow>> {
    ingest_service::update(&state.pool, auth.0.profile.id, resource_id, payload)
        .await
        .map(Json)
}
```

- [ ] **Step 2: Register handler module**

In `crates/temper-api/src/handlers/mod.rs`, add:
```rust
pub mod ingest;
```

- [ ] **Step 3: Add routes and decompression layer**

In `crates/temper-api/src/routes.rs`, add to the `protected` router:
```rust
.route("/api/ingest", post(handlers::ingest::create))
.route("/api/ingest/{id}", put(handlers::ingest::update))
```

Add `tower_http::decompression::RequestDecompressionLayer` to the app layers if not already present. In the `create_app` function, before or after the existing layers:

```rust
use tower_http::decompression::RequestDecompressionLayer;

// Add to the app builder:
app.layer(RequestDecompressionLayer::new())
```

Check if `tower-http` has the `decompression` feature enabled in `crates/temper-api/Cargo.toml`:
```toml
tower-http = { version = "0.6", features = ["cors", "trace", "decompression-gzip"] }
```

- [ ] **Step 4: Verify compilation**

```bash
cargo check -p temper-api
```
Expected: compiles cleanly.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/
git commit -m "feat(api): add ingest handlers (POST + PUT /api/ingest)

Accepts JSON with gzip Content-Encoding. POST creates resource with
pre-processed chunks. PUT updates existing resource with version bump."
```

---

## Task 8: Update `temper-client` IngestClient and add ContextClient

**Files:**
- Create: `crates/temper-client/src/contexts.rs`
- Modify: `crates/temper-client/src/ingest.rs`
- Modify: `crates/temper-client/src/lib.rs`
- Modify: `crates/temper-client/Cargo.toml`

- [ ] **Step 1: Add dependencies**

In `crates/temper-client/Cargo.toml`, verify `temper-core` is a dependency. No additional deps needed — the client uses `reqwest` which handles gzip.

- [ ] **Step 2: Create ContextClient**

Create `crates/temper-client/src/contexts.rs`:

```rust
//! Typed sub-client for the `/api/contexts` endpoints.

use uuid::Uuid;

use crate::auth;
use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::context::{ContextCreateRequest, ContextRow};

/// Sub-client for context operations.
pub struct ContextClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for ContextClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextClient").finish_non_exhaustive()
    }
}

impl<'a> ContextClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// List all visible contexts.
    pub async fn list(&self) -> Result<Vec<ContextRow>> {
        let token = auth::current_token()?;
        let req = self.http.get("/api/contexts");
        self.http.send_json(req, Some(&token)).await
    }

    /// Get a single context by ID.
    pub async fn get(&self, id: Uuid) -> Result<ContextRow> {
        let token = auth::current_token()?;
        let req = self.http.get(&format!("/api/contexts/{id}"));
        self.http.send_json(req, Some(&token)).await
    }

    /// Create a new context.
    pub async fn create(&self, name: &str) -> Result<ContextRow> {
        let token = auth::current_token()?;
        let body = ContextCreateRequest {
            name: name.to_owned(),
        };
        let req = self.http.post("/api/contexts").json(&body);
        self.http.send_json(req, Some(&token)).await
    }
}
```

- [ ] **Step 3: Replace IngestClient with JSON POST**

Replace `crates/temper-client/src/ingest.rs` with:

```rust
//! Typed sub-client for the `/api/ingest` endpoint.
//!
//! Sends a fully-processed payload (content + chunks + embeddings) as JSON.
//! The CLI handles extract → chunk → embed locally.

use uuid::Uuid;

use crate::auth;
use crate::error::Result;
use crate::http::HttpClient;
use temper_core::types::ingest::IngestPayload;
use temper_core::types::resource::ResourceRow;

/// Sub-client for ingest operations.
pub struct IngestClient<'a> {
    http: &'a HttpClient,
}

impl std::fmt::Debug for IngestClient<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IngestClient").finish_non_exhaustive()
    }
}

impl<'a> IngestClient<'a> {
    pub(crate) fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// POST /api/ingest — create resource with pre-processed chunks.
    pub async fn create(&self, payload: &IngestPayload) -> Result<ResourceRow> {
        let token = auth::current_token()?;
        let req = self.http.post("/api/ingest").json(payload);
        self.http.send_json(req, Some(&token)).await
    }

    /// PUT /api/ingest/:id — update resource content with new chunks.
    pub async fn update(&self, id: Uuid, payload: &IngestPayload) -> Result<ResourceRow> {
        let token = auth::current_token()?;
        let req = self.http.put(&format!("/api/ingest/{id}")).json(payload);
        self.http.send_json(req, Some(&token)).await
    }
}
```

- [ ] **Step 4: Register ContextClient in lib.rs**

In `crates/temper-client/src/lib.rs`, add:

```rust
pub mod contexts;
```

And add the accessor method to `impl TemperClient`:
```rust
/// Context CRUD sub-client.
pub fn contexts(&self) -> contexts::ContextClient<'_> {
    contexts::ContextClient::new(&self.http)
}
```

- [ ] **Step 5: Verify compilation**

```bash
cargo check -p temper-client
```
Expected: compiles cleanly. Old `IngestRequest` references are gone from this crate.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-client/
git commit -m "feat(client): add ContextClient, replace IngestClient with JSON POST

ContextClient: list, get, create for /api/contexts.
IngestClient: JSON POST with IngestPayload instead of multipart form."
```

---

## Task 9: Update CLI ingest pipeline

**Files:**
- Modify: `crates/temper-cli/src/actions/ingest.rs`
- Modify: `crates/temper-cli/src/cli.rs`
- Modify: `crates/temper-cli/src/commands/context_cmd.rs`
- Modify: `crates/temper-cli/Cargo.toml`

- [ ] **Step 1: Update CLI Cargo.toml**

In `crates/temper-cli/Cargo.toml`:
- Change `temper-embed` → `temper-ingest` in dependencies
- Add `rmp-serde = "1"` and `base64 = "0.22"`

- [ ] **Step 2: Add context create CLI command**

In `crates/temper-cli/src/cli.rs`, add a variant to `ContextAction`:
```rust
/// Create a new context on the server
Create {
    /// Context name to create
    name: String,
},
```

In the match arm in `main.rs` (or wherever context commands are dispatched), add:
```rust
ContextAction::Create { name } => {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let config = temper_cli::config::load(cli.vault.as_deref())?;
        temper_cli::commands::context_cmd::create_remote(&config, &name).await
    })
}
```

- [ ] **Step 3: Implement context create_remote**

In `crates/temper-cli/src/commands/context_cmd.rs`, add:

```rust
/// Create a context on the remote server.
pub async fn create_remote(config: &TemperConfig, name: &str) -> Result<()> {
    let client = crate::actions::runtime::build_client(config)?;
    let context = client.contexts().create(name).await?;
    println!("Created context '{}' ({})", context.name, context.id);
    Ok(())
}
```

Verify the imports and `build_client` usage match the existing pattern in other async commands (check how `search_cmd.rs` or similar builds the client).

- [ ] **Step 4: Rewrite ingest pipeline**

Replace the core of `crates/temper-cli/src/actions/ingest.rs`. Keep `compute_content_hash`, `title_from_path`, `build_uri`, `build_vault_path`, `build_frontmatter`, and `write_vault_file_and_register` — these are still used. Replace the ingest flow functions:

```rust
use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};
use temper_ingest::chunk::chunk_markdown;

#[cfg(feature = "embed")]
use temper_ingest::embed::embed_texts;

/// Build the wire-ready ingest payload from extracted markdown.
///
/// Performs chunk → embed → pack locally, producing a payload ready
/// for POST /api/ingest.
#[cfg(feature = "embed")]
pub fn build_ingest_payload(
    content: &str,
    title: &str,
    context: &str,
    doc_type: &str,
    slug: &str,
    resource_mode: &str,
    mime_type: &str,
    metadata: Option<serde_json::Value>,
) -> Result<IngestPayload> {
    let content_hash = compute_content_hash(content);
    let origin_uri = build_uri(context, doc_type, slug);

    // Chunk
    let chunk_data = chunk_markdown(content);

    // Embed
    let texts: Vec<&str> = chunk_data.iter().map(|c| c.content.as_str()).collect();
    let embeddings = embed_texts(&texts)
        .map_err(|e| anyhow::anyhow!("embedding failed: {e}"))?;

    // Pack
    let packed_chunks: Vec<PackedChunk> = chunk_data
        .into_iter()
        .zip(embeddings)
        .map(|(cd, emb)| PackedChunk {
            chunk_index: cd.chunk_index,
            header_path: cd.header_path,
            content: cd.content,
            content_hash: cd.content_hash,
            embedding: emb,
        })
        .collect();

    let chunks_packed = pack_chunks(&packed_chunks)
        .map_err(|e| anyhow::anyhow!("chunk packing failed: {e}"))?;

    Ok(IngestPayload {
        title: title.to_owned(),
        origin_uri,
        context_name: context.to_owned(),
        doc_type_name: doc_type.to_owned(),
        resource_mode: resource_mode.to_owned(),
        content_hash,
        slug: slug.to_owned(),
        mimetype: mime_type.to_owned(),
        content: content.to_owned(),
        metadata,
        chunks_packed,
    })
}

/// Ingest a local file: extract → chunk → embed → upload.
pub async fn ingest_file(
    client: &temper_client::TemperClient,
    file_path: &std::path::Path,
    context: &str,
    doc_type: &str,
    resource_mode: &str,
) -> Result<(temper_core::types::resource::ResourceRow, String)> {
    let extraction = temper_ingest::extract::extract_to_markdown(file_path)
        .await
        .map_err(|e| anyhow::anyhow!("extraction failed: {e}"))?;

    let title = title_from_path(file_path);
    let slug = slug_from_title(&title);

    let device_id = temper_client::auth::device_id()?;
    let metadata = serde_json::json!({
        "device_id": device_id,
        "original_path": file_path.to_string_lossy(),
    });

    let payload = build_ingest_payload(
        &extraction.content,
        &title,
        context,
        doc_type,
        &slug,
        resource_mode,
        &extraction.mime_type,
        Some(metadata),
    )?;

    let resource = client.ingest().create(&payload).await?;
    Ok((resource, extraction.content))
}
```

Add a `slug_from_title` helper if one doesn't already exist:
```rust
fn slug_from_title(title: &str) -> String {
    title
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-', "-")
        .trim_matches('-')
        .to_owned()
}
```

Remove the old `build_ingest_request` function and any remaining references to `IngestRequest`.

- [ ] **Step 5: Update import/add commands to pass resource_mode**

Check `crates/temper-cli/src/commands/import_cmd.rs` (or wherever `temper import` dispatches) and `commands/add.rs` — update calls to `ingest_file` to pass `"imported"` and `"added"` respectively.

- [ ] **Step 6: Verify compilation**

```bash
cargo check -p temper-cli --all-features
```
Expected: compiles cleanly with deprecation warnings gone.

- [ ] **Step 7: Run existing CLI tests**

```bash
cargo test -p temper-cli
```
Expected: existing tests pass (may need minor fixes for changed function signatures).

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/
git commit -m "feat(cli): replace ingest pipeline with local extract→chunk→embed→upload

CLI now processes content locally and sends IngestPayload with
MessagePack-encoded chunks to the Axum /api/ingest endpoint.
Adds 'temper context create' command. Removes old multipart path."
```

---

## Task 10: Sync service and handlers in `temper-api`

**Files:**
- Create: `crates/temper-api/src/services/sync_service.rs`
- Create: `crates/temper-api/src/handlers/sync.rs`
- Modify: `crates/temper-api/src/services/mod.rs`
- Modify: `crates/temper-api/src/handlers/mod.rs`
- Modify: `crates/temper-api/src/routes.rs`

- [ ] **Step 1: Create sync_service.rs**

```rust
//! Sync service — computes diffs and finalizes sync rounds.
//!
//! Port of packages/temper-cloud/src/sync.ts. Uses the same SQL functions
//! (sync_diff_for_device) but with batch updates for completeSyncRound
//! (fixes code review audit item 5e).

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;

use temper_core::types::sync::{
    MergedResource, SyncCompleteRequest, SyncCompleteResponse, SyncConflictItem, SyncPullItem,
    SyncPushItem, SyncRemovedItem, SyncStatusRequest, SyncStatusResponse,
};

/// Raw row from sync_diff_for_device().
#[derive(Debug, sqlx::FromRow)]
struct DiffRow {
    resource_id: Option<Uuid>,
    kb_uri: String,
    content_hash: String,
    updated: Option<DateTime<Utc>>,
    diff_type: String,
}

/// Categorize raw diff rows into typed response buckets.
/// Port of TypeScript `categorizeDiffRows()` — pure function.
fn categorize_diff_rows(rows: Vec<DiffRow>) -> SyncStatusResponse {
    let mut to_push = Vec::new();
    let mut to_pull = Vec::new();
    let mut conflicts = Vec::new();
    let mut removed = Vec::new();

    for row in rows {
        match row.diff_type.as_str() {
            "to_push" => to_push.push(SyncPushItem {
                uri: row.kb_uri,
                resource_id: row.resource_id,
            }),
            "to_pull" => to_pull.push(SyncPullItem {
                uri: row.kb_uri,
                resource_id: row.resource_id.expect("to_pull must have resource_id"),
                content_hash: row.content_hash,
            }),
            "conflict" => conflicts.push(SyncConflictItem {
                uri: row.kb_uri,
                resource_id: row.resource_id.expect("conflict must have resource_id"),
                server_hash: row.content_hash,
            }),
            "removed" => removed.push(SyncRemovedItem {
                uri: row.kb_uri,
                resource_id: row.resource_id.expect("removed must have resource_id"),
            }),
            _ => {} // ignore unknown diff types
        }
    }

    SyncStatusResponse {
        to_push,
        to_pull,
        conflicts,
        removed,
    }
}

/// Compute sync diff by calling sync_diff_for_device() and categorizing results.
pub async fn compute_sync_diff(
    pool: &PgPool,
    profile_id: Uuid,
    request: SyncStatusRequest,
) -> ApiResult<SyncStatusResponse> {
    let mut context_names: Vec<String> = Vec::new();
    let mut manifest_entries = Vec::new();

    for ctx in &request.contexts {
        context_names.push(ctx.name.clone());
        for entry in &ctx.entries {
            manifest_entries.push(serde_json::json!({
                "uri": entry.uri,
                "local_hash": entry.local_hash,
                "remote_hash": entry.remote_hash,
            }));
        }
    }

    let manifest_jsonb = serde_json::Value::Array(manifest_entries);

    let rows = sqlx::query_as::<_, DiffRow>(
        r#"
        SELECT resource_id, kb_uri, content_hash, updated, diff_type
          FROM sync_diff_for_device($1, $2::text[], $3::jsonb)
        "#,
    )
    .bind(profile_id)
    .bind(&context_names)
    .bind(&manifest_jsonb)
    .fetch_all(pool)
    .await?;

    Ok(categorize_diff_rows(rows))
}

/// Finalize a sync round: batch-update content hashes and upsert device state.
///
/// Uses a single UPDATE with unnest() instead of per-row loop
/// (fixes code review audit item 5e).
pub async fn complete_sync_round(
    pool: &PgPool,
    profile_id: Uuid,
    request: SyncCompleteRequest,
) -> ApiResult<SyncCompleteResponse> {
    let mut tx = pool.begin().await?;

    let updated_count = if !request.merged_resources.is_empty() {
        let ids: Vec<Uuid> = request.merged_resources.iter().map(|m| m.resource_id).collect();
        let hashes: Vec<String> = request.merged_resources.iter().map(|m| m.content_hash.clone()).collect();

        let result = sqlx::query(
            r#"
            UPDATE kb_resources r
            SET content_hash = u.content_hash, updated = now()
            FROM unnest($1::uuid[], $2::text[]) AS u(resource_id, content_hash)
            WHERE r.id = u.resource_id
            "#,
        )
        .bind(&ids)
        .bind(&hashes)
        .execute(&mut *tx)
        .await?;

        result.rows_affected() as u32
    } else {
        0
    };

    // Upsert device sync state
    sqlx::query(
        r#"
        INSERT INTO kb_device_sync_state (id, profile_id, device_id, last_sync_at)
        VALUES (gen_random_uuid(), $1, $2, now())
        ON CONFLICT (profile_id, device_id)
        DO UPDATE SET last_sync_at = now()
        "#,
    )
    .bind(profile_id)
    .bind(&request.device_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(SyncCompleteResponse {
        last_sync_at: Utc::now(),
        updated_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorize_diff_rows_sorts_correctly() {
        let rows = vec![
            DiffRow {
                resource_id: Some(Uuid::nil()),
                kb_uri: "kb://ctx/task/a".to_owned(),
                content_hash: "h1".to_owned(),
                updated: None,
                diff_type: "to_push".to_owned(),
            },
            DiffRow {
                resource_id: Some(Uuid::nil()),
                kb_uri: "kb://ctx/task/b".to_owned(),
                content_hash: "h2".to_owned(),
                updated: None,
                diff_type: "to_pull".to_owned(),
            },
            DiffRow {
                resource_id: Some(Uuid::nil()),
                kb_uri: "kb://ctx/task/c".to_owned(),
                content_hash: "h3".to_owned(),
                updated: None,
                diff_type: "conflict".to_owned(),
            },
            DiffRow {
                resource_id: Some(Uuid::nil()),
                kb_uri: "kb://ctx/task/d".to_owned(),
                content_hash: "h4".to_owned(),
                updated: None,
                diff_type: "removed".to_owned(),
            },
        ];

        let result = categorize_diff_rows(rows);
        assert_eq!(result.to_push.len(), 1);
        assert_eq!(result.to_pull.len(), 1);
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.to_push[0].uri, "kb://ctx/task/a");
        assert_eq!(result.to_pull[0].uri, "kb://ctx/task/b");
    }
}
```

- [ ] **Step 2: Create sync handler**

Create `crates/temper-api/src/handlers/sync.rs`:

```rust
use axum::extract::State;
use axum::Json;

use crate::error::{ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::sync_service;
use crate::state::AppState;

use temper_core::types::sync::{
    SyncCompleteRequest, SyncCompleteResponse, SyncStatusRequest, SyncStatusResponse,
};

#[utoipa::path(
    post,
    path = "/api/sync/status",
    tag = "Sync",
    security(("bearer_auth" = [])),
    request_body = SyncStatusRequest,
    responses(
        (status = 200, description = "Sync diff result", body = SyncStatusResponse),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn status(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SyncStatusRequest>,
) -> ApiResult<Json<SyncStatusResponse>> {
    sync_service::compute_sync_diff(&state.pool, auth.0.profile.id, body)
        .await
        .map(Json)
}

#[utoipa::path(
    post,
    path = "/api/sync/complete",
    tag = "Sync",
    security(("bearer_auth" = [])),
    request_body = SyncCompleteRequest,
    responses(
        (status = 200, description = "Sync round completed", body = SyncCompleteResponse),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn complete(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SyncCompleteRequest>,
) -> ApiResult<Json<SyncCompleteResponse>> {
    sync_service::complete_sync_round(&state.pool, auth.0.profile.id, body)
        .await
        .map(Json)
}
```

- [ ] **Step 3: Register modules**

In `crates/temper-api/src/services/mod.rs`:
```rust
pub mod sync_service;
```

In `crates/temper-api/src/handlers/mod.rs`:
```rust
pub mod sync;
```

- [ ] **Step 4: Add routes**

In `crates/temper-api/src/routes.rs`, add to the `protected` router:
```rust
.route("/api/sync/status", post(handlers::sync::status))
.route("/api/sync/complete", post(handlers::sync::complete))
```

- [ ] **Step 5: Verify compilation and run unit test**

```bash
cargo check -p temper-api
cargo test -p temper-api sync_service
```
Expected: compiles cleanly, `categorize_diff_rows_sorts_correctly` test passes.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/
git commit -m "feat(api): add sync service and handlers (status + complete)

Port of TypeScript sync.ts. Uses sync_diff_for_device() SQL function.
complete_sync_round uses batch UPDATE via unnest() instead of per-row
loop (addresses code review audit item 5e)."
```

---

## Task 11: Delete TypeScript ingest and sync code

**Files:**
- Delete: `api/ingest.ts`
- Delete: `api/ingest/` directory (if exists)
- Delete: `api/sync/status.ts`
- Delete: `api/sync/complete.ts`

- [ ] **Step 1: Verify which files exist**

```bash
ls -la api/ingest* api/sync/ 2>/dev/null
```

- [ ] **Step 2: Delete the TypeScript endpoints**

```bash
rm -f api/ingest.ts
rm -rf api/ingest/
rm -f api/sync/status.ts
rm -f api/sync/complete.ts
```

If `api/sync/` directory is now empty, remove it:
```bash
rmdir api/sync/ 2>/dev/null || true
```

- [ ] **Step 3: Check for orphaned imports**

Search for imports of the deleted modules in remaining TypeScript:
```bash
grep -r "from.*sync" packages/temper-cloud/src/ --include="*.ts" | grep -v test | grep -v node_modules
grep -r "from.*ingest" packages/temper-cloud/src/ --include="*.ts" | grep -v test | grep -v node_modules
```

The `sync.ts` and `ingest.ts` source files in `packages/temper-cloud/src/` stay — they may still be referenced by tests or by the remaining `api/upload.ts` workflow. Only the API route handler files are deleted.

- [ ] **Step 4: Verify Vercel build**

```bash
cd packages/temper-cloud && bun install && bun run build 2>/dev/null; cd -
```

Or at minimum verify no import errors in remaining API files:
```bash
grep -r "sync" api/ --include="*.ts" -l
```

- [ ] **Step 5: Commit**

```bash
git add -A api/
git commit -m "chore: delete TypeScript ingest and sync API routes

These endpoints are now served by the Axum API. TypeScript source
files in packages/temper-cloud/src/ are retained for tests and the
blob upload workflow."
```

---

## Task 12: Remove deprecated IngestRequest

**Files:**
- Modify: `crates/temper-core/src/types/ingest.rs`
- Modify: `crates/temper-core/src/types/mod.rs`

- [ ] **Step 1: Remove IngestRequest and its tests**

In `crates/temper-core/src/types/ingest.rs`, delete the deprecated `IngestRequest` struct and all its test functions.

In `crates/temper-core/src/types/mod.rs`, remove:
```rust
#[allow(deprecated)]
pub use ingest::IngestRequest;
```

- [ ] **Step 2: Verify no remaining references**

```bash
grep -r "IngestRequest" crates/ --include="*.rs"
```

Fix any remaining references (there should be none after Tasks 8-9).

- [ ] **Step 3: Verify compilation**

```bash
cargo check --workspace --all-features
```

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/
git commit -m "chore: remove deprecated IngestRequest type

The multipart ingest path is fully replaced by IngestPayload."
```

---

## Task 13: End-to-end validation

This task is manual — it validates the full pipeline against a live deployment.

- [ ] **Step 1: Build and install the CLI**

```bash
cargo install --path crates/temper-cli --all-features
```

- [ ] **Step 2: Verify the workspace compiles clean**

```bash
cargo check --workspace --all-features
cargo test --workspace
```

- [ ] **Step 3: Deploy to Vercel**

```bash
vercel --prod
```
Or push to the branch and let CI deploy.

- [ ] **Step 4: Create a context**

```bash
temper context create temper
```
Expected: `Created context 'temper' (<uuid>)`

- [ ] **Step 5: Import a file**

```bash
temper import docs/2026-04-01-i5e-handoff.md --context temper --doc-type task
```
Expected: successful import, resource created with UUID.

- [ ] **Step 6: Verify in database**

Using Neon MCP or psql:
```sql
SELECT id, title, content_hash, resource_mode FROM kb_resources
WHERE title LIKE '%i5e%' ORDER BY created DESC LIMIT 1;

SELECT chunk_index, header_path, length(content), is_current
FROM kb_current_chunks
WHERE resource_id = '<resource-id-from-above>'
ORDER BY chunk_index;
```
Expected: resource with `resource_mode='imported'`, multiple chunks with is_current=true.

- [ ] **Step 7: Search against imported content**

```bash
temper search "config unification"
```
Expected: results referencing the imported document.

- [ ] **Step 8: Test sync flow**

```bash
temper sync run --context temper
```
Expected: sync completes without errors, hitting the new Axum endpoints.

- [ ] **Step 9: Commit any final fixes**

If any adjustments were needed during validation, commit them.

```bash
git add -A
git commit -m "fix: adjustments from end-to-end validation"
```
