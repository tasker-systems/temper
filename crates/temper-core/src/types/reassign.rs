use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// API request to reassign a single resource's owner (resource id is in the path).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "reassign.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReassignResourceRequest {
    pub to_profile_id: Uuid,
}

/// API response acknowledging a single reassignment.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "reassign.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReassignAck {
    pub resource_id: Uuid,
    pub to_profile_id: Uuid,
}

/// API request for bulk team reassignment (from_profile → to_profile).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "reassign.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkReassignRequest {
    pub from_profile_id: Uuid,
    pub to_profile_id: Uuid,
}

/// API response acknowledging a bulk reassignment.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "reassign.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkReassignAck {
    pub resource_ids: Vec<Uuid>,
}
