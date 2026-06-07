# Scenario YAML Seed/Scenario DSL — M1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Re-express the onboarding-cogmap seed + full S6a–h falsification runbook as declarative YAML consumed by a Rust loader/runner in `temper-next`, roundtrip-validated to byte-identical regions and S6 verdicts as `run_eval.sh`, with JSON Schema emitted from the same structs.

**Architecture:** New `scenario/` module set in `crates/temper-next/src/` — `model.rs` (YAML structs + `Step`/`Expectation` enums), `loader.rs` (direct-sqlx writes to the `temper_next` artifact, reusing the `cogmap_genesis()` SQL fn, returning a `key → Uuid` map), `runner.rs` (executes the ordered step runbook **in-process**, reusing `embed::embed_chunks` + `write::materialize_cogmap`). The whole roundtrip becomes one `artifact-tests`-gated `cargo nextest` integration test — no bash.

**Tech Stack:** Rust, `serde`/`serde_yaml`, `sqlx` (Postgres, `temper_next` search path), `schemars` (gated JSON Schema), bge-768 embeddings via `temper-ingest`.

**Spec:** `docs/superpowers/specs/2026-06-07-scenario-yaml-seed-dsl-design.md` — read it before starting. Load-bearing invariant carried verbatim from the spec: *"Same prose → same embeddings → byte-identical regions (by `origin_uri`) → same verdict."* The fingerprint and verdict are keyed on `origin_uri`, NOT UUID, so they are stable across seed paths.

**Grounding tags** (per `~/.claude/skills/temper/guidance/implementation-grounding.md`): each task is tagged CONFORM (honor an existing constraint), EXTEND (build beyond an affordance, spec-authorized), or AMEND (change an existing thing, disk + spec cited). Treat the plan's quoted `file:line` excerpts as pre-grounded facts; verify anything NOT quoted on disk before use.

**Prerequisites for running tests in this plan:**
- Docker Postgres on port 5437: `cargo make docker-up`.
- The artifact schema loaded into `temper_next`: `for f in 01_schema 02_functions; do psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f schema-artifact/$f.sql; done` (the loader replaces `03_seed.sql`).
- `artifact-tests` + ONNX runtime for the embed-dependent integration test (the box that runs the Embed CI job). Pure unit tests need neither.
- `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`.

---

## File structure

| File | Responsibility | Task |
|------|----------------|------|
| `crates/temper-next/Cargo.toml` | Add `serde`/`serde_yaml`/gated `schemars` deps + `scenario-schema` feature | 1 |
| `crates/temper-next/src/lib.rs` | Wire `scenario` module | 1 |
| `crates/temper-next/src/scenario/mod.rs` | Re-export model/loader/runner | 1 |
| `crates/temper-next/src/affinity.rs` | `EdgeKind`: add `Deserialize`(snake_case) + gated `JsonSchema`; drop `label_factor` placeholder | 2 |
| `crates/temper-next/src/substrate.rs` | `parse_kind` exhaustive/erroring; facet reader multi-key/array AMEND | 2, 3 |
| `crates/temper-next/src/scenario/model.rs` | YAML structs + `Step`/`Expectation` enums | 4 |
| `crates/temper-next/src/write.rs` | `materialize_cogmap` takes explicit emitter (AMEND) | 5 |
| `crates/temper-next/src/main.rs` | Pass emitter to `materialize_cogmap` (AMEND caller) | 5 |
| `crates/temper-next/src/scenario/loader.rs` | World preamble, genesis+telos key, resources, content, facets, edges, lenses; returns `KeyMap` | 6–10 |
| `crates/temper-next/src/scenario/runner.rs` | Lens validation, step loop, materialize/emit-event/assert, fingerprint cache, expectation eval | 11–13 |
| `schema-artifact/scenarios/onboarding-cogmap.yaml` | The onboarding scenario, full cast | 14 |
| `crates/temper-next/tests/scenario_roundtrip.rs` | `artifact-tests` integration test + 04b verdict cross-check via sqlx | 15 |
| `schema-artifact/scenarios/scenario.schema.json` | Committed JSON Schema snapshot | 16 |
| `crates/temper-next/tests/scenario_schema.rs` | `scenario-schema` drift test | 16 |

---

## Task 1: Scaffold deps + module skeleton

**Tag:** EXTEND (new module; spec §Architecture authorizes `scenario/` in temper-next).

**Files:**
- Modify: `crates/temper-next/Cargo.toml`
- Modify: `crates/temper-next/src/lib.rs`
- Create: `crates/temper-next/src/scenario/mod.rs`
- Create (empty stubs): `crates/temper-next/src/scenario/{model,loader,runner}.rs`

- [ ] **Step 1: Add deps + feature to `Cargo.toml`**

In `[dependencies]` add:
```toml
serde = { version = "1", features = ["derive"] }
serde_yaml = "0.9"
schemars = { version = "1", features = ["uuid1"], optional = true }
```
In `[features]` add:
```toml
scenario-schema = ["schemars"]
```
(Keep the existing `artifact-tests` feature unchanged.)

- [ ] **Step 2: Create the module stubs**

`crates/temper-next/src/scenario/mod.rs`:
```rust
pub mod loader;
pub mod model;
pub mod runner;
```
Create `model.rs`, `loader.rs`, `runner.rs` as empty files (content added in later tasks).

- [ ] **Step 3: Wire into `lib.rs`**

Add `pub mod scenario;` alongside the existing `pub mod` lines in `crates/temper-next/src/lib.rs` (current contents: `affinity`, `cluster`, `embed`, `substrate`, `write`).

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p temper-next`
Expected: builds clean (empty modules).

- [ ] **Step 5: Commit**
```bash
git add crates/temper-next/Cargo.toml crates/temper-next/src/lib.rs crates/temper-next/src/scenario/
git commit -m "feat(temper-next): scaffold scenario module + serde/serde_yaml/schemars deps"
```

---

## Task 2: `EdgeKind` deserialization + exhaustive `parse_kind` + drop `label_factor`

**Tag:** AMEND. Disk: `affinity.rs:4-9` (EdgeKind), `affinity.rs:68-70` (`label_factor`), `substrate.rs:127-134` (`parse_kind` with `_ => Near`). Spec: "Deserialization must be exhaustive — an unknown edge kind is a hard error, not a silent coerce"; deferred-CR `parse_kind` and `label_factor`.

**Files:**
- Modify: `crates/temper-next/src/affinity.rs`
- Modify: `crates/temper-next/src/substrate.rs:127-134`

- [ ] **Step 1: Write failing test for snake_case EdgeKind deserialization**

Add to `crates/temper-next/src/affinity.rs` (in a `#[cfg(test)] mod tests`):
```rust
#[test]
fn edge_kind_deserializes_snake_case_and_rejects_unknown() {
    assert_eq!(serde_yaml::from_str::<EdgeKind>("leads_to").unwrap(), EdgeKind::LeadsTo);
    assert_eq!(serde_yaml::from_str::<EdgeKind>("express").unwrap(), EdgeKind::Express);
    assert!(serde_yaml::from_str::<EdgeKind>("sideways").is_err());
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p temper-next edge_kind_deserializes -- --nocapture`
Expected: FAIL — `EdgeKind` does not implement `Deserialize`.

- [ ] **Step 3: Add derives to `EdgeKind`**

Change `affinity.rs:4-9` from:
```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EdgeKind {
    Express,
    Contains,
    LeadsTo,
    Near,
}
```
to:
```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum EdgeKind {
    Express,
    Contains,
    LeadsTo,
    Near,
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p temper-next edge_kind_deserializes`
Expected: PASS.

- [ ] **Step 5: Make `parse_kind` exhaustive/erroring**

