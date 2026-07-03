//! HTTP/MCP body types for per-resource capability grants
//! (`POST/DELETE /api/resources/{id}/grants`). The subject is the path `{id}` (a resource),
//! so the body carries only the principal + capabilities. Handlers/tools widen these into a
//! `GrantCapabilityRequest`/`RevokeCapabilityRequest` with `subject_table = "kb_resources"`.
//! Structurally parallel to `CogmapGrantBody`/`CogmapRevokeBody`. Responses reuse the shared
//! `GrantOutcome`/`RevokeOutcome`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Mint/update a `kb_access_grants` row on a resource. Principal `{kb_teams,kb_profiles}`.
/// The DB coherence CHECK enforces `write|delete|grant ⇒ read`; pass a coherent set
/// (a write grant implies read).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource_grant.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceGrantBody {
    pub principal_table: String,
    pub principal_id: Uuid,
    pub can_read: bool,
    pub can_write: bool,
    pub can_delete: bool,
    pub can_grant: bool,
}

/// Delete a `kb_access_grants` row on a resource (the `(subject, principal)` pair). Absent ⇒ no-op.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource_grant.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRevokeBody {
    pub principal_table: String,
    pub principal_id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_grant_body_roundtrips() {
        let id = Uuid::now_v7();
        let body = ResourceGrantBody {
            principal_table: "kb_teams".to_string(),
            principal_id: id,
            can_read: true,
            can_write: true,
            can_delete: false,
            can_grant: false,
        };
        let json = serde_json::to_string(&body).unwrap();
        let back: ResourceGrantBody = serde_json::from_str(&json).unwrap();
        assert_eq!(back.principal_id, id);
        assert!(back.can_read && back.can_write && !back.can_grant);
    }
}
