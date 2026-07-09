//! Shared business logic for cloud ingest operations.
//!
//! Pure helpers consumed by cloud-mode paths: body chunking, frontmatter
//! construction from server resources, body normalization, and URL fetch.
//! Manifest-coupled and local-vault helpers were removed in Chunk 7
//! (Tasks 5 + 8); the sync/manifest stack is retired in Task 7.

use crate::error::{Result, TemperError};

// ---------------------------------------------------------------------------
// One-shot vs segmented ingest mode (streaming-resumable ingestion, Beat 3)
// ---------------------------------------------------------------------------

/// Which create path a body of `source_len` bytes takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestMode {
    /// At or under the budget: the existing single-block `create_resource`/`/api/ingest`
    /// path, unchanged.
    OneShot,
    /// Over the budget: stream through `segment_reader` + the segmented begin/append/
    /// finalize endpoints.
    Segmented,
}

/// The size-threshold seam between the one-shot and segmented create paths — a body at or
/// under `budget` bytes is `OneShot`; anything larger is `Segmented`. Pure and side-effect
/// free so the threshold is unit-testable without a client/runtime; `run_segmented_create`
/// and its call site (`commands::resource::create`) are the wired consumer.
pub fn ingest_mode(source_len: usize, budget: usize) -> IngestMode {
    if source_len <= budget {
        IngestMode::OneShot
    } else {
        IngestMode::Segmented
    }
}

// ---------------------------------------------------------------------------
// Slug / body helpers
// ---------------------------------------------------------------------------

/// Slugify a title for use in URIs and slugs.
///
/// Delegates to `temper_workflow::operations::sluggify` — the one slug function,
/// shared with decorated-ref decoration so URIs/filenames and ref decorations
/// can never drift apart.
pub fn slug_from_title(title: &str) -> String {
    temper_workflow::operations::sluggify(title)
}

/// Body trio extracted from raw markdown — the chunk + hash output that
/// goes onto IngestPayload (cloud create) or ResourceUpdateRequest (cloud update).
#[derive(Debug)]
pub struct BodyChunks {
    pub content_hash: String,
    pub chunks_packed: String,
}

/// Compute (content_hash, chunks_packed) from raw markdown without
/// vault/manifest side effects. Single source of truth for chunk + hash
/// extraction; used by `cmd_to_ingest_payload` (cloud create) and the
/// cloud-mode update path in `cloud_backend/translators.rs`.
#[cfg(feature = "embed")]
pub fn compute_body_chunks(content: &str) -> Result<BodyChunks> {
    use temper_core::types::ingest::pack_chunks;
    use temper_ingest::pipeline::prepare_markdown;

    let content_hash = temper_core::hash::compute_body_hash(content);
    let packed_chunks = prepare_markdown(content)
        .map_err(|e| TemperError::Extraction(format!("embedding failed: {e}")))?;
    let chunks_packed = pack_chunks(&packed_chunks)
        .map_err(|e| TemperError::Extraction(format!("chunk packing failed: {e}")))?;
    Ok(BodyChunks {
        content_hash,
        chunks_packed,
    })
}

// ---------------------------------------------------------------------------
// Segmented (streaming) create orchestration (Beat 3)
// ---------------------------------------------------------------------------

/// Chunk `text` (already prefix-seeded by the caller) and embed+pack it into the
/// `chunks_packed` wire format — the per-segment twin of
/// `temper_ingest::pipeline::prepare_markdown`, operating on chunks the caller already
/// produced (via `chunk_markdown_with_prefix`) rather than re-chunking, since
/// `prepare_markdown` only knows the unprefixed `chunk_markdown`.
#[cfg(feature = "embed")]
fn embed_and_pack(chunks: &[temper_ingest::chunk::ChunkData]) -> Result<String> {
    use temper_core::types::ingest::{pack_chunks, PackedChunk};

    if chunks.is_empty() {
        return Err(TemperError::Extraction(
            "segment produced no chunks".to_string(),
        ));
    }
    let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
    let embeddings = temper_ingest::embed::embed_texts(&texts)
        .map_err(|e| TemperError::Extraction(format!("embed segment: {e}")))?;
    if embeddings.len() != chunks.len() {
        return Err(TemperError::Extraction(format!(
            "chunk/embedding count mismatch: {} chunks, {} embeddings",
            chunks.len(),
            embeddings.len()
        )));
    }
    let packed: Vec<PackedChunk> = chunks
        .iter()
        .zip(embeddings)
        .map(|(chunk, embedding)| PackedChunk {
            chunk_index: chunk.chunk_index,
            header_path: chunk.header_path.clone(),
            heading_depth: chunk.heading_depth,
            content: chunk.content.clone(),
            content_hash: chunk.content_hash.clone(),
            embedding,
        })
        .collect();
    pack_chunks(&packed).map_err(|e| TemperError::Extraction(format!("pack segment chunks: {e}")))
}