`parse_kind` reads `edge_kind::text` from the DB. Change `substrate.rs:127-134` from:
```rust
fn parse_kind(s: String) -> EdgeKind {
    match s.as_str() {
        "express" => EdgeKind::Express,
        "contains" => EdgeKind::Contains,
        "leads_to" => EdgeKind::LeadsTo,
        _ => EdgeKind::Near,
    }
}
```
to:
```rust
fn parse_kind(s: &str) -> anyhow::Result<EdgeKind> {
    Ok(match s {
        "express" => EdgeKind::Express,
        "contains" => EdgeKind::Contains,
        "leads_to" => EdgeKind::LeadsTo,
        "near" => EdgeKind::Near,
        other => anyhow::bail!("unknown edge_kind from DB: {other:?}"),
    })
}
```
Update the call site at `substrate.rs:66` from `kind: parse_kind(r.get::<String, _>("kind")),` to
`kind: parse_kind(r.get::<String, _>("kind").as_str())?,` — and because the closure now uses `?`, change
the `.map(|r| Edge {...}).collect()` (lines 61-70) to a fallible collect:
```rust
    let edges = edge_rows
        .iter()
        .map(|r| -> anyhow::Result<Edge> {
            Ok(Edge {
                src: r.get("source_id"),
                tgt: r.get("target_id"),
                kind: parse_kind(r.get::<String, _>("kind").as_str())?,
                weight: r.get("weight"),
                label: r.get("label"),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
```

- [ ] **Step 6: Drop the `label_factor` dead placeholder**

In `affinity.rs`, remove the `label_factor()` fn (around `affinity.rs:68-70`, body `-> 1.0`) and its
multiplication in `affinity()` (`affinity.rs:91-99` — remove the `* label_factor(&e.label, lens)` factor
so the term is `lens.w_kind(e.kind) * e.weight`). Remove any unit test that asserts only the `1.0`
placeholder. If `lens` becomes unused in a helper after this, fix the warning. (Per spec: drop until a
lens actually overrides labels.)

- [ ] **Step 7: Run the crate's existing unit tests**

Run: `cargo test -p temper-next`
Expected: PASS (cluster_determinism + affinity tests; the new edge_kind test). The affinity change is a
no-op numerically because `label_factor` always returned `1.0`.

- [ ] **Step 8: Commit**
```bash
git add crates/temper-next/src/affinity.rs crates/temper-next/src/substrate.rs
git commit -m "fix(temper-next): exhaustive edge_kind (deser + DB parse), drop label_factor placeholder"
```

---

## Task 3: Facet reader AMEND — multi-key + array expansion

**Tag:** AMEND. Disk: `substrate.rs:73-93` (reads only `v.as_object()?.iter().next()` — first key only). Spec §"Facet model": one `property_key='facet'` row per resource; reader iterates all keys and expands array values into multiple `Facet` entries. Must stay backward-compatible with single-key scalar rows (the onboarding seed shape).

**Files:**
- Modify: `crates/temper-next/src/substrate.rs:81-93`

- [ ] **Step 1: Write failing unit test for multi-key + array expansion**

Add a pure helper `expand_facets(owner: Uuid, value: &serde_json::Value, weight: f64) -> Vec<Facet>` and
test it (no DB):
```rust
#[test]
fn expand_facets_handles_scalar_multikey_and_array() {
    let o = uuid::Uuid::nil();
    // single-key scalar (the seed shape) — unchanged behavior
    let v = serde_json::json!({ "phase": "first-week" });
    let f = expand_facets(o, &v, 1.0);
    assert_eq!(f.len(), 1);
    assert_eq!((f[0].path.as_str(), f[0].value.as_str()), ("phase", "first-week"));
    // multi-key
    let v = serde_json::json!({ "phase": "first-week", "topic": "deployment" });
    assert_eq!(expand_facets(o, &v, 1.0).len(), 2);
    // array value expands per element, sharing row weight
    let v = serde_json::json!({ "topic": ["deployment", "release"] });
    let f = expand_facets(o, &v, 1.5);
    assert_eq!(f.len(), 2);
    assert!(f.iter().all(|x| x.path == "topic" && x.weight == 1.5));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p temper-next expand_facets`
Expected: FAIL — `expand_facets` not defined.

- [ ] **Step 3: Implement `expand_facets` and rewire the reader**

Add the helper (module-private) to `substrate.rs`:
```rust
fn expand_facets(owner: Uuid, value: &serde_json::Value, weight: f64) -> Vec<Facet> {
    let Some(obj) = value.as_object() else { return Vec::new() };
    let mut out = Vec::new();
    for (path, v) in obj {
        match v {
            serde_json::Value::String(s) => out.push(Facet {
                owner, path: path.clone(), value: s.clone(), weight,
            }),
            serde_json::Value::Array(items) => {
                for item in items {
                    if let Some(s) = item.as_str() {
                        out.push(Facet { owner, path: path.clone(), value: s.to_string(), weight });
                    }
                }
            }
            _ => {} // non-string scalars not used by M1's affinity model
        }
    }
    out
}
```
Replace the facet collection at `substrate.rs:81-93` (the `filter_map` that does `iter().next()`) with:
```rust
    let facets = facet_rows
        .iter()
        .flat_map(|r| {
            let v: serde_json::Value = r.get("property_value");
            let weight: f64 = r.get("weight");
            expand_facets(r.get("owner_id"), &v, weight)
        })
        .collect();
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p temper-next expand_facets`
Expected: PASS.

- [ ] **Step 5: Commit**
```bash
git add crates/temper-next/src/substrate.rs
git commit -m "fix(temper-next): facet reader supports multi-key + array values (one facet row per resource)"
```

---

## Task 4: The scenario model (`model.rs`)

**Tag:** EXTEND. Spec §"Rust struct model" + §"The YAML DSL". Reuses `affinity::EdgeKind` (now `Deserialize`).

**Files:**
- Modify: `crates/temper-next/src/scenario/model.rs`

- [ ] **Step 1: Write failing test that deserializes a minimal scenario**

Add a `#[cfg(test)] mod tests` to `model.rs`:
```rust
#[test]
fn deserializes_minimal_scenario_with_steps() {
    let yaml = r#"
name: t
cogmap:
  telos: { title: T, statement: S, questions: [q1] }
  owner: alice
  emitter: agent#1
world:
  profiles: [{ handle: alice, display_name: Alice, system_access: approved }]
  entities: [{ name: agent#1, profile: alice }]
resources:
  - { key: a, origin_uri: "temper://c/a", home: cogmap, body: "hello", facets: { values: { phase: x } } }
  - { key: b, origin_uri: "temper://c/b", home: cogmap, body: "world" }
edges:
  - { from: a, to: b, kind: leads_to, weight: 1.0 }
lenses:
  - { name: L, w_express: 1.0, w_contains: 1.0, w_leads_to: 0.6, w_near: 0.3, w_prop: 0.4, s_telos: 0.5, s_ref: 0.3, s_central: 0.2, resolution: 0.5 }
steps:
  - materialize: { lens: L }
  - assert:
    - { region_count: { lens: L, op: ">=", value: 1 } }
    - { co_region: { lens: L, members: [a, b], expect: true } }
  - emit_event:
      type: relationship_asserted
      edges: [{ from: b, to: a, kind: express, label: related }]
  - assert: [{ stale: { expect: true } }]
"#;
    let s: super::Scenario = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(s.resources.len(), 2);
    assert_eq!(s.steps.len(), 4);
    // facet sugar / explicit both deserialize; resource b has no facets
    assert!(s.resources[1].facets.is_none());
}

#[test]
fn rejects_unknown_edge_kind() {
    let yaml = "from: a\nto: b\nkind: sideways\nweight: 1.0\n";
    assert!(serde_yaml::from_str::<super::EdgeDef>(yaml).is_err());
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p temper-next deserializes_minimal_scenario`
Expected: FAIL — types not defined.

- [ ] **Step 3: Implement the model**

