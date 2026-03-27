use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Transfer status — lifecycle of a resource ownership transfer.
///
/// Maps directly to the `transfer_status` Postgres enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, Deserialize)]
#[sqlx(type_name = "transfer_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    Pending,
    Accepted,
    Declined,
    Cancelled,
}

/// A pending or resolved ownership transfer of a resource.
///
/// Two-step offer/accept for personal transfers. The offerer creates
/// the transfer, the recipient accepts or declines. The offerer can
/// cancel before resolution.
///
/// Constraints:
/// - Only the current `owner_profile_id` can initiate a transfer
/// - One pending transfer per resource at a time (enforced by unique constraint)
/// - Acceptance updates `resources.owner_profile_id` to `to_profile_id`
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ResourceTransfer {
    pub id: Uuid,
    pub resource_id: Uuid,
    pub from_profile_id: Uuid,
    pub to_profile_id: Uuid,
    pub status: TransferStatus,
    pub created: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

/// API request to initiate a resource transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferRequest {
    pub resource_id: Uuid,
    pub to_profile_id: Uuid,
}

/// API request for bulk team reassignment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkReassignRequest {
    pub from_profile_id: Uuid,
    pub to_profile_id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transfer_status_serde_roundtrip() {
        let statuses = [
            TransferStatus::Pending,
            TransferStatus::Accepted,
            TransferStatus::Declined,
            TransferStatus::Cancelled,
        ];
        for status in &statuses {
            let json = serde_json::to_string(status).unwrap();
            let parsed: TransferStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(*status, parsed);
        }
    }

    #[test]
    fn test_transfer_status_json_format() {
        assert_eq!(
            serde_json::to_string(&TransferStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&TransferStatus::Cancelled).unwrap(),
            "\"cancelled\""
        );
    }

    #[test]
    fn test_transfer_request_serde() {
        let req = TransferRequest {
            resource_id: Uuid::nil(),
            to_profile_id: Uuid::nil(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: TransferRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.resource_id, req.resource_id);
        assert_eq!(parsed.to_profile_id, req.to_profile_id);
    }
}
