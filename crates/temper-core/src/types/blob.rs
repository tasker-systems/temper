//! Wire types for blob commits — the shape the API blob surface returns (spec: binary blobs,
//! 2026-09-01). Reads are bytes, not JSON, so the read-through path carries no wire type; a
//! metadata view (list surfaces, CLI/MCP) grows here when those surfaces land.
//!
//! Segmented upload (S3, D7) follows the segmented-ingest precedent: begin/append/finalize,
//! with the append carrying RAW BINARY bytes (not base64-in-JSON — blobs are bytes, the ingest
//! precedent is JSON only because markdown is text), and the finalize payload splitting the
//! ingest precedent's checks the same way: a concurrency token the SERVER hands over (echo it
//! back verbatim, never parse it) and an optional integrity hash the client can derive itself.

use serde::{Deserialize, Serialize};

use crate::types::authorship::ActInput;
use crate::types::graph::{EdgeKind, Polarity};
use crate::types::ids::BlobId;

/// The response of `POST /api/blobs` — get-or-create on the content hash, PER-HOME (D2 as
/// amended 2026-09-02). `blob_id` is the id the bytes live under: freshly minted for a new
/// commit, the SAME id on a re-commit of bytes the caller's own home already holds — always a
/// row the caller can read, asserted by their own event (the same bytes in another
/// principal's scope are that principal's row and never surface here). `deduped` reports
/// that the caller's home already held these bytes, so the provider upload was skipped.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "blob.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct BlobCommitResponse {
    pub blob_id: BlobId,
    /// Bare sha256 hex — the dedup key, the erasure join key, and the proof the ledger keeps
    /// instead of bytes (`ledger-carries-hash-not-bytes`).
    pub content_hash: String,
    /// The row's STORED media type — allowlist-checked at commit (D9). On a dedup hit this
    /// is the FIRST committer's type (what read-through serves), never the re-commit's
    /// declaration (N2, 2026-09-03 review).
    pub content_type: String,
    pub content_bytes: i64,
    pub deduped: bool,
}

/// Begin a segmented upload — `POST /api/blobs/uploads`. Declares the whole upload up
/// front: the home the assembled blob will commit into, and the media type it will
/// commit under. Both are re-examined at finalize — standing by the service (it can
/// change mid-upload), the allowlist by the SQL wrapper (the sole authority, D9).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "blob.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct BlobUploadBeginRequest {
    /// `kb_contexts` or `kb_cogmaps` — a blob needs a home (D2), and so does its upload.
    pub home_table: String,
    pub home_id: uuid::Uuid,
    /// The media type the assembled blob will commit under.
    pub content_type: String,
}

/// The response of `POST /api/blobs/uploads`. The upload id is server-minted and is the
/// only handle the remaining requests carry — a staged session is not a blob (it has no
/// hash yet), not a resource, and invisible to every other surface.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "blob.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct BlobUploadBeginResponse {
    pub upload_id: uuid::Uuid,
}

/// One landed segment, as the progress read and every append report it.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "blob.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct BlobUploadSegmentInfo {
    pub seq: u32,
    /// The SERVER's bare sha256 hex of the segment's raw bytes as received — the caller's
    /// resume check and the idempotent-append identity (the same bytes re-sent at the
    /// same seq is a no-op; DIFFERENT bytes at an occupied seq is a conflict, the
    /// assembled whole must stay unambiguous). The server computes it; the caller sends
    /// no integrity claim, and the whole assembly's check is finalize's
    /// `expected_content_hash`.
    pub segment_hash: String,
    pub segment_bytes: i64,
}

/// The currently-landed segment set — the response of append and of the progress read
/// `GET /api/blobs/uploads/{id}` (the ingest precedent's `BlocksResponse`, in blob terms:
/// the caller's resume manifest and the source of the finalize echo).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "blob.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct BlobUploadProgress {
    pub upload_id: uuid::Uuid,
    pub segments: Vec<BlobUploadSegmentInfo>,
    /// Running byte total across landed segments — the server-handed half of the
    /// finalize echo (`BlobUploadFinalizeRequest::expected_total_bytes`).
    pub total_bytes: i64,
}

/// Declare a segmented upload complete — `POST /api/blobs/uploads/{id}/finalize`. The
/// expected values are CONCURRENCY tokens ("nothing landed since my last append"): both
/// are server-handed in [`BlobUploadProgress`], echoed back verbatim, never parsed. A
/// mismatch refuses with the staging kept, resumable — the ingest precedent's posture.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "blob.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct BlobUploadFinalizeRequest {
    pub expected_segments: u32,
    pub expected_total_bytes: i64,
    /// Bare sha256 hex of the FULL assembled body — an INTEGRITY check over the actual
    /// bytes, distinct from the concurrency tokens. The client that holds the whole file
    /// derives it itself; `None` from a caller that does not (the check is then skipped,
    /// the ingest precedent's honest exemption). A mismatch refuses with the staging
    /// kept, resumable — never silently committed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_content_hash: Option<String>,
}

