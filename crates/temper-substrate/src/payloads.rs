//! Typed event payloads — the ledger's wire contract (2026-06-09 event-payload-formalization spec §3).
//!
//! One struct per event type; `fire()` serializes these into `kb_events.payload` and the SQL
//! `_project_<type>` halves read ONLY the payload. The boundary-shared types have been lifted to
//! temper-core (ids; and now `AgentAuthorship`/`ConfidenceBand`, re-exported below); the remaining
//! payload structs stay HERE as the substrate's wire contract, parity-shaped for a later temper-core
//! lift at convergence (same pattern as the local `EventKind`). The committed JSON-Schema snapshots
//! (tests/fixtures/payloads/) are the cross-system contract meanwhile.
//!
//! The exclusion rule: DERIVED STATE IS NEVER PAYLOAD. Embeddings (recomputed/copied; model identity
//! rides event metadata), block_body_hash / resource body_hash (merkles over carried chunk hashes),
//! and region readouts (centroid/cohesion/salience) are all derivable — the payload records inputs
//! and acts, never derivations.

use crate::affinity::EdgeKind;
use crate::content::PreparedBlock;
use crate::ids::{
    BlockId, ChunkId, CogmapId, ContextId, EdgeId, EntityId, EventId, InvocationId, LensId,
    ProfileId, PropertyId, RegionId, ResourceId,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use temper_core::types::home::HomeAnchor;
use uuid::Uuid;

// ── shared shapes ───────────────────────────────────────────────────────────

/// A polymorphic anchor/endpoint reference. Serializes table names exactly as the DDL spells them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum AnchorTable {
    #[serde(rename = "kb_contexts")]
    Contexts,
    #[serde(rename = "kb_cogmaps")]
    Cogmaps,
    #[serde(rename = "kb_resources")]
    Resources,
    #[serde(rename = "kb_edges")]
    Edges,
    #[serde(rename = "kb_content_blocks")]
    ContentBlocks,
    #[serde(rename = "kb_teams")]
    Teams,
    #[serde(rename = "kb_profiles")]
    Profiles,
}

impl From<HomeAnchor> for AnchorTable {
    fn from(a: HomeAnchor) -> Self {
        match a {
            HomeAnchor::Context(_) => AnchorTable::Contexts,
            HomeAnchor::Cogmap(_) => AnchorTable::Cogmaps,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct AnchorRef {
    pub table: AnchorTable,
    pub id: Uuid,
}

impl AnchorRef {
    pub fn resource(id: ResourceId) -> Self {
        AnchorRef {
            table: AnchorTable::Resources,
            id: id.uuid(),
        }
    }
    pub fn cogmap(id: CogmapId) -> Self {
        AnchorRef {
            table: AnchorTable::Cogmaps,
            id: id.uuid(),
        }
    }
    pub fn context(id: ContextId) -> Self {
        AnchorRef {
            table: AnchorTable::Contexts,
            id: id.uuid(),
        }
    }
}

/// kb_edges.polarity. The projection's only non-parameter column today — carried explicitly so the
/// payload covers every projected column (spec §9 column-coverage obligation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum EdgePolarity {
    #[default]
    Forward,
    Inverse,
}

impl EdgePolarity {
    /// Parse a `polarity` SQL enum label into the typed polarity — used by synthesis to carry a
    /// production `kb_resource_edges.polarity` text value verbatim (§4). `None` for an unrecognized
    /// label (escalates at the call site).
    pub fn from_sql(s: &str) -> Option<Self> {
        match s {
            "forward" => Some(EdgePolarity::Forward),
            "inverse" => Some(EdgePolarity::Inverse),
            _ => None,
        }
    }
}

/// Content-addressed chunk reference: structure + hash, NEVER prose (CAS rule, spec §0.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ChunkManifest {
    pub chunk_id: ChunkId,
    pub chunk_index: i32,
    pub content_hash: String,
}

/// One block's manifest. `block_body_hash` deliberately absent — it is sha256(ordered chunk hashes),
/// derived in the projector (the derived-state rule applied to the spec's own §3 sketch).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct BlockManifest {
    pub block_id: BlockId,
    pub seq: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub chunks: Vec<ChunkManifest>,
    /// The ordered sources this block's content was incorporated from — recorded into
    /// `kb_block_provenance` by the projector. Empty (and skipped on the wire) for the
    /// scenario/charter paths; set by the resource create/update write path from the caller's sources.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub incorporated: Vec<Incorporation>,
}