/// The local, no-network plan for a segmented create: every segment the source splits into,
/// each one's already-chunked `ChunkData` (needed later for embedding), its local `(seq,
/// block-merkle)` identity (matching what the server will report back for the same segment,
/// per `temper_ingest::merkle`), and the resource-level `body_hash` finalize will validate
/// against. Computing this is cheap (chunking only, no embedding) — it exists so
/// `run_segmented_create` can diff against a resumed session's landed set *before* doing any
/// expensive work, and so this planning step is unit-testable without a client or runtime.
#[cfg(feature = "embed")]
struct SegmentPlan {
    segments: Vec<temper_ingest::stream::Segment>,
    chunked: Vec<Vec<temper_ingest::chunk::ChunkData>>,
    infos: Vec<temper_core::types::ingest::SegmentInfo>,
    expected_body_hash: String,
}

/// Split `content` into segments (`temper_ingest::stream::segment_reader`) and chunk each one
/// (`temper_ingest::chunk::chunk_markdown_with_prefix`) up front. Pure aside from the
/// `io::Result` `segment_reader` threads through (there is no actual I/O for an in-memory
/// `Cursor` source — it never fails in practice, but the reader is generic over any
/// `BufRead`).
#[cfg(feature = "embed")]
fn plan_segments(content: &str, budget: usize) -> Result<SegmentPlan> {
    use temper_core::types::ingest::SegmentInfo;

    let segments: Vec<temper_ingest::stream::Segment> =
        temper_ingest::stream::segment_reader(std::io::Cursor::new(content.as_bytes()), budget)
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|e| TemperError::Extraction(format!("segment source: {e}")))?;
    if segments.is_empty() {
        return Err(TemperError::Extraction(
            "segmented create with an empty body".to_string(),
        ));
    }

    let mut infos: Vec<SegmentInfo> = Vec::with_capacity(segments.len());
    let mut chunked: Vec<Vec<temper_ingest::chunk::ChunkData>> = Vec::with_capacity(segments.len());
    for seg in &segments {
        let chunks =
            temper_ingest::chunk::chunk_markdown_with_prefix(&seg.text, &seg.initial_breadcrumb);
        let chunk_hashes: Vec<String> = chunks.iter().map(|c| c.content_hash.clone()).collect();
        infos.push(SegmentInfo {
            seq: seg.seq,
            content_hash: temper_ingest::merkle::block_merkle(&chunk_hashes),
        });
        chunked.push(chunks);
    }
    let expected_body_hash = temper_ingest::merkle::resource_body_hash(
        &infos
            .iter()
            .map(|s| s.content_hash.clone())
            .collect::<Vec<_>>(),
    );

    Ok(SegmentPlan {
        segments,
        chunked,
        infos,
        expected_body_hash,
    })
}

/// Parameters for [`run_segmented_create`], bundled per the >5-domain-params rule. Everything
/// the segmented orchestration needs beyond the CLI's already-built `CreateResource` command:
/// the client to dispatch through, the vault root for the resume manifest, the resolved
/// context ref (mirrors `CloudBackend.context_ref`; forwarded verbatim to the translator,
/// which empties it for a cogmap home exactly like the one-shot path), and the segment
/// budget.
#[cfg(feature = "embed")]
pub struct SegmentedCreateParams<'a> {
    pub client: &'a temper_client::TemperClient,
    pub vault_root: &'a std::path::Path,
    pub cmd: &'a temper_workflow::operations::CreateResource,
    pub context_ref: &'a str,
    pub budget: usize,
}

