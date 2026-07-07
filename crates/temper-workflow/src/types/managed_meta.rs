use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use temper_core::types::ids::ResourceId;

/// Temper-governed **workflow + provenance** metadata for a vault resource.
///
/// This is a **closed, temper-owned vocabulary** of exactly the `KeyFate::Property`
/// keys — optional, smart-defaulted metadata. Identity (`title`/`slug`), type
/// (`doc_type_name`), home (`context`/`cogmap`), and relationships (`goal`) are NOT
/// metadata: they are first-class wire fields, never carried here. The Property field
/// set is kept in lockstep with `temper_substrate::keys::MANAGED_PROPERTY_KEYS` by the
/// drift-guard in `temper-services/tests/managed_meta_property_drift_test.rs`.
///
/// There is no catch-all — a key not named here is not a managed key. Caller-defined
/// ("bring-your-own") fields belong in `open_meta`, the free-form tier. Deserialization
/// rejects unknown keys (`#[serde(deny_unknown_fields)]`) so a mis-filed key (including a
/// former identity key like `temper-title`) fails loudly at the wire boundary instead of
/// silently migrating tiers.
///
/// All fields use `temper-*` YAML/JSON key names via `serde(rename)`.
/// `None` fields are omitted from serialized output.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "managed_meta.ts"))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ManagedMeta {
    /// Task workflow stage (task only)
    #[serde(rename = "temper-stage", skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,

    /// Task execution mode (task only)
    #[serde(rename = "temper-mode", skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,

    /// Task effort estimate (task only)
    #[serde(rename = "temper-effort", skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,

    /// Goal lifecycle status (goal only)
    #[serde(rename = "temper-status", skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,

    /// Sequence number for ordering (task/goal)
    #[serde(rename = "temper-seq", skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    /// Git branch associated with the task (task only)
    #[serde(rename = "temper-branch", skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// Pull request reference (task only)
    #[serde(rename = "temper-pr", skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,

    /// Model that produced this resource
    #[serde(rename = "temper-llm-model", skip_serializing_if = "Option::is_none")]
    pub llm_model: Option<String>,

    /// UUIDv7 stamp from a (historical) LLM-assisted run.
    #[serde(rename = "temper-llm-run", skip_serializing_if = "Option::is_none")]
    pub llm_run: Option<String>,

    /// How this resource was created (LLM-discovered or user-created)
    #[serde(rename = "temper-provenance", skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
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
    /// Per-act correlation (`invocation_id`) + discrete agent authorship. Flattened top-level keys;
    /// all optional. `confidence` required when any other authorship field is supplied. Parity with the
    /// MCP `update_resource_meta` tool — a frontmatter-only update under a run is correlated like any
    /// other authored act.
    #[serde(default, flatten)]
    pub act: temper_core::types::authorship::ActInput,
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
            stage: Some("backlog".to_string()),
            seq: Some(42),
            ..Default::default()
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(
            json.contains("\"temper-stage\""),
            "missing temper-stage key"
        );
        assert!(json.contains("\"temper-seq\""), "missing temper-seq key");
        // None fields are absent from serialized output.
        assert!(
            !json.contains("temper-mode"),
            "temper-mode should be absent"
        );
        assert!(
            !json.contains("temper-branch"),
            "temper-branch should be absent"
        );
        let parsed: ManagedMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, meta);
        assert_eq!(parsed.stage.as_deref(), Some("backlog"));
        assert_eq!(parsed.seq, Some(42));
    }

    #[test]
    fn managed_meta_yaml_roundtrip() {
        let meta = ManagedMeta {
            status: Some("active".to_string()),
            mode: Some("build".to_string()),
            ..Default::default()
        };
        let yaml = serde_yaml::to_string(&meta).unwrap();
        assert!(yaml.contains("temper-status:"), "missing temper-status key");
        assert!(yaml.contains("temper-mode:"), "missing temper-mode key");
        let parsed: ManagedMeta = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, meta);
        assert_eq!(parsed.status.as_deref(), Some("active"));
        assert_eq!(parsed.mode.as_deref(), Some("build"));
    }

    #[test]
    fn meta_update_payload_serde() {
        let payload = MetaUpdatePayload {
            resource_id: ResourceId::from(Uuid::nil()),
            managed_meta: ManagedMeta {
                stage: Some("in-progress".to_string()),
                ..Default::default()
            },
            open_meta: serde_json::json!({"tags": ["rust"]}),
            managed_hash: "sha256:abc123".to_string(),
            open_hash: "sha256:def456".to_string(),
            act: Default::default(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let parsed: MetaUpdatePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.resource_id, ResourceId::from(Uuid::nil()));
        assert_eq!(parsed.managed_hash, "sha256:abc123");
        assert_eq!(parsed.open_hash, "sha256:def456");
        assert_eq!(parsed.managed_meta.stage.as_deref(), Some("in-progress"));
        assert_eq!(parsed.open_meta["tags"][0], "rust");
    }

    #[test]
    fn managed_meta_rejects_unknown_keys() {
        // `managed_meta` is a closed, temper-owned vocabulary. `date` (an open
        // doc-type-schema field) and a caller-invented tag are not managed keys
        // and must be rejected, not silently absorbed.
        let json = r#"{"temper-stage":"backlog","date":"2026-04-13"}"#;
        let err = serde_json::from_str::<ManagedMeta>(json).unwrap_err();
        assert!(
            err.to_string().contains("date"),
            "rejection must name the offending key, got: {err}"
        );
        assert!(
            serde_json::from_str::<ManagedMeta>(r#"{"my-tag":"x"}"#).is_err(),
            "arbitrary caller keys belong in open_meta, not managed_meta"
        );
    }

    #[test]
    fn managed_meta_rejects_former_identity_keys() {
        // Identity/type/home/relationship keys left the managed vocabulary in
        // Phase 2 — they are first-class wire fields now and must be rejected here.
        for k in [
            "temper-title",
            "temper-slug",
            "temper-type",
            "temper-context",
            "temper-goal",
            "temper-updated",
            "temper-source",
        ] {
            let json = format!(r#"{{"{k}":"x"}}"#);
            assert!(
                serde_json::from_str::<ManagedMeta>(&json).is_err(),
                "{k} left managed_meta and must now be rejected"
            );
        }
    }

    #[test]
    fn managed_meta_accepts_the_closed_vocabulary() {
        // Every Property key deserializes cleanly (the readback/projection shape).
        let json = r#"{"temper-stage":"backlog","temper-mode":"build","temper-effort":"small",
            "temper-status":"active","temper-seq":3,"temper-branch":"b","temper-pr":"p",
            "temper-llm-model":"claude","temper-llm-run":"01947b5c-0000-0000-0000-000000000000",
            "temper-provenance":"llm-discovered"}"#;
        let parsed: ManagedMeta = serde_json::from_str(json).expect("closed vocabulary must parse");
        assert_eq!(parsed.stage.as_deref(), Some("backlog"));
        assert_eq!(parsed.pr.as_deref(), Some("p"));
    }

    #[test]
    fn managed_meta_llm_fields_roundtrip() {
        let meta = ManagedMeta {
            provenance: Some("llm-discovered".to_string()),
            llm_model: Some("claude-sonnet-4-20250514".to_string()),
            llm_run: Some("01947b5c-0000-0000-0000-000000000000".to_string()),
            stage: Some("done".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("\"temper-provenance\""));
        assert!(json.contains("\"temper-llm-model\""));
        assert!(json.contains("\"temper-llm-run\""));
        let parsed: ManagedMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, meta);
        assert_eq!(
            parsed.llm_model.as_deref(),
            Some("claude-sonnet-4-20250514")
        );
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
