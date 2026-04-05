# R10 Implementation Guide: Ingest Service Changes for tsvector + Batch Chunks

**Date:** 2026-04-05
**Relates to:** R10 (tsvector FTS), migrations `20260405000001/2`
**Scope:** Rust-side changes to `ingest_service.rs` and `temper-core` types to use batch chunk SQL functions

---

## Context

The R10 migration adds two SQL functions for batch chunk persistence:

- `persist_resource_chunks(resource_id UUID, chunks JSONB) → INT` — new resource, bulk insert
- `replace_resource_chunks(resource_id UUID, chunks JSONB) → INT` — update, version-bump + bulk insert

Both functions gate the tsvector search triggers via `SET LOCAL temper.skip_search_rebuild = 'true'` and call `rebuild_resource_search_vector()` exactly once after all writes complete. This replaces the current `insert_chunks()` loop which fires the trigger per-chunk (O(n²) in chunk reads).

---

## Problem: Current Write Path

### `insert_chunks()` in `ingest_service.rs`

```rust
async fn insert_chunks(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    resource_id: Uuid,
    chunks: &[PackedChunk],
) -> ApiResult<()> {
    for chunk in chunks {
        let chunk_id = Uuid::now_v7();
        let embedding_str = format_embedding(&chunk.embedding);
        sqlx::query(/* INSERT kb_chunks */)
            .execute(&mut **tx).await?;
        sqlx::query(/* INSERT kb_chunk_content */)  // ← trigger fires here
            .execute(&mut **tx).await?;
    }
    Ok(())
}
```

For a 15-chunk resource:
- **Create path**: 31 SQL statements (1 resource + 15 chunk + 15 content), 16 trigger-driven search rebuilds
- **Update path**: ~32 SQL statements (1 resource update + 1 bulk is_current=false hitting 15 rows + 15 chunk + 15 content), ~30 trigger-driven search rebuilds
- Each rebuild aggregates *all current chunks* via `string_agg` → O(n²) total reads

### After the change:
- **Create path**: 2 SQL statements (1 resource INSERT + 1 `persist_resource_chunks` call), 1 search rebuild
- **Update path**: 2 SQL statements (1 resource UPDATE + 1 `replace_resource_chunks` call), 1 search rebuild

---

## New Types: `ChunkRow` JSONB contract

The SQL functions accept a JSONB array. Each element must match this shape:

```json
{
  "chunk_index": 0,
  "header_path": "Title > Section",
  "content": "The actual chunk text...",
  "content_hash": "a1b2c3d4...",
  "embedding": "[0.1,0.2,0.3,...]"
}
```

Note that `embedding` is a **string** — the pgvector literal format `[0.1,0.2,...]` — not a JSON array of numbers. The SQL function casts it via `::vector`.

### Rust struct: `ChunkRowJsonb`

Add to `temper-core/src/types/ingest.rs` (alongside `PackedChunk`):

```rust
/// JSONB-serializable chunk row for the `persist_resource_chunks()` and
/// `replace_resource_chunks()` SQL functions.
///
/// This is the wire format between Rust and the SQL batch-insert functions.
/// It differs from `PackedChunk` in one key way: `embedding` is a pre-formatted
/// pgvector literal string (`"[0.1,0.2,...]"`) rather than a `Vec<f32>`.
///
/// The SQL function casts this string to `vector` via `::vector`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRowJsonb {
    pub chunk_index: u32,
    pub header_path: String,
    pub content: String,
    pub content_hash: String,
    /// Pre-formatted pgvector literal: `"[0.1,0.2,...]"`
    pub embedding: String,
}

impl ChunkRowJsonb {
    /// Convert a `PackedChunk` into a `ChunkRowJsonb` by formatting the
    /// embedding vector as a pgvector literal string.
    pub fn from_packed(chunk: &PackedChunk, format_embedding: fn(&[f32]) -> String) -> Self {
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
pub fn chunks_to_jsonb(
    chunks: &[PackedChunk],
    format_embedding: fn(&[f32]) -> String,
) -> serde_json::Value {
    let rows: Vec<ChunkRowJsonb> = chunks
        .iter()
        .map(|c| ChunkRowJsonb::from_packed(c, format_embedding))
        .collect();
    serde_json::to_value(&rows).expect("ChunkRowJsonb is always serializable")
}
```

