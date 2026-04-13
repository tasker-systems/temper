use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ids::ResourceId;

/// Temper-governed frontmatter fields for a vault resource.
///
/// All fields use `temper-*` YAML/JSON key names via `serde(rename)`.
/// `None` fields are omitted from serialized output.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ManagedMeta {
    /// Document type (e.g., "task", "goal", "research")
    #[serde(rename = "temper-type", skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,

    /// Vault context / namespace
    #[serde(rename = "temper-context", skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,

    /// ISO 8601 timestamp of last managed update
    #[serde(rename = "temper-updated", skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,

    /// Source URL or reference
    #[serde(rename = "temper-source", skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,

    /// Task workflow stage (task only)
    #[serde(rename = "temper-stage", skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,

    /// Task execution mode (task only)
    #[serde(rename = "temper-mode", skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,

    /// Task effort estimate (task only)
    #[serde(rename = "temper-effort", skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,

    /// Parent goal reference (task only)
    #[serde(rename = "temper-goal", skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,

    /// Sequence number for ordering (task/goal)
    #[serde(rename = "temper-seq", skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    /// Git branch associated with the task (task only)
    #[serde(rename = "temper-branch", skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// Pull request reference (task only)
    #[serde(rename = "temper-pr", skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,

    /// Goal lifecycle status (goal only)
    #[serde(rename = "temper-status", skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,

    /// Human-readable title (identity transport, no rename)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// URL-safe slug (identity transport, no rename)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
}

/// Response body for the metadata-only GET endpoint.
///
/// Returns the current managed_meta / open_meta / hashes from a
/// resource's manifest row without reconstructing the markdown body
/// from `kb_chunks`. Used by the CLI sync pull path to fetch just the
/// meta tier when the body side already agrees.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "managed_meta.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceMetaResponse {
    /// UUID of the resource
    pub resource_id: ResourceId,
    /// Serialized managed (temper-*) frontmatter fields from the manifest.
    /// `None` only if the manifest row predates meta population.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<Value>,
    /// Serialized open (user-defined) frontmatter fields from the manifest.
    /// `None` only if the manifest row predates meta population.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<Value>,
    /// SHA-256 hash of the managed_meta JSON
    pub managed_hash: String,
    /// SHA-256 hash of the open_meta JSON
    pub open_hash: String,
}

/// Payload for meta-only sync updates that do not require re-chunking.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct MetaUpdatePayload {
    /// UUID of the resource being updated
    pub resource_id: ResourceId,
    /// Serialized managed (temper-*) frontmatter fields
    pub managed_meta: Value,
    /// Serialized open (user-defined) frontmatter fields
    pub open_meta: Value,
    /// SHA-256 hash of the managed_meta JSON
    pub managed_hash: String,
    /// SHA-256 hash of the open_meta JSON
    pub open_hash: String,
}

/// Row type mapping to the `kb_resource_manifests` table.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceManifestRow {
    /// UUID of the resource
    pub resource_id: ResourceId,
    /// SHA-256 hash of the resource body (frontmatter stripped)
    pub body_hash: String,
    /// Serialized managed (temper-*) frontmatter fields
    pub managed_meta: Value,
    /// Serialized open (user-defined) frontmatter fields
    pub open_meta: Value,
    /// SHA-256 hash of the managed_meta JSON
    pub managed_hash: String,
    /// SHA-256 hash of the open_meta JSON
    pub open_hash: String,
    /// Timestamp of the last manifest update
    pub updated: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn managed_meta_serde_roundtrip() {
        let meta = ManagedMeta {
            doc_type: Some("task".to_string()),
            stage: Some("backlog".to_string()),
            seq: Some(42),
            ..Default::default()
        };

        let json = serde_json::to_string(&meta).unwrap();

        // Verify temper-* keys are present
        assert!(json.contains("\"temper-type\""), "missing temper-type key");
        assert!(
            json.contains("\"temper-stage\""),
            "missing temper-stage key"
        );
        assert!(json.contains("\"temper-seq\""), "missing temper-seq key");

        // Verify None fields are absent
        assert!(
            !json.contains("temper-mode"),
            "temper-mode should be absent"
        );
        assert!(
            !json.contains("temper-goal"),
            "temper-goal should be absent"
        );
        assert!(
            !json.contains("temper-branch"),
            "temper-branch should be absent"
        );

        // Verify roundtrip
        let parsed: ManagedMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, meta);
        assert_eq!(parsed.doc_type.as_deref(), Some("task"));
        assert_eq!(parsed.stage.as_deref(), Some("backlog"));
        assert_eq!(parsed.seq, Some(42));
    }

    #[test]
    fn managed_meta_yaml_roundtrip() {
        let meta = ManagedMeta {
            doc_type: Some("goal".to_string()),
            status: Some("active".to_string()),
            title: Some("Improve sync reliability".to_string()),
            slug: Some("improve-sync-reliability".to_string()),
            ..Default::default()
        };

        let yaml = serde_yaml::to_string(&meta).unwrap();

        // Verify temper-* keys are present
        assert!(yaml.contains("temper-type:"), "missing temper-type key");
        assert!(yaml.contains("temper-status:"), "missing temper-status key");

        // title and slug have no rename
        assert!(yaml.contains("title:"), "missing title key");
        assert!(yaml.contains("slug:"), "missing slug key");

        // Verify roundtrip
        let parsed: ManagedMeta = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, meta);
        assert_eq!(parsed.doc_type.as_deref(), Some("goal"));
        assert_eq!(parsed.status.as_deref(), Some("active"));
        assert_eq!(parsed.title.as_deref(), Some("Improve sync reliability"));
        assert_eq!(parsed.slug.as_deref(), Some("improve-sync-reliability"));
    }

    #[test]
    fn meta_update_payload_serde() {
        let payload = MetaUpdatePayload {
            resource_id: ResourceId::from(Uuid::nil()),
            managed_meta: serde_json::json!({"temper-type": "task"}),
            open_meta: serde_json::json!({"tags": ["rust"]}),
            managed_hash: "sha256:abc123".to_string(),
            open_hash: "sha256:def456".to_string(),
        };

        let json = serde_json::to_string(&payload).unwrap();
        let parsed: MetaUpdatePayload = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.resource_id, ResourceId::from(Uuid::nil()));
        assert_eq!(parsed.managed_hash, "sha256:abc123");
        assert_eq!(parsed.open_hash, "sha256:def456");
        assert_eq!(parsed.managed_meta["temper-type"], "task");
        assert_eq!(parsed.open_meta["tags"][0], "rust");
    }
}
