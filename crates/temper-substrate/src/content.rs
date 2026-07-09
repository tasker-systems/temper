//! Content prepare path — borrow production's chunk/embed machinery, apply it **per content-block**.
//!
//! Deliverable-1 of the scenario-DSL roadmap (content-block/chunk correctness). The temper-substrate write
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
    /// The chunk's bge-768 vector, or `None` when embedding is **deferred** (the async-embed write
    /// path — [`prepare_block_deferred`]): the chunk text + hash persist synchronously and the vector
    /// is backfilled off-request. `None` serializes to an absent sidecar `embedding` key, which the
    /// `_insert_chunk` projector maps to a NULL `kb_chunks.embedding` (issue #299). A genuinely
    /// embedded chunk is always `Some(<768-dim>)`.
    pub embedding: Option<Vec<f32>>,
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
    /// Provenance: the ordered sources this block's content was incorporated from. Empty for the
    /// scenario/charter paths; the resource create/update write path sets it from the caller's
    /// `sources`. Carried onto the block manifest (via `From<&PreparedBlock>`, read directly — NOT
    /// serialized) → recorded in `kb_block_provenance` by the projector. `#[serde(skip)]` keeps
    /// `PreparedBlock`'s own serialized shape (the content sidecar) byte-identical.
    #[serde(skip)]
    pub incorporated: Vec<crate::payloads::Incorporation>,
}

/// Pure chunk plan for one block's prose — chunking + hashing only, **no** embedding (so it is
/// ONNX-free and unit-testable). Each entry is `(chunk_index, content_hash, content, header_path,
/// heading_depth)` straight from the production chunker — the heading fields are carried through so the
/// body read path (`reconstruct_body`) can restore `##`-style markers (`heading_depth == 0` ⇒ preamble).
fn plan_chunks(prose: &str) -> Vec<(i32, String, String, String, u8)> {
    plan_chunks_with_prefix(prose, &[])
}

