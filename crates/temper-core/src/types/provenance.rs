//! Block-provenance source — the shared wire carrier for "where an addressable block came from".
//!
//! Canonical home (CLAUDE.md: "the wire type lives in temper-core"). `temper-substrate` re-exports
//! `ProvenanceSource` from here (the same chain as `crate::ids` and [`crate::types::authorship`]) and
//! records it into `kb_block_provenance` via the `_project_blocks` / `_project_block_mutated`
//! projectors.
//!
//! Tagged to match the DDL's `provenance_source_kind` ENUM (`('event','resource','remote')`). The
//! `'remote'` variant carries a URL string (not a UUID); the projector resolves it to a
//! `kb_remote_sources.id` at write time via `_upsert_remote_source`. Because that variant holds a
//! `String`, this enum is `Clone` but not `Copy`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Tagged like the DDL's provenance_source_kind ({kind, value} sum — content-block spec).
//
// NOTE: the `///` line above is emitted verbatim as this type's JSON-Schema `description` (it lands in
// the `block_mutated` / `block_provenance_corrected` payload-schema snapshots). Keep it byte-identical
// to substrate's prior definition so moving the type here is schema-neutral; enrich the module `//!`
// docs instead. `Resource` = a `kb_resources` id (distilled-from source); `Event` = a `kb_events` id
// (scar/correction path); `Remote` = an external URL (resolved to a `kb_remote_sources` id at write).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// An external URL (e.g. a Linear issue, a GitHub PR, a doc). The value is the URL as supplied;
    /// the projector normalizes + resolves it to a `kb_remote_sources.id` via `_upsert_remote_source`.
    Remote(String),
}

/// One itemized block-provenance record — a single source's contribution to a resource's content
/// block, as returned by the `resource_block_provenance` SQL function in `(block_seq, accretion_seq)`
/// order. `source_kind` is the DDL `provenance_source_kind` enum rendered as text (`"resource"` /
/// `"event"`; `"remote"` arrives in T7c). Access-scoped in SQL — a principal who cannot read the
/// resource gets an empty set, never an error. The shared read shape for the MCP `get_block_provenance`
/// tool, the CLI `--provenance` view, and the HTTP provenance endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct BlockProvenanceRow {
    /// The content block this source contributed to.
    pub block_id: Uuid,
    /// Position of the block within its resource (0-based).
    pub block_seq: i32,
    /// `"resource"`, `"event"`, or `"remote"` (the DDL enum as text).
    pub source_kind: String,
    /// The contributing resource/event id, or (for `"remote"`) the minted `kb_remote_sources` id.
    pub source_id: Uuid,
    /// For a `"remote"` source, the external URL as supplied; `None` for resource/event sources.
    pub source_uri: Option<String>,
    /// Monotonic order in which this source shaped the block.
    pub accretion_seq: i32,
    /// The `block_mutated` event that recorded this incorporation.
    pub contributed_by_event_id: Uuid,
    pub created: chrono::DateTime<chrono::Utc>,
    /// Carried attribution: `true` for a split/absorbed COPY written by a re-block or a
    /// whole-body replace — this block holds only part of the content the source once covered
    /// (or a duplicate of it), distinguishable at row grain from a direct assertion, never
    /// readable as direct. `false` — the serde default, so a NEW client reading an OLD server
    /// (deploy skew, field absent on the wire) parses, reading unmarked rows as asserted, which
    /// is exactly what they were.
    #[serde(default)]
    pub is_carried: bool,
}

/// One chunk's IDENTITY within a live block — structure and hash, never prose (the CAS rule:
/// content rides the body/content reads). The born block read's chunk listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct BlockChunkRef {
    pub chunk_id: Uuid,
    pub chunk_index: i32,
    pub content_hash: String,
}

/// One named successor of a folded block's content. The disposition map's absorbers/carried
/// block ids, each surfaced only when the caller passes that successor's own canonical read
/// predicate — invisible successors are omitted ENTIRELY (no id, no count: aggregate existence
/// is still an existence leak).
///
/// WIRE DECISION, ON THE RECORD: a successor carries only its block id — addressable today
/// because every fold producer folds within one resource, so the successor shares the folded
/// block's home. When span addressing (register clause 2) lets a successor cross a resource
/// boundary, this shape must grow a home-resource field (or the map must) — a deliberate
/// change then, not an accident discovered by a client that cannot construct an address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
#[cfg_attr(feature = "mcp", schemars(inline))]
pub struct BlockSuccessor {
    /// The surviving block (kept or created) holding the folded incumbent's content.
    pub block_id: Uuid,
}