Write `crates/temper-next/src/scenario/model.rs`:
```rust
use crate::affinity::EdgeKind;
use serde::Deserialize;

fn one() -> f64 { 1.0 }

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct Scenario {
    pub name: String,
    pub cogmap: CogmapDef,
    pub world: WorldDef,
    pub resources: Vec<ResourceDef>,
    #[serde(default)]
    pub edges: Vec<EdgeDef>,
    pub lenses: Vec<LensDef>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct CogmapDef {
    pub telos: TelosDef,
    pub owner: String,    // profile handle (in world.profiles)
    pub emitter: String,  // entity name (in world.entities)
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct TelosDef {
    pub title: String,
    pub statement: String,
    #[serde(default)]
    pub questions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct WorldDef {
    pub profiles: Vec<ProfileDef>,
    pub entities: Vec<EntityDef>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ProfileDef {
    pub handle: String,
    pub display_name: String,
    pub system_access: String, // 'none' | 'approved' | 'admin'
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct EntityDef {
    pub name: String,
    pub profile: String, // profile handle
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ResourceDef {
    pub key: String,
    #[serde(default)]
    pub title: Option<String>,
    pub origin_uri: String,
    #[serde(default = "home_cogmap")]
    pub home: String, // "cogmap" for M1
    #[serde(default)]
    pub doc_type: Option<String>,
    pub body: String,
    #[serde(default)]
    pub facets: Option<FacetDef>,
}
fn home_cogmap() -> String { "cogmap".into() }

/// One `property_key='facet'` row per resource. `values` is the coherent multi-key JSONB object
/// (scalar or array values); `weight` applies to every (path,value) pair it expands to.
/// A bare map is sugar: `facets: { phase: x }` == `{ values: { phase: x }, weight: 1.0 }`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum FacetDef {
    Explicit { values: serde_json::Map<String, serde_json::Value>, #[serde(default = "one")] weight: f64 },
    Bare(serde_json::Map<String, serde_json::Value>),
}
impl FacetDef {
    pub fn values(&self) -> &serde_json::Map<String, serde_json::Value> {
        match self { FacetDef::Explicit { values, .. } => values, FacetDef::Bare(v) => v }
    }
    pub fn weight(&self) -> f64 {
        match self { FacetDef::Explicit { weight, .. } => *weight, FacetDef::Bare(_) => 1.0 }
    }
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct EdgeDef {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default = "one")]
    pub weight: f64,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct LensDef {
    pub name: String,
    pub w_express: f64, pub w_contains: f64, pub w_leads_to: f64, pub w_near: f64, pub w_prop: f64,
    pub s_telos: f64, pub s_ref: f64, pub s_central: f64, pub resolution: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum Step {
    Materialize { lens: String },
    EmitEvent { #[serde(rename = "type")] event_type: String, edges: Vec<EdgeDef> },
    Assert(Vec<Expectation>),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum Expectation {
    RegionCount { lens: String, op: CmpOp, value: i64 },
    CoRegion { lens: String, members: Vec<String>, expect: bool },
    CohesionOrder { lens: String, greater: String, lesser: String },
    RegionSize { lens: String, member: String, value: i64 },
    InternalTension { lens: String, member: String, op: CmpOp, value: f64 },
    Reproducible { lens: String },
    FingerprintDiffers { lens_a: String, lens_b: String },
    Stale { expect: bool },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum CmpOp {
    #[serde(rename = ">=")] Ge,
    #[serde(rename = ">")] Gt,
    #[serde(rename = "==")] Eq,
}
impl CmpOp {
    pub fn cmp_f64(self, a: f64, b: f64) -> bool {
        match self { CmpOp::Ge => a >= b, CmpOp::Gt => a > b, CmpOp::Eq => (a - b).abs() < f64::EPSILON }
    }
}
```

⚠️ **Plan/reality note for the implementer:** `serde(untagged)` on `FacetDef` with `schemars` — verify
`schemars` 1.x handles the untagged enum cleanly when the `scenario-schema` feature is on (Task 16). If
it produces an unusable schema, fall back to a single struct `FacetDef { values, weight }` and drop the
bare-map sugar (the onboarding YAML can always use the explicit form). The sugar is a convenience, not a
requirement.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p temper-next deserializes_minimal_scenario rejects_unknown_edge_kind`
Expected: PASS both.

- [ ] **Step 5: Commit**
```bash
git add crates/temper-next/src/scenario/model.rs
git commit -m "feat(temper-next): scenario YAML model (substrate + step/expectation DSL)"
```

---

## Task 5: Explicit emitter in `materialize_cogmap`

**Tag:** AMEND. Disk: `write.rs:25-33` derives the materialization event's emitter from
`(SELECT emitter_entity_id FROM kb_events ORDER BY occurred_at DESC LIMIT 1)` — NULL on an empty log
(NOT NULL violation), arbitrary on `occurred_at` ties. Spec deferred-CR `emitter_entity_id`: pass the
emitter explicitly. Callers: `main.rs:24` (or thereabouts) and `tests/materialize.rs`.

**Files:**
- Modify: `crates/temper-next/src/write.rs:14,25-33`
- Modify: `crates/temper-next/src/main.rs`

- [ ] **Step 1: Change the signature + the INSERT**

`write.rs:14` — add an `emitter: Uuid` parameter:
```rust
pub async fn materialize_cogmap(
    pool: &PgPool,
    cogmap: Uuid,
    lens_name: &str,
    emitter: Uuid,
) -> Result<MaterializeOutcome> {
```
`write.rs:25-33` — bind the emitter instead of the subselect:
```rust
    let ev: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id) \
         SELECT (SELECT id FROM kb_event_types WHERE name='region_materialized'), \
                $2, 'kb_cogmaps', $1 RETURNING id",
    )
    .bind(cogmap)
    .bind(emitter)
    .fetch_one(&mut *tx)
    .await?;
```

- [ ] **Step 2: Update `main.rs` caller**

`main.rs` currently calls `materialize_cogmap(&pool, cogmap, &lens)`. It must now resolve an emitter and
pass it. Resolve the cogmap's genesis emitter (the entity that seeded it) — simplest robust source:
```rust
let emitter: Uuid = sqlx::query_scalar(
    "SELECT emitter_entity_id FROM kb_events \
     WHERE producing_anchor_table='kb_cogmaps' AND producing_anchor_id=$1 \
     ORDER BY occurred_at ASC LIMIT 1",
)
.bind(cogmap)
.fetch_one(&pool)
.await?;
```
Then `materialize_cogmap(&pool, cogmap, &lens, emitter).await?`.

- [ ] **Step 3: Update the existing materialize test caller**

`tests/materialize.rs` calls `materialize_cogmap` twice — add an `emitter` arg. Resolve it the same way
inside the test (or fetch any seeded entity: `SELECT id FROM kb_entities LIMIT 1`). Keep the test's
existing assertions (determinism, ≥2 regions, non-null/non-NaN readouts) unchanged.

- [ ] **Step 4: Verify build + existing tests**

Run: `cargo build -p temper-next` then (if you have the artifact + ONNX)
`cargo nextest run -p temper-next --features artifact-tests materialize`
Expected: build clean; the materialize test still passes (behavior identical — the loader path always has
a genesis event, so the resolved emitter equals the previously-derived one).

- [ ] **Step 5: Commit**
```bash
git add crates/temper-next/src/write.rs crates/temper-next/src/main.rs crates/temper-next/tests/materialize.rs
git commit -m "fix(temper-next): pass materialization emitter explicitly (no latest-event derivation)"
```

---

## Task 6: Loader — connect, world preamble, `KeyMap`

**Tag:** EXTEND. Spec §"World preamble" + §Architecture loader row. Reuses `substrate::connect`.

**Files:**
- Modify: `crates/temper-next/src/scenario/loader.rs`

- [ ] **Step 1: Define `KeyMap` + the world-seeding entry points**

Write the loader's spine in `loader.rs`:
```rust
use crate::scenario::model::*;
use anyhow::{Context, Result};
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

/// Local `key:` → resource Uuid, plus the implicit `telos` key (the cogmap charter resource)
/// and bookkeeping ids the runner needs.
pub struct Loaded {
    pub cogmap: Uuid,
    pub emitter: Uuid,
    pub keys: HashMap<String, Uuid>, // resource keys incl. "telos"
}

/// Idempotently ensure the event-type registry rows the loader/runner emit.
async fn ensure_event_types(pool: &PgPool) -> Result<()> {
    for name in ["cogmap_seeded", "relationship_asserted", "region_materialized"] {
        sqlx::query("INSERT INTO kb_event_types (name) VALUES ($1) ON CONFLICT (name) DO NOTHING")
            .bind(name).execute(pool).await?;
    }
    Ok(())
}

