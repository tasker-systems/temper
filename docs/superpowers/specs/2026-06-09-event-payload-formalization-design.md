# Event payload formalization — design

**Date:** 2026-06-09
**Status:** approved design, pending plan
**Scope:** temper-next (`schema-artifact/` + `crates/temper-next`); production convergence consumes this design but is out of scope here
**Workstream:** event payload formalization (workstream 3 of the `substrate-kernel-to-cognitive-map` goal, temper context)

---

## 0. Context and grounding

### The problem, in code

For an architecture whose bedrock claim is "events are the source of truth; everything else is computed," the payload contract is load-bearing — and temper-next's ledger does not currently honor it.

**Production already solved this once, for one event family.** The relationship lifecycle events carry typed payload structs (`temper-core/src/types/relationship_events.rs`: `RelationshipAsserted { source_resource_id, target, edge_kind, polarity, label, weight }`, plus retyped/reweighted/folded/decayed/corrected), serialized into a dedicated `payload` column, and the edge projection is **rebuilt by deserializing the payload** (`temper-api/src/services/relationship_service.rs:216` onward). Production's ledger also carries `references` (typed `EventReference { kind: Supersedes|DerivedFrom, event_id }`, GIN-indexed) distinct from free-form `metadata`, and an append-only trigger (`migrations/20260522000001_event_ledger_unification.sql:99-108`). For relationships, events-as-source-of-truth genuinely holds in production.

**temper-next regressed on exactly this axis while advancing on the others.** Its ledger (`schema-artifact/01_schema.sql:255-268`) has the better emitter model (`emitter_entity_id`, polymorphic `producing_anchor_(table,id)`) but **no `payload` column, no `references` column, and no append-only trigger**. The five mutation functions in `02_functions.sql` write empty-or-near-empty `metadata` (`cogmap_seeded` carries `{genesis, name}`; `resource_created`, `relationship_asserted`, `property_asserted`, `lens_created` carry nothing). Projections are built from function *arguments*, which vanish at commit. Today you cannot rebuild `kb_edges`, `kb_properties`, or `kb_cogmap_lenses` from the temper-next ledger. The typed `SeedAction` surface (`crates/temper-next/src/events.rs`) exists, but its information dies at the ledger boundary. The ledger is an audit spine, not a source of truth.

### Settled postures (decided in the 2026-06-09 design session)

1. **Content-addressed payloads.** Content-bearing events carry the full block→chunk *structure* (ids, seqs, roles, content hashes, merkle hashes); prose lives once in `kb_chunk_content`, treated as an immutable content-addressed store. Replay = payload + CAS lookup. The retention invariant this creates is stated and tested (§7.4).
2. **Scope: the six live types plus the designed families** (relationship lifecycle five, block family four, `region_materialized`) — one unified vocabulary so the production merge is a payload-compatible superset from day one. The design holds the door open for speculative families (delegation launches, sweeper signals, cascade notices) and for **external systems as entities** (Linear/Notion/GitHub webhook ingest) whose events must be first-class, consumable citizens of the same space: the payload is simultaneously the replay-to-re-materialize record (as audit and history) and the provenance chain.
3. **Rust-authored, registry-published schemas.** Typed payload structs are the authority; `kb_event_types` publishes the JSON-Schema per type so the contract is discoverable inside the system; conformance is enforced by typed construction plus tests, not runtime DB validation.
4. **Approach A — payload-first.** Rust constructs the payload; SQL functions insert the event row with the payload verbatim and build the projection **from the payload**. Projection = f(payload) is structural fact, not convention. This is the mechanism production already runs for relationship events, so convergence aligns on mechanism, not just type names.

---

## 1. Envelope — the ledger adopts the three-field discipline

`kb_events` (artifact) gains:

```sql
payload         JSONB NOT NULL DEFAULT '{}'::jsonb,   -- typed, per-event-type, replay-sufficient
"references"    JSONB NOT NULL DEFAULT '[]'::jsonb,   -- typed pointers (see §4)
payload_version INT   NOT NULL DEFAULT 1,             -- which registered schema version this row conforms to

CREATE INDEX idx_kb_events_references ON kb_events USING GIN ("references" jsonb_path_ops);
```

