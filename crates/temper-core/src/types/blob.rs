//! Wire types for blob commits — the shape the API blob surface returns (spec: binary blobs,
//! 2026-09-01). Reads are bytes, not JSON, so the read-through path carries no wire type; a
//! metadata view (list surfaces, CLI/MCP) grows here when those surfaces land.

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
