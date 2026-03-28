use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Per-device sync state tracked by the server.
///
/// Each device is identified by a `client_id` generated at `temper init` time.
/// The server records the last sync timestamp and manifest hash per device
/// to scope sync/status responses.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct DeviceSyncState {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub client_id: String,
    pub last_sync_at: DateTime<Utc>,
    pub manifest_hash: Option<String>,
}

/// Local device identity, stored at `~/.config/temper/devices/<id>.json`.
///
/// Generated once at `temper init` time. Sent as `X-Temper-Client-Id` header
/// on all API calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub client_id: String,
    pub device_name: Option<String>,
    pub created: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_device_identity_serde_roundtrip() {
        let device = DeviceIdentity {
            client_id: "d7e8f9a0-1234-5678-9abc-def012345678".to_string(),
            device_name: Some("macbook-pro".to_string()),
            created: Utc::now(),
        };
        let json = serde_json::to_string(&device).unwrap();
        let parsed: DeviceIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.client_id, device.client_id);
        assert_eq!(parsed.device_name, device.device_name);
    }

    #[test]
    fn test_device_identity_optional_name() {
        let json = r#"{"client_id":"abc","device_name":null,"created":"2026-03-27T18:00:00Z"}"#;
        let device: DeviceIdentity = serde_json::from_str(json).unwrap();
        assert!(device.device_name.is_none());
    }
}
