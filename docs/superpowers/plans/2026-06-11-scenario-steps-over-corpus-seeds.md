# Scenario steps over the corpus seeds Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the charter-corpus seeds scenario `steps` runbooks that grow and mutate a seeded cognitive map over time — the substrate workstream 5 (drift detection) will test `incremental ≡ full` against — by adding `create_resource`/`set_facet`/`assert_edge`/`fold_edge` step vocabulary, the edge-level `relationship_fold` mutation, smoke runbooks for all four charters, and growth/drift runbooks for storyteller + learning-maths.

**Architecture:** The scenario DSL already runs `materialize`/`emit_event`/`assert` steps in-process against the `temper_next` artifact namespace (`crates/temper-next/src/scenario/`). This plan restructures the `Step` enum so each mutation is its own `do:` variant mirroring the existing `SeedAction` firing surface 1:1, adds one net-new mutation (`relationship_fold`, molded on `relationship_assert`), and authors YAML runbooks over the four charter seeds. Region members are the resources homed in a cogmap, so charter-only seeds (telos only) are degenerate; runbooks supply concept material via steps (the seed/scenario split's purpose). Drift is *exhibited* here via the existing eight expectation kinds (a `co_region` flip across two materializes, a `region_count` delta); the incremental-clustering algorithm and the equivalence check are workstream 5, **not** this plan.

**Tech Stack:** Rust (`temper-next` crate), PostgreSQL plpgsql (`schema-artifact/02_functions.sql`), serde_yaml, sqlx offline macros (`temper_next` namespace), cargo-nextest (`temper-next-write` serialized group), ONNX/bge-768 embeddings (`artifact-tests` feature).

---

## Spec deviation (recorded)

The approved spec (`docs/superpowers/specs/2026-06-11-scenario-steps-over-corpus-seeds-design.md`) originally described D2 smoke runbooks as operating "over the telos blocks alone." Grounding during planning showed region members are *resources homed in the cogmap* (`substrate::load` → `kb_resource_homes`, telos included), so a charter-only seed materializes to a degenerate one-region partition and the smoke assertions would be vacuous. The spec's D2 paragraph was corrected: smoke runbooks now seed 3–4 concept resources via `create_resource` steps for a non-degenerate shape. The seed stays charter-only; the scenario supplies material. No change to deliverable shape or scope.

---

## File structure

| File | Responsibility | Change |
|---|---|---|
| `crates/temper-next/src/scenario/model.rs` | YAML `Step` enum + expectations | Restructure `Step` (Task 1) |
| `crates/temper-next/src/events.rs` | `EventKind`, `SeedAction`, `fire` | Add `RelationshipFolded` + `RelationshipFold` arm (Task 2) |
| `schema-artifact/02_functions.sql` | atomic SQL mutations | Add `relationship_fold` + `_project_relationship_folded` (Task 3) |
| `crates/temper-next/src/scenario/loader.rs` | seed instantiation | Add `owner` to `Loaded` (Task 4) |
| `crates/temper-next/src/scenario/runner.rs` | runbook execution | New step arms + fold edge-resolution (Task 4) |
| `schema-artifact/scenarios/onboarding-cogmap.yaml` | existing S6 runbook | Rewrite `emit_event` → `assert_edge` (Task 1) |
| `crates/temper-next/tests/scenario_steps.rs` | D1 acceptance roundtrip | Create (Task 5) |
| `schema-artifact/scenarios/scenario.schema.json` | wire schema snapshot | Regenerate (Task 6) |
| `schema-artifact/scenarios/{charter}-smoke.yaml` | D2 smoke runbooks ×4 | Create (Tasks 7–8) |
| `crates/temper-next/tests/corpus_smoke.rs` | D2 acceptance | Create (Task 8) |
| `schema-artifact/scenarios/{storyteller,learning-maths}-growth.yaml` | D3 growth runbooks ×2 | Create (Tasks 9–10) |
| `crates/temper-next/tests/corpus_growth.rs` | D3 acceptance | Create (Tasks 9–10) |
| `.config/nextest.toml` | serialized write-group membership | Add new test binaries (Tasks 5, 8, 9) |

**Verification preamble (read once):** All `cargo make` tasks force `SQLX_OFFLINE=true`. After ANY change to `schema-artifact/*.sql` or temper-next SQL, regenerate the per-crate cache with `cargo make prepare-next` before `cargo make check`. The artifact tests need ONNX + a live Docker Postgres on port 5437 (`cargo make docker-up`); they run only under `--features artifact-tests` and are serialized by the `temper-next-write` nextest group.

---

# Deliverable 1 — Step vocabulary + the `relationship_fold` mutation

> **D1 execution order (compile-coherent units).** The tasks below are written topically, but they
> must be *implemented* in compile-coherent bundles so each commit leaves a green tree:
> 1. **Tasks 2 + 3 together** (events.rs additions + the `relationship_fold` SQL + `prepare-next`).
>    Purely additive — the tree stays green. Verify with the events unit test + `cargo make check`.
> 2. **Tasks 1 + 4 together** (the `Step` restructure + the runner/loader update + onboarding YAML).
>    Removing `EmitEvent` breaks `runner.rs` until Task 4 lands, so they are one unit; this is why
>    Task 1's "run the model tests" step only passes once Task 4 is in the same commit/bundle. Verify
>    with the model unit tests + `cargo make check`.
> 3. **Task 5** (D1 acceptance, needs a live DB + ONNX).
> 4. **Task 6** (schema snapshot regen).
>
> Dispatch one implementer per bundle, in this order.

## Task 1: Restructure the `Step` enum

**Files:**
- Modify: `crates/temper-next/src/scenario/model.rs:247-262` (the `Step` enum) and `:343-353` (the `STEPS_YAML` test fixture)
- Modify: `schema-artifact/scenarios/onboarding-cogmap.yaml:30-35` (the one `emit_event` step)

- [ ] **Step 1: Replace the `Step` enum**

Replace the current enum (lines 247-262):

```rust
/// Internally tagged by `do:` — serde_yaml 0.9 rejects the externally-tagged single-key-map form
/// (it wants `!Variant` tags), so the runbook discriminates on a `do` field. Each mutation variant
/// mirrors a `SeedAction` (events.rs) 1:1; `materialize`/`assert` drive + check the projection.
#[derive(Debug, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum Step {
    /// Add a concept resource to the map mid-runbook (one content-block body, optional facets). Its
    /// `key` joins the runner's key map for later steps. Home is always the scenario cogmap.
    CreateResource {
        key: String,
        #[serde(default)]
        title: Option<String>,
        origin_uri: String,
        #[serde(default)]
        doc_type: Option<String>,
        body: String,
        #[serde(default)]
        facets: Option<FacetDef>,
    },
    /// Set a resource's `facet` property (re-facet / re-weight). `resource` is a key.
    SetFacet {
        resource: String,
        values: serde_json::Map<String, serde_json::Value>,
        #[serde(default = "one")]
        weight: f64,
    },
    /// Assert a typed edge between two keyed resources (replaces the old special-cased `emit_event`).
    AssertEdge {
        from: String,
        to: String,
        kind: EdgeKind,
        #[serde(default)]
        label: Option<String>,
        #[serde(default = "one")]
        weight: f64,
    },
    /// Fold the live edge at `{from,to,kind}` coordinates (retire a relationship). The runner resolves
    /// the non-folded edge to its id, then fires `relationship_fold`.
    FoldEdge {
        from: String,
        to: String,
        kind: EdgeKind,
        #[serde(default)]
        reason: Option<String>,
    },
    Materialize {
        lens: String,
    },
    Assert {
        checks: Vec<Expectation>,
    },
}
```

- [ ] **Step 2: Update the `STEPS_YAML` test fixture**

In `model.rs`, replace the `STEPS_YAML` const (lines 343-353) so it exercises the new variants:

```rust
    const STEPS_YAML: &str = r#"
steps:
  - { do: create_resource, key: c, origin_uri: "temper://c/c", body: "a third concept" }
  - { do: set_facet, resource: c, values: { phase: x }, weight: 1.5 }
  - { do: materialize, lens: L }
  - do: assert
    checks:
      - { check: co_region, lens: L, members: [a, b], expect: true }
  - { do: assert_edge, from: b, to: a, kind: express, label: related }
  - { do: fold_edge, from: a, to: b, kind: leads_to, reason: "superseded" }
  - do: assert
    checks:
      - { check: stale, expect: true }
"#;
```

The existing `scenario_embeds_a_seed_inline` test asserts `s.steps.len() == 4`; update that assertion to `== 7` to match the new fixture.

- [ ] **Step 3: Rewrite the `emit_event` step in the onboarding scenario**

In `schema-artifact/scenarios/onboarding-cogmap.yaml`, replace the S6h mutation block (lines 30-35):

```yaml
  - { do: assert_edge, from: solo, to: pair,       kind: express, label: related }   # S6h mutation
  - { do: assert_edge, from: solo, to: smallest,   kind: express, label: related }
  - { do: assert_edge, from: solo, to: confidence, kind: express, label: related }
```

(Three `assert_edge` steps replace the one `emit_event` with three `edges`. The comment moves to the first.)

- [ ] **Step 4: Run the model unit tests to verify they pass**

Run: `cargo nextest run -p temper-next -E 'test(scenario::model)'`
Expected: PASS — `scenario_embeds_a_seed_inline`, `scenario_references_a_seed_file`, and the facet/telos tests all green.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-next/src/scenario/model.rs schema-artifact/scenarios/onboarding-cogmap.yaml
git commit -m "feat(scenario): restructure Step enum to one do: variant per mutation"
```

---

## Task 2: `EventKind::RelationshipFolded` + the `RelationshipFold` fire arm

**Files:**
- Modify: `crates/temper-next/src/events.rs` — `EventKind` (lines 36-56), `SeedAction` (62-110), `event_type` (114-123), `fire` (172-363)

- [ ] **Step 1: Add the `RelationshipFolded` event kind**

In `events.rs`, add the variant to `EventKind` (after `RegionMaterialized`, line 42) and its name (after line 54):

```rust
    RegionMaterialized,
    RelationshipFolded,
```
```rust
            EventKind::RegionMaterialized => "region_materialized",
            EventKind::RelationshipFolded => "relationship_folded",
```

- [ ] **Step 2: Add the `RelationshipFold` action variant**

In `SeedAction` (after the `Materialize` variant, before the closing brace at line 110):

```rust
    RelationshipFold {
        edge: EdgeId,
        reason: Option<&'a str>,
        emitter: EntityId,
    },
```

And in `event_type` (after line 121):

```rust
            SeedAction::RelationshipFold { .. } => EventKind::RelationshipFolded,
```

- [ ] **Step 3: Add the fire dispatch arm**

In `fire`, add an arm after the `Materialize` arm (after line 361). `EdgeId` is already imported (line 24); `payloads::RelationshipFolded` already exists (`payloads.rs:305`):

```rust
        SeedAction::RelationshipFold {
            edge,
            reason,
            emitter,
        } => {
            let payload = payloads::RelationshipFolded {
                edge_id: edge,
                reason: reason.map(str::to_owned),
            };
            let id = sqlx::query_scalar!(
                "SELECT relationship_fold($1,$2)",
                serde_json::to_value(&payload)?,
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?
            .context("relationship_fold returned null")?;
            Ok(Fired::Relationship(EdgeId::from(id)))
        }
```

- [ ] **Step 4: Add a unit assertion for the new name mapping**

In the `events.rs` `tests` module (after the existing assertion ending line 399), add inside `event_type_maps_each_action_to_its_canonical_name`:

```rust
        assert_eq!(
            SeedAction::RelationshipFold {
                edge: crate::ids::EdgeId::from(Uuid::nil()),
                reason: None,
                emitter,
            }
            .event_type()
            .as_canonical_name(),
            "relationship_folded"
        );
```

- [ ] **Step 5: Verify it compiles and the unit test passes**

The `sqlx::query_scalar!("SELECT relationship_fold(...)")` macro needs the SQL function to exist in the offline cache, which Task 3 creates. So this step is expected to FAIL to compile until Task 3 + `cargo make prepare-next` run. Proceed to Task 3, then return:

Run (after Task 3): `cargo nextest run -p temper-next -E 'test(events::tests)'`
Expected: PASS.

- [ ] **Step 6: Commit (after Task 3 makes it compile)**

```bash
git add crates/temper-next/src/events.rs
git commit -m "feat(events): RelationshipFold action + RelationshipFolded event kind"
```

---

## Task 3: The `relationship_fold` SQL mutation

**Files:**
- Modify: `schema-artifact/02_functions.sql` (add after the `relationship_assert` block, around line 678)

- [ ] **Step 1: Add the projection + mutation functions**

Insert after the `relationship_assert` function (after line 678, before the `property_asserted` section):

```sql
-- ── relationship_folded ──────────────────────────────────────────────────────
-- Projection half: flips an edge's visibility (is_folded), reads ONLY the payload
-- (RelationshipFolded, payloads.rs). is_folded is the read gate every shape read honors.
CREATE FUNCTION _project_relationship_folded(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_edge uuid := (p_payload->>'edge_id')::uuid;
BEGIN
    UPDATE kb_edges SET is_folded = true, last_event_id = p_event WHERE id = v_edge;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'relationship_fold: edge % not found', v_edge;
    END IF;
    RETURN v_edge;
END;
$$;

-- Fold a declared edge (retire the relationship). The producing anchor is an ENVELOPE concern read
-- from the edge's own home (never payload data) — the same discipline as facet_set.
CREATE FUNCTION relationship_fold(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_edge uuid := (p_payload->>'edge_id')::uuid;
        v_home_tbl text; v_home uuid;
BEGIN
    SELECT home_anchor_table, home_anchor_id INTO v_home_tbl, v_home
        FROM kb_edges WHERE id = v_edge;
    IF v_home IS NULL THEN
        RAISE EXCEPTION 'relationship_fold: edge % not found', v_edge;
    END IF;
    v_ev := _event_append('relationship_folded', p_emitter, v_home_tbl, v_home, p_payload);
    RETURN _project_relationship_folded(v_ev, p_payload);
END;
$$;
```

- [ ] **Step 2: Regenerate the offline sqlx cache for temper-next**

Run: `cargo make docker-up && cargo make prepare-next`
Expected: regenerates `crates/temper-next/.sqlx` with the new `relationship_fold` query entry (the one in `events.rs` Task 2). No error.

- [ ] **Step 3: Verify the crate compiles and Task 2's unit test passes**

Run: `cargo nextest run -p temper-next -E 'test(events::tests)'`
Expected: PASS.

- [ ] **Step 4: Commit (SQL + the events.rs change from Task 2 together)**

```bash
git add schema-artifact/02_functions.sql crates/temper-next/.sqlx crates/temper-next/src/events.rs
git commit -m "feat(artifact): relationship_fold mutation (event-append + project)"
```

---

## Task 4: Runner support for the new steps

**Files:**
- Modify: `crates/temper-next/src/scenario/loader.rs` — `Loaded` struct (lines 20-24) + `load_seed` return (158-163)
- Modify: `crates/temper-next/src/scenario/runner.rs` — `run_scenario` (21-56), `validate_lenses` (61-96), and a new `apply_mutation` helper

- [ ] **Step 1: Add `owner` to `Loaded`**

In `loader.rs`, extend the struct (lines 20-24):

```rust
pub struct Loaded {
    pub cogmap: Uuid,
    pub emitter: Uuid,
    pub owner: Uuid,
    pub keys: HashMap<String, Uuid>,
}
```

And the return value (lines 158-162) — `owner` is the `ProfileId` already resolved at line 58:

```rust
    Ok(Loaded {
        cogmap: cogmap.uuid(),
        emitter: emitter.uuid(),
        owner: owner.uuid(),
        keys,
    })
```

- [ ] **Step 2: Make the runner's loaded state mutable and handle the new steps**

In `runner.rs::run_scenario`, change `let loaded = ...` to `let mut loaded = ...` (line 23), and replace the `for` loop body's `match step` (lines 31-53) so the mutation arms dispatch through a helper and `materialize`/`assert` keep their current logic:

```rust
    for (i, step) in s.steps.iter().enumerate() {
        match step {
            Step::Materialize { lens } => {
                embed::embed_chunks(pool).await?;
                let out = write::materialize_cogmap(pool, loaded.cogmap, lens, loaded.emitter)
                    .await
                    .with_context(|| format!("step {i}: materialize {lens}"))?;
                if let Some(prev) = current.insert(lens.clone(), out.membership_fingerprint) {
                    previous.insert(lens.clone(), prev);
                }
            }
            Step::Assert { checks } => {
                for c in checks {
                    eval_expectation(pool, &loaded, c, &current, &previous)
                        .await
                        .with_context(|| format!("step {i}: assertion failed"))?;
                }
            }
            mutation => {
                apply_mutation(pool, &mut loaded, mutation)
                    .await
                    .with_context(|| format!("step {i}: mutation failed"))?;
            }
        }
    }
```

- [ ] **Step 3: Write the `apply_mutation` helper**

Add to `runner.rs` (replacing the old `emit_event` helper at lines 113-142). It threads one transaction per mutation, mirroring `loader.rs`'s firing and reusing `crate::content::prepare_blocks` (fully qualified — no import). Imports: extend the existing `use crate::ids::{CogmapId, EntityId};` (line 10) to `use crate::ids::{CogmapId, EntityId, ProfileId, ResourceId};` (`EdgeId` is referenced fully-qualified as `crate::ids::EdgeId`, so it need not be added). `HashMap` and `Uuid` are already imported (lines 16, 18).

```rust
/// Resolve a runbook key to its resource UUID. A free function (not a closure over `loaded`) so the
/// create_resource arm can take `&mut loaded.keys` to insert without a borrow conflict.
fn lookup(keys: &HashMap<String, Uuid>, k: &str) -> Result<Uuid> {
    keys.get(k)
        .copied()
        .with_context(|| format!("mutation references unknown key {k}"))
}

/// Apply one mutation step (create_resource / set_facet / assert_edge / fold_edge) by firing the
/// matching SeedAction in its own transaction. create_resource registers the new key in `loaded.keys`.
async fn apply_mutation(pool: &PgPool, loaded: &mut Loaded, step: &Step) -> Result<()> {
    let mut tx = pool.begin().await?;
    match step {
        Step::CreateResource {
            key: rkey,
            title,
            origin_uri,
            doc_type,
            body,
            facets,
        } => {
            let display = title.clone().unwrap_or_else(|| rkey.clone());
            let blocks = crate::content::prepare_blocks(&[(None, body.as_str())])?;
            let rid = fire(
                &mut tx,
                SeedAction::ResourceCreate {
                    title: &display,
                    origin_uri,
                    home: CogmapId::from(loaded.cogmap),
                    owner: ProfileId::from(loaded.owner),
                    blocks: &blocks,
                    doc_type: doc_type.as_deref(),
                    emitter: EntityId::from(loaded.emitter),
                },
            )
            .await?
            .resource()?;
            if let Some(f) = facets {
                let values = serde_json::Value::Object(f.values().clone());
                fire(
                    &mut tx,
                    SeedAction::FacetSet {
                        resource: rid,
                        values: &values,
                        weight: f.weight(),
                        emitter: EntityId::from(loaded.emitter),
                    },
                )
                .await?;
            }
            tx.commit().await?;
            loaded.keys.insert(rkey.clone(), rid.uuid());
            return Ok(());
        }
        Step::SetFacet {
            resource,
            values,
            weight,
        } => {
            let rid = ResourceId::from(lookup(&loaded.keys, resource)?);
            let v = serde_json::Value::Object(values.clone());
            fire(
                &mut tx,
                SeedAction::FacetSet {
                    resource: rid,
                    values: &v,
                    weight: *weight,
                    emitter: EntityId::from(loaded.emitter),
                },
            )
            .await?;
        }
        Step::AssertEdge {
            from,
            to,
            kind,
            label,
            weight,
        } => {
            fire(
                &mut tx,
                SeedAction::RelationshipAssert {
                    src: ResourceId::from(lookup(&loaded.keys, from)?),
                    tgt: ResourceId::from(lookup(&loaded.keys, to)?),
                    kind: *kind,
                    label: label.as_deref(),
                    weight: *weight,
                    home: CogmapId::from(loaded.cogmap),
                    emitter: EntityId::from(loaded.emitter),
                },
            )
            .await?;
        }
        Step::FoldEdge {
            from,
            to,
            kind,
            reason,
        } => {
            let src = lookup(&loaded.keys, from)?;
            let tgt = lookup(&loaded.keys, to)?;
            // Runtime query (not a !-macro): the live-edge resolution + ambiguity guard is dynamic
            // intent (the per-crate macro-cache exception). query_scalar returns the id column directly.
            let edge_ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM kb_edges \
                 WHERE source_table='kb_resources' AND source_id=$1 \
                   AND target_table='kb_resources' AND target_id=$2 \
                   AND edge_kind=$3::edge_kind \
                   AND home_anchor_table='kb_cogmaps' AND home_anchor_id=$4 \
                   AND NOT is_folded",
            )
            .bind(src)
            .bind(tgt)
            .bind(kind.as_sql())
            .bind(loaded.cogmap)
            .fetch_all(&mut *tx)
            .await?;
            let edge_id = match edge_ids.as_slice() {
                [one] => *one,
                [] => bail!("fold_edge: no live edge {from}-[{kind:?}]->{to}"),
                _ => bail!("fold_edge: ambiguous — >1 live edge {from}-[{kind:?}]->{to}"),
            };
            fire(
                &mut tx,
                SeedAction::RelationshipFold {
                    edge: crate::ids::EdgeId::from(edge_id),
                    reason: reason.as_deref(),
                    emitter: EntityId::from(loaded.emitter),
                },
            )
            .await?;
        }
        Step::Materialize { .. } | Step::Assert { .. } => {
            unreachable!("materialize/assert handled in run_scenario")
        }
    }
    tx.commit().await?;
    Ok(())
}
```

Note: `fold_edge` uses runtime `sqlx::query_scalar(...).fetch_all` (not the `!`-macro) so it can apply the 0/1/many ambiguity guard — the per-crate macro-cache exception for dynamic intent. No new imports beyond `use crate::content;` and `use crate::ids::ProfileId;` (added in Step 3's intro) plus `HashMap` (already imported in `runner.rs:16`).

- [ ] **Step 4: Update `validate_lenses`**

The `validate_lenses` match (lines 68-81) covers `Materialize`/`Assert`/`EmitEvent`. Replace the `Step::EmitEvent { .. } => {}` arm with a catch-all so the new mutation variants (which name no lens) are ignored:

```rust
            Step::Assert { checks } => {
                for c in checks {
                    for l in expectation_lenses(c) {
                        names.insert(l);
                    }
                }
            }
            _ => {}
```

- [ ] **Step 5: Verify the crate compiles**

Run: `cargo make prepare-next && cargo make check`
Expected: clean (no clippy warnings, fmt OK). The runtime `query_scalar`/`query` in `fold_edge` is not macro-checked, so no cache entry is needed for it.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-next/src/scenario/loader.rs crates/temper-next/src/scenario/runner.rs
git commit -m "feat(scenario): runner support for create_resource/set_facet/assert_edge/fold_edge"
```

---

## Task 5: D1 acceptance — the step-vocabulary roundtrip test

**Files:**
- Create: `crates/temper-next/tests/scenario_steps.rs`
- Modify: `.config/nextest.toml` (add `scenario_steps` to the write-group filter)

- [ ] **Step 1: Add the test binary to the serialized write group**

In `.config/nextest.toml`, add `scenario_steps` to the binary-name regex (the `filter` line under `[[profile.default.overrides]]`):

```
filter = 'package(temper-next) & binary(/^bootseed$|^scenario_load$|^scenario_roundtrip$|^scenario_steps$|^content_multichunk$|^cogmap_genesis_charter$|^charter_block_roles$|^charter_yaml_roundtrip$|^ledger_envelope$|^replay_roundtrip$|^seed_load_path_equivalence$|^seed_corpus_sweep$|^access_scenario$/)'
```

- [ ] **Step 2: Write the failing acceptance test**

Create `crates/temper-next/tests/scenario_steps.rs`. It builds an inline scenario exercising every new step: create two resources, facet them, link them, materialize, assert they co-region; then fold the edge and add a third resource pulling one away, re-materialize, assert the membership changed. The fold + new edges must move at least one member, so the two materializes produce different fingerprints.

```rust
#![cfg(feature = "artifact-tests")]
//! D1 acceptance: the new step vocabulary (create_resource / set_facet / assert_edge / fold_edge)
//! drives a real mutation runbook end-to-end. A fold + a competing edge demonstrably change region
//! membership across two materializes — the substrate drift detection (WS5) will later consume.
mod common;

use temper_next::scenario::{bootseed, model::Scenario, runner};
use temper_next::substrate;

const SCENARIO: &str = r#"
name: steps-acceptance
seed:
  name: steps-seed
  cogmap:
    telos: { title: T, statement: "A small map for exercising step mutations.", questions: [{ question: "What groups?" }] }
    owner: pete
    emitter: "agent#1"
  world:
    profiles: [{ handle: pete, display_name: Pete, system_access: approved }]
    entities: [{ name: "agent#1", profile: pete }]
  resources: []
  uses_lenses: [telos-default]
steps:
  - { do: create_resource, key: alpha, origin_uri: "temper://c/alpha", body: "deployment pipeline staging and rollout cadence" }
  - { do: create_resource, key: beta,  origin_uri: "temper://c/beta",  body: "deployment pipeline staging and rollout cadence, closely related" }
  - { do: create_resource, key: gamma, origin_uri: "temper://c/gamma", body: "an unrelated note about tea brewing temperature" }
  - { do: assert_edge, from: alpha, to: beta, kind: express, label: related }
  - { do: materialize, lens: telos-default }
  - { do: assert, checks: [{ check: co_region, lens: telos-default, members: [alpha, beta], expect: true }] }
  - { do: assert, checks: [{ check: stale, expect: false }] }
  - { do: fold_edge, from: alpha, to: beta, kind: express, reason: "the bond is retired" }
  - { do: assert, checks: [{ check: stale, expect: true }] }
  - { do: assert_edge, from: alpha, to: gamma, kind: express, label: related }
  - { do: materialize, lens: telos-default }
  - { do: assert, checks: [{ check: co_region, lens: telos-default, members: [alpha, gamma], expect: true }] }
"#;

#[tokio::test]
async fn step_vocabulary_drives_a_mutation_runbook() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();
    let scenario: Scenario = serde_yaml::from_str(SCENARIO).unwrap();
    runner::run_scenario(&pool, &scenario, std::path::Path::new("."))
        .await
        .expect("inline step runbook passes its declarative asserts");

    // every fired event (incl. relationship_folded) deserializes into its typed payload struct
    temper_next::payloads::verify_ledger_roundtrip(&pool)
        .await
        .expect("ledger payload roundtrip incl. fold");
}
```

- [ ] **Step 3: Run the test to verify it passes**

Run: `cargo make docker-up && cargo nextest run -p temper-next --features artifact-tests -E 'binary(scenario_steps)'`
Expected: PASS. If the final `co_region [alpha, gamma]` fails, the body prose may not separate cleanly under bge-768 — adjust the three bodies so alpha+beta are near-duplicate and gamma is plainly unrelated (the affinity edge plus content drives grouping), and re-run. This calibration is expected and legitimate.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-next/tests/scenario_steps.rs .config/nextest.toml
git commit -m "test(scenario): D1 acceptance — step-vocabulary mutation runbook with fold"
```

---

## Task 6: Regenerate the scenario JSON-Schema snapshot

**Files:**
- Modify: `schema-artifact/scenarios/scenario.schema.json` (regenerated)

- [ ] **Step 1: Confirm the snapshot test currently fails (the schema drifted)**

Run: `cargo nextest run -p temper-next --features scenario-schema -E 'test(scenario_json_schema_matches_snapshot)'`
Expected: FAIL with "scenario schema drifted" — the restructured `Step` enum changed the derived schema.

- [ ] **Step 2: Regenerate the committed snapshot**

Run: `UPDATE_SCHEMA=1 cargo test -p temper-next --features scenario-schema scenario_json_schema_matches_snapshot`
Expected: rewrites `schema-artifact/scenarios/scenario.schema.json`.

- [ ] **Step 3: Verify the snapshot test passes**

Run: `cargo nextest run -p temper-next --features scenario-schema -E 'test(scenario_json_schema_matches_snapshot)'`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add schema-artifact/scenarios/scenario.schema.json
git commit -m "chore(scenario): regenerate scenario.schema.json after Step restructure"
```

---

# Deliverable 2 — Smoke runbooks, all four charters

## Task 7: Author the four smoke runbooks

**Files:**
- Create: `schema-artifact/scenarios/temper-convergence-smoke.yaml`, `temper-foundational-smoke.yaml`, `storyteller-smoke.yaml`, `learning-maths-smoke.yaml`

Each references its seed (`seed: ../seeds/<charter>.yaml`), seeds 3–4 concept resources via `create_resource` (domain-credible bodies drawn from the charter's own framing — NOT filler), lightly facets/links them, then asserts a non-degenerate, reproducible, lens-sensitive shape. All four seeds declare `uses_lenses: [telos-default]`; `telos-default-propheavy` is a global system lens available for the lens-sensitivity check.

- [ ] **Step 1: Write `storyteller-smoke.yaml`** (template for the other three — bodies differ per charter)

```yaml
# Smoke runbook over the storyteller charter: seed a little creative material (personas + a
# commitment), prove the map materializes into a non-degenerate, reproducible, lens-sensitive shape.
# The seed stays charter-only; this scenario supplies the material (seed/scenario split).
name: storyteller-smoke
seed: ../seeds/storyteller.yaml
steps:
  - { do: create_resource, key: narrator,    origin_uri: "temper://storyteller/narrator",    body: "The narrator persona: renders the experience of the story, holds tone and pacing, never scripts a character's move." , facets: { values: { layer: persona } } }
  - { do: create_resource, key: storykeeper, origin_uri: "temper://storyteller/storykeeper", body: "The storykeeper persona: holds narrative gravity and the situation, keeps the world's affordances live so characters have reasons to move." , facets: { values: { layer: persona } } }
  - { do: create_resource, key: gravity,     origin_uri: "temper://storyteller/narrative-gravity", body: "Commitment: narrative gravity over branching trees — pull toward situation, not a menu of scripted options." , facets: { values: { layer: commitment } } }
  - { do: create_resource, key: tea,         origin_uri: "temper://storyteller/aside",       body: "A stray production note about font licensing for the title cards." }
  - { do: assert_edge, from: narrator, to: storykeeper, kind: near, label: collaborates }
  - { do: assert_edge, from: storykeeper, to: gravity, kind: express, label: enacts }
  - { do: materialize, lens: telos-default }
  - do: assert
    checks:
      - { check: region_count, lens: telos-default, op: ">=", value: 2 }
      - { check: co_region, lens: telos-default, members: [narrator, storykeeper], expect: true }
  - { do: materialize, lens: telos-default }
  - { do: assert, checks: [{ check: reproducible, lens: telos-default }] }
  - { do: materialize, lens: telos-default-propheavy }
  - { do: assert, checks: [{ check: fingerprint_differs, lens_a: telos-default, lens_b: telos-default-propheavy }] }
```

- [ ] **Step 2: Write the other three** (`temper-convergence-smoke.yaml`, `temper-foundational-smoke.yaml`, `learning-maths-smoke.yaml`) with the same step shape but bodies/facets/edges drawn from each charter's framing:
  - **temper-convergence:** resources = an adjudicated delta ("edge taxonomy 8→4 with label remap"), a sequencing dependency, a workflow-simplicity guard, plus one unrelated aside; facet `layer: delta|guard`.
  - **temper-foundational:** resources = two landmark terms ("event-as-primary", "the substrate boundary"), a settled mode-of-working ("spec-then-build"), plus an aside; facet `layer: landmark|mode`.
  - **learning-maths:** resources = a section engagement-trace ("Seven Sketches §1 — preorders"), a stabilized concept ("adjunction"), a live correspondence ("substrate-as-presheaf, candidate"), plus an aside; facet `phase: engaged|stabilized|candidate`.

  Keep `region_count >= 2`, the two-persona/two-landmark `co_region`, `reproducible`, and `fingerprint_differs` checks in each.

- [ ] **Step 3: Commit (the test in Task 8 calibrates these)**

```bash
git add schema-artifact/scenarios/*-smoke.yaml
git commit -m "feat(corpus): smoke runbooks for all four charter seeds"
```

---

## Task 8: D2 acceptance — run the four smoke runbooks

**Files:**
- Create: `crates/temper-next/tests/corpus_smoke.rs`
- Modify: `.config/nextest.toml` (add `corpus_smoke`)

- [ ] **Step 1: Add `corpus_smoke` to the write-group filter** (same edit pattern as Task 5 Step 1, adding `|^corpus_smoke$`).

- [ ] **Step 2: Write the test (one case per charter, sharing a helper)**

```rust
#![cfg(feature = "artifact-tests")]
//! D2 acceptance: each charter's smoke runbook materializes into a non-degenerate, reproducible,
//! lens-sensitive shape — the model holds across the diverse corpus.
mod common;

use std::path::Path;
use temper_next::scenario::{bootseed, model::Scenario, runner};
use temper_next::substrate;

async fn run_smoke(file: &str) {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();
    let path = format!(
        "{}/../../schema-artifact/scenarios/{file}",
        env!("CARGO_MANIFEST_DIR")
    );
    let scenario: Scenario = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let base = Path::new(&path).parent().unwrap();
    runner::run_scenario(&pool, &scenario, base)
        .await
        .unwrap_or_else(|e| panic!("{file} smoke runbook failed: {e:#}"));
    temper_next::payloads::verify_ledger_roundtrip(&pool)
        .await
        .expect("ledger roundtrip");
}

#[tokio::test]
async fn storyteller_smoke() { run_smoke("storyteller-smoke.yaml").await }
#[tokio::test]
async fn temper_convergence_smoke() { run_smoke("temper-convergence-smoke.yaml").await }
#[tokio::test]
async fn temper_foundational_smoke() { run_smoke("temper-foundational-smoke.yaml").await }
#[tokio::test]
async fn learning_maths_smoke() { run_smoke("learning-maths-smoke.yaml").await }
```

- [ ] **Step 3: Run and calibrate**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'binary(corpus_smoke)'`
Expected: 4 PASS. If a `region_count >= 2` or `fingerprint_differs` fails for a charter, the seeded bodies are clustering as one region under bge-768 — make the "aside" resource plainly off-topic and the two same-`layer` resources mutually near (an `express`/`near` edge plus near-duplicate phrasing), then re-run. Calibration against real embeddings is expected.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-next/tests/corpus_smoke.rs .config/nextest.toml schema-artifact/scenarios/*-smoke.yaml
git commit -m "test(corpus): D2 acceptance — smoke runbooks pass across the four charters"
```

---

# Deliverable 3 — Growth/drift runbooks: storyteller + learning-maths

## Task 9: The learning-maths growth runbook

**Files:**
- Create: `schema-artifact/scenarios/learning-maths-growth.yaml`
- Create: `crates/temper-next/tests/corpus_growth.rs`
- Modify: `.config/nextest.toml` (add `corpus_growth`)

The learning-maths charter names its own inbound events — *"a section engaged, a concept that stabilized, a correspondence proposed or walked back."* The walk-back is the fold. The runbook: sections + concepts arrive and cluster; a correspondence edge is asserted, then **folded** ("walked back where it stops landing"); the affected concept regroups on re-materialize.

- [ ] **Step 1: Write `learning-maths-growth.yaml`**

```yaml
# Growth/drift runbook over the learning-maths charter. The charter's own inbound events: a section
# engaged, a concept stabilized, a correspondence proposed — or WALKED BACK (the fold). Material
# arrives in stages; a walked-back correspondence folds; the map re-materializes and a concept regroups.
name: learning-maths-growth
seed: ../seeds/learning-maths.yaml
steps:
  # Stage 1 — early sections + a stabilized concept cluster together.
  - { do: create_resource, key: preorders,  origin_uri: "temper://maths/ss1-preorders",  body: "Seven Sketches §1: preorders and monotone maps — order as the first sketch of structure." , facets: { values: { phase: engaged } } }
  - { do: create_resource, key: adjunction, origin_uri: "temper://maths/adjunction",      body: "Galois connections / adjunctions between preorders — the stabilized concept the early sections build toward." , facets: { values: { phase: stabilized } } }
  - { do: assert_edge, from: preorders, to: adjunction, kind: leads_to, label: builds-toward }
  # Stage 2 — a correspondence to the temper substrate is proposed, linked to the adjunction.
  - { do: create_resource, key: presheaf, origin_uri: "temper://maths/substrate-as-presheaf", body: "Candidate correspondence: the temper substrate read as a presheaf; translation as its cohomological obstruction. Held open, under test." , facets: { values: { phase: candidate } } }
  - { do: assert_edge, from: presheaf, to: adjunction, kind: express, label: leans-on }
  - { do: materialize, lens: telos-default }
  - { do: assert, checks: [{ check: co_region, lens: telos-default, members: [presheaf, adjunction], expect: true }] }
  # Stage 3 — the correspondence stops landing with specificity and is WALKED BACK (folded).
  - { do: fold_edge, from: presheaf, to: adjunction, kind: express, reason: "walked back — the presheaf reading stopped landing with specificity once temper-next proved the shape in SQL" }
  - { do: assert, checks: [{ check: stale, expect: true }] }
  # A differential-geometry detour arrives and pulls the walked-back concept into a new neighborhood.
  - { do: create_resource, key: manifolds, origin_uri: "temper://maths/riemannian", body: "Detour: Riemannian manifolds and local-to-global structure — does it still illuminate information topological spaces?" , facets: { values: { phase: engaged } } }
  - { do: assert_edge, from: presheaf, to: manifolds, kind: near, label: regroups-with }
  - { do: materialize, lens: telos-default }
  - do: assert
    checks:
      - { check: co_region, lens: telos-default, members: [presheaf, adjunction], expect: false }  # the fold separated them
      - { check: co_region, lens: telos-default, members: [presheaf, manifolds], expect: true }    # regrouped on the new edge
```

- [ ] **Step 2: Add `corpus_growth` to the write-group filter** (same pattern as Task 5 Step 1).

- [ ] **Step 3: Write the growth acceptance test** (reuses a `run_growth` helper analogous to `run_smoke`)

```rust
#![cfg(feature = "artifact-tests")]
//! D3 acceptance: the growth runbooks exhibit real region drift — material arrives, a relationship
//! folds, and region membership demonstrably changes across materializes. This is the substrate WS5
//! drift detection tests `incremental ≡ full` against.
mod common;

use std::path::Path;
use temper_next::scenario::{bootseed, model::Scenario, runner};
use temper_next::substrate;

async fn run_growth(file: &str) {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();
    let path = format!(
        "{}/../../schema-artifact/scenarios/{file}",
        env!("CARGO_MANIFEST_DIR")
    );
    let scenario: Scenario = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let base = Path::new(&path).parent().unwrap();
    runner::run_scenario(&pool, &scenario, base)
        .await
        .unwrap_or_else(|e| panic!("{file} growth runbook failed: {e:#}"));
    temper_next::payloads::verify_ledger_roundtrip(&pool)
        .await
        .expect("ledger roundtrip");
}

#[tokio::test]
async fn learning_maths_growth() { run_growth("learning-maths-growth.yaml").await }
#[tokio::test]
async fn storyteller_growth() { run_growth("storyteller-growth.yaml").await }
```

(The `storyteller_growth` case will fail until Task 10 creates its YAML — that is expected; this task commits with `learning_maths_growth` passing.)

- [ ] **Step 4: Run and calibrate learning-maths**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'binary(corpus_growth) & test(learning_maths)'`
Expected: PASS. Calibrate the `co_region` flips as in Task 8 if bge-768 groups differently — the fold must separate `presheaf`/`adjunction` and the new `near` edge must regroup `presheaf`/`manifolds`. Adjust bodies/edge kinds until the drift is real and the asserts hold.

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/scenarios/learning-maths-growth.yaml crates/temper-next/tests/corpus_growth.rs .config/nextest.toml
git commit -m "feat(corpus): learning-maths growth runbook — walk-back fold drives region drift"
```

---

## Task 10: The storyteller growth runbook

**Files:**
- Create: `schema-artifact/scenarios/storyteller-growth.yaml`

The storyteller charter accretes personas + constitutive commitments clustering along the tension axis it guards; a superseded commitment's edge folds as the design matures.

- [ ] **Step 1: Write `storyteller-growth.yaml`**

```yaml
# Growth/drift runbook over the storyteller charter (the corpus's non-engineering shape). Personas and
# commitments accrete and cluster along the agency tension axis; a superseded commitment's edge folds
# as the design matures, and the map re-materializes with a regrouped neighborhood.
name: storyteller-growth
seed: ../seeds/storyteller.yaml
steps:
  # Stage 1 — two personas and the commitment they enact group together.
  - { do: create_resource, key: storykeeper, origin_uri: "temper://storyteller/storykeeper", body: "The storykeeper persona: holds narrative gravity and the situation so characters have reasons to move." , facets: { values: { layer: persona } } }
  - { do: create_resource, key: gravity, origin_uri: "temper://storyteller/narrative-gravity", body: "Commitment: narrative gravity over branching trees — pull toward situation, not a menu of scripted options." , facets: { values: { layer: commitment } } }
  - { do: assert_edge, from: storykeeper, to: gravity, kind: express, label: enacts }
  # Stage 2 — an early branching-choice commitment is proposed and linked, then superseded.
  - { do: create_resource, key: branching, origin_uri: "temper://storyteller/branching-choices", body: "Early commitment: explicit branching choice-menus give the player agency. Under tension with narrative gravity." , facets: { values: { layer: commitment } } }
  - { do: assert_edge, from: branching, to: gravity, kind: near, label: contends-with }
  - { do: materialize, lens: telos-default }
  - { do: assert, checks: [{ check: co_region, lens: telos-default, members: [branching, gravity], expect: true }] }
  # Stage 3 — branching is superseded by character-as-tensor; the old edge folds.
  - { do: fold_edge, from: branching, to: gravity, kind: near, reason: "superseded — branching menus lose to character-as-tensor agency" }
  - { do: assert, checks: [{ check: stale, expect: true }] }
  - { do: create_resource, key: tensor, origin_uri: "temper://storyteller/character-as-tensor", body: "Commitment: character-as-tensor — agency as a force-over-time through the field of relationships, not a choice menu." , facets: { values: { layer: commitment } } }
  - { do: assert_edge, from: tensor, to: gravity, kind: express, label: enacts }
  - { do: assert_edge, from: tensor, to: storykeeper, kind: near, label: inhabited-by }
  - { do: materialize, lens: telos-default }
  - do: assert
    checks:
      - { check: co_region, lens: telos-default, members: [tensor, gravity], expect: true }       # the new commitment binds in
      - { check: co_region, lens: telos-default, members: [branching, gravity], expect: false }    # the folded edge let branching drift off
```

- [ ] **Step 2: Run the full growth binary (both cases now present)**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'binary(corpus_growth)'`
Expected: 2 PASS. Calibrate the storyteller `co_region` flips as needed (the fold must let `branching` drift off `gravity`; the new `express` edge must bind `tensor` to `gravity`).

- [ ] **Step 3: Commit**

```bash
git add schema-artifact/scenarios/storyteller-growth.yaml
git commit -m "feat(corpus): storyteller growth runbook — superseded-commitment fold drives drift"
```

---

## Task 11: Full-suite verification gate

- [ ] **Step 1: Run the whole temper-next artifact suite**

Run: `cargo make prepare-next && cargo nextest run -p temper-next --features artifact-tests`
Expected: all green, including the pre-existing `scenario_roundtrip` (which now exercises the rewritten `assert_edge` onboarding YAML) and the new `scenario_steps` / `corpus_smoke` / `corpus_growth` binaries.

- [ ] **Step 2: Run the pure-core + schema tests and the quality gate**

Run: `cargo nextest run -p temper-next && cargo nextest run -p temper-next --features scenario-schema && cargo make check`
Expected: all green; `cargo make check` clean (fmt, clippy `-D warnings`, machete, TS untouched).

- [ ] **Step 3: Final commit if anything was adjusted during verification**

```bash
git add -A && git commit -m "test(corpus): full temper-next artifact suite green for scenario steps"
```

---

## Self-review checklist (completed during authoring)

- **Spec coverage:** D1 (Tasks 1–6: step vocab + `relationship_fold` + restructure + schema) ✓; D2 (Tasks 7–8: four smoke runbooks) ✓; D3 (Tasks 9–10: storyteller + learning-maths growth) ✓; verification (Task 11) ✓. Out-of-scope items (block_fold, incremental clustering, migration, temper-convergence growth, new expectation kinds) are absent by construction ✓.
- **Type consistency:** `SeedAction::RelationshipFold { edge: EdgeId, reason: Option<&str>, emitter: EntityId }` matches the `fire` arm and the `apply_mutation` call; `Loaded.owner: Uuid` matches `ProfileId::from(loaded.owner)`; `EdgeKind::as_sql()` matches the `::edge_kind` bind; `payloads::RelationshipFolded { edge_id, reason }` matches the existing struct.
- **Placeholder scan:** every code/SQL/YAML step shows full content; calibration steps name the exact knob (body separation / edge kind) rather than "adjust as needed" in the abstract.
- **Known calibration risk:** the `co_region` drift assertions depend on real bge-768 grouping; Tasks 5/8/9/10 each carry an explicit calibration instruction. This is inherent to testing emergent clustering, not a plan gap.
