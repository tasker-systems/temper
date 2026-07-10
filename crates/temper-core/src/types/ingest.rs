//! Ingest API types — wire format for CLI → Axum ingest pipeline.

use serde::{Deserialize, Serialize};

use crate::types::authorship::ActInput;

/// Wire payload for POST /api/ingest — resource + pre-processed chunks.
///
/// The CLI performs extract → chunk → embed locally and sends everything
/// in a single request. `chunks_packed` is a base64-encoded MessagePack
/// blob containing `Vec<PackedChunk>`. Both `content_hash` and `chunks_packed`
/// are optional: if absent the server computes them via the ingest pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct IngestPayload {
    pub title: String,
    pub origin_uri: String,
    /// Context **ref** (UUID or `@owner/slug`), resolved server-side.
    /// Bare names (no `@` prefix, not a UUID) are rejected with 400.
    pub context_ref: String,
    /// When set, the resource is homed in this cognitive map (`anchor_table='kb_cogmaps'`)
    /// and `context_ref` is ignored. Resolved client-side (cogmap refs are trailing-UUID-only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_cogmap_id: Option<uuid::Uuid>,
    pub doc_type_name: String,
    /// First-class goal link: the resolved `ResourceId` (as UUID) of the goal this
    /// resource advances. The CLI/MCP resolve the caller's `--goal <ref>` client-side
    /// (trailing-UUID-only); the ingest handler projects it to a live `advances`→goal
    /// edge (`EdgeKind::LeadsTo`, `label="advances"`) after create. `None` = no goal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<uuid::Uuid>,
    /// `"sha256:<hex>"` — server computes if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Full extracted markdown content.
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    /// Managed frontmatter (temper-* fields) as JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<serde_json::Value>,
    /// Open frontmatter (user-owned fields) as JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<serde_json::Value>,
    /// Base64-encoded MessagePack of `Vec<PackedChunk>`.
    /// Server computes via `temper_ingest::pipeline::prepare_markdown` if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunks_packed: Option<String>,
    /// Block-provenance sources this body was distilled from — recorded against the
    /// created resource's body block, position → accretion `seq`. Resource refs only
    /// in T7b; URL/`remote` sources are T7c.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<crate::types::provenance::ProvenanceSource>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship for the create act.
    /// Flattened as top-level keys; all optional (empty when nothing is supplied).
    #[serde(default, flatten)]
    pub act: ActInput,
    /// When set, this create is the **begin** of a segmented (multi-block) ingest: `content`
    /// carries only segment 0's text, and the ingest handler returns a
    /// [`SegmentedBeginResponse`] instead of the one-shot `ResourceRow`. `None` (the default) is
    /// the unchanged one-shot path — every existing small-body caller is unaffected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segmented: Option<SegmentedBegin>,
}

/// Marks an `IngestPayload` as the first request of a segmented (multi-block) ingest session.
/// Presence (not a bool) tells the ingest handler to take the segmented-begin branch and return a
/// [`SegmentedBeginResponse`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct SegmentedBegin {
    /// Best-effort total block count, if the client knows it upfront (e.g. from a
    /// pre-scan of the source). Purely informational — not validated against the
    /// actual landed count (that's `FinalizePayload.expected_blocks`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_blocks_hint: Option<u32>,
    /// The segment budget (bytes of text) this session's boundaries were cut at. Recorded so a
    /// resume re-derives identical segment boundaries — see the design's determinism note.
    pub block_budget: u32,
    /// sha256 of the source bytes, for the resume/re-ingest source-integrity check
    /// (`kb_ingestion_records.source_hash`). `None` when the source has no stable identity
    /// (e.g. piped stdin).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
}

/// One landed segment, as `begin`/`append`/`list-blocks` report it — the resume unit.
///
/// `content_hash` is the **block merkle** (`kb_block_revisions.block_body_hash`, i.e.
/// `temper_substrate::content::body_hash_from_chunk_hashes` over the block's own chunk hashes) —
/// NOT the whole-segment-text sha256 the client sends inbound as `AppendBlockPayload.content_hash`.
/// The two are deliberately distinct hashes over the same segment: the inbound one is the client's
/// cheap identity check on raw text; this outbound one is the server-computed chunk merkle that
/// already exists as the block's canonical hash. The client re-derives this same merkle from its
/// own packed chunk hashes (not the raw text) to diff against on resume — comparing against the
/// raw-text hash would never match.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct SegmentInfo {
    pub seq: u32,
    pub content_hash: String,
}

