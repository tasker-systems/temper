use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ids::{ResourceAuditId, ResourceId};

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

/// Whether a sync item is a full-body push/pull or a metadata-only update.
///
/// `Body` means chunks or content differ and a full re-ingest is needed.
/// `MetaOnly` means only managed_meta/open_meta (frontmatter) changed and
/// the server can update without re-chunking.
///
/// Server-only wire type — matches the rest of sync.rs, which has no
/// ts-rs/utoipa/schemars derives because the sync endpoints aren't
/// exposed to the UI or MCP surfaces.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncItemKind {
    #[default]
    Body,
    MetaOnly,
}

/// A resource the client should push (local-only or locally modified).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPushItem {
    pub uri: String,
    pub resource_id: Option<ResourceId>,
    #[serde(default)]
    pub kind: SyncItemKind,
}

/// A resource the client should pull (server has newer or new resource).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPullItem {
    pub uri: String,
    pub resource_id: ResourceId,
    pub content_hash: String,
    #[serde(default)]
    pub kind: SyncItemKind,
}

/// A resource with conflicting changes on both sides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflictItem {
    pub uri: String,
    pub resource_id: ResourceId,
    pub server_hash: String,
}

/// A resource that was removed from visibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRemovedItem {
    pub uri: String,
    pub resource_id: ResourceId,
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
    pub resource_id: ResourceId,
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
    pub resource_id: ResourceId,
    pub context: String,
    pub doc_type: String,
    pub slug: String,
    pub content_hash: String,
    pub managed_hash: String,
    pub open_hash: String,
    pub uri: String,
    /// Most recent audit ID for this resource on the server.
    #[serde(default)]
    pub last_audit_id: Option<ResourceAuditId>,
}

/// Response body for `GET /api/sync/manifest`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncManifestResponse {
    pub items: Vec<SyncManifestItem>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

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
                resource_id: ResourceId::from(Uuid::nil()),
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
    fn sync_manifest_response_serde_roundtrip() {
        let resp = SyncManifestResponse {
            items: vec![SyncManifestItem {
                resource_id: ResourceId::from(Uuid::nil()),
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
            kind: SyncItemKind::Body,
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("null"));
        let parsed: SyncPushItem = serde_json::from_str(&json).unwrap();
        assert!(parsed.resource_id.is_none());
    }

    #[test]
    fn sync_item_kind_default_is_body() {
        let kind = SyncItemKind::default();
        assert_eq!(kind, SyncItemKind::Body);
    }

    #[test]
    fn sync_push_item_default_kind_body() {
        // Old wire payloads without a `kind` field must deserialize to Body.
        let json = r#"{"uri":"kb://temper/task/abc","resource_id":null}"#;
        let parsed: SyncPushItem = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.kind, SyncItemKind::Body);
    }

    #[test]
    fn sync_push_item_meta_only_roundtrip() {
        let item = SyncPushItem {
            uri: "kb://temper/task/abc".to_string(),
            resource_id: None,
            kind: SyncItemKind::MetaOnly,
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(
            json.contains("\"kind\":\"meta_only\""),
            "expected snake_case meta_only in {json}"
        );
        let parsed: SyncPushItem = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.kind, SyncItemKind::MetaOnly);
    }

    #[test]
    fn sync_pull_item_default_kind_body() {
        // Old wire payloads without a `kind` field must deserialize to Body.
        let json = r#"{"uri":"kb://temper/task/abc","resource_id":"00000000-0000-0000-0000-000000000000","content_hash":"sha256:abc"}"#;
        let parsed: SyncPullItem = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.kind, SyncItemKind::Body);
    }

    #[test]
    fn sync_pull_item_meta_only_roundtrip() {
        let item = SyncPullItem {
            uri: "kb://temper/task/abc".to_string(),
            resource_id: ResourceId::from(Uuid::nil()),
            content_hash: "sha256:abc".to_string(),
            kind: SyncItemKind::MetaOnly,
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(
            json.contains("\"kind\":\"meta_only\""),
            "expected snake_case meta_only in {json}"
        );
        let parsed: SyncPullItem = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.kind, SyncItemKind::MetaOnly);
    }
}
