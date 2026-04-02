use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Per-device sync state tracked by the server.
///
/// Each device is identified by a `device_id` (UUIDv7) generated at first
/// login and stored in `auth.json`. The server records the last sync timestamp
/// and manifest hash per device to scope sync/status responses.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct DeviceSyncState {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub device_id: String,
    pub last_sync_at: DateTime<Utc>,
    pub manifest_hash: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_sync_state_serde_roundtrip() {
        let state = DeviceSyncState {
            id: Uuid::nil(),
            profile_id: Uuid::nil(),
            device_id: "d7e8f9a0-1234-5678-9abc-def012345678".to_string(),
            last_sync_at: Utc::now(),
            manifest_hash: Some("sha256:abc".to_string()),
        };
        let json = serde_json::to_string(&state).unwrap();
        let parsed: DeviceSyncState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.device_id, state.device_id);
        assert_eq!(parsed.manifest_hash, state.manifest_hash);
    }
}
