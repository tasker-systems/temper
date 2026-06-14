//! Content prepare path — borrow production's chunk/embed machinery, apply it **per content-block**.
//!
//! Deliverable-1 of the scenario-DSL roadmap (content-block/chunk correctness). The temper-next write
//! functions used to write the degenerate one-chunk-per-block case with an `md5()` placeholder hash and
//! no embedding (chunks were embedded later by a separate job). Here we instead chunk each block's prose
//! with `temper_ingest::chunk::chunk_markdown` (heading-delimited, 510-token windows, **sha256** content
//! hashes) and embed every chunk **inline** with `temper_ingest::embed::embed_texts` (bge-base-en-v1.5,
//! 768-dim). The result is a `Vec<PreparedBlock>` the SQL functions persist verbatim.
//!
//! Split of responsibility (mirrors production: chunking is Rust-side, SQL only persists):
//!   - Rust (here): prose -> blocks -> chunks, each with its sha256 `content_hash` + bge-768 embedding.
//!   - SQL (`resource_create`/`cogmap_genesis`): insert the rows; derive `block_body_hash` /
//!     `kb_resources.body_hash` with Postgres's built-in `sha256()` over the chunk/block hashes.

use crate::ids::{BlockId, ChunkId};
use anyhow::{Context, Result};
use serde::Serialize;
use uuid::Uuid;

/// One embedding window of a block's prose, ready to persist. `content_hash` is the chunker's lowercase
/// hex sha256 of `content.trim()`; `embedding` is the l2-normalized bge-768 vector.
#[derive(Debug, Clone, Serialize)]
pub struct PreparedChunk {
    /// Pre-generated chunk identity (identity-as-input, payload spec §2): carried into the payload
    /// manifest AND used by the SQL projection as the kb_chunks.id, so replay reproduces row ids.
    pub chunk_id: ChunkId,
    pub chunk_index: i32,
    pub content_hash: String,
    pub content: String,
    pub embedding: Vec<f32>,
    /// Production render metadata (§8 carry-as-is): the heading breadcrumb this chunk sits under and
    /// its heading depth, persisted onto `kb_chunks` so a downstream read reconstructs headed markdown
    /// identically to production. `None` for the scenario-authoring path (no production headings) —
    /// the columns stay NULL, exactly as before this carry existed.
    pub header_path: Option<String>,
    pub heading_depth: Option<i16>,
}

/// One content-block (seq-ordered within its resource) and its ordered chunks. Blocks carry **no**
/// prose of their own (content-block-primitive β) — text lives only in the chunks. `role` is the
/// block's `block_role` (`"statement"`/`"question"`/`"framing"` for a charter; `None` for an ordinary
/// resource body); when present the persist path stamps it as a `block_role` property. Serialized as
/// `null` when `None`.
#[derive(Debug, Clone, Serialize)]
pub struct PreparedBlock {
    /// Pre-generated block identity (identity-as-input) — see `PreparedChunk::chunk_id`.
    pub block_id: BlockId,
    pub seq: i32,
    pub role: Option<String>,
    pub chunks: Vec<PreparedChunk>,
}

/// Pure chunk plan for one block's prose — chunking + hashing only, **no** embedding (so it is
/// ONNX-free and unit-testable). Each entry is `(chunk_index, content_hash, content)` straight from the
/// production chunker.
fn plan_chunks(prose: &str) -> Vec<(i32, String, String)> {
    temper_ingest::chunk::chunk_markdown(prose)
        .into_iter()
        .map(|c| (c.chunk_index as i32, c.content_hash, c.content))
        .collect()
}

/// Prepare one block: chunk its prose, then embed every chunk in a single batched ONNX call.
pub fn prepare_block(seq: i32, role: Option<&str>, prose: &str) -> Result<PreparedBlock> {
    let planned = plan_chunks(prose);
    let texts: Vec<&str> = planned
        .iter()
        .map(|(_, _, content)| content.as_str())
        .collect();
    // Empty prose ⇒ no chunks ⇒ no embedding call (embed_texts on an empty slice is wasteful/undefined).
    let embeddings = if texts.is_empty() {
        Vec::new()
    } else {
        temper_ingest::embed::embed_texts(&texts).context("embed_texts (bge-768) failed")?
    };
    let chunks = planned
        .into_iter()
        .zip(embeddings)
        .map(
            |((chunk_index, content_hash, content), embedding)| PreparedChunk {
                chunk_id: ChunkId::from(Uuid::now_v7()),
                chunk_index,
                content_hash,
                content,
                embedding,
                // the scenario-authoring path has no production headings; carry NULL (unchanged).
                header_path: None,
                heading_depth: None,
            },
        )
        .collect();
    Ok(PreparedBlock {
        block_id: BlockId::from(Uuid::now_v7()),
        seq,
        role: role.map(str::to_owned),
        chunks,
    })
}