Plus the append-only trigger production has (`kb_events_append_only`: supersession and correction are themselves events; the ledger row is final).

`metadata` survives as what it should always have been: **free annotation, never load-bearing**. First resident: `{embedding_model, dim}` recorded at content-write time (§3, exclusions).

**Correlation discipline aligns to production's convention: a root event's `correlation_id` is its own id.** Today `cogmap_genesis` mints a separate uuid (`02_functions.sql:554-555`); that changes. Non-root events in a multi-event act carry the root's id, unchanged.

---

## 2. Identity is an input — Rust pre-generates ids; payloads carry them

For payloads to be replay-*exact* (re-run the projector over the ledger into a fresh namespace; diff projections byte-for-byte), generated ids cannot be minted inside the projection — replay would mint different ones.

So id generation moves up: `fire()` pre-generates UUIDv7s (resource, cogmap, edge, property, lens, block, chunk, region ids) in Rust and carries them **in the payload**; SQL functions use the payload's ids rather than column defaults. (Column `DEFAULT uuid_generate_v7()` stays on the tables for ergonomics elsewhere; the mutation functions simply stop relying on it.)

Three payoffs:

- **Replay becomes exactly reproducible**, and therefore provable (§7.2).
- **`Fired` becomes a confirmation, not a discovery** — the record-set return is now redundant information by construction, kept for ergonomics.
- **External systems minting and referencing their own ids stop being a special case** — identity-as-input is the same posture for native and foreign emitters.

UUIDv7 generation already lives in Rust in this codebase; no new machinery.

---

## 3. Payload vocabulary — fifteen types, typed in temper-next

### Placement

Payload structs are authored in a new **`temper-next::payloads` module** — *not* temper-core, for now. temper-next deliberately carries no temper-core dependency, and temper-core's sqlx currently includes `macros + postgres + runtime-tokio-rustls` — exactly the weight the data-model reconciliation spec plans to shed. Coupling temper-next to the pre-slim crate buys nothing; the whole body of crate rebuilding, data migration, and Rust/TS refactoring arrives with convergence anyway. The structs are written **parity-shaped for the temper-core lift at convergence** (the same pattern as the local `EventKind` in `events.rs`), and the committed JSON-Schema snapshots (§6) are the cross-system contract in the meantime. `ts-rs` derives arrive at the lift (webhook ingest is out of scope here); `serde` + `schemars` derives now (temper-next already gates schemars behind the `scenario-schema` feature; this module joins that gate or a sibling `payload-schema` feature — plan-level call).

### Shared shapes

```rust
pub struct BlockManifest {
    pub block_id: BlockId,
    pub seq: i32,
    pub role: Option<BlockRole>,          // statement | question | framing (open at value level)
    pub block_body_hash: String,          // sha256 merkle over ordered chunk hashes
    pub chunks: Vec<ChunkManifest>,
}
pub struct ChunkManifest {
    pub chunk_id: ChunkId,
    pub chunk_index: i32,
    pub content_hash: String,             // prose by CAS reference, never inline
}
pub struct AnchorRef { pub table: AnchorTable, pub id: Uuid }   // polymorphic anchor/endpoint refs
```

### The six live types