async fn seed_world(pool: &PgPool, world: &WorldDef) -> Result<(HashMap<String, Uuid>, HashMap<String, Uuid>)> {
    let mut profiles = HashMap::new();
    for p in &world.profiles {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name, system_access) VALUES ($1,$2,$3) RETURNING id",
        ).bind(&p.handle).bind(&p.display_name).bind(&p.system_access).fetch_one(pool).await?;
        profiles.insert(p.handle.clone(), id);
    }
    let mut entities = HashMap::new();
    for e in &world.entities {
        let pid = profiles.get(&e.profile).with_context(|| format!("entity {} references unknown profile {}", e.name, e.profile))?;
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb) RETURNING id",
        ).bind(pid).bind(&e.name).fetch_one(pool).await?;
        entities.insert(e.name.clone(), id);
    }
    Ok((profiles, entities))
}
```

⚠️ **Plan/reality note:** `system_access` is a Postgres enum in the artifact (`01_schema.sql`,
`{none, approved, admin}`). Binding a `&str` to an enum column may need a cast. If the insert errors on
type, change the VALUES to `$3::system_access` (confirm the enum's type name via
`grep -n "system_access" schema-artifact/01_schema.sql`). Same pattern applies anywhere a Rust string
binds to a PG enum column.

- [ ] **Step 2: Build check**

Run: `cargo build -p temper-next`
Expected: clean (functions unused yet — allow dead_code or wire in Task 7).

- [ ] **Step 3: Commit**
```bash
git add crates/temper-next/src/scenario/loader.rs
git commit -m "feat(temper-next): scenario loader — connect, event-type registry, world preamble"
```

---

## Task 7: Loader — cogmap genesis + implicit `telos` key

**Tag:** CONFORM. Disk: `02_functions.sql:458` `cogmap_genesis(...)` signature; it homes the telos resource
and stamps `doc_type=cogmap_charter`. Spec §"Full node set": loader seeds an implicit `telos` key.

**Files:**
- Modify: `crates/temper-next/src/scenario/loader.rs`

- [ ] **Step 1: Implement genesis + telos-key resolution**

Add to `loader.rs`:
```rust
async fn seed_cogmap(pool: &PgPool, c: &CogmapDef, owner: Uuid, emitter: Uuid) -> Result<(Uuid, Uuid)> {
    // cogmap_genesis(p_name, p_telos_title, p_telos_statement, p_questions[], p_owner_profile, p_emitter_entity, p_origin_uri DEFAULT)
    let cogmap: Uuid = sqlx::query_scalar(
        "SELECT cogmap_genesis($1,$2,$3,$4,$5,$6)",
    )
    .bind("PLACEHOLDER_NAME") // replaced below
    .bind(&c.telos.title)
    .bind(&c.telos.statement)
    .bind(&c.telos.questions)
    .bind(owner)
    .bind(emitter)
    .fetch_one(pool).await?;
    let telos: Uuid = sqlx::query_scalar("SELECT telos_resource_id FROM kb_cogmaps WHERE id=$1")
        .bind(cogmap).fetch_one(pool).await?;
    Ok((cogmap, telos))
}
```
⚠️ The cogmap **name** comes from `Scenario.name`, not `CogmapDef`. Thread `scenario.name` into this fn
(add a `name: &str` param and bind it as `$1`). Remove the placeholder. `p_questions` binds as a Rust
`&[String]` / `&Vec<String>` → PG `text[]` (sqlx maps `Vec<String>` to `text[]`).

- [ ] **Step 2: Build check**

Run: `cargo build -p temper-next`
Expected: clean.

- [ ] **Step 3: Commit**
```bash
git add crates/temper-next/src/scenario/loader.rs
git commit -m "feat(temper-next): loader cogmap_genesis + implicit telos key"
```

---

## Task 8: Loader — resources + content blocks/chunks

**Tag:** CONFORM. Disk: `03_seed.sql:255-265` (resource → home → content_block → chunk → chunk_content
pattern) and `02_functions.sql:497-518` (genesis block/chunk shape). Content hash mirrors the seed:
`md5(origin_uri)` for concept chunks.

**Files:**
- Modify: `crates/temper-next/src/scenario/loader.rs`

- [ ] **Step 1: Implement resource insertion returning keys**

```rust
async fn seed_resources(
    pool: &PgPool, cogmap: Uuid, owner: Uuid, genesis_ev: Uuid, resources: &[ResourceDef],
) -> Result<HashMap<String, Uuid>> {
    let mut keys = HashMap::new();
    for r in resources {
        let title = r.title.clone().unwrap_or_else(|| r.key.clone());
        let rid: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ($1,$2) RETURNING id",
        ).bind(&title).bind(&r.origin_uri).fetch_one(pool).await?;
        sqlx::query(
            "INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
             VALUES ($1,'kb_cogmaps',$2,$3,$3)",
        ).bind(rid).bind(cogmap).bind(owner).execute(pool).await?;
        // one content block + chunk + content (chunk_index 0, content_hash = md5(origin_uri) per seed)
        let block: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id) \
             VALUES ($1,0,$2,$2) RETURNING id",
        ).bind(rid).bind(genesis_ev).fetch_one(pool).await?;
        let chunk: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash) \
             VALUES ($1,$2,0,md5($3)) RETURNING id",
        ).bind(block).bind(rid).bind(&r.origin_uri).fetch_one(pool).await?;
        sqlx::query("INSERT INTO kb_chunk_content (chunk_id, content) VALUES ($1,$2)")
            .bind(chunk).bind(&r.body).execute(pool).await?;
        if let Some(dt) = &r.doc_type {
            sqlx::query(
                "INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, asserted_by_event_id, last_event_id) \
                 VALUES ('kb_resources',$1,'doc_type',to_jsonb($2::text),$3,$3)",
            ).bind(rid).bind(dt).bind(genesis_ev).execute(pool).await?;
        }
        keys.insert(r.key.clone(), rid);
    }
    Ok(keys)
}
```
⚠️ **Plan/reality note:** the loader needs a "genesis/assert" event id to stamp `asserted_by_event_id`/
`last_event_id`/`genesis_event_id` on the substrate rows (the seed uses `ev_assert`). `cogmap_genesis`
creates its own internal event but does not return it. Create one `relationship_asserted` event up front
(producing anchor = the cogmap) and thread its id as `genesis_ev` through resources/facets/edges — mirror
`03_seed.sql`'s single `ev_assert` used across the cast. Resolve where this event is created in Task 9's
orchestrator (`load_scenario`).

- [ ] **Step 2: Build check** — `cargo build -p temper-next`. Expected: clean.

- [ ] **Step 3: Commit**
```bash
git add crates/temper-next/src/scenario/loader.rs
git commit -m "feat(temper-next): loader resources + content blocks/chunks"
```

---

## Task 9: Loader — facets, edges, lenses + `load_scenario` orchestrator

**Tag:** CONFORM/EXTEND. Disk: facet row shape `03_seed.sql:266-267,314-315` (note the weighted variant
has the `weight` column); edges `03_seed.sql:459-474`; lenses `03_seed.sql:222-235`. Facet write is the
one-row-per-property-type model (spec §"Facet model").

**Files:**
- Modify: `crates/temper-next/src/scenario/loader.rs`

- [ ] **Step 1: Facets — one `property_key='facet'` row per resource**

```rust
async fn seed_facets(pool: &PgPool, keys: &HashMap<String, Uuid>, ev: Uuid, resources: &[ResourceDef]) -> Result<()> {
    for r in resources {
        let Some(f) = &r.facets else { continue };
        let rid = keys[&r.key];
        let value = serde_json::Value::Object(f.values().clone());
        sqlx::query(
            "INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, weight, asserted_by_event_id, last_event_id) \
             VALUES ('kb_resources',$1,'facet',$2,$3,$4,$4)",
        ).bind(rid).bind(value).bind(f.weight()).bind(ev).execute(pool).await?;
    }
    Ok(())
}
```

- [ ] **Step 2: Edges — resolve `from`/`to` via the KeyMap (incl. `telos`)**

```rust
async fn seed_edges(pool: &PgPool, cogmap: Uuid, keys: &HashMap<String, Uuid>, ev: Uuid, edges: &[EdgeDef]) -> Result<()> {
    for e in edges {
        let src = keys.get(&e.from).with_context(|| format!("edge from unknown key {}", e.from))?;
        let tgt = keys.get(&e.to).with_context(|| format!("edge to unknown key {}", e.to))?;
        let kind = edge_kind_sql(e.kind); // "express" | "contains" | "leads_to" | "near"
        sqlx::query(
            "INSERT INTO kb_edges (source_table, source_id, target_table, target_id, edge_kind, label, \
                                   home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
             VALUES ('kb_resources',$1,'kb_resources',$2,$3::edge_kind,$4,'kb_cogmaps',$5,$6,$6)",
        ).bind(src).bind(tgt).bind(kind).bind(&e.label).bind(cogmap).bind(ev).execute(pool).await?;
    }
    Ok(())
}

