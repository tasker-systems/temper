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
| **Write path / mechanics** | **Reusable SQL functions** in `02_functions.sql` (the `cogmap_genesis` mold): each emits its event **and** projects in one txn. The Rust loader is a thin caller passing YAML inputs. | The seed's create/associate work is *reusable* substrate, not throwaway. `02_functions.sql` has no mutation functions yet except `cogmap_genesis` — finishing that pattern (`resource_create`, `relationship_assert`, `facet_set`, `lens_create`) gives the temper-cogmap lift real write paths driven by events, with the YAML as the event input source. (Routing through temper-api targets the *production* schema — deferred to M4.) |
| **SQL checking** | `sqlx::query!`/`query_scalar!`/`query_as!` **macros** for temper-next's reusable (non-test) code; per-crate `crates/temper-next/.sqlx` prepared against the loaded `temper_next` artifact | Compile-time shape safety for code we rely on (CLAUDE.md SQL rule). Thin `SELECT fn(...)` callers over the SQL functions are trivial to cache; runtime `.get()` shape-drift is eliminated. Tests keep runtime `query()` (sqlx can't cache test-target queries). |
| **System boot-seed** | A canonical boot-seed (event-type registry + **global system lenses**) seeded **separately** from scenario seeds | Splits "any temper system needs this" from "this test scenario needs this." Event types already live inline at `03_seed.sql:30-37`; lenses (`telos-default`/`-propheavy`) become global (`cogmap_id IS NULL`) via `lens_created`. Scenarios reference lenses by name, not by re-declaring weights. |
| **Scenario shape** | Substrate + **ordered step runbook** (materialize / emit-event / assert) | `run_eval.sh` is not flat load-then-assert; S6h mutates mid-run and re-materializes. Full parity requires modeling the sequence. |
| **Placement** | `crates/temper-next/src/scenario/` + `schema-artifact/scenarios/*.yaml` + boot-seed at `schema-artifact/seeds/` | Keeps artifact-scenario concepts inside the artifact boundary; temper-core stays the production shared-types crate. Extract to a shared crate only if/when M4 needs it. |
| **Schema emission** | `schemars::JsonSchema` (gated feature), snapshot-tested for drift | Proves the same structs that load the config define the wire shape. ts-rs/OpenAPI deferred until a consumer exists (YAGNI). |

## Architecture

Two layers: **reusable mutation mechanics as SQL functions** in the artifact, and a **thin Rust
loader/runner** in `crates/temper-next/src/scenario/` that calls them with YAML inputs.

### Layer 1 — mutation mechanics (SQL functions in `02_functions.sql`)

Each follows the `cogmap_genesis` precedent: one txn, emit the event, project the rows, return the id.
The Rust side never inserts substrate tables directly — it calls these. This is the event-sourced shape
the temper-cogmap lift reuses: *an event is fired; a reusable function projects it.*

| Function | Event emitted | Projects | New event type? |
|----------|---------------|----------|-----------------|
| `resource_create(title, origin_uri, home_cogmap, owner, body, doc_type, emitter)` | `resource_created` | `kb_resources` + home + `kb_content_blocks`/`kb_chunks`/`kb_chunk_content` + `kb_block_revisions` + optional `doc_type` property; backfills `body_hash` | no (exists) |
| `relationship_assert(src, tgt, kind, label, weight, home_cogmap, emitter)` | `relationship_asserted` | `kb_edges` | no (exists) |
| `facet_set(resource, values_jsonb, weight, emitter)` | `property_asserted` | the single `property_key='facet'` `kb_properties` row | **yes** (`property_asserted`) |
| `lens_create(cogmap_or_null, name, weights…, emitter)` | `lens_created` | `kb_cogmap_lenses` (global when `cogmap_or_null IS NULL`) | **yes** (`lens_created`) |
| `cogmap_genesis(…)` (exists, `02_functions.sql:458`) | `cogmap_seeded` | telos charter + cogmap + home | — |

`region_materialize` stays Rust (`write::materialize_cogmap`) — it is clustering output, not a simple
projection — but emits `region_materialized` with an **explicit** emitter (no latest-event derivation).

### Layer 2 — Rust units (`crates/temper-next/src/scenario/`)

