# Content-Block/Chunk Correctness Foundation + Event-Firing-and-Transaction Parity (temper-next)

**Date:** 2026-06-08
**Status:** Design — **draft, ready for review.** Grounded against the live temper crates
(`temper-ingest`, `temper-events`, `temper-api/src/services/`) and the temper-next artifact
(`schema-artifact/01_schema.sql` + `02_functions.sql` + `seeds/system.yaml`,
`crates/temper-next/src/`).
**Goal:** `substrate-kernel-to-cognitive-map` — forward deliverables **1 + 2** of the scenario-DSL
roadmap ([`2026-06-07-scenario-yaml-seed-dsl-design.md`](2026-06-07-scenario-yaml-seed-dsl-design.md)
§"Roadmap", lines 449-468), designed **together** because both must ground against production before
richer scenarios build on them.
**Builds on (does not re-spec):**
[`content-block-primitive`](2026-06-03-content-block-primitive-design.md) (the `resource ⊃ blocks ⊃
chunks` shape, blocks-carry-no-prose β, the `block_*` event family),
[`domain-b-charter-questions`](2026-06-04-domain-b-charter-questions-regulation-edge-semantics-design.md)
(telos-charter as questions-as-blocks, `cogmap_genesis` composition §5, seeding event names),
[`scenario-yaml-seed-dsl`](2026-06-07-scenario-yaml-seed-dsl-design.md) (the DSL this enriches).

> **Grounding note (per `implementation-grounding.md` GD-1).** Every claim below carries a quoted
> `file:line` excerpt or names the disk artifact it conforms to. Where a section invents beyond
> current affordances, it is tagged **EXTEND/AMEND** with the spec section that authorizes it. The
> two forks this design rests on were decided with the goal-owner: **(1)** keep temper-next's content
> block tier and borrow production's chunker per-block; **(2)** event-firing becomes a generalizable
> Rust action that *speaks-as* the firing, while the SQL functions stay the atomic event+materialize
> mechanism — and the shared `temper-events::EventType` enum is **extended**, judiciously, with the
> seeding taxonomy.

---

## The corrected framing (read this first)

A surface reading says temper-next *added* a `kb_content_blocks` tier that production lacks, and should
match production's flat `resource → chunks → chunk_content`. **That is backwards.** The block tier is
the deliberate Arc-1 forward shape, designed against production's real schema in
[`content-block-primitive`](2026-06-03-content-block-primitive-design.md) (lines 80-99), that production
*itself migrates toward*; temper-next's `schema-artifact` is the fresh-schema proving ground for it.
The parity target is therefore **two-layered**:

- **Block layer** — temper-next's own (and future-production's) forward shape. *Not* borrowed from
  production's current migrated schema, which has not landed it yet. CONFORM to the content-block
  primitive spec.
- **Chunk layer** — the *mechanical* embedding window. **This** is where production parity lives:
  borrow `temper-ingest`'s chunker + embedder and apply them **per block**.

```
temper-next (and future production):   resource ⊃ content_blocks ⊃ chunks ⊃ chunk_content
                                                  └ semantic units   └ borrowed-from-production machinery
production today:                      resource ⊃ chunks ⊃ chunk_content     (block tier not yet migrated)
```

The phrase in the roadmap — *"reviewed against production temper for shape-parity"* (scenario spec
line 457-458) — means parity with the **chunk machinery** (heading-split, 510-token windows,
bge-768, sha256 content-hash) and with the **content-block primitive design**, not with production's
not-yet-migrated flat table layout.

---

## Grounding evidence

### G1 — temper-next writes the degenerate one-chunk-per-block case with placeholder hashes

`cogmap_genesis` writes block-0 (telos) and blocks 1..n (questions), each as exactly **one chunk** whose
`content_hash` is `md5(text)` (`schema-artifact/02_functions.sql:497-519`):

```sql
-- 2a. block-0 = telos statement
INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
VALUES (v_resource, 0, v_event, v_event) RETURNING id INTO v_block;
INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
VALUES (v_block, v_resource, 0, md5(p_telos_statement)) RETURNING id INTO v_chunk;
INSERT INTO kb_chunk_content (chunk_id, content) VALUES (v_chunk, p_telos_statement);
```

