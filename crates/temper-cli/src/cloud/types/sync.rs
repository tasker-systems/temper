use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::config::SyncSubscription;

/// A manifest entry sent to the server for comparison.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncManifestEntry {
    pub resource_id: Uuid,
    pub content_hash: String,
    pub updated_at: DateTime<Utc>,
}

/// Request body for `POST /api/sync/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatusRequest {
    pub subscriptions: Vec<SyncSubscription>,
    pub manifest_entries: Vec<SyncManifestEntry>,
}

/// A resource the client should pull (server has newer version or new resource).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPullEntry {
    pub resource_id: Uuid,
    pub content_hash: String,
    pub title: String,
}

/// A resource the client should push (local changes the server doesn't have).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPushEntry {
    pub resource_id: Uuid,
    pub reason: String,
}

/// A resource with conflicting changes on both sides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflictEntry {
    pub resource_id: Uuid,
    pub local_hash: String,
    pub remote_hash: String,
}

/// A resource removed from visibility (deleted, unshared, or no longer matching).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRemovedEntry {
    pub resource_id: Uuid,
    pub reason: String,
}

/// Response body for `POST /api/sync/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatusResponse {
    pub to_pull: Vec<SyncPullEntry>,
    pub to_push: Vec<SyncPushEntry>,
    pub conflicts: Vec<SyncConflictEntry>,
    pub removed: Vec<SyncRemovedEntry>,
}

/// Request body for `POST /api/sync/pull`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPullRequest {
    pub resource_ids: Vec<Uuid>,
}

/// Metadata sidecar for a pulled resource (included in the zip alongside markdown).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPullResourceMeta {
    pub resource_id: Uuid,
    pub title: String,
    pub context: String,
    pub doc_type: String,
    pub content_hash: String,
    pub tags: Vec<String>,
}

/// Request body for `POST /api/sync/complete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCompleteRequest {
    pub resource_ids: Vec<Uuid>,
    pub manifest_hash: String,
}

/// Response body for `POST /api/sync/complete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCompleteResponse {
    pub ok: bool,
    pub event_ids: Vec<Uuid>,
}

/// Conflict resolution type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionType {
    Local,
    Remote,
    Merged,
}

/// Request body for `POST /api/sync/resolve`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResolveRequest {
    pub resource_id: Uuid,
    pub resolution: ResolutionType,
    pub content_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_status_response_empty() {
        let resp = SyncStatusResponse {
            to_pull: vec![],
            to_push: vec![],
            conflicts: vec![],
            removed: vec![],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: SyncStatusResponse = serde_json::from_str(&json).unwrap();
        assert!(parsed.to_pull.is_empty());
        assert!(parsed.conflicts.is_empty());
    }

    #[test]
    fn test_sync_status_request_serde() {
        let req = SyncStatusRequest {
            subscriptions: vec![SyncSubscription {
                context: Some("temper".to_string()),
                team: None,
                doc_types: vec![],
                merge: super::super::config::MergePolicy::Manual,
            }],
            manifest_entries: vec![SyncManifestEntry {
                resource_id: Uuid::nil(),
                content_hash: "sha256:abc".to_string(),
                updated_at: Utc::now(),
            }],
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: SyncStatusRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.subscriptions.len(), 1);
        assert_eq!(parsed.manifest_entries.len(), 1);
    }

    #[test]
    fn test_resolution_type_serde() {
        assert_eq!(
            serde_json::to_string(&ResolutionType::Local).unwrap(),
            "\"local\""
        );
        assert_eq!(
            serde_json::to_string(&ResolutionType::Merged).unwrap(),
            "\"merged\""
        );
    }

    #[test]
    fn test_sync_complete_response() {
        let resp = SyncCompleteResponse {
            ok: true,
            event_ids: vec![Uuid::nil()],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("true"));
        let parsed: SyncCompleteResponse = serde_json::from_str(&json).unwrap();
        assert!(parsed.ok);
        assert_eq!(parsed.event_ids.len(), 1);
    }
}