impl From<&crate::content::PreparedChunk> for ChunkManifest {
    fn from(c: &crate::content::PreparedChunk) -> Self {
        ChunkManifest {
            chunk_id: c.chunk_id,
            chunk_index: c.chunk_index,
            content_hash: c.content_hash.clone(),
        }
    }
}

impl From<&PreparedBlock> for BlockManifest {
    fn from(b: &PreparedBlock) -> Self {
        BlockManifest {
            block_id: b.block_id,
            seq: b.seq,
            role: b.role.clone(),
            chunks: b
                .chunks
                .iter()
                .map(|c| ChunkManifest {
                    chunk_id: c.chunk_id,
                    chunk_index: c.chunk_index,
                    content_hash: c.content_hash.clone(),
                })
                .collect(),
            incorporated: b.incorporated.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct TelosManifest {
    pub resource_id: ResourceId,
    pub title: String,
    pub origin_uri: String,
    pub blocks: Vec<BlockManifest>,
}

// ── the content sidecar (NOT payload — persisted to the CAS, never on the ledger) ──

/// Either a fresh f32 vector (fire path) or pgvector's text form (replay path); the SQL projector
/// casts both. `None`/absent ⇒ NULL embedding (e.g. the pure-SQL 03_seed path; embed_chunks backfills).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EmbeddingRepr {
    Vector(Vec<f32>),
    Text(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkContent {
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<EmbeddingRepr>,
    /// Production heading metadata (§8 carry-as-is): persisted onto `kb_chunks.header_path` /
    /// `heading_depth` by `_insert_chunk`, never part of the manifest/CAS hash. Skipped when absent
    /// (the scenario-authoring path) so the columns stay NULL exactly as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_depth: Option<i16>,
}

/// Insert one run of prepared chunks into a `{chunk_id: {content, embedding}}` sidecar map (keyed by
/// chunk id string — JSONB object keys are strings). The single place that maps a `PreparedChunk` to
/// its sidecar entry, shared by the block-iterating [`content_sidecar`] (create paths) and the
/// `block_mutated` fire arm (revise path) so the two can never disagree on the entry shape.
pub fn content_sidecar_chunks(
    map: &mut HashMap<String, ChunkContent>,
    chunks: &[crate::content::PreparedChunk],
) {
    for c in chunks {
        map.insert(
            c.chunk_id.to_string(),
            ChunkContent {
                content: c.content.clone(),
                // A deferred chunk (`embedding: None`) drops the sidecar `embedding` key entirely
                // (skip_serializing_if), which `_insert_chunk` reads as a NULL vector — the write-side
                // half of the async-embed path (issue #299). An embedded chunk carries its vector.
                embedding: c.embedding.clone().map(EmbeddingRepr::Vector),
                header_path: c.header_path.clone(),
                heading_depth: c.heading_depth,
            },
        );
    }
}

/// Build the `{chunk_id: {content, embedding}}` sidecar `cogmap_genesis`/`resource_create` take as
/// `p_content`, over an ordered run of blocks.
pub fn content_sidecar(blocks: &[PreparedBlock]) -> HashMap<String, ChunkContent> {
    let mut map = HashMap::new();
    for b in blocks {
        content_sidecar_chunks(&mut map, &b.chunks);
    }
    map
}

// ── references (spec §4) ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum RefRel {
    Supersedes,
    DerivedFrom,
    Touches,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum RefTarget {
    Event(Uuid),
    Resource(Uuid),
    Block(Uuid),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct EventReference {
    pub rel: RefRel,
    pub target: RefTarget,
}

// ── the six live payloads ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct CogmapSeeded {
    pub cogmap_id: CogmapId,
    pub name: String,
    pub owner_profile_id: ProfileId,
    pub telos: TelosManifest,
}

/// `charter_set` payload — replace a cogmap's telos charter with this ordered role-tagged block set.
/// `blocks` is the same `BlockManifest` shape `CogmapSeeded::telos.blocks` carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct CharterSet {
    pub cogmap_id: CogmapId,
    pub blocks: Vec<BlockManifest>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ResourceCreated {
    pub resource_id: ResourceId,
    pub title: String,
    pub origin_uri: String,
    pub home: AnchorRef,
    pub owner_profile_id: ProfileId,
    /// The home's originator (§2: "carrying its current originator/owner"). Absent ⇒ the projector
    /// COALESCEs it to `owner_profile_id` (the scenario path, where originator≡owner). Synthesis sets
    /// it explicitly so a production row whose originator differs from its owner survives the carry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub originator_profile_id: Option<ProfileId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    pub blocks: Vec<BlockManifest>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct RelationshipAsserted {
    pub edge_id: EdgeId,
    pub source: AnchorRef,
    pub target: AnchorRef,
    pub edge_kind: EdgeKind,
    #[serde(default)]
    pub polarity: EdgePolarity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub weight: f64,
    pub home: AnchorRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct PropertyAsserted {
    pub property_id: PropertyId,
    pub owner: AnchorRef,
    pub property_key: String,
    pub value: serde_json::Value,
    pub weight: f64,
}

/// Set a **single-valued** property (WS6 4c): the projection folds prior active rows for
/// `(owner, property_key)` then inserts this value, so the key holds exactly one current value (the
/// resource-frontmatter shape, where each managed/open key has one value). Distinct from
/// `PropertyAsserted` (append — the multi-valued facet shape, kept for `facet_set`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct PropertySet {
    pub property_id: PropertyId,
    pub owner: AnchorRef,
    pub property_key: String,
    pub value: serde_json::Value,
    pub weight: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct LensWeights {
    pub express: f64,
    pub contains: f64,
    pub leads_to: f64,
    pub near: f64,
    pub prop: f64,
    /// The sparse exact-kNN cosine weight — the regime switch (spec §3.1). **Defaulted, not required:**
    /// `kb_events` is append-only, so the `lens_created` events for the pre-kernel lenses are immortal
    /// and carry no `cos` key. A required field would break `replay`'s round-trip through this struct
    /// on every one of them. The default (0.0) is exactly what those lenses ARE — declared-only.
    #[serde(default)]
    pub cos: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct SalienceWeights {
    pub telos: f64,
    #[serde(rename = "ref")]
    pub reference: f64,
    pub central: f64,
}

/// The kNN graph's construction params. Defaulted for the same append-only reason as
/// [`LensWeights::cos`], and to the same values the SQL columns default to — so a pre-kernel
/// `lens_created` event replays to exactly the row that already exists.
fn default_knn_k() -> u32 {
    12
}
fn default_cos_floor() -> f64 {
    0.55
}

/// The goal-liveness constants (spec §3.4) — a context's telos is the liveness-weighted centroid of
/// its goals, and these are the terms of that weighting.
///
/// Grouped rather than flattened onto [`LensCreated`] because they travel as one calibration, and
/// because they are read ONLY on the context branch of the telos: a cogmap declares its telos as a
/// charter resource and never consults them.
///
/// Defaulted as a whole (`#[serde(default)]` on the field) for the same append-only reason as
/// [`LensWeights::cos`]: `kb_events` is immutable, so every `lens_created` event written before T5
/// carries no `telos` key and must still round-trip through `replay`. The values below mirror the SQL
/// column defaults and `_project_lens_created`'s COALESCEs — one calibration, declared in three
/// places that must agree, so replaying an old event reproduces exactly the row that already exists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct TelosConstants {
    #[serde(default = "default_halflife_days")]
    pub halflife_days: f64,
    #[serde(default = "default_sw_in_progress")]
    pub sw_in_progress: f64,
    #[serde(default = "default_sw_backlog")]
    pub sw_backlog: f64,
    /// **Calibrated to 0.0**, and that is load-bearing rather than a rounding-down of 0.15. A positive
    /// weight here is summed over EVERY closed task under a goal, and closing a task *touches* it — so
    /// the decay term is ~1.0 for exactly the tasks that just finished. Measured on the @me/temper
    /// census the spec nominates as its fixture (§3.4, §5), 0.15 ranked the `Maintenance` goal (68
    /// done, 0 in progress) **first of 32**, above every arc under active development. Old completed
    /// work is history, not purpose.
    #[serde(default = "default_sw_done")]
    pub sw_done: f64,
    #[serde(default = "default_damper_paused")]
    pub damper_paused: f64,
    #[serde(default = "default_damper_completed")]
    pub damper_completed: f64,
}

fn default_halflife_days() -> f64 {
    30.0
}
fn default_sw_in_progress() -> f64 {
    1.0
}
fn default_sw_backlog() -> f64 {
    0.35
}
fn default_sw_done() -> f64 {
    0.0
}
fn default_damper_paused() -> f64 {
    0.3
}
fn default_damper_completed() -> f64 {
    0.4
}

impl Default for TelosConstants {
    fn default() -> Self {
        TelosConstants {
            halflife_days: default_halflife_days(),
            sw_in_progress: default_sw_in_progress(),
            sw_backlog: default_sw_backlog(),
            sw_done: default_sw_done(),
            damper_paused: default_damper_paused(),
            damper_completed: default_damper_completed(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct LensCreated {
    pub lens_id: LensId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cogmap_id: Option<CogmapId>,
    pub name: String,
    pub selection_kind: String,
    pub weights: LensWeights,
    pub salience: SalienceWeights,
    pub resolution: f64,
    #[serde(default = "default_knn_k")]
    pub knn_k: u32,
    #[serde(default = "default_cos_floor")]
    pub cos_floor: f64,
    #[serde(default)]
    pub telos: TelosConstants,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct RegionMaterialized {
    /// The anchor the regions were formed over — a context OR a cognitive map (spec §3.6 M2).
    /// Supersedes `cogmap_id`.
    pub home_anchor_table: AnchorTable,
    pub home_anchor_id: Uuid,
    /// VESTIGIAL, dual-written through the expand window. `kb_events` is APPEND-ONLY: every
    /// `region_materialized` event written before T3 carries this key and no anchor pair, and those
    /// rows are immortal. Keeping it written (and OPTIONAL, so a context act can omit it) is what lets
    /// the ledger probe in `replay::last_materialize_event` read old and new acts with one query.
    /// `None` for a context anchor. Do not read this in new code.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cogmap_id: Option<CogmapId>,
    pub lens_id: LensId,
    /// Max event id over the substrate at load time — the point-in-time the projection saw.
    pub watermark_event_id: EventId,
    /// The per-lens membership signature (sorted member-uuid join). Doubles as the drift-detection
    /// decision's persisted fingerprint artifact.
    pub membership_fingerprint: String,
    pub region_ids: Vec<RegionId>,
}

// ── the designed-but-unbuilt families (schemas now, wiring later — spec §3) ──

/// Mirrors production `temper-core/src/types/relationship_events.rs` + `edge_id` (identity-as-input).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct RelationshipRetyped {
    pub edge_id: EdgeId,
    pub edge_kind: EdgeKind,
    pub polarity: EdgePolarity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct RelationshipReweighted {
    pub edge_id: EdgeId,
    pub weight: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct RelationshipFolded {
    pub edge_id: EdgeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct RelationshipDecayed {
    pub edge_id: EdgeId,
    /// Multiplicative decay factor applied to the edge weight (0.0..1.0) — production's shape.
    pub factor: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct RelationshipCorrected {
    pub edge_id: EdgeId,
    /// Structured account of the wrongness — the scar (production's shape).
    pub scar: String,
}

// ── resource mutations (WS6 4c live write path) ──────────────────────────────

/// Soft-delete a resource — projection flips `is_active`. Identity-only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ResourceDeleted {
    pub resource_id: ResourceId,
}

/// Update mutable `kb_resources` columns. Each field is optional — absent ⇒ the projector COALESCEs to
/// the current value, so a partial update carries only what changed (`title`/`origin_uri` are the §9
/// invariants this covers; stage/mode/effort/doc_type live as properties, set via `facet_set`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ResourceUpdated {
    pub resource_id: ResourceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_uri: Option<String>,
}

/// Re-home a resource (context move) — re-point its single `kb_resource_homes` row to `home`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ResourceRehomed {
    pub resource_id: ResourceId,
    pub home: AnchorRef,
}

/// Reassign a resource's owner — set its home row's `owner_profile_id` to `to_profile_id`.
/// `from_profile_id` is recorded for the audit trail; the projector writes only the new owner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ResourceReassigned {
    pub resource_id: ResourceId,
    pub from_profile_id: ProfileId,
    pub to_profile_id: ProfileId,
}

// `ProvenanceSource` is the shared wire carrier — canonical home `temper_core::types::provenance`
// (CLAUDE.md: "the wire type lives in temper-core", the same chain as authorship below). Re-exported
// so substrate's `payloads::ProvenanceSource` users (`Incorporation`, `BlockProvenanceCorrected`, and
// the projectors) stay stable.
pub use temper_core::types::provenance::ProvenanceSource;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct Incorporation {
    pub source: ProvenanceSource,
    pub seq: i32,
}

/// Payload for the (now fired) `block_created` event — one appended segment.
/// The projector (`block_append` → `_project_blocks`) reads `resource_id` + the
/// single-block manifest; the content sidecar carries the chunk prose/embeddings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct BlockCreated {
    pub resource_id: ResourceId,
    pub block: BlockManifest,
}

/// Payload for `resource_finalized` — a segmented ingest declared complete.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ResourceFinalized {
    pub resource_id: ResourceId,
    pub expected_blocks: u32,
    pub expected_body_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct BlockMutated {
    pub block_id: BlockId,
    pub chunks: Vec<ChunkManifest>,
    #[serde(default)]
    pub incorporated: Vec<Incorporation>,
}

/// Payload for `block_provenance_annotated` — attach provenance sources to an EXISTING block
/// without revising its content (issue #355). Carries only the block id + the ordered incorporation
/// list; the projector records them into `kb_block_provenance` via the same `_insert_block_provenance`
/// helper the create/revise paths use, but touches NO chunks — no re-chunk, no re-embed, no
/// `block_body_hash` recompute. `incorporated` is non-empty by construction (the `block_annotate`
/// write path rejects an empty list — an annotate with nothing to attribute is a caller error, not a
/// silent no-op).
///
/// Registered **permissive** (NULL `payload_schema`), like `resource_updated`/`invocation_closed`:
/// it is a post-canonical-seed event added by migration `20260710000001`, so it cannot join the
/// bootseed-stamped typed registry (that set is pinned to the immutable canonical-seed vocabulary via
/// `system.yaml`, and `TYPED_EVENT_NAMES` must equal it). The payload is instead validated Rust-side
/// through `verify_ledger_roundtrip`. The `JsonSchema` derive stays for parity with the other payload
/// structs even though no committed snapshot is stamped for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct BlockProvenanceAnnotated {
    pub block_id: BlockId,
    pub incorporated: Vec<Incorporation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct BlockFolded {
    pub block_id: BlockId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct BlockProvenanceCorrected {
    pub block_id: BlockId,
    pub source: ProvenanceSource,
    pub scar: String,
}

// ── invocation envelope + agent-authorship payloads ─────────────────────────

// Per-act agent-authorship metadata — the canonical home is now `temper_core::types::authorship`
// (CLAUDE.md: "the wire type lives in temper-core"). Re-exported here so substrate's
// `payloads::{AgentAuthorship, ConfidenceBand}` call sites (events.rs `EventContext`, the metadata
// serialization, temper-agents) stay stable. Authorship rides `kb_events.metadata`, invisible to
// projections/affinity by construction.
pub use temper_core::types::authorship::{AgentAuthorship, ConfidenceBand};

/// Terminal disposition of an invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Completed,
    Failed,
    Abandoned,
}

/// `delegated_launch` payload — opens an invocation envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct DelegatedLaunch {
    pub invocation_id: InvocationId,
    pub trigger_kind: String,
    pub originating_cogmap_id: CogmapId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_cogmap_id: Option<CogmapId>,
    pub scoped_entity_id: EntityId,
}

/// `invocation_closed` payload — closes an invocation with a terminal outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct InvocationClosed {
    pub invocation_id: InvocationId,
    pub disposition: Disposition,
    #[serde(default)]
    pub outcome: serde_json::Value,
}

/// The 15 typed event names — the registry-stamping and snapshot surfaces iterate this.
pub const TYPED_EVENT_NAMES: [&str; 15] = [
    "cogmap_seeded",
    "resource_created",
    "relationship_asserted",
    "property_asserted",
    "lens_created",
    "region_materialized",
    "relationship_retyped",
    "relationship_reweighted",
    "relationship_folded",
    "relationship_decayed",
    "relationship_corrected",
    "block_created",
    "block_mutated",
    "block_folded",
    "block_provenance_corrected",
];

/// Proof obligation 1 (payload spec §7.1): every event on the ledger whose type is typed here must
/// deserialize into its struct. Catches drift from ANY write path — Rust, hand-SQL, foreign.
pub async fn verify_ledger_roundtrip(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    let rows = sqlx::query!(
        "SELECT et.name AS type_name, e.id, e.payload \
           FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          ORDER BY e.id"
    )
    .fetch_all(pool)
    .await?;
    for r in rows {
        let res: anyhow::Result<()> = (|| {
            match r.type_name.as_str() {
                "cogmap_seeded" => {
                    serde_json::from_value::<CogmapSeeded>(r.payload.clone())?;
                }
                "resource_created" => {
                    serde_json::from_value::<ResourceCreated>(r.payload.clone())?;
                }
                "relationship_asserted" => {
                    serde_json::from_value::<RelationshipAsserted>(r.payload.clone())?;
                }
                "property_asserted" => {
                    serde_json::from_value::<PropertyAsserted>(r.payload.clone())?;
                }
                "lens_created" => {
                    serde_json::from_value::<LensCreated>(r.payload.clone())?;
                }
                "region_materialized" => {
                    serde_json::from_value::<RegionMaterialized>(r.payload.clone())?;
                }
                "relationship_folded" => {
                    serde_json::from_value::<RelationshipFolded>(r.payload.clone())?;
                }
                "block_provenance_annotated" => {
                    serde_json::from_value::<BlockProvenanceAnnotated>(r.payload.clone())?;
                }
                // Unlisted types (e.g. taxonomy entries no write path emits yet) are intentionally
                // not roundtripped here; add an arm when a write path begins emitting one.
                _ => {}
            }
            Ok(())
        })();
        if let Err(e) = res {
            anyhow::bail!(
                "event {} ({}) payload fails roundtrip: {e}",
                r.id,
                r.type_name
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::{PreparedBlock, PreparedChunk};

    #[test]
    fn manifest_from_prepared_block_excludes_prose_and_embedding() {
        let b = PreparedBlock {
            block_id: BlockId::from(Uuid::now_v7()),
            seq: 0,
            role: Some("statement".into()),
            chunks: vec![PreparedChunk {
                chunk_id: ChunkId::from(Uuid::now_v7()),
                chunk_index: 0,
                content_hash: "ab".repeat(32),
                content: "secret prose".into(),
                embedding: Some(vec![0.5; 4]),
                header_path: None,
                heading_depth: None,
            }],
            incorporated: vec![],
        };
        let m = BlockManifest::from(&b);
        let v = serde_json::to_value(&m).unwrap();
        let text = v.to_string();
        assert!(
            !text.contains("secret prose"),
            "prose must never enter a payload"
        );
        assert!(
            !text.contains("0.5"),
            "embeddings must never enter a payload"
        );
        assert_eq!(v["block_id"], serde_json::to_value(b.block_id).unwrap());
        assert_eq!(v["chunks"][0]["content_hash"], "ab".repeat(32));
    }

    #[test]
    fn anchor_table_serializes_as_ddl_table_names() {
        assert_eq!(
            serde_json::to_value(AnchorTable::Cogmaps).unwrap(),
            serde_json::json!("kb_cogmaps")
        );
        assert_eq!(
            serde_json::to_value(AnchorTable::Resources).unwrap(),
            serde_json::json!("kb_resources")
        );
    }

    #[test]
    fn payloads_roundtrip_serde() {
        let p = RelationshipAsserted {
            edge_id: EdgeId::from(Uuid::now_v7()),
            source: AnchorRef::resource(ResourceId::from(Uuid::now_v7())),
            target: AnchorRef::resource(ResourceId::from(Uuid::now_v7())),
            edge_kind: EdgeKind::Near,
            polarity: EdgePolarity::Forward,
            label: Some("contradicts".into()),
            weight: 1.0,
            home: AnchorRef::cogmap(CogmapId::from(Uuid::now_v7())),
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["polarity"], "forward");
        assert_eq!(v["edge_kind"], "near");
        assert_eq!(
            serde_json::from_value::<RelationshipAsserted>(v).unwrap(),
            p
        );
    }

    #[test]
    fn references_serialize_tagged() {
        let r = EventReference {
            rel: RefRel::DerivedFrom,
            target: RefTarget::Block(Uuid::nil()),
        };
        let v = serde_json::to_value(r).unwrap();
        assert_eq!(v["rel"], "derived_from");
        assert_eq!(v["target"]["kind"], "block");
    }

    #[test]
    fn sidecar_keys_by_chunk_id_and_carries_prose() {
        let b = PreparedBlock {
            block_id: BlockId::from(Uuid::now_v7()),
            seq: 0,
            role: None,
            chunks: vec![PreparedChunk {
                chunk_id: ChunkId::from(Uuid::now_v7()),
                chunk_index: 0,
                content_hash: "cd".repeat(32),
                content: "the prose".into(),
                embedding: Some(vec![1.0, 2.0]),
                header_path: None,
                heading_depth: None,
            }],
            incorporated: vec![],
        };
        let side = content_sidecar(std::slice::from_ref(&b));
        let entry = side.get(&b.chunks[0].chunk_id.to_string()).unwrap();
        assert_eq!(entry.content, "the prose");
        assert!(matches!(entry.embedding, Some(EmbeddingRepr::Vector(_))));
    }

    // A deferred chunk (`embedding: None`) yields a sidecar entry whose `embedding` is None, and that
    // entry serializes with the `embedding` key ABSENT — which `_insert_chunk` reads as a NULL vector
    // (issue #299). Content still rides through, so text + FTS persist while the vector is deferred.
    #[test]
    fn deferred_chunk_sidecar_entry_has_null_embedding() {
        use crate::content::prepare_block_deferred;
        let block = prepare_block_deferred(0, None, "deferred prose for the sidecar");
        let side = content_sidecar(std::slice::from_ref(&block));
        let entry = side.get(&block.chunks[0].chunk_id.to_string()).unwrap();
        assert_eq!(entry.content, "deferred prose for the sidecar");
        assert!(
            entry.embedding.is_none(),
            "deferred sidecar entry carries no vector"
        );
        // The serialized JSON drops the `embedding` key (skip_serializing_if = Option::is_none), which
        // is exactly the absent-key case the projector maps to NULL.
        let json = serde_json::to_value(entry).unwrap();
        assert!(json.get("content").is_some());
        assert!(
            json.get("embedding").is_none(),
            "absent embedding key is the projector's NULL signal: {json}"
        );
    }
}
