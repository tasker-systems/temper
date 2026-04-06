//! Audit trail types — tracks resource mutations with hash snapshots.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Row type matching the `kb_resource_audits` table.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "audit.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceAuditRow {
    pub id: Uuid,
    pub resource_id: Uuid,
    pub event_id: Uuid,
    pub profile_id: Uuid,
    pub device_id: String,
    pub body_hash: String,
    pub managed_hash: String,
    pub open_hash: String,
    pub action: String,
    pub created: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_audit_row_serde_roundtrip() {
        let row = ResourceAuditRow {
            id: Uuid::nil(),
            resource_id: Uuid::nil(),
            event_id: Uuid::nil(),
            profile_id: Uuid::nil(),
            device_id: "test-device".to_string(),
            body_hash: "sha256:abc".to_string(),
            managed_hash: "sha256:def".to_string(),
            open_hash: "sha256:ghi".to_string(),
            action: "create".to_string(),
            created: Utc::now(),
        };
        let json = serde_json::to_string(&row).unwrap();
        let parsed: ResourceAuditRow = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.resource_id, Uuid::nil());
        assert_eq!(parsed.action, "create");
        assert_eq!(parsed.device_id, "test-device");
    }
}
