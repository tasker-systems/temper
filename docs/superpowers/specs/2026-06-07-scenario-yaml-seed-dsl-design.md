# Scenario-based YAML seed/scenario DSL for temper-next

**Date:** 2026-06-07
**Status:** Design approved, pending implementation plan
**Goal:** `substrate-kernel-to-cognitive-map` (this DSL is the empirical-evaluation harness Arc 1/Arc 2 lean on; it is the named enabler for the temper-next ↔ temper schema-delta analysis)
**Supersedes the bespoke harness:** `schema-artifact/03_seed.sql`, `04b_region_suite.sql`, `run_eval.sh` (for the onboarding-cogmap path)

## Problem

The emergent-region projection (merged in #118) proved that regions are a pure projection of the
declared graph — but the proof lives in hand-written SQL (`03_seed.sql`), a SQL verdict suite
(`04b_region_suite.sql`), and a bash runner (`run_eval.sh`). That proof is not reproducible,
composable, or portable to the schema-delta work ahead.

This design moves the **onboarding-cogmap** seed + scenario out of hand-written SQL into
**declarative YAML that Rust consumes**, so a scenario simultaneously becomes (a) seed data,
(b) integration-test material, and (c) the prototype for JSONSchema/OpenAPI wire shapes — all from
**one** set of Rust structs, with no separate schema authoring and no drift.

## Strategic placement

Under the `substrate-kernel-to-cognitive-map` goal. The artifact at `schema-artifact/` is a fresh
one-shot **destination** schema loaded into the `temper_next` Postgres namespace (explicitly **NOT**
the sqlx-migrated production `public.*` schema — see `CLAUDE.md` "artifact-tests"). Declarative
scenarios make the region-projection proof reproducible and portable, which is the precondition for
the eventual temper-next ↔ temper delta analysis (Arc-level strategic payoff, milestone M4 below).

## First-milestone scope (M1)

Re-express the **onboarding-cogmap scenario we already built** in YAML — same telos, same
13-concept α/β/bridge/tension/isolate cast, same edges/facets/lenses — and roundtrip-validate:

> **YAML → write substrate (direct sqlx) → temper-next materialize (in-process) → read back → assert expectations.**

Success = identical regions + identical S6 passes to the current `run_eval.sh`. The full runbook
(S6a–h, including S6h's mid-run event mutation) is in scope for M1 — proving the DSL on the hardest
case derisks later generalization.

## Resolved design decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| **References** | Local stable `key:` ids (not JSON-Schema `$ref`) | Closed document — simpler, clearer. Edges/asserts reference resources by `key`; the loader builds `key → Uuid` as it inserts. |
| **Write path** | Direct sqlx against `temper_next`, reusing `cogmap_genesis()` | Same approach `write.rs`/`embed.rs` already use; whole roundtrip becomes one Rust integration test; types stay typed end-to-end. (Routing through temper-api targets the *production* schema, not the artifact — deferred to M4.) |
| **Scenario shape** | Substrate + **ordered step runbook** (materialize / emit-event / assert) | `run_eval.sh` is not flat load-then-assert; S6h mutates mid-run and re-materializes. Full parity requires modeling the sequence. |
| **Placement** | `crates/temper-next/src/scenario/` + `schema-artifact/scenarios/*.yaml` | Keeps artifact-scenario concepts inside the artifact boundary; temper-core stays the production shared-types crate. Extract to a shared crate only if/when M4 needs it. |
| **Schema emission** | `schemars::JsonSchema` (gated feature), snapshot-tested for drift | Proves the same structs that load the config define the wire shape. ts-rs/OpenAPI deferred until a consumer exists (YAGNI). |

## Architecture

Three new units in `crates/temper-next/src/scenario/`, plus reuse of the existing materialize path.

| Unit | Responsibility | Reuses |
|------|----------------|--------|
| `scenario/model.rs` | YAML structs + `Step`/`Expectation` enums. Derive `serde::Deserialize` + gated `schemars::JsonSchema`. | — |
| `scenario/loader.rs` | `load_scenario(pool, &Scenario) -> Result<KeyMap>` — direct sqlx writes to `temper_next`; calls `cogmap_genesis()` for the charter; inserts world/resources/content/edges/facets/lenses; returns `key → Uuid`. | `cogmap_genesis()` SQL fn |
| `scenario/runner.rs` | `run_scenario(pool, &Scenario) -> Result<()>` — executes steps **in order**, in-process; materialize steps call lib fns; emit-event steps insert events+edges; assert steps evaluate expectations; keeps a per-lens fingerprint cache. | `embed::embed_chunks`, `write::materialize_cogmap`, `substrate::load` |

Key shift from `run_eval.sh`: the runner **calls library functions in-process** rather than shelling
out to `cargo run -p temper-next`. The whole roundtrip becomes one `artifact-tests`-gated integration
test — no bash.

### Grounded reuse points (verified against HEAD, commit f6cff1e)

These are the exact signatures the loader/runner build on — carried into the implementation plan as
pre-grounded facts:

- **Materialize (Job B)** — `crates/temper-next/src/write.rs:14`
  ```rust
  pub async fn materialize_cogmap(pool: &PgPool, cogmap: Uuid, lens_name: &str) -> Result<MaterializeOutcome>
  ```
  returns `MaterializeOutcome { regions: usize, membership_fingerprint: String }` (`write.rs:6`).
- **Embed (Job A)** — `crates/temper-next/src/embed.rs:11`
  ```rust
  pub async fn embed_chunks(pool: &PgPool) -> Result<()>
  ```
  idempotent: only embeds `is_current AND NOT b.is_folded AND embedding IS NULL` chunks.
- **Connect** — `crates/temper-next/src/substrate.rs:14` `pub async fn connect() -> Result<PgPool>`;
  sets `search_path = temper_next, public`; `DATABASE_URL` env, default
  `postgresql://temper:temper@localhost:5437/temper_development`.
- **Cogmap lookup** — `crates/temper-next/src/substrate.rs:31`
  `pub async fn cogmap_by_name(pool: &PgPool, name: &str) -> Result<Uuid>`.
- **Genesis SQL fn** — `schema-artifact/02_functions.sql:458`
  ```sql
  cogmap_genesis(p_name text, p_telos_title text, p_telos_statement text,
                 p_questions text[], p_owner_profile uuid, p_emitter_entity uuid,
                 p_origin_uri text DEFAULT 'temper://genesis') RETURNS uuid
  ```
  Creates the genesis event (`cogmap_seeded`), telos resource (block-0 = statement, blocks 1..n =
  questions), the `kb_cogmaps` row, homes the telos in the cogmap, stamps `doc_type = cogmap_charter`.
- **Verdict cross-check view** — `schema-artifact/04b_region_suite.sql` builds
  `onboarding_s6_verdict` (single source of truth, keys on `origin_uri`); M1 reuses it as an
  independent equivalence check.

### Table write targets (verified against `schema-artifact/01_schema.sql` and `03_seed.sql`)

The loader writes these tables directly (column shapes confirmed from `03_seed.sql` inserts):

- `kb_event_types(name UNIQUE)` — minimal registry preamble (idempotent).
- `kb_profiles(handle, display_name, system_access)` — declared `world.profiles`.
- `kb_entities(profile_id, name, metadata)` — declared `world.entities`; the emitter actor.
- `kb_resources(title, origin_uri)` then `body_hash` backfill — concept resources.
- `kb_resource_homes(resource_id, anchor_table='kb_cogmaps', anchor_id, originator_profile_id, owner_profile_id)` — homes each concept in the cogmap.
- `kb_content_blocks(resource_id, seq, genesis_event_id, last_event_id)` + `kb_chunks(block_id, resource_id, chunk_index, content_hash)` + `kb_chunk_content(chunk_id, content)` — the inline prose (embed job fills `kb_chunks.embedding`). `kb_block_revisions(block_id, block_body_hash, chunk_count)` mirrors `cogmap_genesis`.
- `kb_edges(source_table, source_id, target_table, target_id, edge_kind, label, home_anchor_table='kb_cogmaps', home_anchor_id, asserted_by_event_id, last_event_id)` — declared relationships; `edge_kind ∈ {express, contains, leads_to, near}`.
- `kb_properties(owner_table='kb_resources', owner_id, property_key='facet', property_value JSONB, weight?, asserted_by_event_id, last_event_id)` — facets.
- `kb_cogmap_lenses(cogmap_id, name, selection_kind, w_express, w_contains, w_leads_to, w_near, w_prop, s_telos, s_ref, s_central, resolution, asserted_by_event_id)` — lens config.
- `kb_events(event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id, occurred_at)` — per assertion + the S6h mutation event.

## The YAML DSL (onboarding scenario)

```yaml
name: onboarding-cogmap

cogmap:
  telos:
    title: "Onboarding"
    statement: "Get a newcomer productive and safe by their first real change."
    questions: ["How do we pair?", "How do we deploy safely?"]
  owner: alice                    # profile handle (declared in world)
  emitter: onboarding-agent#1     # entity name (declared in world)

world:                            # minimal actors this scenario references
  profiles: [{ handle: alice, display_name: Alice, system_access: approved }]
  entities: [{ name: onboarding-agent#1, profile: alice }]

resources:
  - key: pair                     # local stable id — edges/asserts reference this
    title: "Pair newcomers"
    origin_uri: "temper://c/pair"
    home: cogmap
    doc_type: concept
    body: "Always pair a newcomer with a maintainer on their first PR."
    facets: [{ phase: first-week }]
  - key: staging
    origin_uri: "temper://c/staging"
    body: "Deploy to staging before production."
    facets: [{ topic: deployment, weight: 1.5 }]
  # … the full 13-concept α/β/bridge/tension/isolate cast …

edges:
  - { from: bluegreen, to: bigbang, kind: near, label: contradicts, weight: 1.0 }
  # … ~8 declared edges …

lenses:
  - { name: telos-default,           w_express: 1.0, w_contains: 1.0, w_leads_to: 0.6, w_near: 0.3, w_prop: 0.4, s_telos: 0.5, s_ref: 0.3, s_central: 0.2, resolution: 0.5 }
  - { name: telos-default-propheavy, w_express: 1.0, w_contains: 1.0, w_leads_to: 0.6, w_near: 0.3, w_prop: 0.9, s_telos: 0.5, s_ref: 0.3, s_central: 0.2, resolution: 0.5 }

steps:
  - materialize: { lens: telos-default }
  - assert:                                            # S6a, S6c, S6d, S6e, S6g
    - { region_count:     { lens: telos-default, op: ">=", value: 2 } }
    - { co_region:        { lens: telos-default, members: [pair, smallest], expect: true } }
    - { cohesion_order:   { lens: telos-default, greater: pair, lesser: staging } }
    - { region_size:      { lens: telos-default, member: solo, value: 1 } }
    - { co_region:        { lens: telos-default, members: [checklist, staging], expect: true } }
    - { internal_tension: { lens: telos-default, member: bluegreen, op: ">", value: 0 } }
  - materialize: { lens: telos-default }
  - assert: [{ reproducible: { lens: telos-default } }]                          # S6b
  - materialize: { lens: telos-default-propheavy }
  - assert:                                                                       # S6f
    - { fingerprint_differs: { lens_a: telos-default, lens_b: telos-default-propheavy } }
    - { co_region: { lens: telos-default,           members: [setup, firstbuild], expect: true } }
    - { co_region: { lens: telos-default-propheavy, members: [setup, firstbuild], expect: false } }
  - materialize: { lens: telos-default }
  - assert: [{ stale: { expect: false } }]                                        # S6h baseline
  - emit_event:                                                                   # S6h mutation
      type: relationship_asserted
      edges:
        - { from: solo, to: pair,       kind: express, label: related }
        - { from: solo, to: smallest,   kind: express, label: related }
        - { from: solo, to: confidence, kind: express, label: related }
  - assert: [{ stale: { expect: true } }]                                         # S6h fresh→stale
  - materialize: { lens: telos-default }
  - assert: [{ co_region: { lens: telos-default, members: [solo, pair], expect: true } }]  # S6h solo joins α
```

The `steps:` sequence is a line-by-line re-expression of `run_eval.sh`.

## Rust struct model (sketch — verify shapes at implementation)

```rust
#[derive(Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct Scenario {
    pub name: String,
    pub cogmap: CogmapDef,
    pub world: WorldDef,
    pub resources: Vec<ResourceDef>,
    pub edges: Vec<EdgeDef>,
    pub lenses: Vec<LensDef>,
    pub steps: Vec<Step>,
}

pub struct CogmapDef { pub telos: TelosDef, pub owner: String, pub emitter: String }
pub struct TelosDef  { pub title: String, pub statement: String, pub questions: Vec<String> }
pub struct WorldDef  { pub profiles: Vec<ProfileDef>, pub entities: Vec<EntityDef> }

pub struct ResourceDef {
    pub key: String, pub title: Option<String>, pub origin_uri: String,
    pub home: HomeRef,            // `cogmap` for M1
    pub doc_type: Option<String>, pub body: String,
    pub facets: Vec<FacetDef>,    // see facet decision below
}
pub struct EdgeDef { pub from: String, pub to: String, pub kind: EdgeKind, pub label: Option<String>, pub weight: f64 }
pub struct LensDef { /* mirrors kb_cogmap_lenses columns; mirrors affinity::Lens fields */ }

#[serde(rename_all = "snake_case")]
pub enum Step {
    Materialize { lens: String },
    EmitEvent   { #[serde(rename = "type")] event_type: String, edges: Vec<EdgeDef> },
    Assert(Vec<Expectation>),
}

#[serde(rename_all = "snake_case")]
pub enum Expectation {
    RegionCount     { lens: String, op: CmpOp, value: i64 },
    CoRegion        { lens: String, members: Vec<String>, expect: bool },
    CohesionOrder   { lens: String, greater: String, lesser: String },
    RegionSize      { lens: String, member: String, value: i64 },
    InternalTension { lens: String, member: String, op: CmpOp, value: f64 },
    Reproducible    { lens: String },
    FingerprintDiffers { lens_a: String, lens_b: String },
    Stale           { expect: bool },
}
```

`EdgeKind` reuses `affinity::EdgeKind` (Express/Contains/LeadsTo/Near). **Deserialization must be
exhaustive** — an unknown edge kind in YAML is a hard error, not a silent coerce (see deferred CR
finding `parse_kind` below).

## Expectation vocabulary → S6 mapping

| Variant | S6 | Compiles to |
|---------|-----|-------------|
| `region_count{lens, op, value}` | S6a | count of non-folded regions for lens |
| `co_region{lens, members[], expect}` | S6a/e/g/h | all members share one region (== `expect`) |
| `cohesion_order{lens, greater, lesser}` | S6c | `content_cohesion(region(greater)) > content_cohesion(region(lesser))` |
| `region_size{lens, member, value}` | S6d | size of member's region |
| `internal_tension{lens, member, op, value}` | S6g | `kb_cogmap_regions.internal_tension` of member's region |
| `reproducible{lens}` | S6b | current fingerprint == previous materialize of same lens (runner cache) |
| `fingerprint_differs{lens_a, lens_b}` | S6f | fingerprints differ across lenses |
| `stale{expect}` | S6h | `cogmap_staleness(cogmap).is_stale == expect` |

Fingerprint = `md5(string_agg(origin_uri ORDER BY region_id, origin_uri))` per lens — the exact
`fp()` shape in `run_eval.sh:44`, keyed on `origin_uri` so it is UUID-independent (stable across
seed paths). The closed set above is all M1 needs; new variants get added only when a scenario
needs one.

## Runner execution semantics

1. `connect()` → pool on `temper_next` search path.
2. `load_scenario` writes the substrate, returns `KeyMap`.
3. For each `Step` in order:
   - **Materialize**: `embed_chunks(pool).await?` then `materialize_cogmap(pool, cogmap_id, lens).await?`; store the returned `membership_fingerprint` in the per-lens cache (the prior value, if any, is what `reproducible` compares against).
   - **EmitEvent**: insert one `kb_events` row (emitter passed **explicitly** from the cogmap's declared emitter — see deferred CR finding `emitter_entity_id`), then the declared edges referencing it.
   - **Assert**: evaluate each `Expectation`; on failure, return an error naming the expectation, the members/keys involved, and actual vs expected (so the test failure reads like the `check` lines in `run_eval.sh`).
4. **Lens-name validation up front**: before the first materialize, validate every lens name referenced in `steps`/`lenses` exists, with a friendly error (see deferred CR finding "unknown lens name").

## Schema emission

- New gated feature on `temper-next` (e.g. `scenario-schema`), mirroring temper-core's `mcp`/`schemars` pattern: `#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]`.
- A test under that feature emits `schemars::schema_for!(Scenario)` and diffs it against committed
  `schema-artifact/scenarios/scenario.schema.json`, failing on drift.
- ts-rs / OpenAPI 3.0 deferred until a consumer exists.

## Testing & equivalence proof

- **Integration test** `crates/temper-next/tests/scenario_roundtrip.rs` (gated `artifact-tests`):
  load `onboarding-cogmap.yaml` → `run_scenario` → all declarative asserts pass → **cross-check**:
  run the committed `04b_region_suite.sql` verdict view against the YAML-seeded substrate and assert
  `onboarding_s6_verdict.all_pass = t`. Same prose → same embeddings → byte-identical regions (by
  `origin_uri`) → same verdict. This is the acceptance criterion's equivalence proof.
- **Unit tests** (ungated): model deserialization (incl. unknown-edge-kind rejection), `key → Uuid`
  resolution, expectation evaluation against a tiny in-memory/fixture shape where feasible.
- **Schema drift test** (gated `scenario-schema`): snapshot comparison.

## Acceptance (M1)

- [ ] The onboarding scenario as YAML roundtrips to the same regions + S6 verdicts as `run_eval.sh`
      (cross-checked via `onboarding_s6_verdict.all_pass = t`).
- [ ] A Rust loader reads the YAML, writes the substrate, runs the harness in-process, and asserts
      the declarative expectations as an `artifact-tests` integration test.
- [ ] `JsonSchema` is emitted from the loader structs and snapshot-tested for drift.

## Deferred code-review findings folded into M1

These were surfaced by `/code-review high` on the emergent-region PR and deferred; the scenario path
forces several at their right altitude:

- **`parse_kind` silent fallback** (`substrate.rs`) — YAML edge-kind deserialization must be
  exhaustive/erroring. **Fix in M1** (the edge loader needs it).
- **`emitter_entity_id` derivation** (`write.rs`) — the materialization/emit-event events must pass
  the emitter **explicitly** (the YAML declares it) rather than copying from "latest event"
  (NULL on empty log → NOT NULL violation; arbitrary on `occurred_at` ties). **Fix in M1.**
- **Unknown lens name → opaque `RowNotFound`** — runner validates lens names up front. **Fix in M1.**
- **`label_factor` dead placeholder** (`affinity.rs`) — `label_factor() -> 1.0` is an unused factor +
  a test for the placeholder. **Drop in M1** (no-premature-abstraction), unless the facet/opposed-labels
  work below first gives it a real job.
- **Multi-key facets** (`substrate.rs`) — the facet parser reads only the FIRST key of a facet's JSON
  object. The YAML `facets:` model must **decide** one-facet-per-row (current convention) vs multi-key
  and align the loader + reader. **Decide in M1** (the facet loader is written here).

Deferred beyond M1 (named in the roadmap):
- **`internal_tension` opposed-labels lens-configurable** (`write.rs` hardcodes `ARRAY['contradicts']`)
  — needs a lens `opposed_labels` column; forced by a scenario that varies opposition vocabulary (M2/M3).
- **Reuse `format_embedding`** (`temper-core::types::ingest`) — `embed.rs`/`write.rs` hand-roll the
  pgvector literal; consolidate (M2 housekeeping).
- **Affinity memoization** (`cluster.rs` recomputes pairwise affinity per merge) — scale concern, not
  at eval size (M2+/production lift).

## Milestone roadmap (recorded under `substrate-kernel-to-cognitive-map`)

- **M1 (this spec):** onboarding scenario as YAML, full S6a–h, direct-sqlx loader, in-process runner,
  JsonSchema snapshot, integration test. Folds in the M1 deferred CR findings above.
- **M2:** generalize — a second (synthetic/minimal) scenario, a dir-driven runner, retire
  `run_eval.sh`'s bespoke parts; reuse `format_embedding`; affinity memoization if scale demands;
  lens-configurable opposed-labels.
- **M3:** access scaffold (teams/profiles/grants) in YAML — the S1–S5 world.
- **M4 (strategic payoff):** temper-next ↔ temper schema-delta analysis; possibly route a subset
  through real write paths (temper-client/temper-api) to test the actual production system.

## Out of scope (M1)

- Access scaffold (teams/profiles/grants) in YAML — M3.
- The full temper-next ↔ temper schema-delta analysis — M4 (this DSL is the enabler, not the deliverable).
- ts-rs / OpenAPI 3.0 emission — until a consumer exists.
- Routing writes through temper-api — wrong schema for the artifact; M4.
```