`resource_create` is worse: one block, one chunk, and `content_hash := md5(p_origin_uri)` — it hashes
the **URI, not the body** (`02_functions.sql:567-572`):

```sql
v_hash := md5(p_origin_uri);
INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
    VALUES (v_block, v_resource, 0, v_hash) RETURNING id INTO v_chunk;
INSERT INTO kb_chunk_content (chunk_id, content) VALUES (v_chunk, p_body);
```

No embedding is written at create; the `kb_chunks.embedding vector(768)` column is populated later by a
separate ONNX job.

### G2 — production chunks in Rust, persists a JSONB chunk-set in SQL

Production's body→chunks happens in `temper-ingest` (Rust), not SQL:
- `temper_ingest::chunk::chunk_markdown(text: &str) -> Vec<ChunkData>` — heading-delimited sections,
  split at paragraph/line boundaries when over `MAX_TOKENS = 510` (`crates/temper-ingest/src/chunk.rs:317,
  49, 67`). `ChunkData` carries `{chunk_index, header_path, heading_depth, content, content_hash}`
  with `content_hash` a **sha256 hex** of the trimmed content (`chunk.rs:41-45`).
- `temper_ingest::embed::embed_texts(texts: &[&str]) -> Result<Vec<Vec<f32>>>` — bge-base-en-v1.5,
  `EMBEDDING_DIM = 768` (`crates/temper-ingest/src/embed.rs:356, 31`).

The SQL side only **persists** a pre-chunked JSONB array — `persist_resource_chunks(resource_id,
revision_id, body_hash, chunks jsonb)` extracts chunk fields and inserts `kb_chunks` + `kb_chunk_content`
(`migrations/20260420000012_uuidv7_portability.sql:39-86`, per grounding sweep). **temper-next already
depends on `temper_ingest`** — `write.rs:54` references `temper_ingest::embed::EMBEDDING_DIM`.

### G3 — the embedding is already consumed by materialization

`materialize_cogmap` computes each region centroid by averaging `ch.embedding` over current chunks
joined through non-folded blocks (`crates/temper-next/src/write.rs:87-94`):

```sql
SELECT avg(ch.embedding) AS mv FROM kb_cogmap_region_members mm
JOIN kb_chunks ch ON ch.resource_id=mm.member_id AND ch.is_current
JOIN kb_content_blocks b ON b.id=ch.block_id AND NOT b.is_folded
WHERE mm.region_id=r.id GROUP BY mm.member_id
```

So real multi-chunk-per-block embeddings flow straight into `content_cohesion` / `telos_alignment`. The
existing onboarding roundtrip already runs the embed job, so coherence is real today — what is *not* real
is the **chunk structure** (everything is one chunk) and the **hash** (md5).

### G4 — temper-next fires events inside SQL functions; one path is a raw Rust INSERT

Every mutation function emits its event in the same txn as its projection
(`02_functions.sql`): `cogmap_genesis`→`cogmap_seeded` (line 486), `resource_create`→`resource_created`
(562), `relationship_assert`→`relationship_asserted` (592), `facet_set`→`property_asserted` (618),
`lens_create`→`lens_created` (639). The **one** exception is `materialize_cogmap`, which fires
`region_materialized` via a raw `sqlx::query_scalar` INSERT in Rust (`write.rs:28-36`). Seeding event
names are registered in `schema-artifact/seeds/system.yaml:10-31`.

### G5 — temper-next's `kb_events` is incommensurate with `temper-events::Event`

temper-next inserts `kb_events (event_type_id, emitter_entity_id, producing_anchor_table,
producing_anchor_id, correlation_id, metadata)` (`02_functions.sql:486-489, 562-563`). The shared
`temper-events::Event`/`EventToWrite` structs are shaped for production's `kb_events`:
`emitter_profile_id`, `topic_id`, `scope_id` (`crates/temper-events/src/types/event.rs:44-72`). **The
two column shapes do not align** — this is the hard boundary the event design must respect (see D2).

### G6 — `temper-events::EventType` is consumed by three exhaustive matches