| Unit | Responsibility | Reuses |
|------|----------------|--------|
| `scenario/model.rs` | YAML structs + `Step`/`Expectation` enums. `serde::Deserialize` + gated `schemars::JsonSchema`. Lenses referenced by **name** (system-seeded), not redeclared. | — |
| `scenario/bootseed.rs` | `seed_system(pool)` — loads the canonical boot-seed (event-type registry + global system lenses via `lens_create`). Idempotent. Separate from any scenario. | `lens_create` SQL fn |
| `scenario/loader.rs` | `load_scenario(pool, &Scenario) -> Result<Loaded>` — thin: calls `cogmap_genesis` then `resource_create`/`facet_set`/`relationship_assert` per YAML element; builds `key → Uuid` (incl. implicit `telos`). All via `query_scalar!`. | the Layer-1 SQL fns |
| `scenario/runner.rs` | `run_scenario(pool, &Scenario)` — executes steps **in order**, in-process; materialize → `embed_chunks` + `materialize_cogmap`; emit-event → `relationship_assert`; assert → expectation eval; per-lens fingerprint cache; up-front lens-name validation. | `embed::embed_chunks`, `write::materialize_cogmap` |

Key shift from `run_eval.sh`: the runner **calls library/SQL functions in-process** rather than shelling
out to `cargo run -p temper-next`. The whole roundtrip becomes one `artifact-tests`-gated integration
test — no bash. The Rust mutation/read queries use `sqlx::query!`/`query_scalar!`/`query_as!` macros
(per-crate `.sqlx` prepared against the loaded `temper_next` artifact); test-target queries stay runtime.

**Spec/assertion separability (built in, exploited later).** The `load_scenario` ↔ `run_scenario` split
is not just a code boundary — it embodies the two roles a scenario file plays. `load_scenario`
instantiates the **substrate template** (cogmap + telos + resources + edges + lenses — the *cogmap/telos
specification*); `run_scenario` is `load_scenario` **plus** driving the `steps:` runbook (the *assertion
specification*). For M1 a single file fuses both, deliberately: the assertions-as-tests must track the
cogmap/telos shape tightly, so co-locating them keeps them honest. But because `steps:` is a separable
field, a future **foundational** cogmap (e.g. `system-default`) is *not a special case* — it is the same
template instantiated via `load_scenario` (+ a materialize) with no `steps:` overlay. This is the seam the
M2/M3 "retire `03_seed.sql` entirely / foundational-cogmaps-as-templates" work grows along (see roadmap).

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

### Table write targets (owned by the Layer-1 SQL functions)

The **SQL functions** own these inserts (column shapes confirmed from `03_seed.sql`). The Rust loader
does not touch these tables directly — it calls the functions. Listed so the function bodies are grounded:

- `kb_event_types(name UNIQUE)` — seeded by the **boot-seed**, not per scenario.
- `kb_profiles(handle, display_name, system_access)` / `kb_entities(profile_id, name, metadata)` — the scenario `world` (loader seeds these directly; they are tiny identity rows, not event-projected substrate for M1).
- `resource_create` → `kb_resources(title, origin_uri)` + `body_hash` backfill; `kb_resource_homes(resource_id, anchor_table='kb_cogmaps', anchor_id, originator_profile_id, owner_profile_id)`; `kb_content_blocks(resource_id, seq, genesis_event_id, last_event_id)` + `kb_chunks(block_id, resource_id, chunk_index, content_hash)` + `kb_chunk_content(chunk_id, content)` + `kb_block_revisions(block_id, block_body_hash, chunk_count)`; optional `doc_type` property.
- `relationship_assert` → `kb_edges(source_table, source_id, target_table, target_id, edge_kind, label, weight, home_anchor_table='kb_cogmaps', home_anchor_id, asserted_by_event_id, last_event_id)`; `edge_kind ∈ {express, contains, leads_to, near}`.
- `facet_set` → `kb_properties(owner_table='kb_resources', owner_id, property_key='facet', property_value JSONB, weight, asserted_by_event_id, last_event_id)` — **exactly one `property_key='facet'` row per resource** (multi-key coherent object; see "Facet model"). The one-row rule is scoped per property type, not per resource.
- `lens_create` → `kb_cogmap_lenses(cogmap_id, name, selection_kind, w_express, w_contains, w_leads_to, w_near, w_prop, s_telos, s_ref, s_central, resolution, asserted_by_event_id)`; `cogmap_id` NULL ⇒ global system lens.
- `kb_events(event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id, occurred_at)` — every function emits one; plus the runner's S6h mutation event.

### `!`-macro prepare ritual (new infra)

