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
use sha2::{Digest, Sha256};
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
/// ONNX-free and unit-testable). Each entry is `(chunk_index, content_hash, content, header_path,
/// heading_depth)` straight from the production chunker — the heading fields are carried through so the
/// body read path (`reconstruct_body`) can restore `##`-style markers (`heading_depth == 0` ⇒ preamble).
fn plan_chunks(prose: &str) -> Vec<(i32, String, String, String, u8)> {
    temper_ingest::chunk::chunk_markdown(prose)
        .into_iter()
        .map(|c| {
            (
                c.chunk_index as i32,
                c.content_hash,
                c.content,
                c.header_path,
                c.heading_depth,
            )
        })
        .collect()
}

/// Prepare one block: chunk its prose, then embed every chunk in a single batched ONNX call.
pub fn prepare_block(seq: i32, role: Option<&str>, prose: &str) -> Result<PreparedBlock> {
    let planned = plan_chunks(prose);
    let texts: Vec<&str> = planned
        .iter()
        .map(|(_, _, content, _, _)| content.as_str())
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
            |((chunk_index, content_hash, content, header_path, heading_depth), embedding)| {
                // Carry the chunker's heading metadata so `reconstruct_body` can re-emit markers.
                // depth 0 / empty breadcrumb ⇒ unheaded preamble: keep NULL (matches reconstruct_body's
                // `heading_depth == 0 ⇒ content as-is` arm). A real heading ⇒ persist depth + breadcrumb.
                let (header_path, heading_depth) = if heading_depth == 0 || header_path.is_empty() {
                    (None, None)
                } else {
                    (Some(header_path), Some(heading_depth as i16))
                };
                PreparedChunk {
                    chunk_id: ChunkId::from(Uuid::now_v7()),
                    chunk_index,
                    content_hash,
                    content,
                    embedding,
                    header_path,
                    heading_depth,
                }
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

/// Lowercase hex sha256 of a string's UTF-8 bytes — the Rust twin of Postgres's
/// `encode(sha256(convert_to(s, 'UTF8')), 'hex')`.
fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}

/// The resource `body_hash` for the live single-block create path, computed Rust-side so a dedup
/// pre-check (WS6 collapse Task F) can key on the SAME value the substrate's create projector stores
/// in `kb_resources.body_hash`. Mirrors `_recompute_resource_body_hash`
/// (`migrations/20260624000002_canonical_functions.sql`) for the create case: [`crate::writes::create_resource`]
/// persists `body` as ONE roleless block at seq 0, so the merkle is `sha256_hex(per_block_hash)`,
/// where `per_block_hash = sha256_hex(concat of the block's chunk content_hashes in chunk_index
/// order)`.
///
/// An empty/whitespace body chunks to nothing — the SQL coalesces the empty per-block aggregate to
/// `''` → `sha256_hex("")` — so this returns `sha256_hex("")` for an empty body. The dedup caller
/// skips empty bodies (matching the legacy `ingest_service::ingest` path, which only deduplicates a
/// caller-supplied hash for non-empty content), so this branch is not reached in practice; it is
/// faithful regardless.
///
/// ONNX-free: only the chunker's content_hashes are needed (`plan_chunks`), not embeddings.
pub fn body_hash_for_body(body: &str) -> String {
    let planned = plan_chunks(body);
    if planned.is_empty() {
        return sha256_hex("");
    }
    let block_concat: String = planned.iter().map(|(_, hash, ..)| hash.as_str()).collect();
    let block_hash = sha256_hex(&block_concat);
    // A single block in seq order → the resource merkle is sha256 of that one per-block hash.
    sha256_hex(&block_hash)
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

// ── Body read assembly (the live GET /content reconstruction) ────────────────
// Moved here from the retired `parity` module: `readback::body` reconstructs a resource's markdown
// from its substrate chunks using `ReadChunk` + `reconstruct_body`. This is the chunk model's home.

/// One chunk as the body reconstruction sees it: ordering index, heading breadcrumb, heading level, and
/// prose. The read-side counterpart of [`PreparedChunk`].
#[derive(Debug, Clone)]
pub struct ReadChunk {
    pub chunk_index: i32,
    pub header_path: String,
    pub heading_depth: i16,
    pub content: String,
}

/// Production `get_content`'s markdown assembly: per chunk (ordered by `chunk_index`),
/// `heading_depth == 0` ⇒ content as-is; else the innermost breadcrumb segment becomes a markdown
/// heading (`{hashes} {title}\n\n{content}`, depth capped at 6, empty breadcrumb ⇒ `"Untitled"`). Pieces
/// join with `"\n\n"`. The live `readback::body` read path's single body assembler.
pub fn reconstruct_body(chunks: &[ReadChunk]) -> String {
    chunks
        .iter()
        .map(|c| {
            if c.heading_depth == 0 {
                // Preamble or unheaded content — emit body only.
                c.content.clone()
            } else {
                // Extract the innermost heading title from the breadcrumb.
                // rsplit always yields at least one element on non-empty input.
                let title = if c.header_path.is_empty() {
                    "Untitled"
                } else {
                    c.header_path.rsplit(" > ").next().unwrap_or(&c.header_path)
                };
                let depth = (c.heading_depth as usize).min(6);
                let hashes = "#".repeat(depth);
                format!("{hashes} {title}\n\n{}", c.content)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_chunk(idx: i32, header_path: &str, depth: i16, content: &str) -> ReadChunk {
        ReadChunk {
            chunk_index: idx,
            header_path: header_path.to_owned(),
            heading_depth: depth,
            content: content.to_owned(),
        }
    }

    #[test]
    fn unheaded_chunk_emits_content_only() {
        assert_eq!(
            reconstruct_body(&[read_chunk(0, "", 0, "Just prose.")]),
            "Just prose."
        );
    }

    #[test]
    fn headed_chunk_uses_innermost_breadcrumb_segment() {
        assert_eq!(
            reconstruct_body(&[read_chunk(0, "Intro > Goals", 2, "Body.")]),
            "## Goals\n\nBody."
        );
    }

    #[test]
    fn mixed_chunks_join_with_blank_line() {
        assert_eq!(
            reconstruct_body(&[
                read_chunk(0, "", 0, "Task intro paragraph."),
                read_chunk(1, "Intro > Goals", 2, "Task goals section body."),
            ]),
            "Task intro paragraph.\n\n## Goals\n\nTask goals section body."
        );
    }

    #[test]
    fn empty_breadcrumb_with_depth_falls_back_to_untitled_and_caps_at_six() {
        assert_eq!(
            reconstruct_body(&[read_chunk(0, "", 9, "x")]),
            "###### Untitled\n\nx"
        );
    }

    // A short, single-paragraph block stays one chunk; its hash is the chunker's sha256 (64 hex chars).
    #[test]
    fn short_prose_is_one_chunk_with_sha256_hash() {
        let planned = plan_chunks("A short onboarding note about first-week confidence.");
        assert_eq!(planned.len(), 1, "short prose must be a single chunk");
        let (idx, hash, content, ..) = &planned[0];
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
        for (i, (idx, hash, ..)) in planned.iter().enumerate() {
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