/// [`plan_chunks`], seeding the chunker's heading-breadcrumb stack from `breadcrumb` so a segment
/// that begins mid-section still carries its ancestor `header_path`. With an empty `breadcrumb`
/// this is byte-identical to whole-document chunking — the equivalence the streaming segment
/// boundary rests on.
fn plan_chunks_with_prefix(
    prose: &str,
    breadcrumb: &[String],
) -> Vec<(i32, String, String, String, u8)> {
    temper_ingest::chunk::chunk_markdown_with_prefix(prose, breadcrumb)
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

/// The one heading rule, shared by every block builder. The two columns answer different
/// questions and are mapped independently:
///
/// - `header_path` — "what section am I under?" Persisted whenever the chunker knows, which
///   includes a chunk that begins mid-section and inherits its ancestors from a seeded breadcrumb
///   (see [`prepare_block_with_prefix`]). This is what search renders as a breadcrumb.
/// - `heading_depth` — "do I *begin* a section?" Persisted only for a chunk that carries its own
///   heading. `reconstruct_body` re-emits a heading line exactly when this is non-zero, and
///   `readback::body` COALESCEs NULL to 0, so a continuation chunk correctly renders as bare prose.
///
/// Collapsing the two (the pre-streaming rule: `depth == 0 ⇒ both NULL`) was invisible while every
/// chunk with depth 0 also had an empty path. A segment cut mid-section produces depth 0 *with* an
/// ancestor path, and the old rule silently discarded that breadcrumb.
fn map_heading(header_path: String, heading_depth: u8) -> (Option<String>, Option<i16>) {
    let path = (!header_path.is_empty()).then_some(header_path);
    let depth = (heading_depth > 0).then_some(heading_depth as i16);
    (path, depth)
}

/// A caller-supplied, already-embedded chunk — the no-embed input to [`prepare_block_from_chunks`].
/// Field-for-field the substrate-native twin of temper-core's wire `PackedChunk` (temper-substrate does NOT
/// depend on temper-core): the client did the extract→chunk→embed locally, so the server carries the
/// vector verbatim instead of re-running ONNX. `heading_depth`/`header_path` map to the chunk's render
/// metadata exactly as [`prepare_block`] maps the chunker's own output. `chunk_index`/`heading_depth`
/// are already the substrate column types (`i32`/`i16`), widened from the wire's `u32`/`u8` by the
/// surface that constructs these.
#[derive(Debug, Clone)]
pub struct IncomingChunk {
    pub chunk_index: i32,
    pub content_hash: String,
    pub content: String,
    pub embedding: Vec<f32>,
    pub header_path: String,
    pub heading_depth: i16,
}

/// Prepare one block from caller-supplied (already-embedded) chunks — the no-embed twin of
/// [`prepare_block`]. Each [`IncomingChunk`] becomes a [`PreparedChunk`] with a freshly minted
/// `chunk_id`; the heading mapping is IDENTICAL to `prepare_block` (`heading_depth == 0` or an empty
/// breadcrumb ⇒ unheaded preamble, NULL columns). There is NO `embed_texts` call — the vector rides
/// through verbatim, so this is ONNX-free. Returns `PreparedBlock` directly (no embed to fail).
pub fn prepare_block_from_chunks(
    seq: i32,
    role: Option<&str>,
    chunks: Vec<IncomingChunk>,
) -> PreparedBlock {
    let chunks = chunks
        .into_iter()
        .map(|c| {
            let (header_path, heading_depth) = map_heading(c.header_path, c.heading_depth as u8);
            PreparedChunk {
                chunk_id: ChunkId::from(Uuid::now_v7()),
                chunk_index: c.chunk_index,
                content_hash: c.content_hash,
                content: c.content,
                embedding: Some(c.embedding),
                header_path,
                heading_depth,
            }
        })
        .collect();
    PreparedBlock {
        block_id: BlockId::from(Uuid::now_v7()),
        seq,
        role: role.map(str::to_owned),
        chunks,
        incorporated: Vec::new(),
    }
}

/// Prepare one block: chunk its prose, then embed every chunk in a single batched ONNX call.
pub fn prepare_block(seq: i32, role: Option<&str>, prose: &str) -> Result<PreparedBlock> {
    prepare_block_with_prefix(seq, role, prose, &[])
}

/// [`prepare_block`], seeding the chunker's heading breadcrumb from `breadcrumb` so a segment that
/// begins mid-section still carries its ancestor `header_path`. With an empty `breadcrumb` this is
/// byte-identical to [`prepare_block`]. Used by the segmented-ingest append path when the caller
/// supplied no pre-chunked content and the server must chunk the segment itself.
pub fn prepare_block_with_prefix(
    seq: i32,
    role: Option<&str>,
    prose: &str,
    breadcrumb: &[String],
) -> Result<PreparedBlock> {
    let planned = plan_chunks_with_prefix(prose, breadcrumb);
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
                let (header_path, heading_depth) = map_heading(header_path, heading_depth);
                PreparedChunk {
                    chunk_id: ChunkId::from(Uuid::now_v7()),
                    chunk_index,
                    content_hash,
                    content,
                    embedding: Some(embedding),
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
        incorporated: Vec::new(),
    })
}

/// Prepare one block **without embedding** — the async-embed twin of [`prepare_block`] (issue #299).
/// Chunks the prose with the same ONNX-free `plan_chunks` and maps headings identically, but emits
/// `embedding: None` on every chunk instead of running `embed_texts`. The chunk text, content hashes,
/// and body merkle are therefore byte-identical to `prepare_block`'s for the same prose — only the
/// vectors are absent — so a deferred create persists a fully FTS-searchable resource, and a later
/// backfill fills `kb_chunks.embedding` from the same chunk text. ONNX-free, so it never pays model
/// load on the request path.
pub fn prepare_block_deferred(seq: i32, role: Option<&str>, prose: &str) -> PreparedBlock {
    prepare_block_deferred_with_prefix(seq, role, prose, &[])
}

