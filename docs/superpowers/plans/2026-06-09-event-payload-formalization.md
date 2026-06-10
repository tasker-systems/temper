# Event Payload Formalization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the temper-next ledger replay-sufficient: typed payloads + references on every event, identity-as-input (Rust pre-generates ids), `_event_append`/`_project_*` split so replay is the same code path as normal operation, registry-published JSON-Schemas, and four proof obligations (roundtrip, replay, schema agreement, CAS retention).

**Architecture:** Approach A (payload-first) per `docs/superpowers/specs/2026-06-09-event-payload-formalization-design.md`. Rust (`fire(SeedAction)`) serializes a typed payload struct and calls a SQL mutation function; the function `_event_append`s the event row (payload verbatim) and projects **from the payload** via a `_project_<type>` half. Content-bearing events carry block→chunk *structure* (ids + hashes); prose+embeddings travel in a **content sidecar** persisted to `kb_chunk_content`/`kb_chunks` (the CAS), never written to the ledger.

**Tech Stack:** Rust (temper-next crate), plpgsql (schema-artifact), sqlx `query!` macros with per-crate offline cache (`cargo make prepare-next`), schemars (gated), cargo-nextest (`temper-next-write` serial group).

---

## Plan-time refinements of the spec (surface these in review; Task 13 records them in the spec)

These were discovered grounding the spec against the real code. Each is tagged with how it relates to the spec.

1. **[AMEND §7.2] Masked-surrogate replay diff.** Payloads carry ids for every row that other rows *reference* (resources, cogmaps, blocks, chunks, edges, lenses, regions, events). Rows whose surrogate `id` has **no inbound references** — `kb_resource_homes`, `kb_properties`, `kb_block_revisions` — regenerate ids on replay; the diff compares them with `id` masked, ordered by natural key. No information escapes through those ids, so the proof's strength is preserved without bloating manifests.
2. **[EXTEND §5] Projected timestamps come from the event.** Every `_project_*` sets projected `created`/`updated` columns from the event's `occurred_at` (never `now()`), making projections replay-stable by construction — and more correct: a projection's timestamp *is* the event time.
3. **[AMEND §3] `BlockManifest` omits `block_body_hash`.** It is a pure sha256 over the ordered chunk hashes — derived state, derivable in the projector. The spec's own exclusion rule ("derived state is never payload") applied to its own sketch.
4. **[CONFORM-narrowed §5] `_project_region_materialized` projects only the watermark** (`kb_cogmaps.shape_materialized_event_id`). Region rows are second-order derived compute (clustering output) and stay Rust-side; the replay proof for regions is: replay substrate → re-run `materialize_cogmap` → membership fingerprint equals the one recorded in the payload.
5. **[AMEND §7.2] Replay proof is harness-level per scenario,** not an in-YAML expectation kind — it drops and rebuilds the `temper_next` namespace, which cannot happen mid-scenario. Every write-path scenario test calls `prove_replay` after `run_scenario`, so every corpus scenario is still a replay proof.
6. **[EXTEND §5] Content sidecar shape.** Content-bearing functions take `(p_payload jsonb, p_content jsonb, p_emitter uuid)`. Sidecar = `{ "<chunk_id>": { "content": text, "embedding": [..]|"[..]"|null } }`. The projector iterates the **payload's** manifests and looks up the sidecar per chunk id (missing entry ⇒ exception; extras ignored), so structural truth comes only from the payload.

## File structure

| File | Action | Responsibility |
|---|---|---|
| `schema-artifact/01_schema.sql` | Modify | Envelope columns (`payload`, `"references"`, `payload_version`), registry columns (`payload_schema`, `schema_version`), GIN index, append-only trigger |
| `schema-artifact/02_functions.sql` | Modify | `_event_append`, `_project_*` halves, payload-first public functions, `_persist_resource_blocks` → `_project_blocks` |
| `schema-artifact/03_seed.sql` | Modify | `cogmap_genesis` call site → new payload signature |
| `schema-artifact/seeds/system.yaml` | Modify | + `relationship_decayed`, `relationship_corrected` |
| `schema-artifact/payloads/*.v1.schema.json` | Create | Committed JSON-Schema snapshots (15 files) |
| `crates/temper-next/src/ids.rs` | Modify | + `ChunkId`, `EdgeId`, `PropertyId`, `RegionId`; schemars derive on the macro |
| `crates/temper-next/src/content.rs` | Modify | `PreparedBlock`/`PreparedChunk` gain pre-generated ids |
| `crates/temper-next/src/payloads.rs` | Create | The 15 payload structs, `EventReference`, shared shapes, sidecar builder, roundtrip verifier |
| `crates/temper-next/src/events.rs` | Modify | `fire()` arms construct payloads + pre-generate ids; `Fired` gains typed ids |
| `crates/temper-next/src/write.rs` | Modify | Cluster-first ordering, fingerprint/region-ids into the payload, watermark capture |
| `crates/temper-next/src/replay.rs` | Create | Snapshot / restore / event-walk / projection-dump primitives |
| `crates/temper-next/src/scenario/bootseed.rs` | Modify | Stamp `payload_schema` from committed snapshot files |
| `crates/temper-next/src/lib.rs` | Modify | + `pub mod payloads; pub mod replay;` |
| `crates/temper-next/tests/ledger_envelope.rs` | Create | Append-only trigger negative test |
| `crates/temper-next/tests/payload_schema.rs` | Create | Schema snapshot test (scenario-schema gated) |
| `crates/temper-next/tests/replay_roundtrip.rs` | Create | The replay proof + CAS retention assertion |
| `.config/nextest.toml` | Modify | New test binaries join `temper-next-write` group |

**Environment for every task below:** `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development` (Docker Postgres: `cargo make docker-up`). Artifact tests need ONNX locally. All `cargo make` tasks run `SQLX_OFFLINE=true`; after ANY SQL change run `cargo make prepare-next` before building tests.

Work on branch `jct/event-payload-formalization-spec` (already exists, carries the spec).

---

### Task 1: Ledger envelope DDL (columns only — trigger comes in Task 8)

**Files:**
- Modify: `schema-artifact/01_schema.sql:239-271`

- [ ] **Step 1: Add registry columns to `kb_event_types`**

Replace lines 239–243 with:

```sql
CREATE TABLE kb_event_types (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    name            TEXT NOT NULL UNIQUE,
    -- The published contract: current JSON-Schema for this type's payload (NULL =
    -- unregistered/permissive — foreign/webhook types may stay NULL). Stamped by the
    -- boot-seed from the committed schema-artifact/payloads/*.schema.json snapshots.
    payload_schema  JSONB,
    -- First-class version declaration. Evolution: additive-only within a version;
    -- a breaking change bumps this and registers the new schema.
    schema_version  INT NOT NULL DEFAULT 1,
    created         TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

- [ ] **Step 2: Add envelope columns to `kb_events`**

In the `CREATE TABLE kb_events` block (line 255), insert after the `correlation_id` line:

```sql
    -- Typed, per-event-type, replay-sufficient (payload-first design, 2026-06-09 spec §1/§3).
    -- The projection halves (_project_*) read ONLY this.
    payload                JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- Typed provenance pointers: [{rel: supersedes|derived_from|touches, target:{kind,id}}]
    "references"           JSONB NOT NULL DEFAULT '[]'::jsonb,
    -- Which registered schema version this row's payload conforms to.
    payload_version        INT   NOT NULL DEFAULT 1,
```

And after the existing `idx_kb_events_correlation` index (line 271) add:

```sql
CREATE INDEX idx_kb_events_references ON kb_events USING GIN ("references" jsonb_path_ops);
```

- [ ] **Step 3: Load the artifact to prove the DDL parses**

Run: `psql "$DATABASE_URL" -q -v ON_ERROR_STOP=1 -f schema-artifact/01_schema.sql -f schema-artifact/02_functions.sql`
Expected: exit 0, no errors.

- [ ] **Step 4: Run the existing write-path suite (unchanged behavior)**

Run: `cargo make prepare-next && cargo nextest run -p temper-next --features artifact-tests`
Expected: all pass (functions don't touch the new columns yet; defaults satisfy NOT NULL).

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/01_schema.sql crates/temper-next/.sqlx
git commit -m "event-payloads task 1: ledger envelope columns (payload/references/payload_version) + registry schema columns"
```

---

### Task 2: New id newtypes + schemars on the macro

**Files:**
- Modify: `crates/temper-next/src/ids.rs`

- [ ] **Step 1: Gate a schemars derive into the `id_newtype!` macro**

In the macro's derive list (ids.rs:15-18), add below the existing `#[sqlx(transparent)]` attribute line:

```rust
        #[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
```

- [ ] **Step 2: Add the four new newtypes** (after the `LensId` invocation, ids.rs:73-76)

```rust
id_newtype!(
    /// A `kb_chunks` row (one embedding window of a block's prose).
    ChunkId
);
id_newtype!(
    /// A `kb_edges` row (a declared relationship assertion).
    EdgeId
);
id_newtype!(
    /// A `kb_properties` row (a facet/doc_type/block_role assertion).
    PropertyId
);
id_newtype!(
    /// A `kb_cogmap_regions` row (one materialized region).
    RegionId
);
```