`temper-next` is the first artifact-schema crate to use `sqlx::query!` macros. To compile under
`SQLX_OFFLINE=true` (CI), commit a per-crate cache at `crates/temper-next/.sqlx`, regenerated whenever its
SQL changes:
```bash
# artifact must be loaded into the temper_next namespace first
for f in 01_schema 02_functions; do psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f schema-artifact/$f.sql; done
DATABASE_URL="postgresql://temper:temper@localhost:5437/temper_development?options=-csearch_path%3Dtemper_next,public" \
  cargo sqlx prepare -p temper-next   # per-crate, NOT --workspace (workspace prepare clobbers per-crate caches)
```
Add a `cargo make prepare-next` task for this. The Embed CI job (which loads the artifact + has ONNX) is
the natural place to verify the cache is current; other jobs compile offline against the committed cache.

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
    facets: { values: { phase: first-week } }            # → ONE kb_properties row, weight 1.0
  - key: staging
    origin_uri: "temper://c/staging"
    body: "Deploy to staging before production."
    facets: { values: { topic: deployment }, weight: 1.5 }   # multi-key & array values allowed in `values`
  # … the full 13-concept α/β/bridge/tension/isolate cast …

edges:
  - { from: bluegreen, to: bigbang, kind: near, label: contradicts, weight: 1.0 }
  # … ~8 declared edges …

uses_lenses: [telos-default, telos-default-propheavy]   # referenced by name; defined in the system boot-seed (below)

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

The `steps:` sequence is a line-by-line re-expression of `run_eval.sh`. Edges and asserts reference
resources by local `key:`; the loader builds the `key → Uuid` map as it inserts. The scenario no longer
declares lens weights — it names the lenses it uses; `uses_lenses` drives up-front validation that those
lenses exist (system-seeded).

**System boot-seed** (`schema-artifact/seeds/system.yaml`) — loaded once, before any scenario, by
`seed_system`. The split: this is what *any* temper system needs; the scenario is what *this test* needs.
```yaml
event_types:   # the registry currently inline at 03_seed.sql:30-37, plus the two new verbs
  - resource_created
  - relationship_asserted
  - region_materialized
  - property_asserted     # NEW — facet_set emits this
  - lens_created          # NEW — lens_create emits this
  # … the rest of the existing registry …
lenses:        # global system lenses (cogmap_id NULL), created via lens_create; EXACT values from 03_seed.sql:225,234
  - { name: telos-default,           w_express: 1.0, w_contains: 1.0, w_leads_to: 0.6, w_near: 0.3, w_prop: 0.4, s_telos: 0.5, s_ref: 0.3, s_central: 0.2, resolution: 0.5 }
  - { name: telos-default-propheavy, w_express: 1.0, w_contains: 1.0, w_leads_to: 0.1, w_near: 0.3, w_prop: 1.2, s_telos: 0.5, s_ref: 0.3, s_central: 0.2, resolution: 0.5 }
```

**Full node set (for exact equivalence):** `cogmap_genesis` homes the **telos charter** resource in the
cogmap, so it is a clustered node too — and `03_seed.sql:197-216` also homes a **regulation** resource
(`temper://reg/pair`, `doc_type=cogmap_regulation`) with a body and a `telos → regulation` express edge
(`operationalized_by`). To roundtrip to byte-identical regions the YAML must reproduce **all 15 nodes**
(telos + regulation + 13 concepts) and **all 10 edges** (the 8-row batch + `setup→firstbuild` +
`telos→regulation`). The loader therefore seeds an implicit **`telos`** key in the `KeyMap` (resolving to
`kb_cogmaps.telos_resource_id`) so an edge like
`{ from: telos, to: regulation, kind: express, label: operationalized_by }` resolves; `regulation` is an
ordinary `resources:` entry with `doc_type: cogmap_regulation`. The seed's `now()` "late-touch" event
(`03_seed.sql:482-485`) is **omitted** — the in-process runner sequences all substrate writes before the
first materialize, so the watermark is naturally latest and the map is fresh without it.

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
    pub uses_lenses: Vec<String>,   // names of system lenses; validated up front
    pub steps: Vec<Step>,
}

// Boot-seed model (schema-artifact/seeds/system.yaml), loaded by seed_system — distinct from Scenario.
pub struct BootSeed { pub event_types: Vec<String>, pub lenses: Vec<LensDef> }

pub struct CogmapDef { pub telos: TelosDef, pub owner: String, pub emitter: String }
pub struct TelosDef  { pub title: String, pub statement: String, pub questions: Vec<String> }
pub struct WorldDef  { pub profiles: Vec<ProfileDef>, pub entities: Vec<EntityDef> }

