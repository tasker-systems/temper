//! Wire types for the `/api/relationships` write endpoints.
//!
//! Shared between `temper-api` (server-side, OpenAPI schema source) and
//! `temper-client` (client-side, typed request builder). The structs both
//! `Serialize` (so the client can post them) and `Deserialize` (so the
//! server can extract them); both sides re-use the same struct rather than
//! string-mirroring a JSON shape.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::authorship::ActInput;
use crate::types::graph::{EdgeKind, Polarity};
use crate::types::ids::ResourceId;

/// Which table the asserted edge's TARGET endpoint lives in. `resource` is the
/// incumbent and the serde default, so every existing payload is unchanged;
/// `blob` is the D3 pointing act — a resource names a blob it can READ as its
/// source doc / evidence / figure, with the edge homed in the SOURCE's home
/// (the same mechanism resource→resource asserts use: point at what you can
/// see). The SOURCE is always a resource — blobs assert through `blob relate`,
/// whose home story is the blob's own. Typed, never a free string: a malformed
/// value must refuse naming the two admissible values (the
/// `BlobRelationDirection` rule).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "mcp", schemars(inline))]
#[serde(rename_all = "snake_case")]
pub enum RelationshipTarget {
    #[default]
    Resource,
    Blob,
}

/// Request body for `POST /api/relationships`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct AssertRelationshipRequest {
    /// Source resource — a pre-resolved id (both endpoints are resolved now).
    pub source: ResourceId,
    /// Target resource — a pre-resolved id (both endpoints are resolved now).
    pub target: ResourceId,
    /// Which table `target` addresses. Defaults to `resource`; `blob` points
    /// the edge at a binary blob the caller can read (task 01a06ee1).
    #[serde(default)]
    pub target_table: RelationshipTarget,
    pub edge_kind: EdgeKind,
    pub polarity: Polarity,
    pub label: String,
    pub weight: f64,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship for the assert act.
    /// Flattened as top-level keys; all optional (empty when nothing is supplied).
    #[serde(default, flatten)]
    pub act: ActInput,
}

/// Request body for `POST /api/relationships/{edge_handle}/retype`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct RetypeRelationshipRequest {
    pub edge_kind: EdgeKind,
    pub polarity: Polarity,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship for the retype act.
    /// Flattened as top-level keys; all optional (empty when nothing is supplied).
    #[serde(default, flatten)]
    pub act: ActInput,
}

/// Request body for `POST /api/relationships/{edge_handle}/reweight`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ReweightRelationshipRequest {
    pub weight: f64,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship for the reweight act.
    /// Flattened as top-level keys; all optional (empty when nothing is supplied).
    #[serde(default, flatten)]
    pub act: ActInput,
}

/// Request body for `POST /api/relationships/{edge_handle}/fold`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct FoldRelationshipRequest {
    pub reason: Option<String>,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship for the fold act.
    /// Flattened as top-level keys; all optional (empty when nothing is supplied).
    #[serde(default, flatten)]
    pub act: ActInput,
}

/// Acknowledgement returned by all relationship write endpoints.
///
/// Carries the `edge_handle` — the backend-opaque handle that identifies the
/// relationship (correlation_id under DbBackend, edge_id under NextBackend) and
/// is fed back into retype/reweight/fold. Future revisions may add the
/// projected edge id or event id.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct RelationshipAck {
    pub edge_handle: Uuid,
}