Extending the enum ripples into compiler-checked matches (no catch-alls):
- `as_canonical_name` (`event.rs:18-29`)
- reference-validation match (`crates/temper-events/src/ledger.rs:67-84`)
- `apply_relationship_event` (`crates/temper-api/src/services/relationship_service.rs:214`), whose last
  arm is `ConceptCreated | ConceptMutated => …` (line 378) — **no `_` fallthrough**.

### G7 — convergence axes are unstarted

Bare `Uuid` throughout (`substrate.rs`, `write.rs`, `loader.rs`); mutation functions return scalar ids,
forcing downstream re-fetch — e.g. the loader calls `cogmap_genesis` then **immediately re-fetches**
`telos_resource_id` (`loader.rs:66-83`). Telos charter is a flat `questions: Vec<String>`
(`scenario/model.rs:55-60`).

---

## Deliverable 1 — content-block/chunk correctness

### D1.1 Chunk in Rust, persist a per-block chunk-set in SQL  — **CONFORM** (G2)

The chunking + embedding move to Rust, mirroring production (`temper-ingest` is the chunker home; SQL
only persists). A new content-preparation step in temper-next:

```rust
// crates/temper-next/src/content.rs  (NEW)
pub struct PreparedChunk { pub chunk_index: i32, pub content_hash: String,
                           pub content: String, pub embedding: Vec<f32> }      // sha256 + bge-768
pub struct PreparedBlock { pub seq: i32, pub block_body_hash: String,
                           pub chunks: Vec<PreparedChunk> }

/// Borrow production's machinery: chunk_markdown per block-prose, embed each chunk inline.
pub fn prepare_block(seq: i32, prose: &str) -> Result<PreparedBlock>;        // chunk_markdown + embed_texts
pub fn prepare_resource(blocks: &[&str]) -> Result<Vec<PreparedBlock>>;       // one entry per content-block
```