fn edge_kind_sql(k: crate::affinity::EdgeKind) -> &'static str {
    use crate::affinity::EdgeKind::*;
    match k { Express => "express", Contains => "contains", LeadsTo => "leads_to", Near => "near" }
}
```
⚠️ Confirm the enum type name is `edge_kind` via `grep -n "edge_kind" schema-artifact/01_schema.sql`.

- [ ] **Step 3: Lenses**

```rust
async fn seed_lenses(pool: &PgPool, cogmap: Uuid, ev: Uuid, lenses: &[LensDef]) -> Result<()> {
    for l in lenses {
        sqlx::query(
            "INSERT INTO kb_cogmap_lenses \
               (cogmap_id, name, selection_kind, w_express, w_contains, w_leads_to, w_near, w_prop, s_telos, s_ref, s_central, resolution, asserted_by_event_id) \
             VALUES ($1,$2,'homed',$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
        ).bind(cogmap).bind(&l.name)
         .bind(l.w_express).bind(l.w_contains).bind(l.w_leads_to).bind(l.w_near).bind(l.w_prop)
         .bind(l.s_telos).bind(l.s_ref).bind(l.s_central).bind(l.resolution).bind(ev)
         .execute(pool).await?;
    }
    Ok(())
}
```

- [ ] **Step 4: The `load_scenario` orchestrator**

```rust
pub async fn load_scenario(pool: &PgPool, s: &Scenario) -> Result<Loaded> {
    ensure_event_types(pool).await?;
    let (profiles, entities) = seed_world(pool, &s.world).await?;
    let owner = *profiles.get(&s.cogmap.owner).context("cogmap.owner not in world.profiles")?;
    let emitter = *entities.get(&s.cogmap.emitter).context("cogmap.emitter not in world.entities")?;
    let (cogmap, telos) = seed_cogmap(pool, &s.name, &s.cogmap, owner, emitter).await?;
    // one assertion event threaded through the substrate rows (mirrors 03_seed.sql's ev_assert)
    let ev: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id, occurred_at) \
         SELECT id, $2, 'kb_cogmaps', $1, now() FROM kb_event_types WHERE name='relationship_asserted' RETURNING id",
    ).bind(cogmap).bind(emitter).fetch_one(pool).await?;
    let mut keys = seed_resources(pool, cogmap, owner, ev, &s.resources).await?;
    keys.insert("telos".into(), telos);
    seed_facets(pool, &keys, ev, &s.resources).await?;
    seed_edges(pool, cogmap, &keys, ev, &s.edges).await?;
    seed_lenses(pool, cogmap, ev, &s.lenses).await?;
    Ok(Loaded { cogmap, emitter, keys })
}
```
(Adjust `seed_cogmap`'s signature to `(pool, name, cogmap_def, owner, emitter)` per Task 7's note.)

- [ ] **Step 5: Build check** — `cargo build -p temper-next`. Expected: clean.

- [ ] **Step 6: Commit**
```bash
git add crates/temper-next/src/scenario/loader.rs
git commit -m "feat(temper-next): loader facets/edges/lenses + load_scenario orchestrator"
```

---

## Task 10: Loader integration test — substrate writes load + read back

**Tag:** EXTEND (test). Verifies the loader produces a substrate `substrate::load` can read.

**Files:**
- Create: `crates/temper-next/tests/scenario_load.rs` (gated `artifact-tests`)

- [ ] **Step 1: Write the test**

```rust
#![cfg(feature = "artifact-tests")]
use temper_next::scenario::{loader, model::Scenario};
use temper_next::substrate;

#[tokio::test]
async fn loads_a_minimal_scenario_into_readable_substrate() {
    let pool = substrate::connect().await.unwrap();
    let yaml = std::fs::read_to_string("tests/fixtures/minimal_scenario.yaml").unwrap();
    let s: Scenario = serde_yaml::from_str(&yaml).unwrap();
    let loaded = loader::load_scenario(&pool, &s).await.unwrap();
    // the telos key is present and resolves
    assert!(loaded.keys.contains_key("telos"));
    // substrate::load sees the homed nodes (telos + the scenario's resources)
    let sub = substrate::load(&pool, loaded.cogmap, "L").await.unwrap();
    assert!(sub.nodes.len() >= s.resources.len() + 1); // +1 telos
    assert!(!sub.edges.is_empty());
}
```
Create `crates/temper-next/tests/fixtures/minimal_scenario.yaml` from the Task 4 test YAML (give it a
lens named `L`, 2 resources `a`/`b`, one edge). **Run against a freshly (re)loaded `01_schema`+`02_functions`
into a clean `temper_next`** — the loader does not reset the namespace.

⚠️ **Plan/reality note:** tests sharing a DB namespace collide. Either (a) run this file's tests serially
with a fresh schema reload in a setup step, or (b) follow the existing temper-next test convention — check
how `tests/materialize.rs` isolates (it currently assumes the seed is loaded). Simplest for M1: a small
helper that `TRUNCATE`s or re-creates the `temper_next` schema before load. Confirm the pattern the other
`artifact-tests` use before inventing one.

- [ ] **Step 2: Run** (needs artifact + ONNX not required here — no embed):

Run: `cargo nextest run -p temper-next --features artifact-tests loads_a_minimal_scenario`
Expected: PASS.

- [ ] **Step 3: Commit**
```bash
git add crates/temper-next/tests/scenario_load.rs crates/temper-next/tests/fixtures/minimal_scenario.yaml
git commit -m "test(temper-next): loader writes a readable substrate (artifact-tests)"
```

---

## Task 11: Runner — lens validation, materialize step, fingerprint cache

**Tag:** EXTEND. Spec §"Runner execution semantics". Reuses `embed::embed_chunks`, `write::materialize_cogmap`.

**Files:**
- Modify: `crates/temper-next/src/scenario/runner.rs`

- [ ] **Step 1: Runner spine + fingerprint helper + materialize**

```rust
use crate::scenario::loader::{self, Loaded};
use crate::scenario::model::*;
use crate::{embed, write};
use anyhow::{bail, Context, Result};
use sqlx::{PgPool, Row};
use std::collections::HashMap;

pub async fn run_scenario(pool: &PgPool, s: &Scenario) -> Result<()> {
    let loaded = loader::load_scenario(pool, s).await?;
    validate_lens_names(s)?;
    let mut fps: HashMap<String, String> = HashMap::new(); // lens → last fingerprint
    let mut prev_fps: HashMap<String, String> = HashMap::new(); // lens → fingerprint BEFORE last materialize
    for (i, step) in s.steps.iter().enumerate() {
        match step {
            Step::Materialize { lens } => {
                embed::embed_chunks(pool).await?;
                let out = write::materialize_cogmap(pool, loaded.cogmap, lens, loaded.emitter).await
                    .with_context(|| format!("step {i}: materialize {lens}"))?;
                if let Some(old) = fps.insert(lens.clone(), out.membership_fingerprint.clone()) {
                    prev_fps.insert(lens.clone(), old);
                }
            }
            Step::EmitEvent { event_type, edges } => {
                emit_event(pool, &loaded, event_type, edges).await
                    .with_context(|| format!("step {i}: emit_event {event_type}"))?;
            }
            Step::Assert(exps) => {
                for e in exps {
                    eval_expectation(pool, &loaded, e, &fps, &prev_fps).await
                        .with_context(|| format!("step {i}: assertion failed"))?;
                }
            }
        }
    }
    Ok(())
}

