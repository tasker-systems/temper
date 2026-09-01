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

use crate::types::ids::BlobId;

/// The response of `POST /api/blobs` — get-or-create on the content hash (D2). `blob_id` is
/// the id the bytes live under: freshly minted for a new commit, the EXISTING id on a dedup
/// hit (whose first home stands, and which the caller may not be able to read back if that
/// home is not theirs — substrate get-or-create semantics, D2, decide this). `deduped`
/// reports that a blob the caller can already read held these bytes, so the provider upload
/// was skipped; the ledger still records the re-commit as provenance.
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
    /// The media type the blob was committed under (allowlist-checked, D9).
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
    /// Bare sha256 hex of the segment's raw bytes — the client's resume check and the
    /// idempotent-append identity (same segment re-sent is a no-op; a DIFFERENT segment
    /// at an occupied seq is a conflict, the assembled whole must stay unambiguous).
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