/// Stream a large body (`cmd.body` over `budget` bytes) through the segmented ingest
/// endpoints: segment 0 lands via `begin_segmented` (the create path), segments `1..N` via
/// `append_block`, then `finalize`. Writes the `.temper/` resume manifest after every landed
/// segment.
///
/// Resumes an interrupted attempt for the *same* source: if an incomplete manifest already
/// matches this source's hash + budget (`ingest_manifest::find_resumable`), its `resource_id`
/// is reused and only the segments `GET .../blocks` doesn't already report are re-chunked,
/// re-embedded, and appended — durable segments are neither re-embedded nor re-sent. A body
/// whose freshly-computed hash doesn't match any local manifest simply begins a fresh session
/// (there is nothing to "clear": a stale manifest for since-changed content just never matches
/// and is left alone — see `find_resumable`'s doc comment).
///
/// Peak memory holds one segment's text + chunks + vectors at a time, never the whole body:
/// segments are chunked up front (cheap — no embedding) to compute the local `(seq,
/// content_hash)` identity used for the resume diff, but embedding/packing only runs for a
/// segment actually being sent.
#[cfg(feature = "embed")]
pub async fn run_segmented_create(
    params: SegmentedCreateParams<'_>,
) -> Result<temper_workflow::types::ResourceRow> {
    use temper_core::types::ingest::{AppendBlockPayload, FinalizePayload, SegmentedBegin};

    let SegmentedCreateParams {
        client,
        vault_root,
        cmd,
        context_ref,
        budget,
    } = params;

    let content = cmd
        .body
        .as_ref()
        .map(|b| b.content.as_str())
        .unwrap_or_default();
    let source_hash = temper_core::hash::sha256_hex(content.as_bytes());

    let SegmentPlan {
        segments,
        chunked,
        infos: local_infos,
        expected_body_hash,
    } = plan_segments(content, budget)?;

    let existing =
        crate::actions::ingest_manifest::find_resumable(vault_root, &source_hash, budget as u32)?;

    let (resource_id, mut manifest, landed) = match existing {
        Some((resource_id, mut manifest)) => {
            // Re-verify against the live server rather than trusting the on-disk cache — the
            // manifest may be stale if a prior attempt crashed between a server-side landing
            // and the next `store` call.
            let landed = client
                .ingest()
                .list_blocks(resource_id)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?
                .blocks;
            manifest.blocks = landed.clone();
            (resource_id, manifest, landed)
        }
        None => {
            // Fresh session: segment 0 always lands via begin_segmented (the create path) —
            // there is no resource to append a block to before this call returns one.
            let chunks_packed = embed_and_pack(&chunked[0])?;
            let segmented = SegmentedBegin {
                total_blocks_hint: Some(segments.len() as u32),
                block_budget: budget as u32,
                source_hash: Some(source_hash.clone()),
            };
            let payload = crate::cloud_backend::translators::cmd_to_segmented_begin_payload(
                cmd,
                context_ref,
                segments[0].text.clone(),
                chunks_packed,
                segmented,
            )?;
            let response = client
                .ingest()
                .begin_segmented(&payload)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            let manifest = crate::actions::ingest_manifest::IngestManifest {
                resource_id: response.resource_id,
                source_hash: source_hash.clone(),
                block_budget: budget as u32,
                correlation_id: response.correlation_id,
                blocks: response.blocks.clone(),
                finalized: false,
            };
            crate::actions::ingest_manifest::store(
                &crate::actions::ingest_manifest::manifest_path(vault_root, response.resource_id),
                &manifest,
            )?;
            (response.resource_id, manifest, response.blocks)
        }
    };

    let missing_seqs = crate::actions::ingest_manifest::resume_gap(&local_infos, &landed);
    for seq in missing_seqs {
        let idx = seq as usize;
        let chunks_packed = embed_and_pack(&chunked[idx])?;
        let append_payload = AppendBlockPayload {
            seq,
            content: segments[idx].text.clone(),
            content_hash: temper_core::hash::sha256_hex(segments[idx].text.as_bytes()),
            chunks_packed,
        };
        let response = client
            .ingest()
            .append_block(resource_id, &append_payload)
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?;
        manifest.blocks = response.blocks;
        let path = crate::actions::ingest_manifest::manifest_path(vault_root, resource_id);
        crate::actions::ingest_manifest::store(&path, &manifest)?;
    }

    client
        .ingest()
        .finalize(
            resource_id,
            &FinalizePayload {
                expected_blocks: segments.len() as u32,
                expected_body_hash,
            },
        )
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;

    manifest.finalized = true;
    let path = crate::actions::ingest_manifest::manifest_path(vault_root, resource_id);
    crate::actions::ingest_manifest::store(&path, &manifest)?;

    client
        .resources()
        .get(resource_id)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)
}

// ---------------------------------------------------------------------------
// URL fetch
// ---------------------------------------------------------------------------

/// Fetch a URL to a temporary file, returning the path and inferred filename.
///
/// The response body is written to a temp file with the appropriate extension
/// (`.html` for HTML content, derived from URL path otherwise). The temp file
/// persists as long as the returned `TempPath` is alive.
pub async fn fetch_url_to_tempfile(url: &str) -> Result<(tempfile::TempPath, String)> {
    let response = reqwest::get(url)
        .await
        .map_err(|e| TemperError::Api(format!("fetch {url}: {e}")))?;

    if !response.status().is_success() {
        return Err(TemperError::Api(format!(
            "fetch {url}: HTTP {}",
            response.status()
        )));
    }

    // Determine file extension from content-type or URL path.
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let extension = extension_from_content_type(&content_type)
        .or_else(|| extension_from_url(url))
        .unwrap_or("html");

    // Derive a display name from the URL path.
    let display_name = display_name_from_url(url);

    let mut tmp = tempfile::Builder::new()
        .suffix(&format!(".{extension}"))
        .tempfile()
        .map_err(|e| TemperError::Extraction(format!("create temp file: {e}")))?;

    let bytes = response
        .bytes()
        .await
        .map_err(|e| TemperError::Api(format!("read response body: {e}")))?;

    std::io::Write::write_all(&mut tmp, &bytes)
        .map_err(|e| TemperError::Extraction(format!("write temp file: {e}")))?;

    let path = tmp.into_temp_path();
    Ok((path, display_name))
}