/// Response to a segmented begin (block 0 landed via the ordinary create path).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct SegmentedBeginResponse {
    pub resource_id: uuid::Uuid,
    /// Client-side ingest-session id (written into the `.temper/` resume manifest). Despite the
    /// name it is **not** the ledger's `kb_events.correlation_id`: this value is minted server-side
    /// and never reaches an event. Since P3 a caller *can* supply an act-grain correlation
    /// (`ActContext::correlation`), so threading a segmented session's `block_created` +
    /// `resource_finalized` events onto one correlation is now possible — but it needs a precedence
    /// rule against a caller-supplied value, and is deliberately left unbuilt (task 019f4a19).
    pub correlation_id: uuid::Uuid,
    pub blocks: Vec<SegmentInfo>,
    /// The live `body_hash` after block 0 — see [`BlocksResponse::body_hash`]. Present here too so a
    /// session that appends nothing still has a value to echo at finalize.
    pub body_hash: String,
}

/// Append one segment to an in-progress (segmented-begin'd) resource —
/// `POST /api/resources/{id}/blocks`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct AppendBlockPayload {
    pub seq: u32,
    pub content: String,
    /// sha256 of this segment's raw text — the client's cheap identity/resume check. Distinct
    /// from the block merkle the server reports back in [`SegmentInfo::content_hash`]; see that
    /// type's doc.
    pub content_hash: String,
    /// Base64-encoded MessagePack of `Vec<PackedChunk>` — this segment's pre-chunked, pre-embedded
    /// content (same wire shape as `IngestPayload::chunks_packed`).
    ///
    /// `None` when the caller has no chunker or embedder (the MCP surface, and any programmatic
    /// client that is not the CLI): the server then chunks [`Self::content`] itself, seeding the
    /// heading breadcrumb from the prior block so `header_path` stays continuous across the block
    /// boundary. Mirrors `IngestPayload::chunks_packed`'s optionality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunks_packed: Option<String>,
}

/// Response to append / `GET /api/resources/{id}/blocks`: the currently landed segment set.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct BlocksResponse {
    pub blocks: Vec<SegmentInfo>,
    /// The resource's live `kb_resources.body_hash` after the landed set — the value a caller
    /// echoes back as [`FinalizePayload::expected_body_hash`].
    ///
    /// A caller that does not chunk locally (the MCP surface) cannot derive this merkle itself, so
    /// the server hands it over. Finalize's comparison then asserts "nothing changed between my
    /// last append and now" — a real consistency check against a dropped or concurrent write —
    /// rather than an assertion such a caller would have to be exempted from. Opaque: echo it back
    /// verbatim, never parse it.
    pub body_hash: String,
}

/// Declare a segmented ingest complete — `POST /api/resources/{id}/finalize`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct FinalizePayload {
    pub expected_blocks: u32,
    pub expected_body_hash: String,
}

/// A single chunk with its embedding, serialized via MessagePack inside
/// `IngestPayload::chunks_packed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackedChunk {
    pub chunk_index: u32,
    pub header_path: String,
    /// Depth of the innermost heading: 0 = no heading, 1 = `#`, 2 = `##`, etc.
    #[serde(default)]
    pub heading_depth: u8,
    pub content: String,
    pub content_hash: String,
    /// 768-dimensional embedding vector.
    pub embedding: Vec<f32>,
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

