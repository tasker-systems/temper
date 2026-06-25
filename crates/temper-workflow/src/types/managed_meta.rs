use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use temper_core::types::ids::ResourceId;

/// Temper-governed frontmatter fields for a vault resource.
///
/// All fields use `temper-*` YAML/JSON key names via `serde(rename)`.
/// `None` fields are omitted from serialized output.
///
/// The `extra` bucket collects any keys the typed fields above don't
/// name — most notably doc-type-schema fields like `date` (sessions)
/// and any server-injected fields the ingest pipeline populates. This
/// makes `ManagedMeta` a round-trip-lossless representation of the
/// JSONB column: deserialize → re-serialize produces byte-equivalent
/// JSON (up to canonicalization) no matter what lives in the blob.
///
/// Without this bucket, the default serde "ignore unknown fields"
/// behavior would silently drop anything not in the typed set, which
/// would break hash stability across a typed round-trip.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "managed_meta.ts"))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
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

    /// How this resource was created (LLM-discovered or user-created)
    #[serde(rename = "temper-provenance", skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,

    /// Model that produced this resource
    #[serde(rename = "temper-llm-model", skip_serializing_if = "Option::is_none")]
    pub llm_model: Option<String>,

    /// UUIDv7 stamp from a (historical) LLM-assisted run.
    #[serde(rename = "temper-llm-run", skip_serializing_if = "Option::is_none")]
    pub llm_run: Option<String>,

    /// Human-readable title. Renamed to `temper-title` per the
    /// temper-prefix contract for managed-tier keys.
    #[serde(rename = "temper-title", skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// URL-safe slug. Renamed to `temper-slug` per the temper-prefix
    /// contract for managed-tier keys.
    #[serde(rename = "temper-slug", skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,

    /// Any additional keys not named by the typed fields above. Includes
    /// doc-type-schema fields the registry knows about (e.g. `date` for
    /// sessions) and any future temper-managed fields added before this
    /// struct catches up. Critically, this bucket is what keeps the
    /// typed round-trip lossless — without it, serde silently drops
    /// unknown fields on deserialize.
    #[serde(flatten)]
    #[cfg_attr(feature = "typescript", ts(skip))]
    #[cfg_attr(feature = "mcp", schemars(skip))]
    pub extra: HashMap<String, Value>,
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
    /// Typed managed (temper-*) frontmatter from the manifest. The
    /// typed fields cover everything temper knows about; any extras
    /// the server stored round-trip through `ManagedMeta::extra`.
    /// `None` only if the manifest row predates meta population.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<ManagedMeta>,
    /// Open (user-defined) frontmatter fields from the manifest.
    /// Intentionally untyped — open_meta is the free-form tier. Typed
    /// extraction of relationship fields lives in `ResourceRelationships`
    /// (see `temper-core::types::graph`), which parses this value on
    /// demand and ignores anything it doesn't recognize.
    /// `None` only if the manifest row predates meta population.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<Value>,
    /// SHA-256 hash of the managed_meta JSON
    pub managed_hash: String,
    /// SHA-256 hash of the open_meta JSON
    pub open_hash: String,
}

/// Paginated meta-only response for resource list endpoints.
///
/// Mirror of [`crate::types::resource::ResourceListResponse`] with the
/// row type swapped to [`ResourceMetaResponse`]. Returned by
/// `GET /api/resources?meta_only=true`. Facets and total are computed
/// identically to the default list response — projection-independent.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "managed_meta.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceMetaListResponse {
    pub rows: Vec<ResourceMetaResponse>,
    pub total: i64,
    pub facets: crate::types::resource::ResourceFacets,
}

