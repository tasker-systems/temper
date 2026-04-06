use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Status endpoint (POST /api/sync/status)
// ---------------------------------------------------------------------------

/// A single manifest entry sent to the server for diff computation.
/// Maps to the JSONB entries consumed by `sync_diff_for_device()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncManifestEntry {
    pub uri: String,
    pub local_hash: String,
    pub remote_hash: String,
    #[serde(default)]
    pub managed_hash: String,
    #[serde(default)]
    pub remote_managed_hash: String,
    #[serde(default)]
    pub open_hash: String,
    #[serde(default)]
    pub remote_open_hash: String,
}

/// Per-context grouping of manifest entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncContextEntries {
    pub name: String,
    pub entries: Vec<SyncManifestEntry>,
}

/// Request body for `POST /api/sync/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatusRequest {
    pub contexts: Vec<SyncContextEntries>,
}

/// A resource the client should push (local-only or locally modified).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPushItem {
    pub uri: String,
    pub resource_id: Option<Uuid>,
}

/// A resource the client should pull (server has newer or new resource).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPullItem {
    pub uri: String,
    pub resource_id: Uuid,
    pub content_hash: String,
}

/// A resource with conflicting changes on both sides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflictItem {
    pub uri: String,
    pub resource_id: Uuid,
    pub server_hash: String,
}

/// A resource that was removed from visibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRemovedItem {
    pub uri: String,
    pub resource_id: Uuid,
}

/// Response body for `POST /api/sync/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatusResponse {
    pub to_push: Vec<SyncPushItem>,
    pub to_pull: Vec<SyncPullItem>,
    pub conflicts: Vec<SyncConflictItem>,
    pub removed: Vec<SyncRemovedItem>,
}

// ---------------------------------------------------------------------------
// Complete endpoint (POST /api/sync/complete)
// ---------------------------------------------------------------------------

/// A resource whose content_hash should be updated after sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedResource {
    pub resource_id: Uuid,
    pub content_hash: String,
}

/// Request body for `POST /api/sync/complete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCompleteRequest {
    pub device_id: String,
    pub merged_resources: Vec<MergedResource>,
}

/// Response body for `POST /api/sync/complete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCompleteResponse {
    pub last_sync_at: DateTime<Utc>,
    pub updated_count: u32,
}

// ---------------------------------------------------------------------------
// Manifest endpoint (GET /api/sync/manifest)
// ---------------------------------------------------------------------------

/// A single resource entry in the server manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncManifestItem {
    pub resource_id: Uuid,
    pub context: String,
    pub doc_type: String,
    pub slug: String,
    pub content_hash: String,
    pub managed_hash: String,
    pub open_hash: String,
    pub uri: String,
    /// Most recent audit ID for this resource on the server.
    #[serde(default)]
    pub last_audit_id: Option<Uuid>,
}

/// Response body for `GET /api/sync/manifest`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncManifestResponse {
    pub items: Vec<SyncManifestItem>,
}

// ---------------------------------------------------------------------------
// Resolve endpoint (I6c — placeholder types)
// ---------------------------------------------------------------------------

/// Conflict resolution type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionType {
    Local,
    Remote,
    Merged,
}

/// Request body for `POST /api/sync/resolve` (I6c).
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
    fn sync_status_request_serde_roundtrip() {
        let req = SyncStatusRequest {
            contexts: vec![SyncContextEntries {
                name: "temper".to_string(),
                entries: vec![SyncManifestEntry {
                    uri: "kb://temper/task/00000000-0000-0000-0000-000000000000".to_string(),
                    local_hash: "sha256:abc".to_string(),
                    remote_hash: "sha256:abc".to_string(),
                    managed_hash: String::new(),
                    remote_managed_hash: String::new(),
                    open_hash: String::new(),
                    remote_open_hash: String::new(),
                }],
            }],
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: SyncStatusRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.contexts.len(), 1);
        assert_eq!(parsed.contexts[0].entries.len(), 1);
        assert_eq!(
            parsed.contexts[0].entries[0].uri,
            "kb://temper/task/00000000-0000-0000-0000-000000000000"
        );
    }

    #[test]
    fn sync_status_response_empty_roundtrip() {
        let resp = SyncStatusResponse {
            to_push: vec![],
            to_pull: vec![],
            conflicts: vec![],
            removed: vec![],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: SyncStatusResponse = serde_json::from_str(&json).unwrap();
        assert!(parsed.to_pull.is_empty());
        assert!(parsed.conflicts.is_empty());
    }

    #[test]
    fn sync_complete_request_serde_roundtrip() {
        let req = SyncCompleteRequest {
            device_id: "device-abc".to_string(),
            merged_resources: vec![MergedResource {
                resource_id: Uuid::nil(),
                content_hash: "sha256:def".to_string(),
            }],
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: SyncCompleteRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.device_id, "device-abc");
        assert_eq!(parsed.merged_resources.len(), 1);
    }

    #[test]
    fn sync_complete_response_serde_roundtrip() {
        let resp = SyncCompleteResponse {
            last_sync_at: Utc::now(),
            updated_count: 3,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: SyncCompleteResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.updated_count, 3);
    }

    #[test]
    fn resolution_type_serde() {
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
    fn sync_manifest_response_serde_roundtrip() {
        let resp = SyncManifestResponse {
            items: vec![SyncManifestItem {
                resource_id: Uuid::nil(),
                context: "temper".to_string(),
                doc_type: "task".to_string(),
                slug: "my-task".to_string(),
                content_hash: "sha256:abc".to_string(),
                managed_hash: "sha256:def".to_string(),
                open_hash: "sha256:ghi".to_string(),
                uri: "kb://temper/task/00000000-0000-0000-0000-000000000000".to_string(),
                last_audit_id: None,
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: SyncManifestResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].context, "temper");
        assert_eq!(parsed.items[0].slug, "my-task");
    }

    #[test]
    fn push_item_with_null_resource_id() {
        let item = SyncPushItem {
            uri: "kb://temper/note/new-uuid".to_string(),
            resource_id: None,
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("null"));
        let parsed: SyncPushItem = serde_json::from_str(&json).unwrap();
        assert!(parsed.resource_id.is_none());
    }
}