- [ ] **Step 3: Run the crate's unit tests (no DB needed)**

Run: `cargo nextest run -p temper-next ids`
Expected: PASS (existing roundtrip tests cover the macro; new types compile under it).

Also: `cargo nextest run -p temper-next --features scenario-schema ids` — proves the schemars gate compiles.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-next/src/ids.rs
git commit -m "event-payloads task 2: ChunkId/EdgeId/PropertyId/RegionId newtypes + gated JsonSchema derive"
```

---

### Task 3: Identity-as-input in the content prepare path

**Files:**
- Modify: `crates/temper-next/src/content.rs`

- [ ] **Step 1: Write the failing test** (append to the `tests` module in content.rs)

```rust
    // Identity-as-input (payload spec §2): prepare pre-generates block/chunk UUIDv7s so payloads can
    // carry them and replay is exact. Ids must be unique and serialized into the JSONB.
    #[test]
    fn prepare_pregenerates_block_and_chunk_ids() {
        let planned = plan_chunks("Some prose.");
        assert_eq!(planned.len(), 1);
        // prepare_block needs ONNX; test the id plumbing through the struct directly.
        let block = PreparedBlock {
            block_id: crate::ids::BlockId::from(uuid::Uuid::now_v7()),
            seq: 0,
            role: None,
            chunks: vec![PreparedChunk {
                chunk_id: crate::ids::ChunkId::from(uuid::Uuid::now_v7()),
                chunk_index: 0,
                content_hash: "ab".repeat(32),
                content: "Some prose.".into(),
                embedding: vec![0.0; 3],
            }],
        };
        let v = serde_json::to_value([&block]).unwrap();
        assert!(v[0]["block_id"].is_string(), "block_id serialized");
        assert!(v[0]["chunks"][0]["chunk_id"].is_string(), "chunk_id serialized");
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p temper-next prepare_pregenerates`
Expected: FAIL to compile — `PreparedBlock` has no field `block_id`.

- [ ] **Step 3: Add the id fields**

In content.rs, add `use crate::ids::{BlockId, ChunkId};` and `use uuid::Uuid;` to the imports, then:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct PreparedChunk {
    /// Pre-generated chunk identity (identity-as-input): carried into the payload manifest AND used
    /// by the SQL projection as the kb_chunks.id, so replay reproduces the same row ids.
    pub chunk_id: ChunkId,
    pub chunk_index: i32,
    pub content_hash: String,
    pub content: String,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreparedBlock {
    /// Pre-generated block identity (identity-as-input) — see chunk_id above.
    pub block_id: BlockId,
    pub seq: i32,
    pub role: Option<String>,
    pub chunks: Vec<PreparedChunk>,
}
```

In `prepare_block`, generate them:

```rust
    let chunks = planned
        .into_iter()
        .zip(embeddings)
        .map(
            |((chunk_index, content_hash, content), embedding)| PreparedChunk {
                chunk_id: ChunkId::from(Uuid::now_v7()),
                chunk_index,
                content_hash,
                content,
                embedding,
            },
        )
        .collect();
    Ok(PreparedBlock {
        block_id: BlockId::from(Uuid::now_v7()),
        seq,
        role: role.map(str::to_owned),
        chunks,
    })
```

Update the existing `prepared_block_serializes_to_expected_jsonb_shape` test's struct literal to include `block_id: crate::ids::BlockId::from(uuid::Uuid::now_v7())` and `chunk_id: crate::ids::ChunkId::from(uuid::Uuid::now_v7())`.

- [ ] **Step 4: Run the crate unit tests**

Run: `cargo nextest run -p temper-next`
Expected: PASS. (The SQL persist path ignores unknown JSONB keys, so the artifact suite is unaffected until Task 6 — do not run it here; the sidecar split isn't in yet.)

- [ ] **Step 5: Commit**

```bash
git add crates/temper-next/src/content.rs
git commit -m "event-payloads task 3: PreparedBlock/PreparedChunk carry pre-generated ids (identity-as-input)"
```

---

### Task 4: The payloads module (15 typed payloads + references + sidecar)

**Files:**
- Create: `crates/temper-next/src/payloads.rs`
- Modify: `crates/temper-next/src/lib.rs` (+ `pub mod payloads;`)
- Modify: `crates/temper-next/src/affinity.rs` (EdgeKind gains `Serialize` + gated `JsonSchema` if not already present — check `#[derive(...)]` on the enum; it already derives `Deserialize` with snake_case for the YAML model. Add `serde::Serialize` to the derive list and `#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]` if missing.)

- [ ] **Step 1: Write the module with tests**

```rust
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
    BlockId, ChunkId, CogmapId, EdgeId, EntityId, EventId, LensId, ProfileId, PropertyId, RegionId,
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
    pub fn resource(id: crate::ids::ResourceId) -> Self {
        AnchorRef { table: AnchorTable::Resources, id: id.uuid() }
    }
    pub fn cogmap(id: CogmapId) -> Self {
        AnchorRef { table: AnchorTable::Cogmaps, id: id.uuid() }
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
/// derived in the projector (derived-state rule applied to the spec's own §3 sketch).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct BlockManifest {
    pub block_id: BlockId,
    pub seq: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub chunks: Vec<ChunkManifest>,
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
    pub resource_id: crate::ids::ResourceId,
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
}

/// Build the `{chunk_id: {content, embedding}}` sidecar `cogmap_genesis`/`resource_create` take as
/// `p_content`. Keyed by chunk id string (JSONB object keys are strings).
pub fn content_sidecar(blocks: &[PreparedBlock]) -> HashMap<String, ChunkContent> {
    let mut map = HashMap::new();
    for b in blocks {
        for c in &b.chunks {
            map.insert(
                c.chunk_id.to_string(),
                ChunkContent {
                    content: c.content.clone(),
                    embedding: Some(EmbeddingRepr::Vector(c.embedding.clone())),
                },
            );
        }
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
    pub resource_id: crate::ids::ResourceId,
    pub title: String,
    pub origin_uri: String,
    pub home: AnchorRef,
    pub owner_profile_id: ProfileId,
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
    pub resource_id: crate::ids::ResourceId,
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
            }],
        };
        let m = BlockManifest::from(&b);
        let v = serde_json::to_value(&m).unwrap();
        let text = v.to_string();
        assert!(!text.contains("secret prose"), "prose must never enter a payload");
        assert!(!text.contains("0.5"), "embeddings must never enter a payload");
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
            source: AnchorRef::resource(crate::ids::ResourceId::from(Uuid::now_v7())),
            target: AnchorRef::resource(crate::ids::ResourceId::from(Uuid::now_v7())),
            edge_kind: EdgeKind::Near,
            polarity: EdgePolarity::Forward,
            label: Some("contradicts".into()),
            weight: 1.0,
            home: AnchorRef::cogmap(CogmapId::from(Uuid::now_v7())),
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["polarity"], "forward");
        assert_eq!(serde_json::from_value::<RelationshipAsserted>(v).unwrap(), p);
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
            }],
        };
        let side = content_sidecar(&[b.clone()]);
        let entry = side.get(&b.chunks[0].chunk_id.to_string()).unwrap();
        assert_eq!(entry.content, "the prose");
        assert!(matches!(entry.embedding, Some(EmbeddingRepr::Vector(_))));
    }
}
```

- [ ] **Step 2: Register the module** — in `lib.rs` add `pub mod payloads;` (alphabetical, after `pub mod ids;`).

- [ ] **Step 3: Make EdgeKind serializable** — in `affinity.rs`, confirm/extend the `EdgeKind` derive list to include `serde::Serialize` (it already has `Deserialize` for the YAML model) and the gated `schemars::JsonSchema` if absent. Its serde rename must be snake_case so it serializes `"leads_to"` — matching both YAML and the `edge_kind` SQL enum.

- [ ] **Step 4: Run unit tests, both feature sets**

Run: `cargo nextest run -p temper-next && cargo nextest run -p temper-next --features scenario-schema`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-next/src/payloads.rs crates/temper-next/src/lib.rs crates/temper-next/src/affinity.rs
git commit -m "event-payloads task 4: typed payload module (15 types, references, content sidecar)"
```

---

### Task 5: `_event_append` + payload-first relationship_assert / facet_set / lens_create

**Files:**
- Modify: `schema-artifact/02_functions.sql` (replace lines 629–695; add `_event_append` before them)
- Modify: `crates/temper-next/src/events.rs` (three fire arms + `Fired`)

- [ ] **Step 1: Add `_event_append` and rewrite the three functions**

In 02_functions.sql, insert before the `relationship_assert` block (and delete the old `relationship_assert`/`facet_set`/`lens_create` bodies):

```sql
-- ============================================================================
-- THE ONE EVENT WRITER (payload-first design §5). Every mutation function appends through here;
-- it is also the foreign-event door: an external/webhook event is _event_append with no projection
-- half. Root-event convention: correlation_id = the event's own id when no correlation is supplied
-- (computed up front — the ledger is append-only, no post-hoc UPDATE).
-- ============================================================================
CREATE FUNCTION _event_append(
    p_type_name text, p_emitter uuid, p_anchor_table text, p_anchor_id uuid,
    p_payload jsonb,
    p_references jsonb DEFAULT '[]'::jsonb,
    p_correlation uuid DEFAULT NULL,
    p_payload_version int DEFAULT 1
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_et uuid; v_ev uuid := uuid_generate_v7();
BEGIN
    SELECT id INTO v_et FROM kb_event_types WHERE name = p_type_name;
    IF v_et IS NULL THEN RAISE EXCEPTION 'event_type % not seeded', p_type_name; END IF;
    INSERT INTO kb_events (id, event_type_id, emitter_entity_id,
                           producing_anchor_table, producing_anchor_id,
                           payload, "references", payload_version, correlation_id)
    VALUES (v_ev, v_et, p_emitter, p_anchor_table, p_anchor_id,
            p_payload, p_references, p_payload_version, COALESCE(p_correlation, v_ev));
    RETURN v_ev;
END;
$$;

-- ── relationship_asserted ────────────────────────────────────────────────────
-- Projection half: reads ONLY the payload (RelationshipAsserted, payloads.rs). Timestamps come from
-- the event's occurred_at (replay-stable; refinement 2).
CREATE FUNCTION _project_relationship_asserted(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_edge uuid := (p_payload->>'edge_id')::uuid;
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    INSERT INTO kb_edges (id, source_table, source_id, target_table, target_id,
                          edge_kind, polarity, label, weight,
                          home_anchor_table, home_anchor_id,
                          asserted_by_event_id, last_event_id, created)
    VALUES (v_edge,
            p_payload#>>'{source,table}', (p_payload#>>'{source,id}')::uuid,
            p_payload#>>'{target,table}', (p_payload#>>'{target,id}')::uuid,
            (p_payload->>'edge_kind')::edge_kind,
            COALESCE(p_payload->>'polarity', 'forward')::edge_polarity,
            p_payload->>'label',
            (p_payload->>'weight')::double precision,
            p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid,
            p_event, p_event, v_occurred);
    RETURN v_edge;
END;
$$;

CREATE FUNCTION relationship_assert(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('relationship_asserted', p_emitter,
                          p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid, p_payload);
    RETURN _project_relationship_asserted(v_ev, p_payload);
END;
$$;

-- ── property_asserted ────────────────────────────────────────────────────────
CREATE FUNCTION _project_property_asserted(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_prop uuid := (p_payload->>'property_id')::uuid;
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    INSERT INTO kb_properties (id, owner_table, owner_id, property_key, property_value, weight,
                               asserted_by_event_id, last_event_id, created)
    VALUES (v_prop,
            p_payload#>>'{owner,table}', (p_payload#>>'{owner,id}')::uuid,
            p_payload->>'property_key', p_payload->'value',
            (p_payload->>'weight')::double precision,
            p_event, p_event, v_occurred);
    RETURN v_prop;
END;
$$;

-- The producing anchor is an ENVELOPE concern derived from the owner resource's home (preferring a
-- cogmap home), exactly as before — never payload data. A homeless resource is an error.
CREATE FUNCTION facet_set(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_anchor_tbl text; v_anchor uuid;
        v_owner uuid := (p_payload#>>'{owner,id}')::uuid;
BEGIN
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_owner ORDER BY (anchor_table='kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'facet_set: resource % has no home to anchor the property event', v_owner;
    END IF;
    v_ev := _event_append('property_asserted', p_emitter, v_anchor_tbl, v_anchor, p_payload);
    RETURN _project_property_asserted(v_ev, p_payload);
END;
$$;

-- ── lens_created ─────────────────────────────────────────────────────────────
CREATE FUNCTION _project_lens_created(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_lens uuid := (p_payload->>'lens_id')::uuid;
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    INSERT INTO kb_cogmap_lenses
        (id, cogmap_id, name, selection_kind,
         w_express, w_contains, w_leads_to, w_near, w_prop,
         s_telos, s_ref, s_central, resolution, asserted_by_event_id, created)
    VALUES (v_lens,
            (p_payload->>'cogmap_id')::uuid,             -- NULL for a global lens
            p_payload->>'name', p_payload->>'selection_kind',
            (p_payload#>>'{weights,express}')::double precision,
            (p_payload#>>'{weights,contains}')::double precision,
            (p_payload#>>'{weights,leads_to}')::double precision,
            (p_payload#>>'{weights,near}')::double precision,
            (p_payload#>>'{weights,prop}')::double precision,
            (p_payload#>>'{salience,telos}')::double precision,
            (p_payload#>>'{salience,ref}')::double precision,
            (p_payload#>>'{salience,central}')::double precision,
            (p_payload->>'resolution')::double precision,
            p_event, v_occurred);
    RETURN v_lens;
END;
$$;

CREATE FUNCTION lens_create(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_anchor_tbl text;
BEGIN
    v_anchor_tbl := CASE WHEN p_payload->>'cogmap_id' IS NULL THEN NULL ELSE 'kb_cogmaps' END;
    v_ev := _event_append('lens_created', p_emitter, v_anchor_tbl, (p_payload->>'cogmap_id')::uuid, p_payload);
    RETURN _project_lens_created(v_ev, p_payload);
END;
$$;
```

- [ ] **Step 2: Update the three fire arms + `Fired`**

In events.rs: add `use crate::payloads;` and `use crate::ids::{EdgeId, PropertyId};` to imports. Replace the `Fired::Relationship(Uuid)` variant with `Relationship(EdgeId)` (its doc comment about "no EdgeId newtype yet" is now stale — delete it) and `Facet` with `Facet(PropertyId)`. Replace the three arms:

```rust
        SeedAction::RelationshipAssert { src, tgt, kind, label, weight, home, emitter } => {
            let payload = payloads::RelationshipAsserted {
                edge_id: EdgeId::from(Uuid::now_v7()),
                source: payloads::AnchorRef::resource(src),
                target: payloads::AnchorRef::resource(tgt),
                edge_kind: kind,
                polarity: payloads::EdgePolarity::Forward,
                label: label.map(str::to_owned),
                weight,
                home: payloads::AnchorRef::cogmap(home),
            };
            let id = sqlx::query_scalar!(
                "SELECT relationship_assert($1,$2)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?
            .context("relationship_assert returned null")?;
            Ok(Fired::Relationship(EdgeId::from(id)))
        }

        SeedAction::FacetSet { resource, values, weight, emitter } => {
            let payload = payloads::PropertyAsserted {
                property_id: PropertyId::from(Uuid::now_v7()),
                owner: payloads::AnchorRef::resource(resource),
                property_key: "facet".into(),
                value: values.clone(),
                weight,
            };
            let id = sqlx::query_scalar!(
                "SELECT facet_set($1,$2)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?
            .context("facet_set returned null")?;
            Ok(Fired::Facet(PropertyId::from(id)))
        }

        SeedAction::LensCreate { cogmap, lens, emitter } => {
            let payload = payloads::LensCreated {
                lens_id: LensId::from(Uuid::now_v7()),
                cogmap_id: cogmap,
                name: lens.name.clone(),
                selection_kind: "homed".into(),
                weights: payloads::LensWeights {
                    express: lens.w_express,
                    contains: lens.w_contains,
                    leads_to: lens.w_leads_to,
                    near: lens.w_near,
                    prop: lens.w_prop,
                },
                salience: payloads::SalienceWeights {
                    telos: lens.s_telos,
                    reference: lens.s_ref,
                    central: lens.s_central,
                },
                resolution: lens.resolution,
            };
            let id = sqlx::query_scalar!(
                "SELECT lens_create($1,$2)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?
            .context("lens_create returned null")?;
            Ok(Fired::Lens(id.into()))
        }
```

(`LensId` import already exists. Note `selection_kind: "homed"` reproduces the old SQL default.)

- [ ] **Step 3: Reload artifact, regenerate cache, run write-path tests**

Run:
```bash
psql "$DATABASE_URL" -q -v ON_ERROR_STOP=1 -f schema-artifact/01_schema.sql -f schema-artifact/02_functions.sql
cargo make prepare-next
cargo nextest run -p temper-next --features artifact-tests
```
Expected: PASS — the loader/runner call `fire()` whose external API is unchanged. (`03_seed.sql` doesn't call these three functions, so the cross-path test is unaffected by this task.)

- [ ] **Step 4: Run crate unit tests** — `cargo nextest run -p temper-next` — PASS.

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/02_functions.sql crates/temper-next/src/events.rs crates/temper-next/.sqlx
git commit -m "event-payloads task 5: _event_append + payload-first relationship_assert/facet_set/lens_create"
```

---

### Task 6: Payload-first cogmap_genesis / resource_create (content sidecar)

**Files:**
- Modify: `schema-artifact/02_functions.sql` (`_persist_resource_blocks` → `_project_blocks`; rewrite `cogmap_genesis` lines 533–592 and `resource_create` lines 603–627)
- Modify: `crates/temper-next/src/events.rs` (two fire arms)
- Modify: `schema-artifact/03_seed.sql` (genesis call site, lines 182–210)

- [ ] **Step 1: Rewrite the block persist helper as a payload+sidecar projector**

Replace `_persist_resource_blocks` (02_functions.sql:470-516) with:

```sql
-- Shared block→chunk projector (content-block write path, payload-first). p_manifests is the
-- payload's BlockManifest array — ids + seqs + roles + content hashes, NO prose (CAS rule).
-- p_content is the sidecar { "<chunk_id>": { "content": text, "embedding": [..]|"[..]"|null } } —
-- persisted to kb_chunk_content / kb_chunks.embedding, never written to the ledger. Structural truth
-- comes ONLY from the manifests: a manifest chunk missing from the sidecar is an exception; sidecar
-- extras are ignored. Projected timestamps come from the owning event's occurred_at (replay-stable).
-- block_body_hash / resource body_hash stay DERIVED (sha256 merkles over the carried chunk hashes).
CREATE FUNCTION _project_blocks(p_resource uuid, p_event uuid, p_manifests jsonb, p_content jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_block uuid; v_chunk uuid;
    v_block_json jsonb; v_chunk_json jsonb; v_side jsonb; v_emb jsonb;
    v_block_hash text; v_chunk_hashes text; v_chunk_count int;
    v_resource_hashes text := '';
    v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    FOR v_block_json IN SELECT jsonb_array_elements(p_manifests) LOOP
        v_block := (v_block_json->>'block_id')::uuid;
        INSERT INTO kb_content_blocks (id, resource_id, seq, genesis_event_id, last_event_id, created)
            VALUES (v_block, p_resource, (v_block_json->>'seq')::int, p_event, p_event, v_occurred);
        IF v_block_json ? 'role' AND jsonb_typeof(v_block_json->'role') = 'string' THEN
            INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value,
                                       asserted_by_event_id, last_event_id, created)
            VALUES ('kb_content_blocks', v_block, 'block_role', v_block_json->'role',
                    p_event, p_event, v_occurred);
        END IF;
        v_chunk_hashes := '';
        v_chunk_count := 0;
        FOR v_chunk_json IN SELECT jsonb_array_elements(v_block_json->'chunks') LOOP
            v_chunk := (v_chunk_json->>'chunk_id')::uuid;
            v_side := p_content->(v_chunk_json->>'chunk_id');
            IF v_side IS NULL THEN
                RAISE EXCEPTION '_project_blocks: content sidecar missing chunk %', v_chunk;
            END IF;
            v_emb := v_side->'embedding';
            INSERT INTO kb_chunks (id, block_id, resource_id, chunk_index, content_hash, embedding, created)
                VALUES (v_chunk, v_block, p_resource, (v_chunk_json->>'chunk_index')::int,
                        v_chunk_json->>'content_hash',
                        CASE
                            WHEN v_emb IS NULL OR jsonb_typeof(v_emb) = 'null' THEN NULL
                            WHEN jsonb_typeof(v_emb) = 'string' THEN (v_emb #>> '{}')::vector  -- replay carries pgvector text
                            ELSE (v_emb::text)::vector                                          -- fire carries a JSON array
                        END,
                        v_occurred);
            INSERT INTO kb_chunk_content (chunk_id, content)
                VALUES (v_chunk, v_side->>'content');
            v_chunk_hashes := v_chunk_hashes || (v_chunk_json->>'content_hash');
            v_chunk_count := v_chunk_count + 1;
        END LOOP;
        v_block_hash := encode(sha256(convert_to(v_chunk_hashes, 'UTF8')), 'hex');
        INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count, created)
            VALUES (v_block, v_block_hash, v_chunk_count, v_occurred);
        v_resource_hashes := v_resource_hashes || v_block_hash;
    END LOOP;
    UPDATE kb_resources SET body_hash = encode(sha256(convert_to(v_resource_hashes, 'UTF8')), 'hex'),
                            updated = v_occurred
        WHERE id = p_resource;
END;
$$;
```

- [ ] **Step 2: Rewrite `cogmap_genesis` and `resource_create`**

```sql
-- ── cogmap_seeded (payload-first genesis). Identity-as-input: cogmap/resource/block/chunk ids all
-- arrive in the payload, so the producing anchor is known UP FRONT — the old post-hoc backfill
-- UPDATE on kb_events is gone (the ledger is append-only). Resource-first ordering preserved.
CREATE FUNCTION _project_cogmap_seeded(p_event uuid, p_payload jsonb, p_content jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_cogmap   uuid := (p_payload->>'cogmap_id')::uuid;
        v_resource uuid := (p_payload#>>'{telos,resource_id}')::uuid;
        v_owner    uuid := (p_payload->>'owner_profile_id')::uuid;
BEGIN
    -- telos resource FIRST (telos_resource_id NOT NULL holds at the cogmap insert)
    INSERT INTO kb_resources (id, title, origin_uri, created, updated)
        VALUES (v_resource, p_payload#>>'{telos,title}', p_payload#>>'{telos,origin_uri}',
                v_occurred, v_occurred);
    PERFORM _project_blocks(v_resource, p_event, p_payload#>'{telos,blocks}', p_content);
    INSERT INTO kb_cogmaps (id, name, telos_resource_id, created)
        VALUES (v_cogmap, p_payload->>'name', v_resource, v_occurred);
    INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id,
                                   originator_profile_id, owner_profile_id, created)
        VALUES (v_resource, 'kb_cogmaps', v_cogmap, v_owner, v_owner, v_occurred);
    INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value,
                               asserted_by_event_id, last_event_id, created)
        VALUES ('kb_resources', v_resource, 'doc_type', '"cogmap_charter"'::jsonb,
                p_event, p_event, v_occurred);
END;
$$;

CREATE FUNCTION cogmap_genesis(p_payload jsonb, p_content jsonb, p_emitter uuid)
RETURNS TABLE(cogmap_id uuid, telos_resource_id uuid) LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('cogmap_seeded', p_emitter,
                          'kb_cogmaps', (p_payload->>'cogmap_id')::uuid, p_payload);
    PERFORM _project_cogmap_seeded(v_ev, p_payload, p_content);
    cogmap_id := (p_payload->>'cogmap_id')::uuid;
    telos_resource_id := (p_payload#>>'{telos,resource_id}')::uuid;
    RETURN NEXT;
END;
$$;

-- ── resource_created (payload-first) ────────────────────────────────────────
CREATE FUNCTION _project_resource_created(p_event uuid, p_payload jsonb, p_content jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_resource uuid := (p_payload->>'resource_id')::uuid;
        v_owner    uuid := (p_payload->>'owner_profile_id')::uuid;
BEGIN
    INSERT INTO kb_resources (id, title, origin_uri, created, updated)
        VALUES (v_resource, p_payload->>'title', p_payload->>'origin_uri', v_occurred, v_occurred);
    INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id,
                                   originator_profile_id, owner_profile_id, created)
        VALUES (v_resource, p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid,
                v_owner, v_owner, v_occurred);
    PERFORM _project_blocks(v_resource, p_event, p_payload->'blocks', p_content);
    IF p_payload->>'doc_type' IS NOT NULL THEN
        INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value,
                                   asserted_by_event_id, last_event_id, created)
            VALUES ('kb_resources', v_resource, 'doc_type', p_payload->'doc_type',
                    p_event, p_event, v_occurred);
    END IF;
    RETURN v_resource;
END;
$$;

CREATE FUNCTION resource_create(p_payload jsonb, p_content jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('resource_created', p_emitter,
                          p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid, p_payload);
    RETURN _project_resource_created(v_ev, p_payload, p_content);
END;
$$;
```

- [ ] **Step 3: Update the two fire arms** (events.rs):

```rust
        SeedAction::CogmapGenesis { name, telos_title, charter, owner, emitter } => {
            let payload = payloads::CogmapSeeded {
                cogmap_id: CogmapId::from(Uuid::now_v7()),
                name: name.to_owned(),
                owner_profile_id: owner,
                telos: payloads::TelosManifest {
                    resource_id: ResourceId::from(Uuid::now_v7()),
                    title: telos_title.to_owned(),
                    origin_uri: "temper://genesis".into(),
                    blocks: charter.iter().map(payloads::BlockManifest::from).collect(),
                },
            };
            let sidecar = serde_json::to_value(payloads::content_sidecar(charter))?;
            let row = sqlx::query!(
                "SELECT cogmap_id, telos_resource_id FROM cogmap_genesis($1,$2,$3)",
                serde_json::to_value(&payload)?,
                sidecar,
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?;
            Ok(Fired::CogmapGenesis {
                cogmap: row.cogmap_id.context("cogmap_genesis returned null cogmap_id")?.into(),
                telos_resource: row
                    .telos_resource_id
                    .context("cogmap_genesis returned null telos_resource_id")?
                    .into(),
            })
        }

        SeedAction::ResourceCreate { title, origin_uri, home, owner, blocks, doc_type, emitter } => {
            let payload = payloads::ResourceCreated {
                resource_id: ResourceId::from(Uuid::now_v7()),
                title: title.to_owned(),
                origin_uri: origin_uri.to_owned(),
                home: payloads::AnchorRef::cogmap(home),
                owner_profile_id: owner,
                doc_type: doc_type.map(str::to_owned),
                blocks: blocks.iter().map(payloads::BlockManifest::from).collect(),
            };
            let sidecar = serde_json::to_value(payloads::content_sidecar(blocks))?;
            let id = sqlx::query_scalar!(
                "SELECT resource_create($1,$2,$3)",
                serde_json::to_value(&payload)?,
                sidecar,
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?
            .context("resource_create returned null")?;
            Ok(Fired::Resource(id.into()))
        }
```

(`ResourceId` is already in events.rs's ids import — no import change needed.)

- [ ] **Step 4: Update the `03_seed.sql` genesis call site** (lines 182–210) to the new signature — manifests + sidecar built from the same rows, ids pre-generated in SQL:

```sql
    -- ── Genesis: seed onboarding-cogmap via the single-txn function ───────
    -- Payload-first: the pure-SQL seed builds the BlockManifest payload (ids + hashes, no prose) and
    -- the content sidecar (prose, NULL embedding — embed_chunks backfills later) from the same rows.
    -- Prose is verbatim shared with schema-artifact/scenarios/onboarding-cogmap.yaml.
    DECLARE v_manifests jsonb; v_content jsonb;
            v_cg uuid := uuid_generate_v7(); v_telos uuid := uuid_generate_v7();
    BEGIN
        WITH rows AS (
            SELECT ord, txt, uuid_generate_v7() AS block_id, uuid_generate_v7() AS chunk_id
            FROM (VALUES
                (0, 'Help a new EPD engineer reach first-merge confidence in week one.'),
                (1, 'What does this person already know that transfers?'),
                (2, 'What is the smallest real change that builds confidence?'),
                (3, 'Where are the sharp edges that scar newcomers?')
            ) AS t(ord, txt)
        )
        SELECT
            jsonb_agg(jsonb_build_object(
                'block_id', block_id,
                'seq', ord,
                'role', CASE WHEN ord = 0 THEN 'statement' ELSE 'question' END,
                'chunks', jsonb_build_array(jsonb_build_object(
                    'chunk_id', chunk_id,
                    'chunk_index', 0,
                    'content_hash', encode(sha256(convert_to(txt, 'UTF8')), 'hex')
                ))
            ) ORDER BY ord),
            jsonb_object_agg(chunk_id::text, jsonb_build_object('content', txt, 'embedding', NULL))
        INTO v_manifests, v_content
        FROM rows;

        SELECT g.cogmap_id INTO c_onboarding FROM cogmap_genesis(
            jsonb_build_object(
                'cogmap_id', v_cg,
                'name', 'onboarding-cogmap',
                'owner_profile_id', p_dave,
                'telos', jsonb_build_object(
                    'resource_id', v_telos,
                    'title', 'Onboarding charter',
                    'origin_uri', 'temper://genesis',
                    'blocks', v_manifests)),
            v_content, e_agent) g;
    END;
```

- [ ] **Step 5: Reload artifact (now including 03_seed for the legacy check), regenerate, run the full write-path suite**

```bash
psql "$DATABASE_URL" -q -v ON_ERROR_STOP=1 -f schema-artifact/01_schema.sql -f schema-artifact/02_functions.sql
cargo make prepare-next
cargo nextest run -p temper-next --features artifact-tests
```
Expected: PASS, including `scenario_roundtrip` (the cross-path test loads 03_seed itself via `reset_artifact_with_seed`).

- [ ] **Step 6: Commit**

```bash
git add schema-artifact/02_functions.sql schema-artifact/03_seed.sql crates/temper-next/src/events.rs crates/temper-next/.sqlx
git commit -m "event-payloads task 6: payload-first cogmap_genesis/resource_create with content sidecar; no event backfill UPDATE"
```

---

### Task 7: region_materialized payload (fingerprint + watermark + region ids)

**Files:**
- Modify: `schema-artifact/02_functions.sql` (new `region_materialize` + `_project_region_materialized`)
- Modify: `crates/temper-next/src/events.rs` (`SeedAction::Materialize` fields + arm)
- Modify: `crates/temper-next/src/write.rs` (cluster-first ordering)

- [ ] **Step 1: Add the SQL pair** (after `lens_create` in 02_functions.sql):

```sql
-- ── region_materialized ──────────────────────────────────────────────────────
-- Region ROWS are second-order derived compute (clustering output) and stay Rust-side; the
-- projection half records only the act's bookkeeping: the materialization watermark on the cogmap.
-- Replay proof for regions = replay substrate → re-run materialize → fingerprint matches the payload.
CREATE FUNCTION _project_region_materialized(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    UPDATE kb_cogmaps SET shape_materialized_event_id = p_event
     WHERE id = (p_payload->>'cogmap_id')::uuid;
END;
$$;

CREATE FUNCTION region_materialize(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('region_materialized', p_emitter,
                          'kb_cogmaps', (p_payload->>'cogmap_id')::uuid, p_payload);
    PERFORM _project_region_materialized(v_ev, p_payload);
    RETURN v_ev;
END;
$$;
```

- [ ] **Step 2: Extend `SeedAction::Materialize` and its arm** (events.rs):

```rust
    Materialize {
        cogmap: CogmapId,
        lens: LensId,
        /// Max event id over the substrate at load time — the point-in-time the projection saw.
        watermark: EventId,
        membership_fingerprint: &'a str,
        region_ids: &'a [RegionId],
        emitter: EntityId,
    },
```

(add `RegionId` to the ids import) and the arm:

```rust
        SeedAction::Materialize { cogmap, lens, watermark, membership_fingerprint, region_ids, emitter } => {
            let payload = payloads::RegionMaterialized {
                cogmap_id: cogmap,
                lens_id: lens,
                watermark_event_id: watermark,
                membership_fingerprint: membership_fingerprint.to_owned(),
                region_ids: region_ids.to_vec(),
            };
            let id = sqlx::query_scalar!(
                "SELECT region_materialize($1,$2)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?
            .context("region_materialize returned null")?;
            Ok(Fired::Materialize(EventId::from(id)))
        }
```

Update the events.rs unit test's `Materialize` construction accordingly (nil ids, `watermark: EventId::from(Uuid::nil())`, `membership_fingerprint: ""`, `region_ids: &[]`).

- [ ] **Step 3: Reorder `materialize_cogmap`** (write.rs). After `let clusters = cluster(...)`:

```rust
    // fingerprint + region ids BEFORE the event: the payload records the act's full identity.
    let mut fingerprint_parts: Vec<String> = clusters
        .iter()
        .map(|members| {
            let mut ms: Vec<String> = members.iter().map(|m| m.to_string()).collect();
            ms.sort();
            ms.join("+")
        })
        .collect();
    fingerprint_parts.sort();
    let fingerprint = fingerprint_parts.join("|");
    let region_ids: Vec<RegionId> = clusters.iter().map(|_| RegionId::from(Uuid::now_v7())).collect();

    let mut tx = pool.begin().await?;
    // the substrate point-in-time this projection saw (uuidv7 — time-ordered)
    let watermark: Uuid = sqlx::query_scalar!("SELECT max(id) FROM kb_events")
        .fetch_one(&mut *tx)
        .await?
        .context("materialize on an empty ledger (no events)")?;
    let ev: Uuid = fire(
        &mut tx,
        SeedAction::Materialize {
            cogmap: CogmapId::from(cogmap),
            lens: LensId::from(s.lens_id),
            watermark: EventId::from(watermark),
            membership_fingerprint: &fingerprint,
            region_ids: &region_ids,
            emitter: EntityId::from(emitter),
        },
    )
    .await?
    .materialize_event()?
    .uuid();
```

Then: keep the fold-prior-regions UPDATE; change the region INSERT to use the pre-generated id (`INSERT INTO kb_cogmap_regions (id, cogmap_id, …) VALUES ($6, $1, …)` binding `region_ids[i].uuid()`, iterating `clusters.iter().zip(&region_ids)`); **delete** the per-cluster fingerprint accumulation at the bottom of the loop and the final `UPDATE kb_cogmaps SET shape_materialized_event_id…` (now done by `_project_region_materialized`); return `MaterializeOutcome { regions: clusters.len(), membership_fingerprint: fingerprint }`. Imports: `use crate::ids::{CogmapId, EntityId, EventId, LensId, RegionId};` and `use anyhow::Context;`.

- [ ] **Step 4: Reload, regenerate, run both suites**

```bash
psql "$DATABASE_URL" -q -v ON_ERROR_STOP=1 -f schema-artifact/01_schema.sql -f schema-artifact/02_functions.sql
cargo make prepare-next
cargo nextest run -p temper-next
cargo nextest run -p temper-next --features artifact-tests
```
Expected: PASS — `S6b reproducible` and `S6f fingerprint_differs` still hold (fingerprint computation unchanged in value, only in timing).

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/02_functions.sql crates/temper-next/src/events.rs crates/temper-next/src/write.rs crates/temper-next/.sqlx
git commit -m "event-payloads task 7: region_materialized carries lens/watermark/fingerprint/region-ids; watermark projection in SQL"
```

---

### Task 8: Append-only trigger + envelope test

**Files:**
- Modify: `schema-artifact/01_schema.sql` (after the kb_events indexes)
- Create: `crates/temper-next/tests/ledger_envelope.rs`
- Modify: `.config/nextest.toml`

- [ ] **Step 1: Add the trigger** (01_schema.sql, after `idx_kb_events_references`):

```sql
-- Append-only enforcement (parity with production 20260522000001): supersession and correction are
-- themselves events; the ledger row is final. Safe to add now — no mutation function UPDATEs
-- kb_events anymore (identity-as-input made the genesis anchor known up front).
CREATE FUNCTION kb_events_append_only() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'event ledger is append-only';
END;
$$;
CREATE TRIGGER kb_events_append_only
    BEFORE UPDATE OR DELETE ON kb_events
    FOR EACH ROW EXECUTE FUNCTION kb_events_append_only();
```

- [ ] **Step 2: Write the test**

```rust
#![cfg(feature = "artifact-tests")]
//! Ledger envelope invariants: append-only, root-correlation convention, payload presence.

mod common;

use sqlx::PgPool;

async fn pool() -> PgPool {
    PgPool::connect(&std::env::var("DATABASE_URL").unwrap()).await.unwrap()
}

#[tokio::test]
async fn ledger_is_append_only_and_roots_self_correlate() {
    common::reset_artifact();
    let pool = pool().await;
    temper_next::scenario::bootseed::seed_system(&pool).await.unwrap();

    // any seeded event will do — the boot-seed fired lens_created events
    let (id, correlation): (uuid::Uuid, Option<uuid::Uuid>) = sqlx::query_as(
        "SELECT id, correlation_id FROM kb_events ORDER BY id LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(correlation, Some(id), "a root event's correlation_id is its own id");

    let upd = sqlx::query("UPDATE kb_events SET payload_version = 2 WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await;
    let err = upd.expect_err("UPDATE must be rejected").to_string();
    assert!(err.contains("append-only"), "got: {err}");

    let del = sqlx::query("DELETE FROM kb_events WHERE id = $1").bind(id).execute(&pool).await;
    assert!(del.is_err(), "DELETE must be rejected");
}
```

- [ ] **Step 3: Join the serial group** — in `.config/nextest.toml` extend the binary regex:

```toml
filter = 'package(temper-next) & binary(/^bootseed$|^scenario_load$|^scenario_roundtrip$|^content_multichunk$|^cogmap_genesis_charter$|^charter_block_roles$|^ledger_envelope$|^replay_roundtrip$/)'
```

(`replay_roundtrip` added now so Task 12 doesn't have to touch this file again.)

- [ ] **Step 4: Reload + run**

```bash
psql "$DATABASE_URL" -q -v ON_ERROR_STOP=1 -f schema-artifact/01_schema.sql -f schema-artifact/02_functions.sql
cargo nextest run -p temper-next --features artifact-tests
```
Expected: PASS including the new test.

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/01_schema.sql crates/temper-next/tests/ledger_envelope.rs .config/nextest.toml
git commit -m "event-payloads task 8: append-only ledger trigger + envelope invariants test"
```

---

### Task 9: Payload JSON-Schema snapshots (committed contract files)

**Files:**
- Create: `crates/temper-next/tests/payload_schema.rs`
- Create: `schema-artifact/payloads/*.v1.schema.json` (15 files, generated)

- [ ] **Step 1: Write the snapshot test**

```rust
#![cfg(feature = "scenario-schema")]
//! Payload JSON-Schemas are emitted from the SAME structs `fire()` serializes — the wire contract
//! and the code can't drift. One committed snapshot per (type, version); the boot-seed stamps these
//! files into kb_event_types.payload_schema, so repo == registry == Rust types (spec §6 chain).
//! Regenerate: UPDATE_SCHEMA=1 cargo test -p temper-next --features scenario-schema --test payload_schema

use temper_next::payloads as p;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schema-artifact/payloads");

fn check<T: schemars::JsonSchema>(name: &str) {
    let schema = schemars::SchemaGenerator::default().into_root_schema_for::<T>();
    let rendered = serde_json::to_string_pretty(&schema).unwrap() + "\n";
    let path = format!("{DIR}/{name}.v1.schema.json");
    if std::env::var("UPDATE_SCHEMA").is_ok() {
        std::fs::create_dir_all(DIR).unwrap();
        std::fs::write(&path, &rendered).unwrap();
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(rendered, committed, "{name} payload schema drifted — re-run with UPDATE_SCHEMA=1");
}

#[test]
fn payload_schemas_match_snapshots() {
    check::<p::CogmapSeeded>("cogmap_seeded");
    check::<p::ResourceCreated>("resource_created");
    check::<p::RelationshipAsserted>("relationship_asserted");
    check::<p::PropertyAsserted>("property_asserted");
    check::<p::LensCreated>("lens_created");
    check::<p::RegionMaterialized>("region_materialized");
    check::<p::RelationshipRetyped>("relationship_retyped");
    check::<p::RelationshipReweighted>("relationship_reweighted");
    check::<p::RelationshipFolded>("relationship_folded");
    check::<p::RelationshipDecayed>("relationship_decayed");
    check::<p::RelationshipCorrected>("relationship_corrected");
    check::<p::BlockCreated>("block_created");
    check::<p::BlockMutated>("block_mutated");
    check::<p::BlockFolded>("block_folded");
    check::<p::BlockProvenanceCorrected>("block_provenance_corrected");
}

#[test]
fn snapshot_files_cover_exactly_the_typed_names() {
    let mut on_disk: Vec<String> = std::fs::read_dir(DIR)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter_map(|f| f.strip_suffix(".v1.schema.json").map(str::to_owned))
        .collect();
    on_disk.sort();
    let mut expected: Vec<String> = p::TYPED_EVENT_NAMES.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(on_disk, expected);
}
```

- [ ] **Step 2: Generate the snapshots, then verify clean**

```bash
UPDATE_SCHEMA=1 cargo test -p temper-next --features scenario-schema --test payload_schema
cargo test -p temper-next --features scenario-schema --test payload_schema
```
Expected: first run writes 15 files; second run passes clean.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-next/tests/payload_schema.rs schema-artifact/payloads/
git commit -m "event-payloads task 9: committed JSON-Schema snapshots for the 15 payload types"
```

---

### Task 10: Boot-seed stamps the registry; taxonomy additions

**Files:**
- Modify: `schema-artifact/seeds/system.yaml` (+2 names)
- Modify: `crates/temper-next/src/scenario/bootseed.rs`
- Modify: `crates/temper-next/tests/bootseed.rs` (assertion)

- [ ] **Step 1: Add the two missing lifecycle names** to system.yaml's `event_types` (after `relationship_folded`):

```yaml
  - relationship_decayed
  - relationship_corrected
```

- [ ] **Step 2: Stamp `payload_schema` from the committed files** — in `seed_system`, replace the event-types loop:

```rust
    // Registry rows + their published contract: stamp payload_schema/schema_version from the
    // committed schema-artifact/payloads/<name>.v1.schema.json snapshots (spec §6 — repo, registry,
    // and Rust types are one chain; the snapshot test pins repo==types, this pins registry==repo).
    // A name with no snapshot (foreign/not-yet-typed) stays NULL = unregistered/permissive.
    let payloads_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schema-artifact/payloads");
    for et in &boot.event_types {
        let schema: Option<serde_json::Value> =
            std::fs::read_to_string(format!("{payloads_dir}/{et}.v1.schema.json"))
                .ok()
                .map(|s| serde_json::from_str(&s))
                .transpose()?;
        sqlx::query!(
            "INSERT INTO kb_event_types (name, payload_schema, schema_version) VALUES ($1, $2, 1) \
             ON CONFLICT (name) DO UPDATE SET payload_schema = EXCLUDED.payload_schema, \
                                              schema_version = EXCLUDED.schema_version",
            et,
            schema.as_ref(),
        )
        .execute(pool)
        .await?;
    }
```

- [ ] **Step 3: Extend the bootseed integration test** (tests/bootseed.rs — append to the existing test or add a new `#[tokio::test]` following the file's pattern):

```rust
#[tokio::test]
async fn bootseed_publishes_payload_schemas() {
    common::reset_artifact();
    let pool = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").unwrap()).await.unwrap();
    temper_next::scenario::bootseed::seed_system(&pool).await.unwrap();

    let stamped: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_event_types WHERE payload_schema IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stamped, 15, "exactly the 15 typed names carry a published schema");

    let permissive: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT payload_schema FROM kb_event_types WHERE name = 'delegated_launch'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(permissive.is_none(), "untyped names stay NULL (unregistered/permissive)");
}
```

(Adjust imports/`mod common;` to match the existing file's conventions.)

- [ ] **Step 4: Regenerate cache, run**

```bash
cargo make prepare-next
cargo nextest run -p temper-next --features artifact-tests bootseed
cargo nextest run -p temper-next --features artifact-tests
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/seeds/system.yaml crates/temper-next/src/scenario/bootseed.rs crates/temper-next/tests/bootseed.rs crates/temper-next/.sqlx
git commit -m "event-payloads task 10: boot-seed publishes payload schemas to the registry; + decayed/corrected names"
```

---

### Task 11: Roundtrip verifier (proof obligation 1)

**Files:**
- Modify: `crates/temper-next/src/payloads.rs` (append the verifier)
- Modify: `crates/temper-next/tests/scenario_roundtrip.rs` (call it after `run_scenario`)

- [ ] **Step 1: Add the verifier** to payloads.rs:

```rust
/// Proof obligation 1 (spec §7.1): every event on the ledger whose type is typed here must
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
                "cogmap_seeded" => { serde_json::from_value::<CogmapSeeded>(r.payload.clone())?; }
                "resource_created" => { serde_json::from_value::<ResourceCreated>(r.payload.clone())?; }
                "relationship_asserted" => { serde_json::from_value::<RelationshipAsserted>(r.payload.clone())?; }
                "property_asserted" => { serde_json::from_value::<PropertyAsserted>(r.payload.clone())?; }
                "lens_created" => { serde_json::from_value::<LensCreated>(r.payload.clone())?; }
                "region_materialized" => { serde_json::from_value::<RegionMaterialized>(r.payload.clone())?; }
                _ => {}
            }
            Ok(())
        })();
        if let Err(e) = res {
            anyhow::bail!("event {} ({}) payload fails roundtrip: {e}", r.id, r.type_name);
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Wire into the scenario test** — in tests/scenario_roundtrip.rs, after each `run_scenario(...)await.unwrap()` (and after the cross-path SQL-seed materialization), add:

```rust
    temper_next::payloads::verify_ledger_roundtrip(&pool).await.unwrap();
```

- [ ] **Step 3: Regenerate + run**

```bash
cargo make prepare-next
cargo nextest run -p temper-next --features artifact-tests scenario_roundtrip
cargo nextest run -p temper-next --features artifact-tests
```
Expected: PASS — and note this proves the **hand-SQL 03_seed path** also emits conformant payloads.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-next/src/payloads.rs crates/temper-next/tests/scenario_roundtrip.rs crates/temper-next/.sqlx
git commit -m "event-payloads task 11: ledger roundtrip verifier wired into the scenario corpus"
```

---

### Task 12: The replay proof (proof obligations 2 + 4)

**Files:**
- Create: `crates/temper-next/src/replay.rs`
- Modify: `crates/temper-next/src/lib.rs` (+ `pub mod replay;`)
- Create: `crates/temper-next/tests/replay_roundtrip.rs`

- [ ] **Step 1: Write the replay module**

```rust
//! Replay primitives (proof obligation 2, spec §7.2): walk the ledger through the SAME `_project_*`
//! halves normal operation uses, into a freshly reset namespace, and prove the projections come back
//! byte-identical (masked-surrogate rule: tables whose `id` has no inbound references — homes,
//! properties, block_revisions — compare with id masked, ordered by natural key).
//!
//! Region tables are excluded from the diff: they are second-order derived compute. Their proof is
//! re-materialization — the fingerprint must equal the one recorded in the region_materialized payload.
//!
//! Dumps/restores are dynamic-table operations, so this module uses runtime `sqlx::query` (the
//! established exception class) rather than compile-checked macros.

use anyhow::{Context, Result};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

/// (table, dump-query) — canonical, deterministic row dumps. Masked tables subtract 'id' and order
/// by natural key; everything else orders by id.
const PROJECTION_DUMPS: &[(&str, &str)] = &[
    ("kb_resources",       "SELECT jsonb_agg(to_jsonb(t) ORDER BY t.id) FROM kb_resources t"),
    ("kb_resource_homes",  "SELECT jsonb_agg((to_jsonb(t) - 'id') ORDER BY t.resource_id) FROM kb_resource_homes t"),
    ("kb_cogmaps",         "SELECT jsonb_agg(to_jsonb(t) ORDER BY t.id) FROM kb_cogmaps t"),
    ("kb_content_blocks",  "SELECT jsonb_agg(to_jsonb(t) ORDER BY t.id) FROM kb_content_blocks t"),
    ("kb_chunks",          "SELECT jsonb_agg(to_jsonb(t) ORDER BY t.id) FROM kb_chunks t"),
    ("kb_chunk_content",   "SELECT jsonb_agg(to_jsonb(t) ORDER BY t.chunk_id) FROM kb_chunk_content t"),
    ("kb_block_revisions", "SELECT jsonb_agg((to_jsonb(t) - 'id') ORDER BY t.block_id, t.block_body_hash) FROM kb_block_revisions t"),
    ("kb_properties",      "SELECT jsonb_agg((to_jsonb(t) - 'id') ORDER BY t.owner_table, t.owner_id, t.property_key, t.property_value) FROM kb_properties t"),
    ("kb_edges",           "SELECT jsonb_agg(to_jsonb(t) ORDER BY t.id) FROM kb_edges t"),
    ("kb_cogmap_lenses",   "SELECT jsonb_agg(to_jsonb(t) ORDER BY t.id) FROM kb_cogmap_lenses t"),
];

/// Non-projected input tables, copied verbatim into the replay namespace (restore order matters:
/// FK dependencies). kb_team_cogmaps restores AFTER the event walk (it references projected cogmaps).
const INPUT_TABLES: &[&str] = &[
    "kb_profiles",
    "kb_entities",
    "kb_teams",
    "kb_teams_parents",
    "kb_team_members",
    "kb_contexts",
    "kb_topics",
    "kb_event_types",
    "kb_events",
];

pub struct LedgerSnapshot {
    inputs: Vec<(String, serde_json::Value)>,
    team_cogmaps: serde_json::Value,
    /// event id → content sidecar for the content-bearing types, reconstructed from the CAS
    /// (kb_chunk_content prose + the stored chunk embedding as pgvector text — a derived-cache
    /// carry-over so the diff stays total without re-running ONNX).
    sidecars: HashMap<Uuid, serde_json::Value>,
}

pub async fn dump_projections(pool: &PgPool) -> Result<Vec<(String, serde_json::Value)>> {
    let mut out = Vec::new();
    for (table, q) in PROJECTION_DUMPS {
        let v: Option<serde_json::Value> = sqlx::query_scalar(q).fetch_one(pool).await?;
        out.push((table.to_string(), v.unwrap_or(serde_json::Value::Null)));
    }
    Ok(out)
}

async fn dump_table(pool: &PgPool, table: &str) -> Result<serde_json::Value> {
    let q = format!("SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t), '[]'::jsonb) FROM {table} t");
    Ok(sqlx::query_scalar(&q).fetch_one(pool).await?)
}

/// Capture everything replay needs BEFORE the namespace reset.
pub async fn snapshot(pool: &PgPool) -> Result<LedgerSnapshot> {
    let mut inputs = Vec::new();
    for t in INPUT_TABLES {
        inputs.push((t.to_string(), dump_table(pool, t).await?));
    }
    let team_cogmaps = dump_table(pool, "kb_team_cogmaps").await?;

    // sidecars for the content-bearing events: payload manifests → chunk ids → CAS lookups
    let mut sidecars = HashMap::new();
    let rows = sqlx::query(
        "SELECT e.id, et.name, e.payload \
           FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name IN ('cogmap_seeded','resource_created') ORDER BY e.id",
    )
    .fetch_all(pool)
    .await?;
    for r in rows {
        let event_id: Uuid = r.get(0);
        let name: String = r.get(1);
        let payload: serde_json::Value = r.get(2);
        let manifests = if name == "cogmap_seeded" {
            payload.pointer("/telos/blocks").cloned()
        } else {
            payload.get("blocks").cloned()
        }
        .context("content-bearing payload missing blocks")?;
        let mut side = serde_json::Map::new();
        for block in manifests.as_array().context("blocks not an array")? {
            for chunk in block["chunks"].as_array().context("chunks not an array")? {
                let chunk_id: Uuid = chunk["chunk_id"]
                    .as_str()
                    .context("chunk_id missing")?
                    .parse()?;
                let row = sqlx::query(
                    "SELECT cc.content, c.embedding::text \
                       FROM kb_chunk_content cc JOIN kb_chunks c ON c.id = cc.chunk_id \
                      WHERE cc.chunk_id = $1",
                )
                .bind(chunk_id)
                .fetch_one(pool)
                .await
                .with_context(|| format!("CAS retention violated: chunk {chunk_id} has no content row"))?;
                let content: String = row.get(0);
                let embedding: Option<String> = row.get(1);
                side.insert(
                    chunk_id.to_string(),
                    serde_json::json!({ "content": content, "embedding": embedding }),
                );
            }
        }
        sidecars.insert(event_id, serde_json::Value::Object(side));
    }
    Ok(LedgerSnapshot { inputs, team_cogmaps, sidecars })
}

async fn restore_table(pool: &PgPool, table: &str, rows: &serde_json::Value) -> Result<()> {
    let conflict = if table == "kb_team_members" { " ON CONFLICT DO NOTHING" } else { "" };
    let q = format!(
        "INSERT INTO {table} SELECT * FROM jsonb_populate_recordset(NULL::{table}, $1){conflict}"
    );
    sqlx::query(&q).bind(rows).execute(pool).await?;
    Ok(())
}

/// Restore inputs and walk the ledger through the projection halves — the SAME code normal
/// operation runs. Call against a freshly reset (01+02, un-seeded) namespace.
pub async fn replay(pool: &PgPool, snap: &LedgerSnapshot) -> Result<()> {
    for (table, rows) in &snap.inputs {
        restore_table(pool, table, rows).await?;
    }
    let events = sqlx::query(
        "SELECT e.id, et.name, e.payload \
           FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id ORDER BY e.id",
    )
    .fetch_all(pool)
    .await?;
    for r in events {
        let id: Uuid = r.get(0);
        let name: String = r.get(1);
        let payload: serde_json::Value = r.get(2);
        match name.as_str() {
            "cogmap_seeded" => {
                let side = snap.sidecars.get(&id).context("missing sidecar")?;
                sqlx::query("SELECT _project_cogmap_seeded($1,$2,$3)")
                    .bind(id).bind(&payload).bind(side)
                    .execute(pool).await?;
            }
            "resource_created" => {
                let side = snap.sidecars.get(&id).context("missing sidecar")?;
                sqlx::query("SELECT _project_resource_created($1,$2,$3)")
                    .bind(id).bind(&payload).bind(side)
                    .execute(pool).await?;
            }
            "relationship_asserted" => {
                sqlx::query("SELECT _project_relationship_asserted($1,$2)")
                    .bind(id).bind(&payload).execute(pool).await?;
            }
            "property_asserted" => {
                sqlx::query("SELECT _project_property_asserted($1,$2)")
                    .bind(id).bind(&payload).execute(pool).await?;
            }
            "lens_created" => {
                sqlx::query("SELECT _project_lens_created($1,$2)")
                    .bind(id).bind(&payload).execute(pool).await?;
            }
            "region_materialized" => {
                sqlx::query("SELECT _project_region_materialized($1,$2)")
                    .bind(id).bind(&payload).execute(pool).await?;
            }
            other => anyhow::bail!("replay: no projector for event type {other}"),
        }
    }
    restore_table(pool, "kb_team_cogmaps", &snap.team_cogmaps).await?;
    Ok(())
}

/// The recorded materialization acts (last per lens) — for the region fingerprint re-proof.
pub async fn recorded_materializations(pool: &PgPool) -> Result<Vec<(Uuid, Uuid, String)>> {
    let rows = sqlx::query(
        "SELECT DISTINCT ON (e.payload->>'lens_id') \
                (e.payload->>'cogmap_id')::uuid, (e.payload->>'lens_id')::uuid, \
                e.payload->>'membership_fingerprint' \
           FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = 'region_materialized' \
          ORDER BY e.payload->>'lens_id', e.id DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect())
}
```

- [ ] **Step 2: Register** — `pub mod replay;` in lib.rs.

- [ ] **Step 3: Write the replay proof test**

```rust
#![cfg(feature = "artifact-tests")]
//! Proof obligation 2 (replay) + 4 (CAS retention): run a scenario, snapshot, reset the namespace,
//! walk the ledger through the SAME _project_* halves, and prove the projections are byte-identical
//! (masked-surrogate rule). Regions re-prove by re-materialization fingerprint.

mod common;

use sqlx::PgPool;

const SCENARIO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../schema-artifact/scenarios/onboarding-cogmap.yaml"
);

#[tokio::test]
async fn replay_reproduces_projections_byte_identically() {
    common::reset_artifact();
    let pool = PgPool::connect(&std::env::var("DATABASE_URL").unwrap()).await.unwrap();
    temper_next::scenario::bootseed::seed_system(&pool).await.unwrap();

    let scenario: temper_next::scenario::model::Scenario =
        serde_yaml::from_str(&std::fs::read_to_string(SCENARIO).unwrap()).unwrap();
    temper_next::scenario::runner::run_scenario(&pool, &scenario).await.unwrap();

    // capture: projections (the diff baseline), inputs + sidecars (the replay substrate)
    let before = temper_next::replay::dump_projections(&pool).await.unwrap();
    let snap = temper_next::replay::snapshot(&pool).await.unwrap();
    let recorded = temper_next::replay::recorded_materializations(&pool).await.unwrap();
    assert!(!recorded.is_empty(), "scenario must have materialized at least once");

    // reset to a clean, UN-seeded namespace; replay the ledger through the projection halves
    common::reset_artifact();
    temper_next::replay::replay(&pool, &snap).await.unwrap();

    let after = temper_next::replay::dump_projections(&pool).await.unwrap();
    for ((table_a, a), (table_b, b)) in before.iter().zip(after.iter()) {
        assert_eq!(table_a, table_b);
        assert_eq!(a, b, "projection table {table_a} diverged under replay");
    }

    // regions: second-order derived — re-materialize and match the recorded fingerprints
    for (cogmap, lens_id, fingerprint) in recorded {
        let lens_name: String =
            sqlx::query_scalar("SELECT name FROM kb_cogmap_lenses WHERE id = $1")
                .bind(lens_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let emitter: uuid::Uuid =
            sqlx::query_scalar("SELECT id FROM kb_entities ORDER BY id LIMIT 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        let out = temper_next::write::materialize_cogmap(&pool, cogmap, &lens_name, emitter)
            .await
            .unwrap();
        assert_eq!(
            out.membership_fingerprint, fingerprint,
            "re-materialization under lens {lens_name} must reproduce the recorded fingerprint"
        );
    }
}
```

(`snapshot()` itself asserts proof obligation 4: a manifest chunk with no `kb_chunk_content` row fails with "CAS retention violated".)

- [ ] **Step 4: Run**

```bash
cargo make prepare-next
cargo nextest run -p temper-next --features artifact-tests replay
cargo nextest run -p temper-next --features artifact-tests
```
Expected: PASS. If the projection diff fails, the diff message names the table — the usual suspects are a `now()` that should be `v_occurred` or a default-generated id that should come from the payload.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-next/src/replay.rs crates/temper-next/src/lib.rs crates/temper-next/tests/replay_roundtrip.rs
git commit -m "event-payloads task 12: replay proof — ledger walk through the same projectors, byte-identical projections + fingerprint re-proof"
```

---

### Task 13: Wrap-up — quality gates, spec amendments, suites

**Files:**
- Modify: `docs/superpowers/specs/2026-06-09-event-payload-formalization-design.md` (append amendments section)

- [ ] **Step 1: Record the plan-time refinements in the spec** — append to the spec, before `## Connections`:

```markdown
## 10. Amendments discovered at plan time (2026-06-09, implementation plan)

1. **Masked-surrogate replay diff** (§7.2 refined): ids are payload-carried for every *referenced* row;
   `kb_resource_homes` / `kb_properties` / `kb_block_revisions` surrogate ids carry no inbound
   references and are masked in the replay diff (natural-key ordered). No information escapes
   through them; manifests stay lean.
2. **Projected timestamps come from the event** (§5 extended): `_project_*` sets `created`/`updated`
   from the event's `occurred_at` — replay-stable and semantically truer.
3. **`BlockManifest` omits `block_body_hash`** (§3 refined): derived merkle, recomputed by the
   projector — the §3 exclusion rule applied to the spec's own sketch.
4. **`_project_region_materialized` projects only the watermark** (§5 narrowed): region rows are
   second-order derived compute and stay Rust-side; their replay proof is re-materialization
   matching the payload's recorded membership fingerprint.
5. **The replay proof is harness-level per scenario** (§7.2 refined), not an in-YAML expectation —
   it resets the namespace, which cannot happen mid-scenario. Every write-path scenario test calls
   it, so the corpus property is preserved.
6. **Content sidecar** (§5 extended): content-bearing functions take `(p_payload, p_content,
   p_emitter)`; the sidecar is `{chunk_id: {content, embedding}}`, persisted to the CAS, never on
   the ledger; the projector trusts only the payload's manifests and errors on a missing sidecar entry.
```

- [ ] **Step 2: Quality gates and suites**

```bash
cargo make fix
cargo make check
cargo nextest run -p temper-next
cargo nextest run -p temper-next --features scenario-schema
cargo nextest run -p temper-next --features artifact-tests
```
Expected: all green. (Workspace-wide nextest is PR-prep, not per-task — CI covers the rest.)

- [ ] **Step 3: Verify the offline cache is current**

Run: `cargo make prepare-next && git status --short crates/temper-next/.sqlx`
Expected: no uncommitted cache changes (or commit them here).

- [ ] **Step 4: Final commit**

```bash
git add docs/superpowers/specs/2026-06-09-event-payload-formalization-design.md crates/temper-next/.sqlx
git commit -m "event-payloads task 13: spec amendments from plan grounding; quality gates green"
```

---

## Self-review checklist (run after writing, before execution)

- **Spec coverage:** §1 envelope → Tasks 1+8; §2 identity-as-input → Tasks 3+5+6+7; §3 vocabulary+placement → Task 4; §4 references → Task 4 (column in Task 1; first consumer is the foreign-event door — no native emitter sets references yet, correct per §8 scope); §5 firing split → Tasks 5+6+7; §6 registry/versioning → Tasks 1+9+10; §7 proofs → Tasks 8 (envelope), 9 (schema agreement), 11 (roundtrip), 12 (replay + CAS); §9 column-coverage → polarity carried, timestamps event-derived, masked-surrogate rule for the rest.
- **Known intentional gaps:** the nine designed-but-unbuilt payload types have schemas + registry rows but no SQL functions (per spec §3 "schemas now, wiring later"); `references` is always `'[]'` from native emitters (no current native use case; the column, type, and index exist for the provenance-chain door).
- **Type consistency spot-checks:** `Fired::Relationship(EdgeId)` / `Fired::Facet(PropertyId)` accessors — callers in loader.rs/runner.rs ignore both returns today (verify no `.relationship()`-style accessor is referenced); `SeedAction::Materialize` is constructed in write.rs only; `MaterializeOutcome` unchanged shape.