/// Prepare an ordered run of blocks (`seq` = position). Each spec is `(role, prose)`: the charter
/// passes `[(Some("statement"), …), (Some("question"), …), …, (Some("framing"), …)]`; an ordinary
/// resource passes its single body as one roleless block `[(None, body)]`. A block whose prose exceeds
/// one 510-token window yields >1 chunk — real multi-chunk-per-block.
pub fn prepare_blocks(specs: &[(Option<&str>, &str)]) -> Result<Vec<PreparedBlock>> {
    specs
        .iter()
        .enumerate()
        .map(|(i, (role, prose))| prepare_block(i as i32, *role, prose))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A short, single-paragraph block stays one chunk; its hash is the chunker's sha256 (64 hex chars).
    #[test]
    fn short_prose_is_one_chunk_with_sha256_hash() {
        let planned = plan_chunks("A short onboarding note about first-week confidence.");
        assert_eq!(planned.len(), 1, "short prose must be a single chunk");
        let (idx, hash, content) = &planned[0];
        assert_eq!(*idx, 0);
        assert_eq!(hash.len(), 64, "sha256 hex is 64 chars");
        assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(content.contains("first-week"));
    }

    // A block well past one 510-token (~1785-char) window splits into multiple chunks with sequential
    // indices — the multi-chunk-per-block path the degenerate seed never exercised.
    #[test]
    fn long_prose_splits_into_multiple_sequential_chunks() {
        // ~30 sentences of ~80 chars each ≈ 2400+ chars, comfortably over MAX_CHARS (~1785), as separate
        // paragraphs so the chunker has split points.
        let para =
            "This paragraph explains one facet of reaching first-merge confidence in onboarding week one.\n\n";
        let prose = para.repeat(30);
        let planned = plan_chunks(&prose);
        assert!(
            planned.len() > 1,
            "long prose must split into >1 chunk, got {}",
            planned.len()
        );
        for (i, (idx, hash, _)) in planned.iter().enumerate() {
            assert_eq!(*idx, i as i32, "chunk_index must be sequential 0..n");
            assert_eq!(hash.len(), 64);
        }
    }

    // Blocks serialize to the JSONB shape the SQL functions consume (array of {block_id, seq, chunks:[…]}).
    #[test]
    fn prepared_block_serializes_to_expected_jsonb_shape() {
        let block = PreparedBlock {
            block_id: BlockId::from(Uuid::now_v7()),
            seq: 2,
            role: Some("question".into()),
            chunks: vec![PreparedChunk {
                chunk_id: ChunkId::from(Uuid::now_v7()),
                chunk_index: 0,
                content_hash: "ab".repeat(32),
                content: "hi".into(),
                embedding: vec![0.1, 0.2, 0.3],
                header_path: None,
                heading_depth: None,
            }],
        };
        let v = serde_json::to_value([&block]).unwrap();
        assert_eq!(v[0]["seq"], 2);
        assert_eq!(v[0]["role"], "question");
        // identity-as-input: pre-generated ids ride the JSONB into the SQL projection
        assert!(v[0]["block_id"].is_string());
        assert!(v[0]["chunks"][0]["chunk_id"].is_string());
        assert_eq!(v[0]["chunks"][0]["chunk_index"], 0);
        assert_eq!(v[0]["chunks"][0]["content"], "hi");
        // embedding is a JSON array (exact f32 values drift in JSON; the SQL `::vector` cast consumes
        // the array verbatim — shape is what matters here).
        assert_eq!(v[0]["chunks"][0]["embedding"].as_array().unwrap().len(), 3);
    }
}