- **EXTEND** (scenario spec line 449-458, authorizes "a content model + creation path supporting
  multi-paragraph, multi-content-block, multi-chunk-per-block resources, … borrowing patterns/code from
  the live temper crates"): a block whose prose exceeds one 510-token window yields **>1**
  `PreparedChunk` — the first real multi-chunk-per-block in temper-next.
- **CONFORM** (G2): `content_hash` is the chunker's **sha256** of trimmed content, retiring md5; the
  block's `block_body_hash` is computed over its ordered chunk hashes; the resource `body_hash` stays the
  merkle over ordered block hashes already present (`02_functions.sql:521-522`).

### D1.2 SQL functions accept a block→chunk JSONB  — **AMEND** (G1 + scenario spec line 449-458)

`resource_create` and `cogmap_genesis` change signature to receive prepared blocks as JSONB and iterate
them, inserting `kb_content_blocks` / `kb_chunks (… , embedding)` / `kb_chunk_content` /
`kb_block_revisions` per block, instead of the hardcoded single-chunk writes. This **conforms to
production's `persist_resource_chunks(… chunks jsonb)` pattern** (G2) but at block grain. The embedding is
written **inline at create** (decided: retire the deferred md5 placeholder; full bge-768 parity with
production's ingest path).

```
resource_create(p_title, p_origin_uri, p_home_cogmap, p_owner, p_blocks jsonb, p_doc_type, p_emitter)
cogmap_genesis(p_name, p_telos_title, p_charter_blocks jsonb, p_owner, p_emitter, p_origin_uri)
   -- p_charter_blocks: ordered [ block-0 telos statement, blocks 1..n questions-with-context, framing… ]
   -- p_blocks / p_charter_blocks element:
   --   { seq, block_body_hash, chunks: [ { chunk_index, content_hash, content, embedding: [f32;768] } ] }
```

> **Load-bearing invariant carried verbatim** (content-block-primitive line 103-108, **CONFORM**):
> *"A block has no `content` column. Block text stays emergent from its chunks."* The JSONB carries chunk
> prose into `kb_chunk_content`, never into `kb_content_blocks`. The implementer reads
> content-block-primitive §"Two decisions" + §"Write path" before touching these functions.

### D1.3 Telos charter becomes content-blocks  — **AMEND** (G7 + domain-B §1-2)

`TelosDef` stops being `{ title, statement, questions: Vec<String> }` and becomes the charter shape
domain-B §1 specifies — block-0 statement, blocks 1..n questions-with-context, framing blocks:

```rust
// crates/temper-next/src/scenario/model.rs   (AMEND TelosDef)
pub struct TelosDef {
    pub title: String,
    pub statement: String,                 // block-0 prose (multi-paragraph ⇒ multi-chunk)
    #[serde(default)] pub questions: Vec<QuestionDef>,   // each ⇒ one block
    #[serde(default)] pub framing:   Vec<String>,        // framing statements ⇒ blocks (situate the telos)
}
pub struct QuestionDef { pub question: String, #[serde(default)] pub context: String }  // "question-with-context"
```

> **Invariant carried verbatim** (domain-B lines 86-92, **CONFORM**): *"block-0 is the telos; blocks
> 1..n are questions" is a positional convention … No `block_kind` is introduced.* The charter's blocks
> are distinguished by `seq`, not a kind column.

A question-block's prose is `question + "\n\n" + context`, so a rich question-with-context naturally
chunks into >1 window — the charter alone exercises **both** multi-block (statement + N questions + M
framing) **and** multi-chunk-per-block. This is the minimal real charter shape for *correctness*; the
full evolvable seed/telos data-shape design (framing topology, evolvability) is **deliverable 3**, out of
scope here (scenario spec lines 470-476).

### D1.4 Onboarding scenario YAML updated; roundtrip must still pass  — **CONFORM** (task acceptance)

`schema-artifact/scenarios/onboarding-cogmap.yaml` migrates its `telos.questions: [string]` to the
structured shape, and at least one resource gets genuinely multi-paragraph `body` prose so the roundtrip
exercises a multi-chunk block. **Acceptance gate:** `scenario_roundtrip.rs` (the S6a-h runbook + the
cross-path membership-equivalence proof) still passes on the richer content model. The cross-path proof
compares membership signatures byte-for-byte against the SQL-seeded path, so the SQL scenarios
(`03_seed.sql`/`04_scenarios.sql`) that still feed it must move in lockstep or the proof is updated to the
new content path.

---

## Deliverable 2 — event-firing-and-transaction parity

The decided design is a **hybrid**: the SQL functions stay the atomic event+materialize+commit mechanism
(*"the most effective and discrete method"*), and a Rust-side **generalizable fire-event action** lets
Rust *speak-as* the firing — mirroring production's `append_and_project` (Rust speaks the firing,
`relationship_service.rs:167-177`) while keeping temper-next's single-SQL-call atomicity instead of
production's Rust-held two-step.

### D2.1 The Rust fire-event action  — **EXTEND** (scenario spec lines 460-468)

```rust
// crates/temper-next/src/events.rs   (NEW)
/// The bespoke per-kind seeding logic, one variant per SQL mutation function. Carries the params;
/// the dispatcher calls the matching SQL function and returns the produced ids (record-set, D3).
pub enum SeedAction {
    CogmapGenesis { name, telos_title, charter: Vec<PreparedBlock>, owner: ProfileId, emitter: EntityId, … },
    ResourceCreate { title, origin_uri, home: CogmapId, owner: ProfileId, blocks: Vec<PreparedBlock>, doc_type, emitter: EntityId },
    RelationshipAssert { src: ResourceId, tgt: ResourceId, kind: EdgeKind, label, weight, home: CogmapId, emitter: EntityId },
    FacetSet { resource: ResourceId, values, weight, emitter: EntityId },
    LensCreate { … },
    Materialize { cogmap: CogmapId, lens, emitter: EntityId },
}
impl SeedAction { pub fn event_type(&self) -> EventType { … } }   // the taxonomy tag (D2.2)

/// "Speak-as" firing: every seed/scenario/test write goes through here, never a raw SQL string.
pub async fn fire(tx: &mut PgTx<'_>, action: SeedAction) -> Result<Fired>;   // dispatches to the SQL fn
```

- The loader (`loader.rs:66-138`), the runner's emit-event step, **and** the tests all call `fire(...)`
  instead of inline `sqlx::query_scalar!("SELECT cogmap_genesis(...)")` — satisfying *"applied
  consistently in seeding, scenario creation, and tests, not just the materialize path"* (scenario spec
  line 464).