/// Payload for meta-only sync updates that do not require re-chunking.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct MetaUpdatePayload {
    /// UUID of the resource being updated
    pub resource_id: ResourceId,
    /// Typed managed (temper-*) frontmatter. The typed fields cover
    /// everything temper knows about; extras round-trip through
    /// `ManagedMeta::extra` without loss. Hash stability is preserved
    /// because `managed_hash` is computed over the canonicalized form.
    pub managed_meta: ManagedMeta,
    /// Serialized open (user-defined) frontmatter fields. Intentionally
    /// untyped — the open tier is the free-form bucket. Edge-relevant
    /// fields are parsed on demand via `ResourceRelationships`.
    pub open_meta: Value,
    /// SHA-256 hash of the managed_meta JSON (computed over canonical form)
    pub managed_hash: String,
    /// SHA-256 hash of the open_meta JSON (computed over canonical form)
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
    use crate::types::resource::ResourceFacets;
    use std::collections::HashMap;
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

        assert!(yaml.contains("temper-title:"), "missing temper-title key");
        assert!(yaml.contains("temper-slug:"), "missing temper-slug key");

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
            managed_meta: ManagedMeta {
                doc_type: Some("task".to_string()),
                ..Default::default()
            },
            open_meta: serde_json::json!({"tags": ["rust"]}),
            managed_hash: "sha256:abc123".to_string(),
            open_hash: "sha256:def456".to_string(),
        };

        let json = serde_json::to_string(&payload).unwrap();
        let parsed: MetaUpdatePayload = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.resource_id, ResourceId::from(Uuid::nil()));
        assert_eq!(parsed.managed_hash, "sha256:abc123");
        assert_eq!(parsed.open_hash, "sha256:def456");
        assert_eq!(parsed.managed_meta.doc_type.as_deref(), Some("task"));
        assert_eq!(parsed.open_meta["tags"][0], "rust");
    }

    #[test]
    fn managed_meta_extras_bucket_round_trips_unknown_fields() {
        // The flatten extras bucket is what makes the typed representation
        // lossless: a field the server wrote but the typed struct doesn't
        // name (e.g. `date` on a session) must survive a full round-trip.
        let json = r#"{"temper-type":"session","temper-title":"test","date":"2026-04-13"}"#;
        let parsed: ManagedMeta = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.doc_type.as_deref(), Some("session"));
        assert_eq!(parsed.title.as_deref(), Some("test"));
        assert_eq!(
            parsed.extra.get("date"),
            Some(&serde_json::json!("2026-04-13")),
            "unknown fields must land in the extras bucket",
        );

        // Re-serialize and deserialize again — `date` must survive.
        let reserialized = serde_json::to_string(&parsed).unwrap();
        let reparsed: ManagedMeta = serde_json::from_str(&reserialized).unwrap();
        assert_eq!(
            reparsed.extra.get("date"),
            Some(&serde_json::json!("2026-04-13")),
            "round-trip must preserve extras",
        );
    }

    #[test]
    fn managed_meta_llm_fields_roundtrip() {
        let meta = ManagedMeta {
            provenance: Some("llm-discovered".to_string()),
            llm_model: Some("claude-sonnet-4-20250514".to_string()),
            llm_run: Some("01947b5c-0000-0000-0000-000000000000".to_string()),
            doc_type: Some("task".to_string()),
            ..Default::default()
        };

        let json = serde_json::to_string(&meta).unwrap();

        // Verify temper-* LLM keys are present
        assert!(
            json.contains("\"temper-provenance\""),
            "missing temper-provenance key"
        );
        assert!(
            json.contains("\"temper-llm-model\""),
            "missing temper-llm-model key"
        );
        assert!(
            json.contains("\"temper-llm-run\""),
            "missing temper-llm-run key"
        );

        // Verify values are preserved
        assert!(
            json.contains("llm-discovered"),
            "llm-discovered value missing"
        );
        assert!(
            json.contains("claude-sonnet-4-20250514"),
            "model value missing"
        );

        // Verify roundtrip
        let parsed: ManagedMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, meta);
        assert_eq!(parsed.provenance.as_deref(), Some("llm-discovered"));
        assert_eq!(
            parsed.llm_model.as_deref(),
            Some("claude-sonnet-4-20250514")
        );
        assert_eq!(
            parsed.llm_run.as_deref(),
            Some("01947b5c-0000-0000-0000-000000000000")
        );
    }

    #[test]
    fn managed_meta_serializes_title_as_temper_title_key() {
        let meta = ManagedMeta {
            title: Some("Improve sync".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(
            json.contains("\"temper-title\""),
            "expected temper-title key, got: {json}"
        );
        assert!(
            !json.contains("\"title\":"),
            "bare title key must not appear, got: {json}"
        );
    }

    #[test]
    fn managed_meta_deserializes_temper_title_into_title_field() {
        let json = r#"{"temper-title":"Improve sync"}"#;
        let parsed: ManagedMeta = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.title.as_deref(), Some("Improve sync"));
    }

    #[test]
    fn managed_meta_serializes_slug_as_temper_slug_key() {
        let meta = ManagedMeta {
            slug: Some("improve-sync".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(
            json.contains("\"temper-slug\""),
            "expected temper-slug key, got: {json}"
        );
        assert!(
            !json.contains("\"slug\":"),
            "bare slug key must not appear, got: {json}"
        );
    }

    #[test]
    fn managed_meta_deserializes_temper_slug_into_slug_field() {
        let json = r#"{"temper-slug":"improve-sync"}"#;
        let parsed: ManagedMeta = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.slug.as_deref(), Some("improve-sync"));
    }

    #[test]
    fn resource_meta_list_response_roundtrip() {
        let response = ResourceMetaListResponse {
            rows: vec![],
            total: 0,
            facets: ResourceFacets {
                doc_type: HashMap::new(),
            },
        };
        let json = serde_json::to_value(&response).expect("serialize");
        let back: ResourceMetaListResponse =
            serde_json::from_value(json.clone()).expect("deserialize");
        assert_eq!(back.total, 0);
        assert!(back.rows.is_empty());
        assert_eq!(json["rows"], serde_json::json!([]));
        assert_eq!(json["total"], 0);
        assert!(json["facets"].is_object());
    }
}