```rust
CogmapSeeded {
    cogmap_id: CogmapId, name: String, owner_profile_id: ProfileId,
    telos: TelosManifest,                 // { resource_id, title, origin_uri, blocks: Vec<BlockManifest> }
}
ResourceCreated {
    resource_id: ResourceId, title: String, origin_uri: String,
    home: AnchorRef, owner_profile_id: ProfileId,
    doc_type: Option<String>, blocks: Vec<BlockManifest>,
}
RelationshipAsserted {
    edge_id: EdgeId, source: AnchorRef, target: AnchorRef,
    edge_kind: EdgeKind, label: Option<String>, weight: f64, home: AnchorRef,
}
PropertyAsserted {
    property_id: Uuid, owner: AnchorRef,
    property_key: String, value: serde_json::Value, weight: f64,
}
LensCreated {
    lens_id: LensId, cogmap_id: Option<CogmapId>, name: String, selection_kind: String,
    weights: LensWeights,                 // { express, contains, leads_to, near, prop }
    salience: SalienceWeights,            // { telos, ref, central }
    resolution: f64,
}
RegionMaterialized {
    cogmap_id: CogmapId, lens_id: LensId,
    watermark_event_id: EventId,          // the substrate point-in-time the projection saw
    membership_fingerprint: String,       // the per-lens membership signature (drift-detection artifact)
    region_ids: Vec<Uuid>,
}
```

### The exclusion rule: derived state is never payload

Two deliberate exclusions, both instances of one rule:

- **Embeddings.** Recomputed on replay; never carried. The embedding model identity (`{embedding_model: "BAAI/bge-base-en-v1.5", dim: 768}`) is recorded in event `metadata` at content-write time so divergence across model versions is attributable. This is the written form of the standing commitment: **embeddings are non-replayed derived state.**
- **Region readouts.** Centroids, content-cohesion, decomposed salience, internal tension are deterministically recomputable from declared substrate; the payload records the materialization *act* — which lens, at which watermark, producing which membership fingerprint. The fingerprint doubles as the persisted artifact the drift-detection decision needs (its per-component fingerprints are a refinement of this grain).

### The designed-but-unbuilt families (schemas now, wiring later)

- **Relationship lifecycle (5):** `relationship_retyped { edge_id, edge_kind, polarity }`, `relationship_reweighted { edge_id, weight }`, `relationship_folded { edge_id }`, `relationship_decayed { edge_id, weight }`, `relationship_corrected { edge_id, ... }` — production's existing structs adopted, extended with `edge_id` (identity-as-input).
- **Block family (4):** `block_created { block_id, resource_id, seq }`, `block_mutated { block_id, block_body_hash, chunks: Vec<ChunkManifest>, incorporated: Vec<Incorporation> }`, `block_folded { block_id, reason: Option<String> }`, `block_provenance_corrected { block_id, source: ProvenanceSource, scar: String }` — shapes lifted from the content-block primitive spec.

### Naming

Native event types keep bare snake_case names (parity with both existing systems). External types use dotted names (`github.issue_closed`, `linear.issue_updated`) — namespace by convention in the unique `kb_event_types.name`, no new column. Foreign types may register a permissive or absent `payload_schema` (§6).

An obvious additive future type — `property_folded` (facet retract) — is named here so it lands in this vocabulary when a fold function exists; it is not built now.

---

## 4. References — the typed provenance chain

Unify production's `EventReference` with the content-block spec's reference vocabulary:

```rust
pub struct EventReference { pub rel: RefRel, pub target: RefTarget }
pub enum RefRel    { Supersedes, DerivedFrom, Touches }
pub enum RefTarget { Event(Uuid), Resource(Uuid), Block(Uuid) }
```

Serialized into the `"references"` jsonb array, GIN-indexed. Small, closed-for-now, additive-by-design. This column — not payload spelunking — is what "which events touched block B" reads, and what future cascade-notice correlation queries traverse. Blocks remain reference targets only, never graph-edge endpoints (the addressable-not-findable invariant, unchanged).

---

## 5. The single firing surface, refactored for replay

Each mutation function splits into two halves, with the event insert shared:

```sql
-- the one event writer (also the foreign-event door: append with no projection half)
_event_append(p_type_name text, p_emitter uuid, p_anchor_table text, p_anchor_id uuid,
              p_payload jsonb, p_references jsonb, p_correlation uuid, p_occurred_at timestamptz)
    RETURNS uuid  -- event id

-- ALL projection logic for a type; reads ONLY the payload
_project_cogmap_seeded(p_event uuid, p_payload jsonb)
_project_resource_created(p_event uuid, p_payload jsonb)
_project_relationship_asserted(p_event uuid, p_payload jsonb)
_project_property_asserted(p_event uuid, p_payload jsonb)
_project_lens_created(p_event uuid, p_payload jsonb)
_project_region_materialized(p_event uuid, p_payload jsonb)

-- public mutation function = _event_append + _project_<type>, one transaction
resource_create(p_payload jsonb, p_emitter uuid) RETURNS uuid
-- (signature pattern for all six; cogmap_genesis keeps its name)
```

