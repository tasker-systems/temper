//! Block-provenance source — the shared wire carrier for "where an addressable block came from".
//!
//! Canonical home (CLAUDE.md: "the wire type lives in temper-core"). `temper-substrate` re-exports
//! `ProvenanceSource` from here (the same chain as `crate::ids` and [`crate::types::authorship`]) and
//! records it into `kb_block_provenance` via the `_project_blocks` / `_project_block_mutated`
//! projectors.
//!
//! Tagged to match the DDL's `provenance_source_kind` ENUM (`('event','resource')`); the `'remote'`
//! value (URL/external sources) is a later increment (T7c) — this enum has no URL variant yet.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Tagged like the DDL's provenance_source_kind ({kind, value} sum — content-block spec).
//
// NOTE: the `///` line above is emitted verbatim as this type's JSON-Schema `description` (it lands in
// the `block_mutated` / `block_provenance_corrected` payload-schema snapshots). Keep it byte-identical
// to substrate's prior definition so moving the type here is schema-neutral; enrich the module `//!`
// docs instead. `Resource` = a `kb_resources` id (distilled-from source); `Event` = a `kb_events` id
// (scar/correction path).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
// Inline the enum in MCP tool schemas. A `$ref` into `$defs` reaches the Anthropic tool-use layer
// with no type signal and comes back as `null` (the same bug fixed for `EdgeKind`/`ConfidenceBand`);
// inlining emits the variant shapes directly so the source is visible.
#[cfg_attr(feature = "mcp", schemars(inline))]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ProvenanceSource {
    Event(Uuid),
    Resource(Uuid),
}

/// One itemized block-provenance record — a single source's contribution to a resource's content
/// block, as returned by the `resource_block_provenance` SQL function in `(block_seq, accretion_seq)`
/// order. `source_kind` is the DDL `provenance_source_kind` enum rendered as text (`"resource"` /
/// `"event"`; `"remote"` arrives in T7c). Access-scoped in SQL — a principal who cannot read the
/// resource gets an empty set, never an error. The shared read shape for the MCP `get_block_provenance`
/// tool, the CLI `--provenance` view, and the HTTP provenance endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct BlockProvenanceRow {
    /// The content block this source contributed to.
    pub block_id: Uuid,
    /// Position of the block within its resource (0-based).
    pub block_seq: i32,
    /// `"resource"` or `"event"` (the DDL enum as text).
    pub source_kind: String,
    /// The contributing resource/event id.
    pub source_id: Uuid,
    /// Monotonic order in which this source shaped the block.
    pub accretion_seq: i32,
    /// The `block_mutated` event that recorded this incorporation.
    pub contributed_by_event_id: Uuid,
    pub created: chrono::DateTime<chrono::Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_source_is_tagged_kind_value() {
        let s = ProvenanceSource::Resource(Uuid::nil());
        let v = serde_json::to_value(s).unwrap();
        assert_eq!(v["kind"], "resource");
        assert_eq!(v["value"], "00000000-0000-0000-0000-000000000000");
        let back: ProvenanceSource = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn provenance_source_event_variant_roundtrips() {
        let s = ProvenanceSource::Event(Uuid::nil());
        let back: ProvenanceSource =
            serde_json::from_value(serde_json::to_value(s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
