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

/// One team context a removed member still owns resources in, with the count.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "reassign.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResidualContext {
    /// Decorated context ref `{owner_ref}/{slug}`.
    pub context_ref: String,
    pub count: u32,
}

/// The resources a removed member still OWNS in the team's contexts — the reach
/// an admin should hand off via `team reassign`. `count == 0` is the clean case.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "reassign.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResidualOwnedReach {
    pub count: u32,
    pub contexts: Vec<ResidualContext>,
}

/// Response to a member removal (or self-leave): the removal happened; this
/// reports the residual owned-resource reach so the caller can hand it off.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "reassign.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveMemberOutcome {
    pub residual_owned: ResidualOwnedReach,
}