/// [`prepare_block_deferred`] with a seeded heading breadcrumb — the ONNX-free twin of
/// [`prepare_block_with_prefix`]. Chunks land with a NULL vector, backfilled off-request by the
/// embed drain (issue #299).
pub fn prepare_block_deferred_with_prefix(
    seq: i32,
    role: Option<&str>,
    prose: &str,
    breadcrumb: &[String],
) -> PreparedBlock {
    let chunks = plan_chunks_with_prefix(prose, breadcrumb)
        .into_iter()
        .map(
            |(chunk_index, content_hash, content, header_path, heading_depth)| {
                // Identical heading mapping to `prepare_block`: depth 0 / empty breadcrumb ⇒ unheaded
                // preamble (NULL columns); a real heading ⇒ persist depth + breadcrumb.
                let (header_path, heading_depth) = map_heading(header_path, heading_depth);
                PreparedChunk {
                    chunk_id: ChunkId::from(Uuid::now_v7()),
                    chunk_index,
                    content_hash,
                    content,
                    // Deferred: no vector yet. Backfilled off-request; NULL at the projector.
                    embedding: None,
                    header_path,
                    heading_depth,
                }
            },
        )
        .collect();
    PreparedBlock {
        block_id: BlockId::from(Uuid::now_v7()),
        seq,
        role: role.map(str::to_owned),
        chunks,
        incorporated: Vec::new(),
    }
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

/// The resource `body_hash` for a CALLER-SUPPLIED chunk set — the no-embed twin of [`body_hash_for_body`]
/// that reproduces its merkle from the chunks' content hashes directly (the create path persists the body
/// as ONE roleless block at seq 0, so the merkle is `sha256_hex(per_block_hash)`, where `per_block_hash =
/// sha256_hex(concat of the chunk content_hashes in chunk_index order)`). `chunk_hashes` MUST already be
/// in chunk_index order. This must equal `body_hash_for_body` for the same chunk-hash set so the create
/// dedup pre-check and the projector's stored `kb_resources.body_hash` stay consistent.
///
/// An empty set ⇒ `sha256_hex("")`, matching `body_hash_for_body`'s empty-body case (and the SQL's
/// coalesce-empty-aggregate-to-`''` in `_recompute_resource_body_hash`).
pub fn body_hash_from_chunk_hashes(chunk_hashes: &[String]) -> String {
    if chunk_hashes.is_empty() {
        return sha256_hex("");
    }
    let block_concat: String = chunk_hashes.concat();
    let block_hash = sha256_hex(&block_concat);
    sha256_hex(&block_hash)
}

/// The resource `body_hash` for a MULTI-BLOCK caller-supplied chunk set (the charter shape). Reproduces
/// `_recompute_resource_body_hash`'s two-level merkle: per block, `sha256_hex(concat of the block's chunk
/// content_hashes in chunk_index order)`; then the resource hash is `sha256_hex(concat of the per-block
/// hashes in block seq order)`. `blocks` MUST already be in seq order and each inner vec in chunk_index
/// order. An empty set ⇒ `sha256_hex("")` (the SQL coalesces the empty aggregate to `''`). For a single
/// block this equals [`body_hash_from_chunk_hashes`].
pub fn body_hash_from_block_chunk_hashes(blocks: &[Vec<String>]) -> String {
    if blocks.is_empty() {
        return sha256_hex("");
    }
    let per_block: String = blocks
        .iter()
        .map(|chunk_hashes| sha256_hex(&chunk_hashes.concat()))
        .collect();
    sha256_hex(&per_block)
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

    // The caller-supplied-chunk merkle MUST equal the chunk-the-prose merkle for the same chunk-hash
    // set, so a client that pre-chunks dedups against a server-chunked twin (and vice versa).
    #[test]
    fn body_hash_from_chunk_hashes_matches_body_hash_for_body() {
        let prose = "A short onboarding note about first-week confidence.";
        let planned = plan_chunks(prose);
        let hashes: Vec<String> = planned.iter().map(|(_, h, ..)| h.clone()).collect();
        assert_eq!(
            body_hash_from_chunk_hashes(&hashes),
            body_hash_for_body(prose),
            "supplied-chunk merkle must equal the chunk-the-prose merkle"
        );
    }

    // Empty chunk set ⇒ sha256_hex("") — the same value body_hash_for_body returns for an empty body.
    #[test]
    fn body_hash_from_empty_chunk_set_matches_empty_body() {
        assert_eq!(body_hash_from_chunk_hashes(&[]), body_hash_for_body(""));
    }

    // prepare_block_deferred chunks like prepare_block but emits `embedding: None` on every chunk —
    // the ONNX-free write half of the async-embed path. Content, hashes, index, and headings are
    // populated exactly as the embedded path would; only the vector is absent.
    #[test]
    fn prepare_block_deferred_emits_null_embeddings_with_full_text() {
        let block = prepare_block_deferred(0, None, "A short deferred-embedding note.");
        assert_eq!(block.chunks.len(), 1, "short prose ⇒ one chunk");
        let c = &block.chunks[0];
        assert!(c.embedding.is_none(), "deferred chunk carries no vector");
        assert_eq!(c.chunk_index, 0);
        assert_eq!(c.content_hash.len(), 64, "sha256 hex is 64 chars");
        assert!(c.content.contains("deferred-embedding"));
        // Unheaded preamble ⇒ NULL heading columns, same as prepare_block.
        assert_eq!(c.header_path, None);
        assert_eq!(c.heading_depth, None);
    }

    // A deferred block's chunk hashes reproduce the SAME body merkle as chunking the prose inline —
    // deferral changes only the vector, never the resource's body_hash identity, so dedup/readback
    // stay consistent whether a create embedded on-request or off.
    #[test]
    fn prepare_block_deferred_merkle_matches_inline_chunking() {
        let prose =
            "First paragraph of the note.\n\n## Details\n\nSecond paragraph with more text.";
        let deferred = prepare_block_deferred(0, None, prose);
        let deferred_hashes: Vec<String> = deferred
            .chunks
            .iter()
            .map(|c| c.content_hash.clone())
            .collect();
        assert_eq!(
            body_hash_from_block_chunk_hashes(&[deferred_hashes]),
            body_hash_for_body(prose),
            "deferred chunking must yield the same body merkle as inline chunking"
        );
    }

    // Empty prose ⇒ no chunks (same as prepare_block's empty-prose arm), so a deferred create of an
    // empty body writes a contentless block the write layer will reject upstream.
    #[test]
    fn prepare_block_deferred_empty_prose_yields_no_chunks() {
        assert!(prepare_block_deferred(0, None, "").chunks.is_empty());
    }

    // The no-regression guard for the breadcrumb-carrying variants: every existing single-block
    // caller passes no breadcrumb, and must be bit-for-bit unaffected by the new parameter.
    #[test]
    fn prefix_variants_with_empty_breadcrumb_match_the_originals() {
        let prose = "# Title\n\nalpha\n\n## Section\n\nbeta\n";
        let plain = prepare_block_deferred(0, None, prose);
        let prefixed = prepare_block_deferred_with_prefix(0, None, prose, &[]);

        assert_eq!(plain.chunks.len(), prefixed.chunks.len());
        for (a, b) in plain.chunks.iter().zip(prefixed.chunks.iter()) {
            assert_eq!(a.chunk_index, b.chunk_index);
            assert_eq!(a.content_hash, b.content_hash);
            assert_eq!(a.content, b.content);
            assert_eq!(a.header_path, b.header_path);
            assert_eq!(a.heading_depth, b.heading_depth);
        }
    }

    // A segment cut mid-section carries no heading of its own; without the seeded breadcrumb its
    // chunks would land with a NULL header_path, breaking search breadcrumbs across block
    // boundaries. This is the property the streaming segment boundary rests on.
    #[test]
    fn prefix_seeds_ancestor_breadcrumb_for_a_mid_section_segment() {
        let block = prepare_block_deferred_with_prefix(
            1,
            None,
            "beta continues here\n",
            &["Title".to_owned(), "Section".to_owned()],
        );

        assert_eq!(block.chunks.len(), 1);
        assert_eq!(
            block.chunks[0].header_path.as_deref(),
            Some("Title > Section"),
            "a mid-section segment must inherit its ancestor path"
        );
        assert_eq!(
            block.chunks[0].heading_depth, None,
            "a continuation chunk begins no section, so reconstruct_body must not re-emit a heading"
        );
    }

    // A true preamble — no heading of its own and no ancestors — still maps to NULL/NULL, so the
    // rule change is confined to the case that could not previously arise.
    #[test]
    fn unheaded_preamble_still_maps_to_null_columns() {
        let block = prepare_block_deferred_with_prefix(0, None, "just prose\n", &[]);
        assert_eq!(block.chunks[0].header_path, None);
        assert_eq!(block.chunks[0].heading_depth, None);
    }

    // Splitting a document at a heading and prefixing the tail must reproduce the same chunk
    // content hashes as chunking the whole document in one pass — the equivalence that lets a
    // segmented ingest and a one-shot create agree on body_hash.
    #[test]
    fn prefix_variant_merkle_matches_whole_document_chunking() {
        let whole = "# Title\n\nalpha\n\n## Section\n\nbeta\n";
        let head = "# Title\n\nalpha\n";
        let tail = "## Section\n\nbeta\n";

        let one_pass = prepare_block_deferred(0, None, whole);
        let b0 = prepare_block_deferred_with_prefix(0, None, head, &[]);
        let b1 = prepare_block_deferred_with_prefix(1, None, tail, &["Title".to_owned()]);

        let one_pass_hashes: Vec<&str> = one_pass
            .chunks
            .iter()
            .map(|c| c.content_hash.as_str())
            .collect();
        let split_hashes: Vec<&str> = b0
            .chunks
            .iter()
            .chain(b1.chunks.iter())
            .map(|c| c.content_hash.as_str())
            .collect();

        assert_eq!(one_pass_hashes, split_hashes);
    }

    // prepare_block_from_chunks carries the supplied embedding verbatim and maps headings like
    // prepare_block (depth 0 / empty breadcrumb ⇒ NULL; a real heading ⇒ Some).
    #[test]
    fn prepare_block_from_chunks_carries_embedding_and_maps_headings() {
        let block = prepare_block_from_chunks(
            0,
            None,
            vec![
                IncomingChunk {
                    chunk_index: 0,
                    content_hash: "ab".repeat(32),
                    content: "preamble".into(),
                    embedding: vec![0.5; 4],
                    header_path: String::new(),
                    heading_depth: 0,
                },
                IncomingChunk {
                    chunk_index: 1,
                    content_hash: "cd".repeat(32),
                    content: "headed".into(),
                    embedding: vec![0.25; 4],
                    header_path: "Intro > Goals".into(),
                    heading_depth: 2,
                },
            ],
        );
        assert_eq!(block.chunks.len(), 2);
        // unheaded preamble ⇒ NULL heading columns
        assert_eq!(block.chunks[0].header_path, None);
        assert_eq!(block.chunks[0].heading_depth, None);
        // embedding carried verbatim (no re-embed)
        assert_eq!(block.chunks[0].embedding, Some(vec![0.5; 4]));
        // a real heading is carried
        assert_eq!(
            block.chunks[1].header_path.as_deref(),
            Some("Intro > Goals")
        );
        assert_eq!(block.chunks[1].heading_depth, Some(2));
        assert_eq!(block.chunks[1].embedding, Some(vec![0.25; 4]));
    }

    #[test]
    fn block_chunk_hashes_single_block_matches_single_block_helper() {
        // One block ⇒ identical to the single-block helper (which assumes one roleless block).
        let hashes = vec!["aa".to_string(), "bb".to_string()];
        assert_eq!(
            body_hash_from_block_chunk_hashes(std::slice::from_ref(&hashes)),
            body_hash_from_chunk_hashes(&hashes),
        );
    }

    #[test]
    fn block_chunk_hashes_two_level_merkle() {
        // Two blocks: per-block sha256(concat), then resource sha256(concat per-block hashes).
        let b0 = vec!["aa".to_string()];
        let b1 = vec!["bb".to_string(), "cc".to_string()];
        let expect = {
            let h0 = sha256_hex("aa");
            let h1 = sha256_hex("bbcc");
            sha256_hex(&format!("{h0}{h1}"))
        };
        assert_eq!(body_hash_from_block_chunk_hashes(&[b0, b1]), expect);
    }

    #[test]
    fn block_chunk_hashes_empty_matches_empty_body() {
        assert_eq!(
            body_hash_from_block_chunk_hashes(&[]),
            body_hash_for_body("")
        );
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
                embedding: Some(vec![0.1, 0.2, 0.3]),
                header_path: None,
                heading_depth: None,
            }],
            incorporated: vec![],
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