/// Where a folded block's content went, as the read surface states it (the defined-dangling-state
/// design, D-D2). `located`/`content_gone` carry the fold event's disposition map; `unrecorded`
/// is the defined arm for folds the ledger does not map — `charter_set`, historical
/// `block_mutated` replaces-body folds, and any event predating the map. It states "the ledger
/// does not carry where this content went", which is honest and never reads as a live citation —
/// a different true statement from "gone", never an approximation of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
#[cfg_attr(feature = "mcp", schemars(inline))]
#[serde(tag = "disposition", rename_all = "snake_case")]
pub enum BlockFoldDisposition {
    /// Content locatable: `absorbers` are the surviving blocks whose section holds the
    /// incumbent's full chunk-hash multiset (kept AND created — on whole-body rewrites a
    /// freshly minted section is the common absorber); `carried` are the blocks holding a
    /// strict subset. Empty absorbers with content in carries is legitimate (a split with no
    /// whole home); both empty is `content_gone`, never an empty located.
    Located {
        absorbers: Vec<BlockSuccessor>,
        carried: Vec<BlockSuccessor>,
    },
    /// Nothing locatable: the incumbent had no current chunks, or its chunk hashes appear in
    /// no section of the new partition. No successor is named.
    ContentGone,
    /// The ledger does not carry the mapping for this fold — a defined, distinguishable
    /// disposition, never a guessed one.
    Unrecorded,
}

/// The three-state resolution of a block-addressed read (D-D1): every read surface states
/// `live`, `folded`, or `absent` BY NAME. On HTTP these map 200 / 410 Gone / 404 Not Found —
/// no redirect (D-D3): a Location would hand the caller a successor they may not be authorized
/// to follow, so successor-naming rides as data inside the gated envelope instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "block_read.ts"))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BlockRead {
    /// The block resolves and is live: identity, its chunks' identity, its provenance rows —
    /// a born assembly no earlier read returned.
    Live {
        block_id: Uuid,
        /// Position within the resource's live partition.
        seq: i32,
        /// The derived block merkle (kept-identity currency), `None` for derived-era rows.
        block_body_hash: Option<String>,
        /// The block's current chunks, in chunk order — identity, never prose.
        chunks: Vec<BlockChunkRef>,
        /// The block's provenance rows, the same shape and posture as the resource-grain
        /// provenance read.
        provenance: Vec<BlockProvenanceRow>,
    },
    /// The row persists, folded away by a re-partition: the already-persisted attribution
    /// history stays on the folded row (never reconstructed), and the disposition states where
    /// the content went — or that it is gone, or that the ledger does not record it. Successor
    /// relationships resolve from the fold event via the folded row's `last_event_id` (read-path
    /// only, O(1) per hop; no successor pointer is born on block rows — the ledger stays the
    /// authority). A named successor that is itself folded resolves by addressing it: each hop
    /// is one O(1) `last_event_id` walk, no ledger scan.
    Folded {
        block_id: Uuid,
        /// The fold event the resolution walked — the folded row's own `last_event_id`
        /// (NOT NULL; every fold face stamps it).
        folded_by_event_id: Uuid,
        disposition: BlockFoldDisposition,
        /// The folded block's attribution history, gated by the home resource's read.
        attribution_history: Vec<BlockProvenanceRow>,
    },
    /// No such row under this resource. Never serialized on HTTP — `absent` renders as the
    /// ordinary 404 face; the arm exists so every surface names the state instead of an
    /// undifferentiated error.
    Absent { block_id: Uuid },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_source_is_tagged_kind_value() {
        let s = ProvenanceSource::Resource(Uuid::nil());
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["kind"], "resource");
        assert_eq!(v["value"], "00000000-0000-0000-0000-000000000000");
        let back: ProvenanceSource = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn provenance_source_event_variant_roundtrips() {
        let s = ProvenanceSource::Event(Uuid::nil());
        let back: ProvenanceSource =
            serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn provenance_source_remote_carries_the_url_verbatim() {
        // `remote` sources ride the same tagged {kind,value} wire shape as resource/event, but the
        // value is the URL string (not a UUID) — the projector resolves it to a kb_remote_sources id.
        let s = ProvenanceSource::Remote("https://Example.com/Doc?q=1".to_owned());
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["kind"], "remote");
        assert_eq!(v["value"], "https://Example.com/Doc?q=1");
        let back: ProvenanceSource = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }
}