- **AMEND** (G4): the raw `region_materialized` INSERT in `write.rs:28-36` is reconciled into
  `SeedAction::Materialize`, so there is exactly one firing surface.

### D2.2 Extend `temper-events::EventType` — judiciously  — **AMEND** (G5/G6 + goal-owner decision)

The shared taxonomy enum gains the seeding variants (`RelationshipAsserted` already exists):

```rust
// crates/temper-events/src/types/event.rs   (AMEND EventType + as_canonical_name)
CogmapSeeded        => "cogmap_seeded"
ResourceCreated     => "resource_created"
PropertyAsserted    => "property_asserted"
LensCreated         => "lens_created"
RegionMaterialized  => "region_materialized"
```

> **The judicious boundary (the "unless hard divergence" guard).** Extend **only the enum taxonomy** —
> the variant list + `as_canonical_name`, plus the two other compiler-forced arms (`ledger.rs:67-84`,
> `relationship_service.rs:214`, where the new variants get explicit *reject* arms: they are not
> relationship/concept events). **Do NOT** route temper-next writes through `temper-events`'s
> `Event`/`EventToWrite`/`append_event_tx` machinery — its `emitter_profile_id`/`topic_id`/`scope_id`
> shape is **incommensurate** with temper-next's `emitter_entity_id`/`producing_anchor_*` ledger (G5).
> temper-next keeps its SQL-function write path and references the shared enum **for the taxonomy name
> only**. The cost is bounded and compiler-enforced: three exhaustive matches grow arms for events
> production's relationship code will never receive.
>
> **Escalation trigger (GD-5):** if review judges that mixing two systems' taxonomies in one enum is the
> hard divergence — or if a fourth, non-additive coupling appears — fall back to a **temper-next-local**
> `EventKind` enum mirroring `kb_event_types`, and revisit unification at deliverable 6 (convergence).
> Flag, don't fabricate alignment.

### D2.3 Seeding event types are canonical  — **CONFORM** (G4, domain-B plan-Q4)

The seeding event names already live in `schema-artifact/seeds/system.yaml:10-31` and the registry; D2.2
makes the Rust enum the single typed source for them. No new event names are invented (`cogmap_seeded`,
not a `cogmap_*` family — domain-B line 369 resolved).

---

## Convergence carry (scenario spec lines 493-498)

Applied **within** deliverables 1+2, not deferred to deliverable 6:

- **Typed-UUID newtypes** — **EXTEND** (G7). Introduce `ResourceId`, `CogmapId`, `BlockId`, `ProfileId`,
  `EntityId`, `EventId`, `LensId` newtypes (`#[derive(sqlx::Type)] #[sqlx(transparent)]` over `Uuid`) in a
  temper-next `ids` module; thread them through `SeedAction`/`Fired`/the loader key-map. Bare `Uuid` at
  SQL-bind boundaries only.
- **Record-set returns** — **AMEND** (G7). `fire(SeedAction::CogmapGenesis…)` returns `Fired { cogmap:
  CogmapId, telos_resource: ResourceId, charter_blocks: Vec<BlockId>, event: EventId }`, sparing the
  loader's immediate `telos_resource_id` re-fetch (`loader.rs:78-83`). The SQL functions return the
  needed ids (composite/`RETURNS TABLE`) rather than a single scalar.
- **`!`-macro sweep** is **deliverable 4**, not here (scenario spec lines 478-485) — but new temper-next
  non-vector queries added by this work use `sqlx::query!`/`query_scalar!` with a regenerated per-crate
  `.sqlx` cache (`cargo make prepare-next`).

---

## Sequenced implementation plan (the build order)

Each step is a CONFORM/EXTEND/AMEND-tagged unit; the implementer reads the cited spec section and
disk excerpt before writing code (GD-3/GD-4). TDD per step (a failing test first).

1. **`ids` newtypes module** (EXTEND, G7) — the typed-UUID newtypes; no behavior change. Unit test:
   round-trip through sqlx.