pub struct ResourceDef {
    pub key: String, pub title: Option<String>, pub origin_uri: String,
    pub home: HomeRef,            // `cogmap` for M1
    pub doc_type: Option<String>, pub body: String,
    pub facets: Option<FacetDef>, // ONE facet row per resource (see facet model below)
}
// One kb_properties row per resource: `values` is a multi-key JSONB object whose values
// may be scalars OR arrays; `weight` (default 1.0) applies to every (path,value) pair the
// row expands to. A bare map (`facets: { phase: x }`) is sugar for `{ values: {phase: x}, weight: 1.0 }`.
pub struct FacetDef { pub values: serde_json::Map<String, serde_json::Value>, #[serde(default = "one")] pub weight: f64 }
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

### Facet model (resolved)

A resource's facets materialize to **exactly one `kb_properties` row per property type** — the single
`property_key='facet'` row — whose `property_value` is the coherent multi-key JSONB object and whose
`weight` column carries the row weight (default 1.0). The one-row rule is scoped to the property type,
**not** to the resource: a resource still carries other property rows (e.g. its `property_key='doc_type'`
row, and keyword/other property shapes as they evolve), each its own coherent record. Rationale: within
a property type, facets stay coherent — no risk of inconsistent/half-updated facets spread across rows,
and no need to fold/disambiguate multiple facet-style rows for one resource — and array-valued keys
(`{ topic: [deployment, release] }`) read as **one intentional thing** instead of being ambiguously
split into separate rows.

The affinity reader expands that one row into the `Facet { owner, path, value, weight }` entries the
clustering needs: each `(key, value)` pair becomes one `Facet`; an array value
`{ topic: [deployment, release] }` expands to one `Facet` per element, all sharing the row weight.
This is an **AMEND** to the current reader `crates/temper-next/src/substrate.rs:73-93`, which today
reads only the first key (`v.as_object()?.iter().next()`) — the deferred multi-key-facet finding is
fixed here by *supporting* multi-key, not by restricting to one-key-per-row. (Per-`(path,value)`
weight differentiation within one resource is a future extension; M1's onboarding cast has at most
one facet per resource, so row-level weight is sufficient and exact.)

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
  evaluate the `04b` verdict logic against the YAML-seeded substrate and assert `all_pass = t`. Same
  prose → same embeddings → byte-identical regions (by `origin_uri`) → same verdict. This is the
  acceptance criterion's equivalence proof.
- **Nextest-native cross-check:** the cross-check runs **inside the integration test via `sqlx`**, not
  by shelling to `psql`/`run_eval.sh` — so the entire proof runs under `cargo nextest`. The simplest
  faithful encoding is to execute the `04b_region_suite.sql` verdict query (the `WITH td … SELECT …
  all_pass` body) as one `sqlx::query` and read `all_pass`; it stays an **independent** encoding (a
  SQL aggregate over `origin_uri`) of the same checks the Rust asserts cover separately, which is the
  point of running both. **Retire path:** `04b_region_suite.sql` and `run_eval.sh`'s bespoke S6 gate
  are retired in **M2** once the declarative asserts are trusted against the SQL verdict; during the
  transition both encodings run side-by-side in this one nextest test.
- **Unit tests** (ungated): model deserialization (incl. unknown-edge-kind rejection), `key → Uuid`
  resolution, expectation evaluation against a tiny in-memory/fixture shape where feasible.
- **Schema drift test** (gated `scenario-schema`): snapshot comparison.

## Acceptance (M1)

- [ ] Reusable mutation SQL functions (`resource_create`, `relationship_assert`, `facet_set`,
      `lens_create`) exist in `02_functions.sql` in the `cogmap_genesis` mold (emit event + project),
      with the two new event types (`property_asserted`, `lens_created`).
- [ ] A canonical system boot-seed (`schema-artifact/seeds/system.yaml`) carries the event-type registry
      + global system lenses; `seed_system` loads it, separately from any scenario.
- [ ] The onboarding scenario as YAML roundtrips to the same regions + S6 verdicts as `run_eval.sh`,
      cross-checked via the `04b` verdict logic run through sqlx inside a nextest integration test.
- [ ] A thin Rust loader reads the YAML and calls the SQL functions; the runner drives the step runbook
      in-process and asserts the declarative expectations as an `artifact-tests` integration test.
- [ ] temper-next's reusable (non-test) queries use `sqlx::query!`/`query_scalar!`/`query_as!` with a
      committed `crates/temper-next/.sqlx` cache; `cargo make prepare-next` regenerates it.
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
- **Multi-key facets** (`substrate.rs:73-93`) — the facet parser reads only the FIRST key
  (`v.as_object()?.iter().next()`). **Fixed in M1** (AMEND): exactly one `property_key='facet'` row
  per resource; reader iterates all keys and expands array values into multiple `Facet` entries
  (see "Facet model").

Deferred beyond M1 (named in the roadmap):
- **`internal_tension` opposed-labels lens-configurable** (`write.rs` hardcodes `ARRAY['contradicts']`)
  — needs a lens `opposed_labels` column; forced by a scenario that varies opposition vocabulary (M2/M3).
- **Reuse `format_embedding`** (`temper-core::types::ingest`) — `embed.rs`/`write.rs` hand-roll the
  pgvector literal; consolidate (M2 housekeeping).
- **Affinity memoization** (`cluster.rs` recomputes pairwise affinity per merge) — scale concern, not
  at eval size (M2+/production lift).

## Milestone roadmap (recorded under `substrate-kernel-to-cognitive-map`)

- **M1 (this spec):** reusable mutation SQL functions (`resource_create`/`relationship_assert`/`facet_set`/
  `lens_create`) + system boot-seed (event types + global lenses); thin `!`-macro Rust loader/runner;
  onboarding scenario as YAML, full S6a–h; JsonSchema snapshot; nextest-native integration test. Folds in
  the M1 deferred CR findings above.
- **M2:** generalize — a second (synthetic/minimal) scenario, a dir-driven runner, **retire
  `04b_region_suite.sql` + `run_eval.sh`'s bespoke S6 gate** once the declarative asserts are trusted
  against the SQL verdict; reuse `format_embedding`; affinity memoization if scale demands;
  lens-configurable opposed-labels.
- **M3:** access scaffold (teams/profiles/grants) in YAML — the S1–S5 world — and **the full retirement
  of `03_seed.sql`**: every entity the seed creates (the access world, the foundational `system-default`
  cogmap) moves to scenario YAML, so the loader is the *only* seed path. A **foundational cogmap is the
  same-structure template, not a special case** — `system-default` becomes a scenario file instantiated
  via `load_scenario` at foundation time (no `steps:` overlay), exactly the spec/assertion seam described
  in §Architecture. The cogmap/telos *specification* half and the *assertion* half may split into separate
  files here if foundational instantiation wants the template without a test overlay.
- **M4 (strategic payoff):** temper-next ↔ temper schema-delta analysis; possibly route a subset
  through real write paths (temper-client/temper-api) to test the actual production system.

### Forward intentions (held separate from M1, captured so they aren't lost)

- **Scenarios become self-sufficient:** the end state is *no `03_seed.sql`* — cogmap-scenarios fully seed
  what the artifact needs. M1 already demonstrates this for the onboarding cogmap (its roundtrip test loads
  only `01_schema`+`02_functions`+the YAML, never `03_seed.sql`); M3 finishes the job for the rest.
- **Foundational = template, not special case:** system-foundational cogmaps run through the same loader
  as test scenarios. No bespoke foundational SQL.
- **Spec ⟂ assertion may separate:** today one file fuses the cogmap/telos specification with its assertion
  runbook (correct — the tests must track the spec). The `load_scenario`/`run_scenario` split already keeps
  them mechanically separable; a later milestone may split them into distinct files once foundational
  instantiation (template only) and testing (template + asserts) have genuinely different consumers.
- **Create-and-home, NOT re-home (an access-gated future, not a casual add):** `resource_create` bundles
  creation with homing in one cogmap — correct for scenarios, which co-create concepts in their map. The
  *only* reason to separate create from associate would be to **re-home** an existing resource into another
  cogmap (a shallow-copy-with-pointers / share). That is deliberately out of scope and **must not be added
  without an access reconciliation first**: a cogmap's visibility/RBAC grounds which concept-content is
  surfaceable in it, and those boundaries are not identical across cogmaps — re-homing across them risks
  **information leak of concept-content** across access boundaries (the goal's *awareness-is-access-bounded*
  invariant). If re-homing is ever pursued, the gate is: reconcile `resources_accessible_to_cogmap` /
  `resources_visible_to` on both sides before any cross-cogmap association, never the homing mechanic alone.

## Out of scope (M1)

- Access scaffold (teams/profiles/grants) in YAML — M3.
- The full temper-next ↔ temper schema-delta analysis — M4 (this DSL is the enabler, not the deliverable).
- ts-rs / OpenAPI 3.0 emission — until a consumer exists.
- Routing writes through temper-api — wrong schema for the artifact; M4.
- **Re-homing** a resource into another cogmap (share / shallow-copy-with-pointers) — `resource_create`
  only creates-new-and-homes. Re-homing is access-gated future work (see Forward intentions: information-leak
  risk across non-identical cogmap visibility boundaries).
```
