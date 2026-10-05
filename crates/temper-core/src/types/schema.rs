//! The schema doors' answers: the document types' JSON Schemas and the open-meta convention, as
//! `GET /api/schema/*` describes them. Wire types, so temper-core's; temper-workflow builds them
//! from the schema files it embeds.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A discouraged open_meta key and the managed field that supersedes it.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "schema.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscouragedOpenMetaKey {
    pub key: String,
    pub use_instead: String,
}

/// The self-describing open_meta convention, returned by [`describe_open_meta`] and rendered by the
/// CLI `resource describe-open-meta` command, the MCP `describe_open_meta` tool, and
/// `GET /api/schema/open-meta`. All three surfaces share this type so the guidance can never drift
/// between them.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "schema.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenMetaConvention {
    /// The recognized-conventions JSON Schema. Self-describing: each property's `description` states
    /// whether the key is FTS-indexed (and at what weight) or shape-only, and the schema `title`
    /// carries the convention version. The tier stays open (`additionalProperties: true`), so this is
    /// guidance, not a closed vocabulary.
    pub schema: serde_json::Value,
    /// Discouraged bare keys — absent from `schema` because the tier is open, surfaced here so callers
    /// can see them → the managed field that supersedes each.
    pub discouraged_keys: Vec<DiscouragedOpenMetaKey>,
}

/// Summary of a document type — the row shape of the doc-type list.
///
/// Doc-types are name-keyed in the substrate (no `kb_doc_types` table), so the summary
/// carries no UUID: callers address doc-types by name.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "schema.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocTypeSummary {
    pub name: String,
    pub has_schema: bool,
    pub required_fields: Vec<String>,
}

/// Full description of one document type: its JSON Schema, the fields it requires, the
/// closed vocabularies its fields carry, and a filled-in example of the managed tier.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "schema.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocTypeDescription {
    pub name: String,
    /// The doc-type's own JSON Schema. Deliberately **not** merged with the base schema —
    /// [`schema_value`] says why, and [`base_schema_value`] is the other half for a caller
    /// that wants the whole field surface.
    pub schema: serde_json::Value,
    /// Doc-type-level required fields only (the base schema's are merged via `allOf` at
    /// validation time).
    pub required_fields: Vec<String>,
    /// Every field of this doc-type that carries a closed vocabulary, field name → values.
    /// This is the answer to *"which states does this kind of work have?"*.
    pub enum_fields: BTreeMap<String, Vec<String>>,
    pub example_managed_meta: serde_json::Value,
}