2. **`content.rs` prepare path** (CONFORM G2 / EXTEND) — `prepare_block`/`prepare_resource` over
   `temper_ingest::chunk_markdown` + `embed_texts`; sha256 hashes, bge-768. Test: a >510-token block
   yields >1 chunk; hashes match the chunker.
3. **SQL functions take block→chunk JSONB** (AMEND D1.2) — rewrite `resource_create` + `cogmap_genesis`;
   regenerate `.sqlx` (`cargo make prepare-next`). `artifact-tests` test: a multi-block / multi-chunk
   resource round-trips to the right `kb_content_blocks`/`kb_chunks`/`kb_chunk_content` rows + correct
   `body_hash` merkle.
4. **Extend `EventType`** (AMEND D2.2) — variants + `as_canonical_name` + reject arms in `ledger.rs` /
   `relationship_service.rs`. Test: `as_canonical_name` round-trips the seeding names; `cargo make check`
   green across temper-api (the ripple is compiler-verified).
5. **`events.rs` fire action** (EXTEND D2.1) — `SeedAction` + `fire`; record-set `Fired` returns
   (AMEND, convergence). Migrate `loader.rs` + the runner emit-event step + `write.rs` materialize INSERT
   to `fire`. Test: loader no longer re-fetches `telos_resource_id`.
6. **Telos charter shape** (AMEND D1.3) — `TelosDef`/`QuestionDef`/framing; loader threads them through
   `SeedAction::CogmapGenesis`. Update the JsonSchema snapshot.
7. **Onboarding YAML + roundtrip** (CONFORM D1.4) — migrate the scenario; add multi-paragraph prose.
   **Gate:** `cargo nextest run -p temper-next --features artifact-tests` (roundtrip + cross-path proof)
   passes; `run_eval.sh` still ALL-S6-PASS if still wired.

> **Verification (GD-2):** steps 3 and 7 are executable against the `temper_next` artifact — load the
> schema, run the roundtrip, quote the verdict line. Steps 1/2/4/5 are Rust units — `cargo make check` +
> nextest. The embed-gated path needs `cargo make test-e2e-embed`-class features locally (ONNX); the
> Embed CI job is the parity check.

---

## Acceptance (from the task)

- temper-next content/resource creation matches the production content-block/chunk shape, reviewed and
  **test-provable** — multi-block / multi-chunk-per-block exercised (steps 3, 7).
- Event-firing-and-transaction parity established and applied in **seed/scenario/test** code; seeding
  event types defined as the typed `EventType` taxonomy (steps 4, 5).
- The existing onboarding roundtrip + cross-path membership proof still pass on the richer content model
  (step 7).

## Out of scope

- The full evolvable seed/telos cognitive-map **data-shape** design (questions-with-context topology as a
  rigorous shape, framing neighborhoods) — **deliverable 3** (scenario spec lines 470-476).
- Richer multi-scenario authoring, dir-driven runner, retiring the SQL scenarios, the full `!`-macro
  sweep — **deliverable 4**.
- Access scaffold (team-visibility-intersection RBAC) — **deliverable 5**.
- temper-next ↔ temper migration / dual-write / release-parity — **deliverable 6**. (The newtypes +
  record-returns + event-parity *axes* mature here; the migration itself does not.)
- Reusing `temper-events`'s write machinery against temper-next's ledger — hard divergence (G5), held off.

## Open questions (resolve at plan/implementation)

1. **EventType home (D2.2)** — extend shared enum vs temper-next-local `EventKind`. Recommended: extend,
   with the reject-arm boundary; escalate to local if review finds cross-system mixing distasteful.
2. **Charter framing blocks** — does this deliverable seed any `framing` blocks, or only
   statement+questions (framing arrives with deliverable 3)? Lean: leave `framing` modeled but empty in
   the onboarding scenario; exercise it in a unit fixture so the multi-block path is tested without
   pre-empting deliverable 3's design.
3. **Cross-path proof under inline embeddings** — md5→sha256 + inline bge-768 changes chunk rows; confirm
   the cross-path membership signature is computed over *membership*, not chunk hashes, so it stays stable
   (it is membership-level per the M1 session note — verify before relying on it).