fn validate_lens_names(s: &Scenario) -> Result<()> {
    let declared: std::collections::HashSet<&str> = s.lenses.iter().map(|l| l.name.as_str()).collect();
    let mut check = |name: &str| -> Result<()> {
        if !declared.contains(name) { bail!("scenario references undeclared lens {name:?}"); }
        Ok(())
    };
    for step in &s.steps {
        match step {
            Step::Materialize { lens } => check(lens)?,
            Step::Assert(exps) => for e in exps { for l in expectation_lenses(e) { check(l)?; } },
            Step::EmitEvent { .. } => {}
        }
    }
    Ok(())
}

fn expectation_lenses(e: &Expectation) -> Vec<&str> {
    match e {
        Expectation::RegionCount { lens, .. } | Expectation::CoRegion { lens, .. }
        | Expectation::CohesionOrder { lens, .. } | Expectation::RegionSize { lens, .. }
        | Expectation::InternalTension { lens, .. } | Expectation::Reproducible { lens } => vec![lens.as_str()],
        Expectation::FingerprintDiffers { lens_a, lens_b } => vec![lens_a.as_str(), lens_b.as_str()],
        Expectation::Stale { .. } => vec![],
    }
}
```

⚠️ **Reproducible semantics:** `reproducible{lens}` must compare the fingerprint of the **two most recent
materializes of that lens**. The cache above stores current in `fps` and the pre-current in `prev_fps`,
updated only on Materialize. So `reproducible` passes iff `fps[lens] == prev_fps[lens]`. This matches
`run_eval.sh` S6b (materialize → A, materialize → B, assert A==B).

- [ ] **Step 2: Build check** — `cargo build -p temper-next` (emit_event/eval_expectation stubs added next; add `todo!()` temporarily or implement in Tasks 12–13 before building). Expected: clean once 12–13 land.

- [ ] **Step 3: Commit** (after 12–13 compile; or commit a `todo!()` skeleton now)
```bash
git add crates/temper-next/src/scenario/runner.rs
git commit -m "feat(temper-next): scenario runner spine — lens validation, materialize, fp cache"
```

---

## Task 12: Runner — `emit_event` step (explicit emitter)

**Tag:** CONFORM. Disk: `run_eval.sh:97-113` (the S6h `relationship_asserted` event + express edges).
Emitter passed explicitly (Task 5 rationale).

**Files:**
- Modify: `crates/temper-next/src/scenario/runner.rs`

- [ ] **Step 1: Implement `emit_event`**

```rust
async fn emit_event(pool: &PgPool, loaded: &Loaded, event_type: &str, edges: &[EdgeDef]) -> Result<()> {
    let ev: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id, occurred_at) \
         SELECT id, $2, 'kb_cogmaps', $1, now() FROM kb_event_types WHERE name=$3 RETURNING id",
    ).bind(loaded.cogmap).bind(loaded.emitter).bind(event_type).fetch_one(pool).await?;
    for e in edges {
        let src = loaded.keys.get(&e.from).with_context(|| format!("emit_event edge from unknown key {}", e.from))?;
        let tgt = loaded.keys.get(&e.to).with_context(|| format!("emit_event edge to unknown key {}", e.to))?;
        let kind = crate::scenario::loader::edge_kind_sql(e.kind); // make this fn pub(crate) in loader
        sqlx::query(
            "INSERT INTO kb_edges (source_table, source_id, target_table, target_id, edge_kind, label, \
                                   home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
             VALUES ('kb_resources',$1,'kb_resources',$2,$3::edge_kind,$4,'kb_cogmaps',$5,$6,$6)",
        ).bind(src).bind(tgt).bind(kind).bind(&e.label).bind(loaded.cogmap).bind(ev).execute(pool).await?;
    }
    Ok(())
}
```
(Promote `edge_kind_sql` in `loader.rs` to `pub(crate)`.)

- [ ] **Step 2: Build check** — `cargo build -p temper-next`. Expected: clean (once Task 13 lands).

- [ ] **Step 3: Commit**
```bash
git add crates/temper-next/src/scenario/runner.rs crates/temper-next/src/scenario/loader.rs
git commit -m "feat(temper-next): runner emit_event step (mutation events + edges)"
```

---

## Task 13: Runner — expectation evaluation

**Tag:** CONFORM. Each variant maps to an S6 query (spec §"Expectation vocabulary → S6 mapping").
Queries are keyed on `origin_uri` and the lens name, mirroring `run_eval.sh` / `04b_region_suite.sql`.

**Files:**
- Modify: `crates/temper-next/src/scenario/runner.rs`

- [ ] **Step 1: Implement `eval_expectation` + helpers**

Member resolution goes key → `origin_uri` → region. Build a reverse map (key → origin_uri) from the
scenario once, OR query the resource's `origin_uri` by the resolved id from `loaded.keys`. Simplest:
resolve key → uuid via `loaded.keys`, then query region by member id.

```rust
async fn region_of(pool: &PgPool, cogmap: uuid::Uuid, lens: &str, member_id: uuid::Uuid) -> Result<Option<uuid::Uuid>> {
    let row = sqlx::query(
        "SELECT m.region_id FROM kb_cogmap_region_members m \
         JOIN kb_cogmap_regions r ON r.id=m.region_id AND NOT r.is_folded \
         JOIN kb_cogmap_lenses l ON l.id=r.lens_id AND l.name=$2 \
         WHERE r.cogmap_id=$1 AND m.member_id=$3",
    ).bind(cogmap).bind(lens).bind(member_id).fetch_optional(pool).await?;
    Ok(row.map(|r| r.get::<uuid::Uuid, _>("region_id")))
}

