//! Wire types for the `/api/relationships` write endpoints.
//!
//! Shared between `temper-api` (server-side, OpenAPI schema source) and
//! `temper-client` (client-side, typed request builder). The structs both
//! `Serialize` (so the client can post them) and `Deserialize` (so the
//! server can extract them); both sides re-use the same struct rather than
//! string-mirroring a JSON shape.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::graph::{EdgeKind, Polarity};
use crate::types::ids::ResourceId;

/// Request body for `POST /api/relationships`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct AssertRelationshipRequest {
    /// Source resource — a pre-resolved id (both endpoints are resolved now).
    pub source: ResourceId,
    /// Target resource — a pre-resolved id (both endpoints are resolved now).
    pub target: ResourceId,
    pub edge_kind: EdgeKind,
    pub polarity: Polarity,
    pub label: String,
    pub weight: f64,
}

/// Request body for `POST /api/relationships/{edge_handle}/retype`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct RetypeRelationshipRequest {
    pub edge_kind: EdgeKind,
    pub polarity: Polarity,
}

/// Request body for `POST /api/relationships/{edge_handle}/reweight`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ReweightRelationshipRequest {
    pub weight: f64,
}

/// Request body for `POST /api/relationships/{edge_handle}/fold`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct FoldRelationshipRequest {
    pub reason: Option<String>,
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
