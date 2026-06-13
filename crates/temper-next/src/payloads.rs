//! Typed event payloads — the ledger's wire contract (2026-06-09 event-payload-formalization spec §3).
//!
//! One struct per event type; `fire()` serializes these into `kb_events.payload` and the SQL
//! `_project_<type>` halves read ONLY the payload. Authored HERE (not temper-core) for now —
//! temper-next deliberately carries no temper-core dependency pre-slim; these are parity-shaped for
//! the temper-core lift at convergence (same pattern as the local `EventKind`). The committed
//! JSON-Schema snapshots (schema-artifact/payloads/) are the cross-system contract meanwhile.
//!
//! The exclusion rule: DERIVED STATE IS NEVER PAYLOAD. Embeddings (recomputed/copied; model identity
//! rides event metadata), block_body_hash / resource body_hash (merkles over carried chunk hashes),
//! and region readouts (centroid/cohesion/salience) are all derivable — the payload records inputs
//! and acts, never derivations.

use crate::affinity::EdgeKind;
use crate::content::PreparedBlock;
use crate::ids::{
    BlockId, ChunkId, CogmapId, ContextId, EdgeId, EventId, LensId, ProfileId, PropertyId,
    RegionId, ResourceId,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
                embedding: Some(EmbeddingRepr::Vector(c.embedding.clone())),
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct LensWeights {
    pub express: f64,
    pub contains: f64,
    pub leads_to: f64,
    pub near: f64,
    pub prop: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct SalienceWeights {
    pub telos: f64,
    #[serde(rename = "ref")]
    pub reference: f64,
    pub central: f64,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct RegionMaterialized {
    pub cogmap_id: CogmapId,
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

/// Tagged like the DDL's provenance_source_kind ({kind, value} sum — content-block spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum ProvenanceSource {
    Event(Uuid),
    Resource(Uuid),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct Incorporation {
    pub source: ProvenanceSource,
    pub seq: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct BlockCreated {
    pub block_id: BlockId,
    pub resource_id: ResourceId,
    pub seq: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct BlockMutated {
    pub block_id: BlockId,
    pub chunks: Vec<ChunkManifest>,
    #[serde(default)]
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
                embedding: vec![0.5; 4],
                header_path: None,
                heading_depth: None,
            }],
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
                embedding: vec![1.0, 2.0],
                header_path: None,
                heading_depth: None,
            }],
        };
        let side = content_sidecar(std::slice::from_ref(&b));
        let entry = side.get(&b.chunks[0].chunk_id.to_string()).unwrap();
        assert_eq!(entry.content, "the prose");
        assert!(matches!(entry.embedding, Some(EmbeddingRepr::Vector(_))));
    }
}