async fn eval_expectation(
    pool: &PgPool, loaded: &Loaded, e: &Expectation,
    fps: &HashMap<String, String>, prev_fps: &HashMap<String, String>,
) -> Result<()> {
    let key = |k: &str| -> Result<uuid::Uuid> { loaded.keys.get(k).copied().with_context(|| format!("unknown member key {k}")) };
    match e {
        Expectation::RegionCount { lens, op, value } => {
            let n: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM kb_cogmap_regions r JOIN kb_cogmap_lenses l ON l.id=r.lens_id \
                 WHERE r.cogmap_id=$1 AND l.name=$2 AND NOT r.is_folded",
            ).bind(loaded.cogmap).bind(lens).fetch_one(pool).await?;
            if !op.cmp_f64(n as f64, *value as f64) { bail!("region_count {n} !{op:?} {value} for lens {lens}"); }
        }
        Expectation::CoRegion { lens, members, expect } => {
            let mut regions = Vec::new();
            for m in members { regions.push(region_of(pool, loaded.cogmap, lens, key(m)?).await?); }
            let all_same = regions.windows(2).all(|w| w[0].is_some() && w[0] == w[1]);
            if all_same != *expect { bail!("co_region {members:?} expected {expect}, got regions {regions:?} (lens {lens})"); }
        }
        Expectation::RegionSize { lens, member, value } => {
            let region = region_of(pool, loaded.cogmap, lens, key(member)?).await?.context("member has no region")?;
            let n: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_cogmap_region_members WHERE region_id=$1")
                .bind(region).fetch_one(pool).await?;
            if n != *value { bail!("region_size of {member} = {n}, expected {value} (lens {lens})"); }
        }
        Expectation::CohesionOrder { lens, greater, lesser } => {
            let rg = region_of(pool, loaded.cogmap, lens, key(greater)?).await?.context("greater has no region")?;
            let rl = region_of(pool, loaded.cogmap, lens, key(lesser)?).await?.context("lesser has no region")?;
            let cg: f64 = sqlx::query_scalar("SELECT content_cohesion FROM kb_cogmap_regions WHERE id=$1").bind(rg).fetch_one(pool).await?;
            let cl: f64 = sqlx::query_scalar("SELECT content_cohesion FROM kb_cogmap_regions WHERE id=$1").bind(rl).fetch_one(pool).await?;
            if !(cg > cl) { bail!("cohesion_order: {greater}({cg}) !> {lesser}({cl}) (lens {lens})"); }
        }
        Expectation::InternalTension { lens, member, op, value } => {
            let region = region_of(pool, loaded.cogmap, lens, key(member)?).await?.context("member has no region")?;
            let t: f64 = sqlx::query_scalar("SELECT internal_tension FROM kb_cogmap_regions WHERE id=$1").bind(region).fetch_one(pool).await?;
            if !op.cmp_f64(t, *value) { bail!("internal_tension of {member} = {t} !{op:?} {value} (lens {lens})"); }
        }
        Expectation::Reproducible { lens } => {
            let now = fps.get(lens).context("reproducible: lens never materialized")?;
            let before = prev_fps.get(lens).context("reproducible: lens materialized only once")?;
            if now != before { bail!("reproducible: lens {lens} fingerprints differ ({before} vs {now})"); }
        }
        Expectation::FingerprintDiffers { lens_a, lens_b } => {
            let a = fps.get(lens_a).context("fingerprint_differs: lens_a not materialized")?;
            let b = fps.get(lens_b).context("fingerprint_differs: lens_b not materialized")?;
            if a == b { bail!("fingerprint_differs: {lens_a} and {lens_b} are identical"); }
        }
        Expectation::Stale { expect } => {
            let is_stale: bool = sqlx::query_scalar("SELECT is_stale FROM cogmap_staleness($1)")
                .bind(loaded.cogmap).fetch_one(pool).await?;
            if is_stale != *expect { bail!("stale = {is_stale}, expected {expect}"); }
        }
    }
    Ok(())
}
```

⚠️ **Plan/reality note:** the runner's `membership_fingerprint` (from `MaterializeOutcome`) must equal
`run_eval.sh`'s `fp()` (md5 over `origin_uri` ordered by `r.id, origin_uri`) for `reproducible`/
`fingerprint_differs` to mean the same thing. Verify `write.rs`'s fingerprint construction matches that
ordering; if it differs, the `reproducible`/`differs` checks still work *internally* (they compare runner
fingerprints to each other), so exact equality with `run_eval.sh` is not required — but note the
divergence. Do NOT change `write.rs`'s fingerprint to match unless a test needs it.

- [ ] **Step 2: Build + run the model/unit tests** — `cargo build -p temper-next && cargo test -p temper-next`. Expected: clean + unit tests pass.

- [ ] **Step 3: Commit**
```bash
git add crates/temper-next/src/scenario/runner.rs
git commit -m "feat(temper-next): runner expectation evaluation (S6a-h vocabulary)"
```

---

## Task 14: Author `onboarding-cogmap.yaml`

**Tag:** CONFORM. Transcribe the exact cast from `03_seed.sql` — this is executable grounding: every
value must match the seed or the regions/verdict diverge.

**Files:**
- Create: `schema-artifact/scenarios/onboarding-cogmap.yaml`

- [ ] **Step 1: Author the YAML — transcribe verbatim from `03_seed.sql`**

Authoritative source values (do not paraphrase the prose — copy it byte-for-byte from the cited lines):
- **Genesis** (`03_seed.sql:183-193`): name `onboarding-cogmap`; telos title `Onboarding charter`;
  statement `Help a new EPD engineer reach first-merge confidence in week one.`; questions =
  the 3 at lines 188-190.
- **World:** one profile (owner) + one entity (emitter). The seed's owner is `p_dave`; emitter is
  `onboarding-agent#1` (`03_seed.sql:100-102`). Declare: profile `{ handle: dave, display_name: Dave, system_access: approved }`, entity `{ name: onboarding-agent#1, profile: dave }`. (The exact handle/display values don't affect regions — only owner/emitter wiring does.)
- **Resources (15 total):** `telos` is implicit (genesis). Author the other 14:
  - `regulation` — origin_uri `temper://reg/pair`, doc_type `cogmap_regulation`, body line 210, NO facet.
  - 13 concepts — keys/origin_uris/prose/facets from `03_seed.sql:255-450`:
    | key | origin_uri | facet | body lines |
    |-----|-----------|-------|-----------|
    | pair | temper://c/pair | `{ phase: first-week }` (w 1.0) | 263-265 |
    | smallest | temper://c/smallest | `{ phase: first-week }` | 278-280 |
    | confidence | temper://c/confidence | `{ phase: first-week }` | 293-295 |
    | staging | temper://c/staging | `{ topic: deployment }` weight 1.5 | 311-313 |
    | flags | temper://c/flags | `{ topic: deployment }` weight 1.5 | 326-328 |
    | rollback | temper://c/rollback | `{ topic: deployment }` weight 1.5 | 341-343 |
    | oncall | temper://c/oncall | `{ topic: deployment }` weight 1.5 | 356-358 |
    | checklist | temper://c/checklist | `{ topic: deployment }` weight 1.5 | 373-375 |
    | bluegreen | temper://c/bluegreen | `{ topic: deployment }` weight 1.5 | 390-392 |
    | bigbang | temper://c/bigbang | `{ topic: deployment }` weight 1.5 | 405-407 |
    | solo | temper://c/solo | NONE | 422-423 |
    | setup | temper://c/setup | NONE | 438-439 |
    | firstbuild | temper://c/firstbuild | NONE | 449-450 |
- **Edges (10 total):**
  - `{ from: telos, to: regulation, kind: express, label: operationalized_by }` (line 214-216)
  - `{ from: setup, to: firstbuild, kind: leads_to, label: then }` (line 454)
  - the 8 from `03_seed.sql:464-471`: pair→smallest (near), pair→confidence (near), smallest→pair (near),
    confidence→pair (express), staging→flags (leads_to), flags→rollback (leads_to), rollback→oncall (leads_to),
    bluegreen→bigbang (near, label contradicts). All weight 1.0 (default).
- **Lenses (2):** EXACT values from `03_seed.sql:225,234` — telos-default
  (`w_express 1.0, w_contains 1.0, w_leads_to 0.6, w_near 0.3, w_prop 0.4, s_telos 0.5, s_ref 0.3, s_central 0.2, resolution 0.5`)
  and telos-default-propheavy (`… w_leads_to 0.1, w_prop 1.2 …`, rest identical).
- **Steps:** the full S6a–h runbook from the spec §"The YAML DSL" `steps:` block (already grounded in `run_eval.sh`).

⚠️ Facet weight: the `{ phase: first-week }` facets have **no** weight in the seed (default 1.0) — use
`facets: { values: { phase: first-week } }`. The `{ topic: deployment }` facets are **weight 1.5** —
use `facets: { values: { topic: deployment }, weight: 1.5 }`.

- [ ] **Step 2: Validate it deserializes** (pure, no DB)

Add a unit test in `model.rs` (or a small `tests/onboarding_parses.rs`, ungated):
```rust
#[test]
fn onboarding_yaml_parses() {
    let y = std::fs::read_to_string("../../schema-artifact/scenarios/onboarding-cogmap.yaml").unwrap();
    let s: temper_next::scenario::model::Scenario = serde_yaml::from_str(&y).unwrap();
    assert_eq!(s.resources.len(), 14); // telos is implicit
    assert_eq!(s.edges.len(), 10);
    assert_eq!(s.lenses.len(), 2);
}
```
⚠️ Confirm the relative path from the test's CWD (cargo runs tests with CWD = crate dir
`crates/temper-next`), so `../../schema-artifact/...`. Adjust if needed.

Run: `cargo test -p temper-next onboarding_yaml_parses`
Expected: PASS.

- [ ] **Step 3: Commit**
```bash
git add schema-artifact/scenarios/onboarding-cogmap.yaml crates/temper-next/
git commit -m "feat(scenarios): onboarding-cogmap.yaml — full 15-node cast transcribed from 03_seed.sql"
```

---

## Task 15: Integration test — roundtrip + 04b verdict cross-check (nextest-native)

**Tag:** EXTEND (test). Spec §"Testing & equivalence proof". The acceptance gate.

**Files:**
- Create: `crates/temper-next/tests/scenario_roundtrip.rs` (gated `artifact-tests`)