/// Map a Content-Type header to a file extension.
fn extension_from_content_type(ct: &str) -> Option<&'static str> {
    let ct = ct.split(';').next().unwrap_or("").trim();
    match ct {
        "text/html" => Some("html"),
        "text/plain" => Some("txt"),
        "text/markdown" => Some("md"),
        "application/pdf" => Some("pdf"),
        _ => None,
    }
}

/// Extract a file extension from the URL path.
fn extension_from_url(url: &str) -> Option<&'static str> {
    let path = url.split('?').next().unwrap_or(url);
    let last_segment = path.rsplit('/').next().unwrap_or("");
    let ext = last_segment.rsplit('.').next().unwrap_or("");
    match ext {
        "html" | "htm" => Some("html"),
        "md" | "markdown" => Some("md"),
        "txt" => Some("txt"),
        "pdf" => Some("pdf"),
        _ => None,
    }
}

/// Derive a human-readable display name from a URL.
fn display_name_from_url(url: &str) -> String {
    let path = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split('?')
        .next()
        .unwrap_or(url);
    // Use the last meaningful path segment, or the domain
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segments.last() {
        Some(&seg) if seg.contains('.') => {
            // Strip extension for title
            seg.rsplit_once('.')
                .map(|(name, _)| name)
                .unwrap_or(seg)
                .to_string()
        }
        Some(&seg) => seg.to_string(),
        None => path.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Frontmatter construction
// ---------------------------------------------------------------------------

/// Build a complete `Frontmatter` from a server `ResourceRow` plus the
/// caller-resolved canonical owner sigil.
///
/// `canonical_owner` is the value to write into `temper-owner`. The caller
/// is responsible for resolving the API's `@me` shorthand to
/// `@<profile.slug>`.
///
/// Combines resource-level fields (id, type, context, created, title) with
/// managed_meta fields (temper-* keys, stage, mode, effort, etc.) and
/// open_meta fields (user-defined keys: tags, relates_to, extends,
/// depends_on, and any other custom frontmatter) for complete frontmatter
/// that matches what the CLI would produce locally.
/// Bundled inputs for [`build_frontmatter_from_resource`]. `resource`,
/// `context`, `doc_type`, and `canonical_owner` are borrowed; `body` is
/// owned because it is moved into the new `Frontmatter`. `managed_meta` and
/// `open_meta` are the optional two metadata tiers.
#[derive(Debug)]
pub struct BuildFrontmatterParams<'a> {
    pub resource: &'a temper_workflow::types::ResourceRow,
    pub context: &'a str,
    pub doc_type: &'a str,
    pub canonical_owner: &'a str,
    pub body: String,
    pub managed_meta: Option<&'a serde_json::Value>,
    pub open_meta: Option<&'a serde_json::Value>,
}

pub fn build_frontmatter_from_resource(
    params: BuildFrontmatterParams<'_>,
) -> crate::error::Result<temper_workflow::frontmatter::Frontmatter> {
    use temper_workflow::frontmatter::{DocType, Frontmatter};

    let BuildFrontmatterParams {
        resource,
        context,
        doc_type,
        canonical_owner,
        body,
        managed_meta,
        open_meta,
    } = params;

    // Open tail (Task A2): don't fail-fast on an unrecognized doc_type — the
    // enum is only used (as `Frontmatter::new`'s seed) when recognized. For
    // an open-tail label, `Frontmatter::new` still needs *some* variant to
    // construct (its typed `doc_type` has no "unknown" case), so `Task` is
    // used as an inert scaffold and immediately overwritten below with the
    // real label. This local-vault projection is a non-authoritative cache
    // (see CLAUDE.md); the written `temper-type` value is what round-trips.
    let dt = DocType::from_str(doc_type);
    let is_open_tail = dt.is_err();
    let mut fm = Frontmatter::new(dt.unwrap_or(DocType::Task), body);
    if is_open_tail {
        fm.set_managed_field(
            "temper-type",
            serde_json::Value::String(doc_type.to_string()),
        );
    }
    fm.set_managed_field(
        "temper-id",
        serde_json::Value::String(resource.id.to_string()),
    );
    fm.set_managed_field(
        "temper-context",
        serde_json::Value::String(context.to_string()),
    );
    fm.set_managed_field(
        "temper-created",
        serde_json::Value::String(resource.created.to_rfc3339()),
    );
    fm.set_managed_field(
        "temper-title",
        serde_json::Value::String(resource.title.clone()),
    );
    if !canonical_owner.is_empty() {
        fm.set_managed_field(
            "temper-owner",
            serde_json::Value::String(canonical_owner.to_string()),
        );
    }
    if let Some(obj) = managed_meta.and_then(|m| m.as_object()) {
        for (k, v) in obj {
            // System fields plus temper-title/temper-slug are set above from
            // resource-row columns; skip them as defense-in-depth so a drifted
            // managed_meta payload can't overwrite the canonical values.
            if temper_workflow::frontmatter::fields::SYSTEM_MANAGED_FIELDS.contains(&k.as_str())
                || k == "temper-title"
            {
                continue;
            }
            fm.set_managed_field(k, v.clone());
        }
    }
    if let Some(obj) = open_meta.and_then(|m| m.as_object()) {
        for (k, v) in obj {
            fm.set_open_field(k, v.clone());
        }
    }
    Ok(fm)
}

/// Normalize the markdown body to include the blank-line separator the
/// historical text-level `build_frontmatter` emitted between the closing
/// `---` and the first content line.
///
/// Old flow: `format!("---\n<yaml>---\n\n{content}")` — always a blank
/// line between the frontmatter fence and the body.
///
/// New flow: `Frontmatter::serialize()` produces `---\n<yaml>---\n{body}`,
/// so the caller must include the leading newline to preserve the
/// separator. This helper does that normalization conservatively: prepend
/// `\n` only if the content doesn't already start with one.
pub fn normalize_body_for_vault(content: &str) -> String {
    if content.is_empty() || content.starts_with('\n') {
        content.to_string()
    } else {
        format!("\n{content}")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- ingest_mode (one-shot vs segmented threshold) ---

    #[test]
    fn ingest_mode_at_or_under_budget_is_one_shot() {
        assert_eq!(ingest_mode(0, 262_144), IngestMode::OneShot);
        assert_eq!(ingest_mode(262_144, 262_144), IngestMode::OneShot);
    }

    #[test]
    fn ingest_mode_over_budget_is_segmented() {
        assert_eq!(ingest_mode(262_145, 262_144), IngestMode::Segmented);
        assert_eq!(ingest_mode(10_000_000, 262_144), IngestMode::Segmented);
    }

    #[test]
    fn ingest_mode_respects_a_custom_budget() {
        assert_eq!(ingest_mode(100, 100), IngestMode::OneShot);
        assert_eq!(ingest_mode(101, 100), IngestMode::Segmented);
    }

    // --- plan_segments (local, no-network segmented-create planning) ---
    //
    // No ONNX runtime needed — chunking + merkle hashing only, no embedding — so these run
    // under plain `embed` (unlike the `test-embed`-gated `compute_body_chunks` tests below).

    #[cfg(feature = "embed")]
    #[test]
    fn plan_segments_splits_a_large_body_and_carries_breadcrumbs() {
        let content = "# Top\n\n".to_string() + &"filler line\n".repeat(200);
        let plan = plan_segments(&content, 256).expect("should succeed");

        assert!(plan.segments.len() > 1, "expected a mid-section split");
        assert_eq!(plan.segments.len(), plan.chunked.len());
        assert_eq!(plan.segments.len(), plan.infos.len());
        for (i, seg) in plan.segments.iter().enumerate() {
            assert_eq!(seg.seq as usize, i, "segments are seq-ordered");
        }
        assert_eq!(
            plan.infos[1].content_hash,
            temper_ingest::merkle::block_merkle(
                &plan.chunked[1]
                    .iter()
                    .map(|c| c.content_hash.clone())
                    .collect::<Vec<_>>()
            ),
            "each segment's local content_hash is its own block merkle"
        );
    }

    #[cfg(feature = "embed")]
    #[test]
    fn plan_segments_small_body_is_a_single_segment() {
        let plan = plan_segments("# H\n\nshort", 262_144).expect("should succeed");
        assert_eq!(plan.segments.len(), 1);
        assert_eq!(plan.infos.len(), 1);
        assert_eq!(plan.infos[0].seq, 0);
        // For exactly one segment, the resource body_hash is the double-sha256 over that
        // segment's own chunk-hash concatenation (see `merkle::resource_body_hash`'s doc).
        assert_eq!(
            plan.expected_body_hash,
            temper_ingest::merkle::resource_body_hash(std::slice::from_ref(
                &plan.infos[0].content_hash
            ))
        );
    }

    #[cfg(feature = "embed")]
    #[test]
    fn plan_segments_rejects_an_empty_body() {
        assert!(plan_segments("", 262_144).is_err());
    }

    #[cfg(feature = "embed")]
    #[test]
    fn plan_segments_is_deterministic() {
        let content = "# A\n".to_string() + &"body text here\n".repeat(500);
        let first = plan_segments(&content, 8192).expect("should succeed");
        let second = plan_segments(&content, 8192).expect("should succeed");
        assert_eq!(first.infos.len(), second.infos.len());
        for (a, b) in first.infos.iter().zip(second.infos.iter()) {
            assert_eq!(a.seq, b.seq);
            assert_eq!(a.content_hash, b.content_hash);
        }
        assert_eq!(first.expected_body_hash, second.expected_body_hash);
    }

    // --- Content hash ---

    #[test]
    fn content_hash_is_deterministic() {
        let content = "# Hello\n\nThis is a test document.\n";
        let hash1 = temper_core::hash::compute_body_hash(content);
        let hash2 = temper_core::hash::compute_body_hash(content);
        assert_eq!(hash1, hash2);
        assert!(hash1.starts_with("sha256:"));
        // "sha256:" prefix (7 chars) + 64 hex chars = 71 total
        assert_eq!(hash1.len(), 71);
    }

    #[test]
    fn content_hash_differs_for_different_content() {
        let hash_a = temper_core::hash::compute_body_hash("content A");
        let hash_b = temper_core::hash::compute_body_hash("content B");
        assert_ne!(hash_a, hash_b);
    }

    #[test]
    fn content_hash_has_sha256_prefix() {
        let hash = temper_core::hash::compute_body_hash("test");
        assert!(hash.starts_with("sha256:"));
        let hex_part = &hash[7..];
        assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(hex_part.chars().all(|c| !c.is_uppercase()));
    }

    // --- URL helpers ---

    #[test]
    fn extension_from_content_type_html() {
        assert_eq!(extension_from_content_type("text/html"), Some("html"));
        assert_eq!(
            extension_from_content_type("text/html; charset=utf-8"),
            Some("html")
        );
    }

    #[test]
    fn extension_from_content_type_plain() {
        assert_eq!(extension_from_content_type("text/plain"), Some("txt"));
    }

    #[test]
    fn extension_from_content_type_unknown() {
        assert_eq!(extension_from_content_type("application/json"), None);
        assert_eq!(extension_from_content_type(""), None);
    }

    #[test]
    fn extension_from_url_with_extension() {
        assert_eq!(
            extension_from_url("https://example.com/docs/guide.html"),
            Some("html")
        );
        assert_eq!(
            extension_from_url("https://example.com/paper.pdf"),
            Some("pdf")
        );
    }

    #[test]
    fn extension_from_url_no_extension() {
        assert_eq!(extension_from_url("https://example.com/docs/guide"), None);
        assert_eq!(extension_from_url("https://example.com/"), None);
    }

    #[test]
    fn extension_from_url_with_query() {
        assert_eq!(
            extension_from_url("https://example.com/doc.html?version=2"),
            Some("html")
        );
    }

    #[test]
    fn display_name_from_url_path_segment() {
        assert_eq!(
            display_name_from_url("https://example.com/docs/getting-started.html"),
            "getting-started"
        );
    }

    #[test]
    fn display_name_from_url_no_extension() {
        assert_eq!(display_name_from_url("https://example.com/about"), "about");
    }

    #[test]
    fn display_name_from_url_root() {
        // Domain "example.com" is treated as a filename — dot stripped → "example"
        assert_eq!(display_name_from_url("https://example.com/"), "example");
    }

    fn test_resource_row() -> temper_workflow::types::ResourceRow {
        use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
        temper_workflow::types::ResourceRow {
            id: ResourceId(uuid::Uuid::nil()),
            kb_context_id: Some(ContextId(uuid::Uuid::nil())),
            origin_uri: "test://origin".to_string(),
            title: "Test".to_string(),
            originator_profile_id: ProfileId(uuid::Uuid::nil()),
            owner_profile_id: ProfileId(uuid::Uuid::nil()),
            is_active: true,
            created: chrono::Utc::now(),
            updated: chrono::Utc::now(),
            context_name: Some("temper".to_string()),
            doc_type_name: "research".to_string(),
            owner_handle: "@me".to_string(),
            context_slug: Some("temper".to_string()),
            context_owner_ref: Some("@me".to_string()),
            cogmap_id: None,
            cogmap_name: None,
            stage: None,
            seq: None,
            mode: None,
            effort: None,
            body_hash: None,
        }
    }

    #[test]
    fn build_frontmatter_from_resource_writes_canonical_owner_for_at_me() {
        let resource = test_resource_row();
        // Caller is responsible for resolving @me -> @<slug> before calling.

        let fm = build_frontmatter_from_resource(BuildFrontmatterParams {
            resource: &resource,
            context: "temper",
            doc_type: "research",
            canonical_owner: "@j-cole-taylor",
            body: String::new(),
            managed_meta: None,
            open_meta: None,
        })
        .unwrap();

        let owner = fm
            .value()
            .get("temper-owner")
            .and_then(|v| v.as_str())
            .expect("temper-owner must be set");
        assert_eq!(
            owner, "@j-cole-taylor",
            "frontmatter must record the canonical owner the caller passed in, \
             not the API's @me shorthand"
        );
    }

    #[test]
    fn build_frontmatter_from_resource_passes_team_handle_through() {
        let resource = test_resource_row();

        let fm = build_frontmatter_from_resource(BuildFrontmatterParams {
            resource: &resource,
            context: "temper",
            doc_type: "research",
            canonical_owner: "+platform-eng",
            body: String::new(),
            managed_meta: None,
            open_meta: None,
        })
        .unwrap();

        let owner = fm
            .value()
            .get("temper-owner")
            .and_then(|v| v.as_str())
            .expect("temper-owner must be set");
        assert_eq!(owner, "+platform-eng");
    }

    #[test]
    fn build_frontmatter_from_resource_open_tail_doctype_passes_through() {
        // Open tail (Task A2): an unrecognized doc_type must not fail-fast
        // when projecting a resource fetched from the server into the local
        // vault cache — the written temper-type reflects the real label.
        let resource = test_resource_row();

        let fm = build_frontmatter_from_resource(BuildFrontmatterParams {
            resource: &resource,
            context: "temper",
            doc_type: "anecdote",
            canonical_owner: "@j-cole-taylor",
            body: String::new(),
            managed_meta: None,
            open_meta: None,
        })
        .expect("open-tail doctype should not fail-fast");

        let temper_type = fm
            .value()
            .get("temper-type")
            .and_then(|v| v.as_str())
            .expect("temper-type must be set");
        assert_eq!(
            temper_type, "anecdote",
            "unrecognized doctype must round-trip verbatim into the projected file"
        );
    }

    #[test]
    fn test_build_frontmatter_from_resource_preserves_arrays_and_objects() {
        let resource = test_resource_row();

        let meta = serde_json::json!({
            "depends_on": ["slug-a", "slug-b"],
            "extends": ["parent-doc"],
            "tags": ["rust", "graph"],
            "config": {"key": "value", "nested": true}
        });

        let fm = build_frontmatter_from_resource(BuildFrontmatterParams {
            resource: &resource,
            context: "temper",
            doc_type: "research",
            canonical_owner: "@me",
            body: String::new(),
            managed_meta: Some(&meta),
            open_meta: None,
        })
        .unwrap();
        let v = fm.value();

        let depends = v
            .get("depends_on")
            .and_then(|x| x.as_sequence())
            .expect("depends_on should be a sequence");
        let slugs: Vec<&str> = depends.iter().filter_map(|x| x.as_str()).collect();
        assert!(
            slugs.contains(&"slug-a"),
            "depends_on should contain slug-a. Got:\n{v:?}"
        );
        assert!(
            slugs.contains(&"slug-b"),
            "depends_on should contain slug-b. Got:\n{v:?}"
        );
        assert!(
            v.get("extends").is_some(),
            "extends array should be present. Got:\n{v:?}"
        );
        assert!(
            v.get("config").is_some(),
            "config object should be present. Got:\n{v:?}"
        );
    }

    #[test]
    fn test_build_frontmatter_emits_open_meta_arrays() {
        let resource = test_resource_row();

        let open_meta = serde_json::json!({
            "relates_to": ["task://foo", "task://bar"],
            "tags": ["alpha", "beta"],
        });

        let fm = build_frontmatter_from_resource(BuildFrontmatterParams {
            resource: &resource,
            context: "temper",
            doc_type: "research",
            canonical_owner: "@me",
            body: String::new(),
            managed_meta: None,
            open_meta: Some(&open_meta),
        })
        .unwrap();
        let v = fm.value();

        let relates = v
            .get("relates_to")
            .and_then(|x| x.as_sequence())
            .expect("relates_to should be a sequence");
        let entries: Vec<&str> = relates.iter().filter_map(|x| x.as_str()).collect();
        assert!(
            entries.contains(&"task://foo"),
            "relates_to should contain task://foo. Got:\n{v:?}"
        );
        assert!(
            entries.contains(&"task://bar"),
            "relates_to should contain task://bar. Got:\n{v:?}"
        );
        let tags = v
            .get("tags")
            .and_then(|x| x.as_sequence())
            .expect("tags should be a sequence");
        let tag_strs: Vec<&str> = tags.iter().filter_map(|x| x.as_str()).collect();
        assert!(
            tag_strs.contains(&"alpha"),
            "tags should contain alpha. Got:\n{v:?}"
        );
        assert!(
            tag_strs.contains(&"beta"),
            "tags should contain beta. Got:\n{v:?}"
        );
    }

    #[test]
    fn test_build_frontmatter_emits_open_meta_nested_objects() {
        let resource = test_resource_row();

        let open_meta = serde_json::json!({
            "custom_block": {"key": "value", "nested": {"inner": true}},
        });

        let fm = build_frontmatter_from_resource(BuildFrontmatterParams {
            resource: &resource,
            context: "temper",
            doc_type: "research",
            canonical_owner: "@me",
            body: String::new(),
            managed_meta: None,
            open_meta: Some(&open_meta),
        })
        .unwrap();
        let v = fm.value();

        let block = v
            .get("custom_block")
            .expect("custom_block should be present");
        assert_eq!(
            block.get("key").and_then(|x| x.as_str()),
            Some("value"),
            "nested key should be 'value'. Got:\n{block:?}"
        );
        let nested = block.get("nested").expect("nested should be present");
        assert_eq!(
            nested.get("inner").and_then(|x| x.as_bool()),
            Some(true),
            "deeply nested inner should be true. Got:\n{nested:?}"
        );
    }

    #[test]
    fn test_build_frontmatter_emits_both_tiers() {
        let resource = test_resource_row();

        let managed_meta = serde_json::json!({
            "stage": "draft",
            "effort": "M",
        });
        let open_meta = serde_json::json!({
            "relates_to": ["task://alpha"],
            "custom_tag": "hello",
        });

        let fm = build_frontmatter_from_resource(BuildFrontmatterParams {
            resource: &resource,
            context: "temper",
            doc_type: "research",
            canonical_owner: "@me",
            body: String::new(),
            managed_meta: Some(&managed_meta),
            open_meta: Some(&open_meta),
        })
        .unwrap();
        let v = fm.value();

        // Both tiers present
        assert!(
            v.get("stage").is_some(),
            "managed stage missing. Got:\n{v:?}"
        );
        assert!(
            v.get("effort").is_some(),
            "managed effort missing. Got:\n{v:?}"
        );
        assert!(
            v.get("relates_to").is_some(),
            "open relates_to missing. Got:\n{v:?}"
        );
        assert!(
            v.get("custom_tag").is_some(),
            "open custom_tag missing. Got:\n{v:?}"
        );

        // Canonical serialization places known open fields (Tier 3) before
        // schema-extra managed fields (Tier 4). Verify that identity/system
        // fields come before everything else — that's the invariant the
        // canonical ordering function guarantees.
        let serialized = fm.serialize().unwrap();
        let id_pos = serialized.find("temper-id:").expect("temper-id: present");
        let stage_pos = serialized.find("stage:").expect("stage: present");
        let effort_pos = serialized.find("effort:").expect("effort: present");
        let relates_pos = serialized.find("relates_to:").expect("relates_to: present");
        // Identity field must precede all data fields.
        assert!(
            id_pos < stage_pos.min(effort_pos).min(relates_pos),
            "identity fields must precede data fields. Got:\n{serialized}"
        );
    }

    #[test]
    #[cfg(feature = "test-embed")]
    fn compute_body_chunks_returns_hash_and_packed_chunks() {
        let content = "# Heading\n\nParagraph one.\n\nParagraph two.";
        let result = compute_body_chunks(content).expect("compute should succeed");
        assert_eq!(
            result.content_hash,
            temper_core::hash::compute_body_hash(content)
        );
        assert!(!result.chunks_packed.is_empty());
    }

    #[test]
    fn test_build_frontmatter_tolerates_none_open_meta() {
        let resource = test_resource_row();

        let managed_meta = serde_json::json!({
            "stage": "draft",
            "effort": "M",
        });

        let fm = build_frontmatter_from_resource(BuildFrontmatterParams {
            resource: &resource,
            context: "temper",
            doc_type: "research",
            canonical_owner: "@me",
            body: String::new(),
            managed_meta: Some(&managed_meta),
            open_meta: None,
        })
        .unwrap();
        let v = fm.value();

        assert_eq!(
            v.get("stage").and_then(|x| x.as_str()),
            Some("draft"),
            "stage should be rendered. Got:\n{v:?}"
        );
        assert_eq!(
            v.get("effort").and_then(|x| x.as_str()),
            Some("M"),
            "effort should be rendered. Got:\n{v:?}"
        );
        // Serialized form should have no blank lines inside the frontmatter block.
        let serialized = fm.serialize().unwrap();
        let inside = serialized
            .strip_prefix("---\n")
            .expect("leading ---")
            .split("\n---\n")
            .next()
            .expect("closing ---");
        for line in inside.lines() {
            assert!(
                !line.trim().is_empty(),
                "no blank lines expected inside frontmatter. Got:\n{serialized}"
            );
        }
    }
}
