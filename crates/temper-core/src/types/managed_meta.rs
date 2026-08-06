//! Temper-governed managed (`temper-*`) frontmatter and the manifest/meta wire shapes.
//!
//! Lives here rather than in temper-workflow because [`ManagedMeta`] is a field of
//! [`ResourceView`], which temper-substrate — the layer *below* temper-workflow —
//! reads back directly. `temper_workflow::types::managed_meta` re-exports every item
//! here at its incumbent path.
//!
//! [`ResourceView`]: crate::types::resource_view::ResourceView

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::ids::ResourceId;

/// `temper-provenance` value for a resource authored by a model.
///
/// This and [`PROVENANCE_USER_CREATED`] are the closed vocabulary declared by
/// `schemas/base.schema.json` (`"enum": ["llm-discovered", "user-created"]`).
///
/// [`ManagedMeta::provenance`] stays a `String` rather than a typed enum: `ManagedMeta`
/// is a `deny_unknown_fields` deserialization target for rows already in the database,
/// and a closed enum would turn any historical value outside the pair into a hard
/// readback failure.
pub const PROVENANCE_LLM_DISCOVERED: &str = "llm-discovered";

/// `temper-provenance` value for a resource authored by a person.
///
/// See [`PROVENANCE_LLM_DISCOVERED`].
pub const PROVENANCE_USER_CREATED: &str = "user-created";

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
// The `rename` and the `skip_serializing_if` are **two separate `#[serde(...)]` attributes on
// purpose.** ts-rs parses each attribute whole and *discards the entire attribute* when any part
// of it is unsupported — `skip_serializing_if` is unsupported ("ts-rs failed to parse this
// attribute. It will be ignored."), so combining them silently dropped the rename too, and
// `managed_meta.ts` declared `stage` where the wire carries `temper-stage`. Nothing consumed the
// generated type until `ResourceView` put this tier on the wire, which is when the divergence
// became reachable. Split, ts-rs keeps the rename and only warns about the skip.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "managed_meta.ts"))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ManagedMeta {
    /// Task workflow stage (task only)
    #[serde(rename = "temper-stage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,

    /// Task execution mode (task only)
    #[serde(rename = "temper-mode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,

    /// Task effort estimate (task only)
    #[serde(rename = "temper-effort")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,

    /// Goal lifecycle status (goal only)
    #[serde(rename = "temper-status")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,

    /// Sequence number for ordering (task/goal)
    #[serde(rename = "temper-seq")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,

    /// Git branch associated with the task (task only)
    #[serde(rename = "temper-branch")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,

    /// Pull request reference (task only)
    #[serde(rename = "temper-pr")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,

    /// Model that produced this resource
    #[serde(rename = "temper-llm-model")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_model: Option<String>,

    /// UUIDv7 stamp from a (historical) LLM-assisted run.
    #[serde(rename = "temper-llm-run")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_run: Option<String>,

    /// How this resource was created (LLM-discovered or user-created)
    #[serde(rename = "temper-provenance")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
}

/// Payload for meta-only sync updates that do not require re-chunking.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct MetaUpdatePayload {
    /// UUID of the resource being updated
    pub resource_id: ResourceId,
    /// Typed managed (temper-*) frontmatter — the closed Property vocabulary.
    /// Only the named `temper-*` keys are accepted; there is no catch-all.
    /// Hash stability is preserved because `managed_hash` is computed over the
    /// canonicalized form.
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
    pub act: crate::types::authorship::ActInput,
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
}