### Why `format_embedding` is a function parameter

`format_embedding()` currently lives in `temper-api::services::search_service`. Rather than creating a cross-crate dependency from `temper-core` to `temper-api`, we pass the formatter as a function pointer. The signature `fn(&[f32]) -> String` is simple and testable. The implementation is trivial:

```rust
pub fn format_embedding(embedding: &[f32]) -> String {
    format!(
        "[{}]",
        embedding.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
    )
}
```

This function should move to `temper-core` (it's a pure data transformation with no API dependency). Then `ChunkRowJsonb::from_packed` can call it directly without the function pointer indirection. This is a minor refactor — the function pointer approach works as a transitional step if you don't want to move `format_embedding` right away.

**Alternative: move `format_embedding` to `temper-core`** (recommended):

```rust
// temper-core/src/types/ingest.rs (or a new temper-core/src/util.rs)

/// Format an embedding vector as a pgvector literal string: `[0.1,0.2,...]`
pub fn format_embedding(embedding: &[f32]) -> String {
    format!(
        "[{}]",
        embedding.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
    )
}
```

Then `ChunkRowJsonb` becomes:

```rust
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

pub fn chunks_to_jsonb(chunks: &[PackedChunk]) -> serde_json::Value {
    let rows: Vec<ChunkRowJsonb> = chunks
        .iter()
        .map(ChunkRowJsonb::from_packed)
        .collect();
    serde_json::to_value(&rows).expect("ChunkRowJsonb is always serializable")
}
```

---

## Changes to `ingest_service.rs`

### New helper functions

```rust
use temper_core::types::ingest::{chunks_to_jsonb, PackedChunk};

/// Batch-insert chunks for a new resource via SQL function.
/// Gates search triggers, does bulk INSERT, rebuilds search index once.
async fn persist_chunks(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    resource_id: Uuid,
    chunks: &[PackedChunk],
) -> ApiResult<i32> {
    let chunks_json = chunks_to_jsonb(chunks);

    let (count,): (i32,) = sqlx::query_as(
        "SELECT persist_resource_chunks($1, $2)"
    )
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

    let (count,): (i32,) = sqlx::query_as(
        "SELECT replace_resource_chunks($1, $2)"
    )
    .bind(resource_id)
    .bind(&chunks_json)
    .fetch_one(&mut **tx)
    .await?;

    Ok(count)
}
```

### `ingest()` — change at step 7

```diff
     // 7. Insert chunks with embeddings
-    insert_chunks(&mut tx, resource_id, &chunks).await?;
+    persist_chunks(&mut tx, resource_id, &chunks).await?;
```

### `update()` — replace version-bump + loop

```diff
-    // Version-bump old chunks
-    sqlx::query(
-        "UPDATE kb_chunks SET is_current = false WHERE resource_id = $1 AND is_current = true",
-    )
-    .bind(resource_id)
-    .execute(&mut *tx)
-    .await?;
-
-    // Insert new chunks (version auto-computed)
-    for chunk in &chunks {
-        let chunk_id = Uuid::now_v7();
-        let embedding_str = format_embedding(&chunk.embedding);
-        sqlx::query(
-            r#"
-            INSERT INTO kb_chunks (
-                id, resource_id, chunk_index, version, header_path,
-                content_hash, embedding, is_current
-            )
-            VALUES ($1, $2, $3,
-                    COALESCE((SELECT MAX(version) FROM kb_chunks
-                              WHERE resource_id = $2 AND chunk_index = $3), 0) + 1,
-                    $4, $5, $6::vector, true)
-            "#,
-        )
-        .bind(chunk_id)
-        .bind(resource_id)
-        .bind(chunk.chunk_index as i32)
-        .bind(&chunk.header_path)
-        .bind(&chunk.content_hash)
-        .bind(&embedding_str)
-        .execute(&mut *tx)
-        .await?;
-
-        sqlx::query("INSERT INTO kb_chunk_content (chunk_id, content) VALUES ($1, $2)")
-            .bind(chunk_id)
-            .bind(&chunk.content)
-            .execute(&mut *tx)
-            .await?;
-    }
+    // Replace chunks — version-bump + batch insert + search rebuild in one call
+    replace_chunks(&mut tx, resource_id, &chunks).await?;
```

### Dead code removal

After the migration, `insert_chunks()` can be removed entirely. The `format_embedding()` function in `search_service.rs` should be moved to `temper-core` and re-exported from `search_service` for backward compatibility until all call sites are updated.

---

## Tests to add

### Unit test: `ChunkRowJsonb` serialization

```rust
#[test]
fn chunk_row_jsonb_serializes_correctly() {
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

    let json = serde_json::to_value(&row).unwrap();
    assert_eq!(json["chunk_index"], 0);
    assert_eq!(json["embedding"], "[0.1,0.2,0.3]");
    assert!(json["embedding"].is_string()); // NOT a JSON array
}
```

### Unit test: `chunks_to_jsonb` produces valid array

```rust
#[test]
fn chunks_to_jsonb_produces_array() {
    let chunks = vec![
        PackedChunk {
            chunk_index: 0,
            header_path: "A".into(),
            content: "first".into(),
            content_hash: "h1".into(),
            embedding: vec![0.1; 768],
        },
        PackedChunk {
            chunk_index: 1,
            header_path: "B".into(),
            content: "second".into(),
            content_hash: "h2".into(),
            embedding: vec![0.2; 768],
        },
    ];

    let json = chunks_to_jsonb(&chunks);
    assert!(json.is_array());
    assert_eq!(json.as_array().unwrap().len(), 2);
    assert_eq!(json[0]["chunk_index"], 0);
    assert_eq!(json[1]["chunk_index"], 1);
    assert!(json[0]["embedding"].as_str().unwrap().starts_with("[0.1,"));
}
```

### Integration test: `persist_resource_chunks` SQL function

```rust
#[sqlx::test]
async fn persist_resource_chunks_inserts_all_chunks(pool: PgPool) {
    // Setup: create a profile, context, doc_type, and resource
    // ...

    let chunks = serde_json::json!([
        {
            "chunk_index": 0,
            "header_path": "Title",
            "content": "First chunk content",
            "content_hash": "hash0",
            "embedding": format_embedding(&vec![0.1_f32; 768])
        },
        {
            "chunk_index": 1,
            "header_path": "Title > Section",
            "content": "Second chunk content",
            "content_hash": "hash1",
            "embedding": format_embedding(&vec![0.2_f32; 768])
        }
    ]);

    let (count,): (i32,) = sqlx::query_as("SELECT persist_resource_chunks($1, $2)")
        .bind(resource_id)
        .bind(&chunks)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(count, 2);

    // Verify chunks exist
    let chunk_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM kb_chunks WHERE resource_id = $1 AND is_current = true"
    )
    .bind(resource_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(chunk_count.0, 2);

    // Verify search index was built
    let has_index: Option<(Uuid,)> = sqlx::query_as(
        "SELECT resource_id FROM kb_resource_search_index WHERE resource_id = $1"
    )
    .bind(resource_id)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(has_index.is_some());
}
```

---

## Migration checklist

- [ ] Run migration `20260405000001_fts_search_index.sql`
- [ ] Run migration `20260405000002_fts_backfill.sql`
- [ ] Move `format_embedding()` from `temper-api` to `temper-core`
- [ ] Add `ChunkRowJsonb` and `chunks_to_jsonb()` to `temper-core/src/types/ingest.rs`
- [ ] Replace `insert_chunks()` calls with `persist_chunks()` in `ingest_service::ingest()`
- [ ] Replace version-bump loop with `replace_chunks()` in `ingest_service::update()`
- [ ] Remove dead `insert_chunks()` function
- [ ] Update `SearchParams` — `embedding` becomes `Option<Vec<f32>>`, add `query: Option<String>`
- [ ] Update `search_service::search()` to route through `unified_search()` SQL function
- [ ] Add unit tests for `ChunkRowJsonb` serialization
- [ ] Add integration tests for `persist_resource_chunks` and `replace_resource_chunks`
- [ ] Verify MCP `search` tool exposes `query` field in schema