`_persist_resource_blocks` becomes a payload-consuming projection helper (it reads `BlockManifest` JSON — which is what its block JSONB argument already approximates — and stamps `block_role` properties from the manifest's `role` fields).

`fire(SeedAction)` serializes the typed payload struct and calls the mutation function. **Replay is not a parallel code path:** the replay harness walks the ledger in `occurred_at, id` order calling each `_project_<type>(event_id, payload)` against a fresh namespace — identical code, provably identical output. Foreign events are `_event_append` with no projection half, at zero structural cost.

The scenario DSL's external surface does not change; payload construction is internal to `fire()`. The trade accepted with Approach A: SQL functions extract from jsonb (`p_payload->>'title'`) rather than typed parameters, moving some type errors from SQL-bind time to test time — mitigated because every payload is constructed from a typed Rust struct one call up, and §7.1 closes the loop.

---

## 6. Registry and versioning

`kb_event_types` gains:

```sql
payload_schema  JSONB,                       -- current published JSON-Schema (NULL = unregistered/permissive, foreign types)
schema_version  INT NOT NULL DEFAULT 1
```

- **Schema version is a first-class declaration** alongside the schema itself; event rows carry `payload_version` (§1) stating which version they conform to.
- **Evolution rule:** additive-only within a version; a breaking change bumps `schema_version` and registers the new schema. Consumers tolerate unknown fields.
- **One artifact chain:** schemars-emitted schemas are committed as snapshot files (the `tests/scenario_schema.rs` precedent — one file per type+version); the boot-seed (`schema-artifact/seeds/system.yaml`) loads the registry's `payload_schema` from those committed files. Repo, registry, and Rust types are one chain, with §7.3 asserting all three agree.
- **Published within the system:** the registry row is the discoverable contract — any consumer (including external systems and future agent surfaces) can read the schema for a type from the database it is consuming.

---

## 7. Proof obligations (scenario-corpus assertions)

1. **Roundtrip.** Every event fired across the scenario corpus deserializes into its typed payload struct. Catches drift from any path — Rust, hand-SQL, foreign.
2. **Replay (the headline).** Run a scenario → walk the ledger into a fresh namespace via `_project_*` → projections diff **byte-identical**, with embeddings excluded-and-recomputed. Lands as a new scenario expectation kind so every scenario in the corpus is also a replay proof.
3. **Schema agreement.** Emitted schemars schema == committed snapshot == registry row, per type and version.
4. **CAS retention invariant.** `kb_chunk_content` rows are immutable and never deleted — folding/superseding affects visibility, never existence. Replay of content-bearing events depends on it. Stated here as a named invariant; tested by replaying a scenario that includes supersession. (CAS is keyed by `chunk_id` with `content_hash` verification against the manifest — ids are inputs (§2), so the lookup is direct and the hash check is the integrity proof.)

---

## 8. Out of scope

- **Production-side migration** of this discipline — the convergence deliverable consumes this design; production's relationship events already conform in mechanism.
- **Speculative family payloads** (delegation launches, sweeper signals, cascade notices) — feasibility is held open by §3 naming, §4 references, and §5's `_event_append` door; semantics are not frozen here.
- **Webhook ingest build** (Linear/Notion/GitHub) — the external-entity door is designed (§3 naming, §5, §6 permissive schemas), not built.
- **`ts-rs` derives and the temper-core lift** — arrive with convergence (§3 placement).

## 9. Plan-level notes

- **Column-coverage obligation:** at plan time, verify field-by-field that each payload covers every column its `_project_*` writes (walk `01_schema.sql` per projected table). Known instance: `kb_edges.edge_polarity` is not a `relationship_assert` parameter today — it either joins `RelationshipAsserted` or is confirmed as a projection-fixed default. The struct sketches in §3 are design grain, not the exhaustive field census; the replay proof (§7.2) is the backstop that catches any miss.
- Module home and feature gate for `payloads` (sibling of `scenario-schema` or shared gate) — implementer's call.
- `Fired` record-set stays as the ergonomic return; now derivable from the payload by construction.
- `BlockRole` enum lives with the payload module; value-open at the wire level (string), enum at the Rust level — matching the block-role property's open-at-value-level design.
- After SQL changes: `cargo make prepare-next` (per-crate `.sqlx`, `temper_next` search_path); never workspace-wide prepare.
- The `region_materialized` raw INSERT in `crates/temper-next/src/write.rs` moves onto `_event_append` + `_project_region_materialized` like the rest (it already shares the `fire()` surface).

## 10. Amendments discovered at plan/build time (2026-06-09/10)

1. **Masked-surrogate replay diff** (§7.2 refined): ids are payload-carried for every *referenced*
   row; `kb_resource_homes` / `kb_properties` / `kb_block_revisions` surrogate ids carry no inbound
   references and are masked in the replay diff (natural-key ordered). No information escapes
   through them; manifests stay lean.
2. **Projected timestamps come from the event** (§5 extended): `_project_*` sets `created`/`updated`
   from the event's `occurred_at` — replay-stable by construction, and semantically truer (a
   projection's timestamp IS the event time).
3. **`BlockManifest` omits `block_body_hash`** (§3 refined): a derived merkle, recomputed by the
   projector — the §3 exclusion rule applied to the spec's own sketch.
4. **`_project_region_materialized` projects only the watermark** (§5 narrowed): region rows are
   second-order derived compute and stay Rust-side; their replay proof is re-materialization
   matching the payload's recorded membership fingerprint. Build-time refinement: the re-proof
   applies only to lenses whose recorded watermark is still current — a lens with
   formation-affecting events after its watermark is *legitimately stale* (the drift-detection
   concept surfacing in the proof) and is skipped, with at least one fresh lens required.
5. **The replay proof is harness-level per scenario** (§7.2 refined), not an in-YAML expectation —
   it resets the namespace, which cannot happen mid-scenario.
6. **Content sidecar** (§5 extended): content-bearing functions take `(p_payload, p_content,
   p_emitter)`; the sidecar is `{chunk_id: {content, embedding}}`, persisted to the CAS, never on
   the ledger; the projector trusts only the payload's manifests and errors on a missing sidecar
   entry.
7. **The roundtrip verifier caught real non-conformance on first contact** (§7.1 vindicated): the
   legacy `03_seed.sql` carried three raw events with empty payloads — one was retyped to its honest
   semantics (`relationship_reweighted` for an edge-touch), one became two honest `lens_created`
   events, one gained its real `RelationshipAsserted` payload.

## Connections

- Goal: `substrate-kernel-to-cognitive-map` (temper context) — workstream 3.
- Grounds against: `migrations/20260522000001_event_ledger_unification.sql` (envelope discipline, append-only); `temper-core/src/types/relationship_events.rs` + `temper-api/src/services/relationship_service.rs` (projection-from-payload mechanism); `schema-artifact/01_schema.sql` + `02_functions.sql` (current artifact ledger); `crates/temper-next/src/events.rs` (`SeedAction`/`fire`/`Fired`).
- Composes with: `2026-06-08-content-block-chunk-correctness-and-event-firing-parity-design.md` (block family shapes, fire surface); `2026-06-03-content-block-primitive-design.md` (reference vocabulary, provenance); `2026-06-07-scenario-yaml-seed-dsl-design.md` (boot-seed, expectation kinds); decision `2026-06-07-cogmap-region-drift-detection-...` (membership fingerprint grain).
- Carries forward: the standing replay-purity tension, resolved here as the embeddings-are-non-replayed-derived-state commitment (§3).