// ── Blob list + relations (S4) ────────────────────────────────────────────────────

/// One blob as the list surface reports it (`GET /api/blobs`). The list can only ever
/// contain blobs the caller can read — the gate lives server-side on the same
/// `blob_readable_by_profile` predicate the read-through uses, so this shape is a view of
/// the caller's own blob set, never a discovery oracle.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "blob.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct BlobSummary {
    pub blob_id: BlobId,
    /// Bare sha256 hex — the dedup key and the erasure join key.
    pub content_hash: String,
    /// The stored media type; `None` only on a post-erasure row (metadata nulled, bytes
    /// unreachable — the erased shape renders honestly rather than being hidden).
    pub content_type: Option<String>,
    pub content_bytes: i64,
    pub created: chrono::DateTime<chrono::Utc>,
}

/// One edge incident to a blob, as `GET /api/blobs/{id}/relations` reports it. The peer is
/// whatever sits on the other end — a resource (with its title), a cogmap, or another blob
/// — so `peer_table` rides along and `peer_title` is null for non-resource peers.
/// `direction` is the edge listing's own vocabulary (`outgoing` = the blob is the source).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "blob.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct BlobRelationRow {
    pub edge_id: uuid::Uuid,
    /// `kb_resources` | `kb_cogmaps` | `kb_blobs` — spelled exactly as the DDL.
    pub peer_table: String,
    pub peer_id: uuid::Uuid,
    pub peer_title: Option<String>,
    pub edge_kind: EdgeKind,
    pub polarity: Polarity,
    pub label: Option<String>,
    pub direction: BlobRelationEdgeDirection,
    pub weight: f64,
    pub created: chrono::DateTime<chrono::Utc>,
}

/// The direction a listed edge runs relative to the blob — the relations listing's own
/// vocabulary (`outgoing` = the blob is the edge's source, `incoming` = the blob is the
/// target). Typed, not a free string (the typed-structs rule, C-C3 2026-09-04 review):
/// the readback's SQL CASE is the only writer, so a value outside the pair is a DB/code
/// disagreement and is refused at the parse boundary, never passed through. Distinct
/// from [`BlobRelationDirection`] — that is the ASSERT request's vocabulary (which end
/// of a new edge the blob occupies); this is what a LISTED edge already does.
///
/// Its ts-rs export owns a PER-TYPE file, deliberately: this is the one core blob type a
/// temper-workflow wire type's transitive closure reaches (through `GraphEdgeRow`), and a
/// ts-rs pass truncates every file its closure touches. Exporting into `blob.ts` let the
/// workflow pass truncate the file to this one type, silently dropping the other eleven
/// (found by review 2026-09-05). A per-type file is single-owner, so the truncation is a
/// no-op — the `EdgeId.ts` precedent.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    feature = "typescript",
    ts(export, export_to = "BlobRelationEdgeDirection.ts")
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum BlobRelationEdgeDirection {
    Outgoing,
    Incoming,
}

/// Which end of the asserted edge the blob occupies. `blob_as_source` is the natural
/// `figure_of`-shaped act (the figure points at what it figures); `blob_as_target` is the
/// derivation-source act (resource → blob, the file it was created from). Typed, not a
/// free string: the refusal for a malformed value must name the two admissible values.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "blob.ts"))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
// Inline the enum in MCP tool input schemas — as a named type schemars would emit a `$ref`
// into `$defs`, which the Anthropic tool-use layer reads with no type signal and sends back
// as `null` (the EdgeKind/Polarity constraint; see tools/relationships.rs).
#[cfg_attr(feature = "mcp", schemars(inline))]
#[serde(rename_all = "snake_case")]
pub enum BlobRelationDirection {
    #[default]
    BlobAsSource,
    BlobAsTarget,
}

/// Assert a relation between a blob and another anchor — `POST /api/blobs/{id}/relations`.
/// The edge homes on the BLOB's home anchor (the blob-scoped surface answers to the blob's
/// standing), and the peer must be readable by the caller (`endpoint_readable_by_profile`)
/// — "relations are to resources the actor can already see", generalized to all three
/// endpoint kinds.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "blob.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct BlobRelationAssertRequest {
    #[serde(default)]
    pub direction: BlobRelationDirection,
    /// `kb_resources` | `kb_cogmaps` | `kb_blobs` — the peer endpoint's table.
    pub peer_table: String,
    pub peer_id: uuid::Uuid,
    pub edge_kind: EdgeKind,
    pub polarity: Polarity,
    pub label: String,
    pub weight: f64,
    /// Per-act correlation + discrete agent authorship, the relationship endpoints' shape.
    #[serde(default, flatten)]
    pub act: ActInput,
}

/// The acknowledgement of `POST /api/blobs/{id}/relations` — the edge handle, feeding the
/// incumbent fold endpoint (`POST /api/relationships/{edge_handle}/fold`) for retraction.
/// Relations come and go individually (`one-blob-many-relations`); folding rides the
/// relationship machinery every edge already answers to.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "blob.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct BlobRelationAck {
    pub edge_handle: uuid::Uuid,
}