- [ ] **Step 1: Write the roundtrip test**

```rust
#![cfg(feature = "artifact-tests")]
use temper_next::scenario::{model::Scenario, runner};
use temper_next::substrate;

#[tokio::test]
async fn onboarding_scenario_roundtrips_to_s6_verdict() {
    // Precondition: temper_next has 01_schema + 02_functions loaded and is otherwise empty
    // (reset helper — mirror the isolation pattern the other artifact-tests use).
    let pool = substrate::connect().await.unwrap();
    let yaml = std::fs::read_to_string("../../schema-artifact/scenarios/onboarding-cogmap.yaml").unwrap();
    let s: Scenario = serde_yaml::from_str(&yaml).unwrap();

    // 1) the declarative asserts (S6a-h) — run_scenario returns Err on any failed expectation
    runner::run_scenario(&pool, &s).await.expect("declarative S6a-h asserts pass");

    // 2) independent cross-check: the 04b verdict logic, evaluated via sqlx (no psql/bash).
    //    Same checks, different encoding (SQL aggregate over origin_uri).
    let all_pass: bool = sqlx::query_scalar(VERDICT_SQL).fetch_one(&pool).await.unwrap();
    assert!(all_pass, "04b onboarding_s6_verdict all_pass must be true");
}

// The WITH td … SELECT … all_pass body of 04b_region_suite.sql, inlined as one query.
// Transcribe verbatim from schema-artifact/04b_region_suite.sql (the `v AS (...) SELECT ... all_pass`).
const VERDICT_SQL: &str = r#"
WITH td AS (
  SELECT res.origin_uri, m.region_id
  FROM kb_cogmap_region_members m
  JOIN kb_cogmap_regions r ON r.id = m.region_id AND NOT r.is_folded
  JOIN kb_cogmap_lenses  l ON l.id = r.lens_id AND l.name = 'telos-default'
  JOIN kb_resources    res ON res.id = m.member_id
)
SELECT (
  (SELECT count(*) FROM kb_cogmap_regions r JOIN kb_cogmap_lenses l ON l.id=r.lens_id
     WHERE l.name='telos-default' AND NOT r.is_folded) >= 2
  AND (SELECT a.region_id = b.region_id FROM td a, td b
         WHERE a.origin_uri='temper://c/pair' AND b.origin_uri='temper://c/smallest')
  AND (SELECT ca.content_cohesion > cb.content_cohesion FROM kb_cogmap_regions ca, kb_cogmap_regions cb
         WHERE ca.id=(SELECT region_id FROM td WHERE origin_uri='temper://c/pair')
           AND cb.id=(SELECT region_id FROM td WHERE origin_uri='temper://c/staging'))
  AND (SELECT count(*)=1 FROM td WHERE region_id=(SELECT region_id FROM td WHERE origin_uri='temper://c/solo'))
  AND (SELECT (SELECT region_id FROM td WHERE origin_uri='temper://c/checklist')
            = (SELECT region_id FROM td WHERE origin_uri='temper://c/staging'))
  AND (SELECT (SELECT region_id FROM td WHERE origin_uri='temper://c/bluegreen')
            = (SELECT region_id FROM td WHERE origin_uri='temper://c/bigbang')
       AND (SELECT internal_tension FROM kb_cogmap_regions
              WHERE id=(SELECT region_id FROM td WHERE origin_uri='temper://c/bluegreen')) > 0)
) AS all_pass
"#;
```
⚠️ **Transcribe `VERDICT_SQL` from the real file** (`04b_region_suite.sql`, the `v AS (...)` block) rather
than trusting the sketch above — keep it byte-faithful to the committed view so the cross-check is the
genuine S6a/c/d/e/g verdict. (S6b/f/h are covered by the declarative asserts in step 1.)

- [ ] **Step 2: Run the full roundtrip** (needs artifact schema + ONNX runtime):

Setup then run:
```bash
for f in 01_schema 02_functions; do psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f schema-artifact/$f.sql; done
cargo nextest run -p temper-next --features artifact-tests onboarding_scenario_roundtrips
```
Expected: PASS — both the declarative asserts and `all_pass = true`.

- [ ] **Step 3: Cross-validate against `run_eval.sh`** (manual confidence check during transition):

Run `schema-artifact/run_eval.sh` (SQL-seed path) and confirm it still prints ALL S6 PASS. The two seed
paths (SQL and YAML) should both pass the same verdict — that is the equivalence the spec calls for.

- [ ] **Step 4: Commit**
```bash
git add crates/temper-next/tests/scenario_roundtrip.rs
git commit -m "test(temper-next): onboarding YAML roundtrips to S6 verdict (nextest-native cross-check)"
```

---

## Task 16: Schema emission + drift snapshot

**Tag:** EXTEND. Spec §"Schema emission".

**Files:**
- Create: `crates/temper-next/tests/scenario_schema.rs` (gated `scenario-schema`)
- Create: `schema-artifact/scenarios/scenario.schema.json`

- [ ] **Step 1: Write the drift test**

```rust
#![cfg(feature = "scenario-schema")]
use temper_next::scenario::model::Scenario;

#[test]
fn scenario_json_schema_matches_snapshot() {
    let schema = schemars::schema_for!(Scenario);
    let rendered = serde_json::to_string_pretty(&schema).unwrap() + "\n";
    let path = "../../schema-artifact/scenarios/scenario.schema.json";
    if std::env::var("UPDATE_SCHEMA").is_ok() {
        std::fs::write(path, &rendered).unwrap();
    }
    let committed = std::fs::read_to_string(path).unwrap_or_default();
    assert_eq!(rendered, committed, "scenario schema drifted — re-run with UPDATE_SCHEMA=1 to refresh");
}
```

- [ ] **Step 2: Generate the snapshot**

Run: `UPDATE_SCHEMA=1 cargo test -p temper-next --features scenario-schema scenario_json_schema_matches_snapshot`
Expected: writes `scenario.schema.json`, test passes.

⚠️ If `schemars` rejects the `#[serde(untagged)]` `FacetDef` or the `CmpOp` rename, apply the Task 4
fallback (struct `FacetDef`) and regenerate.

- [ ] **Step 3: Verify drift detection**

Run (no UPDATE): `cargo test -p temper-next --features scenario-schema scenario_json_schema_matches_snapshot`
Expected: PASS against the committed snapshot.

- [ ] **Step 4: Commit**
```bash
git add crates/temper-next/tests/scenario_schema.rs schema-artifact/scenarios/scenario.schema.json
git commit -m "feat(temper-next): emit + snapshot-test scenario JSON Schema (scenario-schema feature)"
```

---

## Final verification (end-of-plan, run inline in the controller session)

- [ ] `cargo make check` (fmt + clippy + machete + TS) — clean.
- [ ] `cargo nextest run -p temper-next` (ungated unit tests) — pass.
- [ ] With artifact schema loaded + ONNX: `cargo nextest run -p temper-next --features artifact-tests` — pass (loader, roundtrip, materialize).
- [ ] `cargo test -p temper-next --features scenario-schema` — schema snapshot passes.
- [ ] `schema-artifact/run_eval.sh` still prints ALL S6 PASS (SQL-seed path unbroken).
- [ ] Update `temper-next`'s CLAUDE.md note if the `scenario-schema` feature changes the artifact-tests story (per the keep-CLAUDE.md-current rule).

## Self-review notes (spec coverage)

- Write path (direct sqlx) → Tasks 6–10. Full runbook S6a–h → Tasks 11–14 (steps), Task 15 (gate).
- References by local `key` + implicit `telos` → Tasks 7, 9, 13.
- Facet model (one row per property type, multi-key, array) → Tasks 3, 9.
- Deferred CR findings folded into M1: `parse_kind` (T2), `label_factor` (T2), multi-key facets (T3),
  explicit emitter (T5), lens-name validation (T11). Deferred-beyond-M1 (opposed-labels, format_embedding,
  affinity memo) are NOT in this plan by design.
- Schema emission + drift → Task 16. Equivalence proof (nextest-native 04b cross-check) → Task 15.
- Out of scope (access scaffold, ts-rs/OpenAPI, temper-api routing) → not present, correct.