/// JSONB-serializable chunk row for the `persist_resource_chunks()` and
/// `replace_resource_chunks()` SQL functions.
///
/// `embedding` is a pre-formatted pgvector literal string (`"[0.1,0.2,...]"`)
/// rather than a `Vec<f32>`. The SQL function casts it to `vector` via `::vector`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRowJsonb {
    pub chunk_index: u32,
    pub header_path: String,
    pub heading_depth: u32,
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
            heading_depth: chunk.heading_depth as u32,
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
                heading_depth: 1,
                content: "Hello world".to_owned(),
                content_hash: "abc123".to_owned(),
                embedding: vec![0.1; 768],
            },
            PackedChunk {
                chunk_index: 1,
                header_path: "Title > Section".to_owned(),
                heading_depth: 2,
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
            context_ref: "ctx".to_owned(),
            home_cogmap_id: None,
            doc_type_name: "task".to_owned(),
            goal: None,
            content_hash: Some("sha256:abc".to_owned()),
            content: "# Test".to_owned(),
            metadata: None,
            managed_meta: Some(serde_json::json!({"temper-stage": "backlog"})),
            open_meta: Some(serde_json::json!({"tags": ["rust"]})),
            chunks_packed: Some(pack_chunks(&sample_chunks()).unwrap()),
            sources: Vec::new(),
            segmented: None,
            act: Default::default(),
        };

        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: IngestPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.title, "Test");
        assert_eq!(deserialized.context_ref, "ctx");
        assert_eq!(
            deserialized.managed_meta,
            Some(serde_json::json!({"temper-stage": "backlog"}))
        );
        assert_eq!(
            deserialized.open_meta,
            Some(serde_json::json!({"tags": ["rust"]}))
        );

        let chunks = unpack_chunks(&deserialized.chunks_packed.unwrap()).unwrap();
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn payload_serializes_with_optional_chunks_absent() {
        let payload = IngestPayload {
            title: "Test".to_owned(),
            origin_uri: "kb://ctx/task/test".to_owned(),
            context_ref: "ctx".to_owned(),
            home_cogmap_id: None,
            doc_type_name: "task".to_owned(),
            goal: None,
            content: "# Test".to_owned(),
            content_hash: None,
            metadata: None,
            managed_meta: None,
            open_meta: None,
            chunks_packed: None,
            sources: Vec::new(),
            segmented: None,
            act: Default::default(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(
            !json.contains("chunks_packed"),
            "absent field should not serialize"
        );
        assert!(
            !json.contains("content_hash"),
            "absent field should not serialize"
        );
    }

    #[test]
    fn payload_deserializes_with_optional_chunks_absent() {
        let json = r#"{"title":"Test","origin_uri":"kb://ctx/task/test","context_ref":"ctx","doc_type_name":"task","content":"Heading"}"#;
        let payload: IngestPayload = serde_json::from_str(json).unwrap();
        assert!(payload.chunks_packed.is_none());
        assert!(payload.content_hash.is_none());
    }

    #[test]
    fn payload_with_chunks_present_roundtrips() {
        let payload = IngestPayload {
            title: "Test".to_owned(),
            origin_uri: "kb://ctx/task/test".to_owned(),
            context_ref: "ctx".to_owned(),
            home_cogmap_id: None,
            doc_type_name: "task".to_owned(),
            goal: None,
            content: "# Test".to_owned(),
            content_hash: Some("sha256:abc".to_owned()),
            metadata: None,
            managed_meta: None,
            open_meta: None,
            chunks_packed: Some(pack_chunks(&sample_chunks()).unwrap()),
            sources: Vec::new(),
            segmented: None,
            act: Default::default(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: IngestPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.content_hash, Some("sha256:abc".to_owned()));
        assert!(deserialized.chunks_packed.is_some());
    }

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
            heading_depth: 2,
            content: "Hello world".to_owned(),
            content_hash: "abc123".to_owned(),
            embedding: vec![0.1, 0.2, 0.3],
        };
        let row = ChunkRowJsonb::from_packed(&packed);
        assert_eq!(row.chunk_index, 0);
        assert_eq!(row.heading_depth, 2);
        assert_eq!(row.embedding, "[0.1,0.2,0.3]");
        assert_eq!(row.content, "Hello world");
    }

    #[test]
    fn chunks_to_jsonb_produces_array() {
        let chunks = vec![
            PackedChunk {
                chunk_index: 0,
                header_path: "A".into(),
                heading_depth: 0,
                content: "first".into(),
                content_hash: "h1".into(),
                embedding: vec![0.1, 0.2],
            },
            PackedChunk {
                chunk_index: 1,
                header_path: "B".into(),
                heading_depth: 0,
                content: "second".into(),
                content_hash: "h2".into(),
                embedding: vec![0.3, 0.4],
            },
        ];
        let json = chunks_to_jsonb(&chunks);
        assert!(json.is_array());
        assert_eq!(json.as_array().unwrap().len(), 2);
        assert_eq!(json[0]["chunk_index"], 0);
        assert_eq!(json[0]["embedding"], "[0.1,0.2]");
        assert!(json[0]["embedding"].is_string());
    }

    // ── Beat 2 Task 2.1: segmented begin/append/finalize wire types ──────────────────────────

    #[test]
    fn append_payload_round_trips() {
        let p = super::AppendBlockPayload {
            seq: 2,
            content: "x".into(),
            content_hash: "h".into(),
            chunks_packed: Some("b64".into()),
        };
        let j = serde_json::to_string(&p).unwrap();
        let back: super::AppendBlockPayload = serde_json::from_str(&j).unwrap();
        assert_eq!(back.seq, 2);
        assert_eq!(back.chunks_packed.as_deref(), Some("b64"));
    }

    // The MCP shape: a caller with no chunker omits the field entirely, and it must neither be
    // required on the way in nor emitted on the way out.
    #[test]
    fn append_payload_round_trips_without_packed_chunks() {
        let json = r#"{"seq":2,"content":"x","content_hash":"h"}"#;
        let p: super::AppendBlockPayload = serde_json::from_str(json).unwrap();
        assert!(p.chunks_packed.is_none());
        assert!(!serde_json::to_string(&p).unwrap().contains("chunks_packed"));
    }

    #[test]
    fn segment_info_round_trips() {
        let s = SegmentInfo {
            seq: 3,
            content_hash: "merkle-hash".to_owned(),
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: SegmentInfo = serde_json::from_str(&j).unwrap();
        assert_eq!(back.seq, 3);
        assert_eq!(back.content_hash, "merkle-hash");
    }

    #[test]
    fn blocks_response_round_trips() {
        let r = BlocksResponse {
            blocks: vec![
                SegmentInfo {
                    seq: 0,
                    content_hash: "h0".to_owned(),
                },
                SegmentInfo {
                    seq: 1,
                    content_hash: "h1".to_owned(),
                },
            ],
            body_hash: "sha256:deadbeef".to_owned(),
        };
        let j = serde_json::to_string(&r).unwrap();
        let back: BlocksResponse = serde_json::from_str(&j).unwrap();
        assert_eq!(back.blocks.len(), 2);
        assert_eq!(back.blocks[1].seq, 1);
        assert_eq!(
            back.body_hash, "sha256:deadbeef",
            "the echo-back value must survive the wire"
        );
    }

    #[test]
    fn finalize_payload_round_trips() {
        let f = FinalizePayload {
            expected_blocks: 4,
            expected_body_hash: "sha256:deadbeef".to_owned(),
        };
        let j = serde_json::to_string(&f).unwrap();
        let back: FinalizePayload = serde_json::from_str(&j).unwrap();
        assert_eq!(back.expected_blocks, 4);
        assert_eq!(back.expected_body_hash, "sha256:deadbeef");
    }

    #[test]
    fn segmented_begin_response_round_trips() {
        let r = SegmentedBeginResponse {
            resource_id: uuid::Uuid::now_v7(),
            correlation_id: uuid::Uuid::now_v7(),
            blocks: vec![SegmentInfo {
                seq: 0,
                content_hash: "h0".to_owned(),
            }],
            body_hash: "sha256:beef".to_owned(),
        };
        let j = serde_json::to_string(&r).unwrap();
        let back: SegmentedBeginResponse = serde_json::from_str(&j).unwrap();
        assert_eq!(back.resource_id, r.resource_id);
        assert_eq!(back.correlation_id, r.correlation_id);
        assert_eq!(back.blocks.len(), 1);
        assert_eq!(
            back.body_hash, "sha256:beef",
            "begin hands the caller its first echo-back value"
        );
    }

    #[test]
    fn ingest_payload_segmented_field_round_trips_when_present() {
        let payload = IngestPayload {
            title: "Big Doc".to_owned(),
            origin_uri: "kb://ctx/task/big".to_owned(),
            context_ref: "ctx".to_owned(),
            home_cogmap_id: None,
            doc_type_name: "task".to_owned(),
            goal: None,
            content_hash: None,
            content: "segment 0 text".to_owned(),
            metadata: None,
            managed_meta: None,
            open_meta: None,
            chunks_packed: None,
            sources: Vec::new(),
            act: Default::default(),
            segmented: Some(SegmentedBegin {
                total_blocks_hint: Some(3),
                block_budget: 262_144,
                source_hash: Some("sha256:abc".to_owned()),
            }),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(
            json.contains("\"segmented\""),
            "present segmented field must serialize"
        );
        let back: IngestPayload = serde_json::from_str(&json).unwrap();
        let seg = back.segmented.expect("segmented round-trips");
        assert_eq!(seg.total_blocks_hint, Some(3));
        assert_eq!(seg.block_budget, 262_144);
        assert_eq!(seg.source_hash, Some("sha256:abc".to_owned()));
    }

    #[test]
    fn ingest_payload_segmented_absent_by_default() {
        // The one-shot small-body path (every existing caller) must be unaffected: no
        // `segmented` field on the wire, and it deserializes to `None` when omitted.
        let json = r#"{"title":"Test","origin_uri":"kb://ctx/task/test","context_ref":"ctx","doc_type_name":"task","content":"Heading"}"#;
        let payload: IngestPayload = serde_json::from_str(json).unwrap();
        assert!(payload.segmented.is_none());

        let payload = IngestPayload {
            title: "Test".to_owned(),
            origin_uri: "kb://ctx/task/test".to_owned(),
            context_ref: "ctx".to_owned(),
            home_cogmap_id: None,
            doc_type_name: "task".to_owned(),
            goal: None,
            content_hash: None,
            content: "# Test".to_owned(),
            metadata: None,
            managed_meta: None,
            open_meta: None,
            chunks_packed: None,
            sources: Vec::new(),
            act: Default::default(),
            segmented: None,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(
            !json.contains("segmented"),
            "absent segmented field should not serialize"
        );
    }
}
