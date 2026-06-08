# Scenario YAML Seed/Scenario DSL — M1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Re-express the onboarding-cogmap seed + full S6a–h falsification runbook as declarative YAML, driven by **reusable event-sourced SQL functions** (the `cogmap_genesis` mold) and a thin `!`-macro Rust loader/runner in `temper-next`, roundtrip-validated to byte-identical regions + S6 verdicts as `run_eval.sh`, with JSON Schema emitted from the loader structs.

**Architecture:** Two layers. **Layer 1** — mutation mechanics as SQL functions in `02_functions.sql` (`resource_create`, `relationship_assert`, `facet_set`, `lens_create`), each emitting its event + projecting in one txn. **Layer 2** — a thin Rust loader that calls those functions with YAML inputs, a runner that drives the ordered step runbook in-process (reusing `embed_chunks` + `materialize_cogmap`), and a system **boot-seed** (event-type registry + global lenses) seeded separately from scenarios. The whole roundtrip is one `artifact-tests` nextest — no bash.

**Tech Stack:** Rust, `serde`/`serde_yaml`, `sqlx` (`query!`/`query_scalar!` macros for non-vector reusable queries; runtime `query()` retained for pgvector `::vector` queries and test targets), `schemars` (gated), bge-768 embeddings.

**Spec:** `docs/superpowers/specs/2026-06-07-scenario-yaml-seed-dsl-design.md` — read it first. Load-bearing invariant (verbatim): *"Same prose → same embeddings → byte-identical regions (by `origin_uri`) → same verdict."* Fingerprint + verdict key on `origin_uri`, not UUID — stable across seed paths.

**Grounding tags** (per `~/.claude/skills/temper/guidance/implementation-grounding.md`): CONFORM / EXTEND / AMEND per task. Quoted `file:line` excerpts are pre-grounded; verify anything un-quoted before use. ⚠️ marks a plan/reality check the implementer must resolve on disk.

**Verified schema facts (from `01_schema.sql`, do not re-derive):**
- `kb_events`: `emitter_entity_id UUID NOT NULL`; `producing_anchor_table CHECK IN ('kb_contexts','kb_cogmaps')` (NOT `kb_resources`); `CHECK ((producing_anchor_table IS NULL) = (producing_anchor_id IS NULL))` — so a **system event with no anchor sets both NULL**.
- `kb_edges`: `weight DOUBLE PRECISION NOT NULL DEFAULT 1.0`; `edge_kind edge_kind NOT NULL`.
- `kb_cogmap_lenses`: `cogmap_id UUID` nullable (NULL = global default); `selection_kind TEXT NOT NULL DEFAULT 'homed'`.
- `kb_properties`: `owner_table CHECK IN ('kb_resources','kb_cogmaps','kb_edges')`; `UNIQUE (owner_table, owner_id, property_key, property_value)`.

**Prerequisites for every macro-compiling + DB step:**
- Docker PG: `cargo make docker-up`.
- Artifact loaded into `temper_next`: `for f in 01_schema 02_functions; do psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f schema-artifact/$f.sql; done`.
- **Compile-time DB for `!` macros** — export a search-path-scoped URL so the macros resolve `temper_next` tables:
  ```bash
  export DATABASE_URL="postgresql://temper:temper@localhost:5437/temper_development?options=-csearch_path%3Dtemper_next,public"
  ```
  (Local online compile checks against the live artifact; CI compiles offline against the committed `crates/temper-next/.sqlx` — stood up in Task 12.)
- `artifact-tests` + ONNX runtime for embed-dependent integration tests.

---

## File structure

| File | Responsibility | Task |
|------|----------------|------|
| `schema-artifact/02_functions.sql` | Add `resource_create`, `relationship_assert`, `facet_set`, `lens_create` (cogmap_genesis mold) | 2–5 |
| `schema-artifact/seeds/system.yaml` | Canonical boot-seed: event-type registry + global lenses | 6 |
| `crates/temper-next/Cargo.toml` | deps + `scenario-schema` feature | 1 |
| `crates/temper-next/src/lib.rs`, `src/scenario/mod.rs` | wire modules | 1 |
| `crates/temper-next/src/scenario/model.rs` | YAML structs + `Step`/`Expectation` + `BootSeed` | 7 |
| `crates/temper-next/src/scenario/bootseed.rs` | `seed_system` (system actor + event types + global lenses) | 6 |
| `crates/temper-next/src/affinity.rs`, `src/substrate.rs` | EdgeKind deser, exhaustive parse_kind, facet reader AMEND, drop label_factor | 8 |
| `crates/temper-next/src/write.rs`, `src/main.rs` | explicit materialize emitter (AMEND) | 8 |
| `crates/temper-next/src/scenario/loader.rs` | thin `load_scenario` over the SQL functions | 10 |
| `crates/temper-next/src/scenario/runner.rs` | validate/materialize/emit/assert + fp cache | 11 |
| `crates/temper-next/.sqlx`, `Makefile.toml` | committed offline cache + `prepare-next` target | 12 |
| `schema-artifact/scenarios/onboarding-cogmap.yaml` | the onboarding scenario | 13 |
| `crates/temper-next/tests/scenario_roundtrip.rs` | roundtrip + 04b verdict cross-check | 14 |
| `schema-artifact/scenarios/scenario.schema.json`, `tests/scenario_schema.rs` | JSON Schema snapshot + drift test | 15 |

**Execution order (compile dependencies, since task numbers group by theme):** 1 (scaffold) → 2–5 (SQL functions, pure SQL, no Rust deps) → **7 (model)** → **8 (EdgeKind `Deserialize` + fixes)** → **6 (boot-seed — needs `BootSeed` from 7 and `lens_create` from 5)** → 9 (loader) → 10 (loader test) → 11 (runner) → 12 (.sqlx cache, after all macros exist) → 13 (YAML) → 14 (roundtrip) → 15 (schema). If a subagent hits an undefined-type compile error, it's an ordering signal — pull the dependency task forward.

---

## Task 1: Scaffold deps + module skeleton

**Tag:** EXTEND.

**Files:** `crates/temper-next/Cargo.toml`, `src/lib.rs`, `src/scenario/{mod,model,bootseed,loader,runner}.rs`

- [ ] **Step 1: Cargo.toml** — in `[dependencies]` add:
```toml
serde = { version = "1", features = ["derive"] }
serde_yaml = "0.9"
schemars = { version = "1", features = ["uuid1"], optional = true }
```
in `[features]` add `scenario-schema = ["schemars"]` (keep `artifact-tests`).

- [ ] **Step 2: `scenario/mod.rs`:**
```rust
pub mod bootseed;
pub mod loader;
pub mod model;
pub mod runner;
```
Create the four submodule files empty.

- [ ] **Step 3: `lib.rs`** — add `pub mod scenario;` to the existing `pub mod` list.

- [ ] **Step 4:** `cargo build -p temper-next` — clean.

- [ ] **Step 5: Commit** — `git commit -am "feat(temper-next): scaffold scenario module + serde/serde_yaml/schemars deps"`

---

## Task 2: SQL function `resource_create`

**Tag:** EXTEND (new artifact function; spec §"Layer 1"). Pattern grounded in `cogmap_genesis` (`02_functions.sql:458-540`) and the resource shape `03_seed.sql:255-267`.

**Files:** `schema-artifact/02_functions.sql`

- [ ] **Step 1: Add the function** (place near `cogmap_genesis`):
```sql
-- Reusable: create a resource, home it in a cogmap, give it one content block, optionally stamp a
-- doc_type property. Emits one `resource_created` event (the projection root). Returns the resource id.
CREATE FUNCTION resource_create(
    p_title text, p_origin_uri text, p_home_cogmap uuid, p_owner uuid,
    p_body text, p_doc_type text, p_emitter uuid
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_et uuid; v_ev uuid; v_resource uuid; v_block uuid; v_chunk uuid; v_hash text;
BEGIN
    SELECT id INTO v_et FROM kb_event_types WHERE name='resource_created';
    IF v_et IS NULL THEN RAISE EXCEPTION 'event_type resource_created not seeded'; END IF;
    INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id)
        VALUES (v_et, p_emitter, 'kb_cogmaps', p_home_cogmap) RETURNING id INTO v_ev;
    INSERT INTO kb_resources (title, origin_uri) VALUES (p_title, p_origin_uri) RETURNING id INTO v_resource;
    INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
        VALUES (v_resource, 'kb_cogmaps', p_home_cogmap, p_owner, p_owner);
    v_hash := md5(p_origin_uri);
    INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
        VALUES (v_resource, 0, v_ev, v_ev) RETURNING id INTO v_block;
    INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash)
        VALUES (v_block, v_resource, 0, v_hash) RETURNING id INTO v_chunk;
    INSERT INTO kb_chunk_content (chunk_id, content) VALUES (v_chunk, p_body);
    INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count) VALUES (v_block, v_hash, 1);
    UPDATE kb_resources SET body_hash = md5(v_hash) WHERE id = v_resource;
    IF p_doc_type IS NOT NULL THEN
        INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, asserted_by_event_id, last_event_id)
            VALUES ('kb_resources', v_resource, 'doc_type', to_jsonb(p_doc_type), v_ev, v_ev);
    END IF;
    RETURN v_resource;
END $$;
```

- [ ] **Step 2: Smoke-test it** — reload functions + call it under a seeded event type:
```bash
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f schema-artifact/02_functions.sql
psql "$DATABASE_URL" -c "SET search_path=temper_next,public; \
  INSERT INTO kb_event_types(name) VALUES('resource_created') ON CONFLICT DO NOTHING;"
```
⚠️ A full call needs a cogmap + owner profile + emitter entity — defer the live call to the loader test (Task 11). For now confirm the function **loads** without error (the `psql -f` above).
Expected: `CREATE FUNCTION`, no error.

- [ ] **Step 3: Commit** — `git commit -am "feat(artifact): resource_create SQL function (emit resource_created + project)"`

---

## Task 3: SQL function `relationship_assert`

**Tag:** EXTEND. Grounded in edges `03_seed.sql:459-474`; `kb_edges.weight DEFAULT 1.0`.

**Files:** `schema-artifact/02_functions.sql`

- [ ] **Step 1: Add the function:**
```sql
-- Reusable: assert a typed edge between two resources, homed in a cogmap. Emits `relationship_asserted`.
CREATE FUNCTION relationship_assert(
    p_src uuid, p_tgt uuid, p_kind edge_kind, p_label text, p_weight double precision,
    p_home_cogmap uuid, p_emitter uuid
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_et uuid; v_ev uuid; v_edge uuid;
BEGIN
    SELECT id INTO v_et FROM kb_event_types WHERE name='relationship_asserted';
    IF v_et IS NULL THEN RAISE EXCEPTION 'event_type relationship_asserted not seeded'; END IF;
    INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id)
        VALUES (v_et, p_emitter, 'kb_cogmaps', p_home_cogmap) RETURNING id INTO v_ev;
    INSERT INTO kb_edges (source_table, source_id, target_table, target_id, edge_kind, label, weight,
                          home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id)
        VALUES ('kb_resources', p_src, 'kb_resources', p_tgt, p_kind, p_label, p_weight,
                'kb_cogmaps', p_home_cogmap, v_ev, v_ev) RETURNING id INTO v_edge;
    RETURN v_edge;
END $$;
```

- [ ] **Step 2:** `psql -f 02_functions.sql` loads clean.
- [ ] **Step 3: Commit** — `git commit -am "feat(artifact): relationship_assert SQL function"`

---

## Task 4: SQL function `facet_set`

**Tag:** EXTEND. ⚠️ The event **must** anchor to the home cogmap — `kb_events.producing_anchor_table` CHECK forbids `kb_resources`. Emits the new `property_asserted` event. One `property_key='facet'` row per resource (spec §"Facet model").

**Files:** `schema-artifact/02_functions.sql`

- [ ] **Step 1: Add the function:**
```sql
-- Reusable: set a resource's facet property as ONE coherent kb_properties row. Emits `property_asserted`.
-- Event anchors to the resource's home cogmap (producing_anchor CHECK forbids kb_resources).
CREATE FUNCTION facet_set(p_resource uuid, p_values jsonb, p_weight double precision, p_emitter uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_et uuid; v_ev uuid; v_cogmap uuid;
BEGIN
    SELECT id INTO v_et FROM kb_event_types WHERE name='property_asserted';
    IF v_et IS NULL THEN RAISE EXCEPTION 'event_type property_asserted not seeded'; END IF;
    SELECT anchor_id INTO v_cogmap FROM kb_resource_homes
        WHERE resource_id=p_resource AND anchor_table='kb_cogmaps' LIMIT 1;
    INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id)
        VALUES (v_et, p_emitter, 'kb_cogmaps', v_cogmap) RETURNING id INTO v_ev;
    INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, weight, asserted_by_event_id, last_event_id)
        VALUES ('kb_resources', p_resource, 'facet', p_values, p_weight, v_ev, v_ev);
END $$;
```

- [ ] **Step 2:** `psql -f 02_functions.sql` loads clean.
- [ ] **Step 3: Commit** — `git commit -am "feat(artifact): facet_set SQL function (one facet property row per resource)"`

---

## Task 5: SQL function `lens_create`

**Tag:** EXTEND. Global lens ⇒ `cogmap_id IS NULL` and the `lens_created` event has **both anchor columns NULL** (system event). Grounded in lens insert `03_seed.sql:222-235`.

**Files:** `schema-artifact/02_functions.sql`

- [ ] **Step 1: Add the function:**
```sql
-- Reusable: create a lens (global when p_cogmap IS NULL). Emits `lens_created`; a global lens is a
-- system event with no producing anchor (both NULL — satisfies the both-null-or-both-set CHECK).
CREATE FUNCTION lens_create(
    p_cogmap uuid, p_name text,
    p_w_express double precision, p_w_contains double precision, p_w_leads_to double precision,
    p_w_near double precision, p_w_prop double precision,
    p_s_telos double precision, p_s_ref double precision, p_s_central double precision,
    p_resolution double precision, p_emitter uuid
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_et uuid; v_ev uuid; v_lens uuid; v_anchor_tbl text; 
BEGIN
    SELECT id INTO v_et FROM kb_event_types WHERE name='lens_created';
    IF v_et IS NULL THEN RAISE EXCEPTION 'event_type lens_created not seeded'; END IF;
    v_anchor_tbl := CASE WHEN p_cogmap IS NULL THEN NULL ELSE 'kb_cogmaps' END;
    INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id)
        VALUES (v_et, p_emitter, v_anchor_tbl, p_cogmap) RETURNING id INTO v_ev;
    INSERT INTO kb_cogmap_lenses
        (cogmap_id, name, selection_kind, w_express, w_contains, w_leads_to, w_near, w_prop,
         s_telos, s_ref, s_central, resolution, asserted_by_event_id)
    VALUES (p_cogmap, p_name, 'homed', p_w_express, p_w_contains, p_w_leads_to, p_w_near, p_w_prop,
            p_s_telos, p_s_ref, p_s_central, p_resolution, v_ev)
    RETURNING id INTO v_lens;
    RETURN v_lens;
END $$;
```
⚠️ Verify there is no `UNIQUE(cogmap_id, name)` that a re-seed would violate; if there is, make `seed_system` idempotent (Task 6) by guarding on existence. (`grep -n "kb_cogmap_lenses" schema-artifact/01_schema.sql`.)

- [ ] **Step 2:** `psql -f 02_functions.sql` loads clean.
- [ ] **Step 3: Commit** — `git commit -am "feat(artifact): lens_create SQL function (global system lenses)"`

---

## Task 6: System boot-seed (`system.yaml` + `seed_system`)

**Tag:** EXTEND. Spec §"System boot-seed". Event-type registry grounded in `03_seed.sql:30-37` (+ the two new verbs). Needs a system actor (events require a NOT NULL emitter).

**Files:** `schema-artifact/seeds/system.yaml`, `crates/temper-next/src/scenario/bootseed.rs`, (model: `BootSeed` in Task 7 — define it here if Task 7 not yet done, or stub)

- [ ] **Step 1: `system.yaml`** — the registry + the two global lenses (EXACT values from `03_seed.sql:225,234`):
```yaml
event_types:
  - resource_created
  - resource_updated
  - resource_deleted
  - relationship_asserted
  - relationship_retracted
  - relationship_retyped
  - cogmap_seeded
  - region_materialized
  - delegated_launch
  - property_asserted     # NEW: facet_set
  - lens_created          # NEW: lens_create
lenses:
  - { name: telos-default,           w_express: 1.0, w_contains: 1.0, w_leads_to: 0.6, w_near: 0.3, w_prop: 0.4, s_telos: 0.5, s_ref: 0.3, s_central: 0.2, resolution: 0.5 }
  - { name: telos-default-propheavy, w_express: 1.0, w_contains: 1.0, w_leads_to: 0.1, w_near: 0.3, w_prop: 1.2, s_telos: 0.5, s_ref: 0.3, s_central: 0.2, resolution: 0.5 }
```
⚠️ Transcribe the FULL event-type list from `03_seed.sql:30-37` (the snippet above may elide rows). The registry should match what any temper system needs.

- [ ] **Step 2: `seed_system`** — system actor, registry, global lenses. Uses `!` macros (online compile needs the artifact loaded + the search-path URL):
```rust
use crate::scenario::model::BootSeed;
use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn seed_system(pool: &PgPool) -> Result<()> {
    let raw = std::fs::read_to_string("../../schema-artifact/seeds/system.yaml")?;
    let boot: BootSeed = serde_yaml::from_str(&raw)?;
    // system actor (idempotent upsert)
    let profile: Uuid = sqlx::query_scalar!(
        "INSERT INTO kb_profiles (handle, display_name, system_access) VALUES ('system','System','admin') \
         ON CONFLICT (handle) DO UPDATE SET display_name=EXCLUDED.display_name RETURNING id"
    ).fetch_one(pool).await?;
    let emitter: Uuid = sqlx::query_scalar!(
        "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,'system',$2) \
         ON CONFLICT (name) DO UPDATE SET profile_id=EXCLUDED.profile_id RETURNING id",
        profile, serde_json::json!({})
    ).fetch_one(pool).await?;
    for et in &boot.event_types {
        sqlx::query!("INSERT INTO kb_event_types (name) VALUES ($1) ON CONFLICT (name) DO NOTHING", et)
            .execute(pool).await?;
    }
    for l in &boot.lenses {
        // global lens: cogmap_id NULL. Guard idempotency (see Task 5 ⚠️).
        let exists: Option<Uuid> = sqlx::query_scalar!(
            "SELECT id FROM kb_cogmap_lenses WHERE cogmap_id IS NULL AND name=$1", l.name
        ).fetch_optional(pool).await?;
        if exists.is_none() {
            sqlx::query_scalar!(
                "SELECT lens_create(NULL,$1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
                l.name, l.w_express, l.w_contains, l.w_leads_to, l.w_near, l.w_prop,
                l.s_telos, l.s_ref, l.s_central, l.resolution, emitter
            ).fetch_one(pool).await?;
        }
    }
    Ok(())
}
```
⚠️ Confirm `kb_profiles.handle` and `kb_entities.name` have UNIQUE constraints for the `ON CONFLICT` targets; if not, adjust to a SELECT-then-INSERT. ⚠️ `query_scalar!` infers nullability — `lens_create` returns `uuid` (non-null); `fetch_one` is fine. ⚠️ Online compile of these macros requires `DATABASE_URL` with the search-path option + artifact loaded (see Prerequisites).

- [ ] **Step 3:** `cargo build -p temper-next` (online, artifact loaded) — clean.
- [ ] **Step 4: Commit** — `git commit -am "feat(temper-next): system boot-seed (event-type registry + global lenses)"`

---

## Task 7: The scenario model (`model.rs`)

**Tag:** EXTEND. Spec §"Rust struct model". Lenses referenced by name (`uses_lenses`); `BootSeed` distinct.

**Files:** `crates/temper-next/src/scenario/model.rs`

- [ ] **Step 1: Failing test** — same as the prior plan's `deserializes_minimal_scenario_with_steps`, but the scenario YAML uses `uses_lenses: [L]` instead of a `lenses:` block, and add a `BootSeed` parse test:
```rust
#[test]
fn deserializes_scenario_and_bootseed() {
    let scenario_yaml = r#"
name: t
cogmap: { telos: { title: T, statement: S, questions: [q1] }, owner: alice, emitter: "agent#1" }
world: { profiles: [{ handle: alice, display_name: Alice, system_access: approved }], entities: [{ name: "agent#1", profile: alice }] }
resources:
  - { key: a, origin_uri: "temper://c/a", home: cogmap, body: "hello", facets: { values: { phase: x } } }
  - { key: b, origin_uri: "temper://c/b", home: cogmap, body: "world" }
edges: [{ from: a, to: b, kind: leads_to, weight: 1.0 }]
uses_lenses: [L]
steps:
  - materialize: { lens: L }
  - assert: [{ co_region: { lens: L, members: [a, b], expect: true } }]
"#;
    let s: super::Scenario = serde_yaml::from_str(scenario_yaml).unwrap();
    assert_eq!(s.uses_lenses, vec!["L".to_string()]);
    assert!(s.resources[1].facets.is_none());

    let boot: super::BootSeed = serde_yaml::from_str(
        "event_types: [resource_created, lens_created]\nlenses:\n  - { name: L, w_express: 1.0, w_contains: 1.0, w_leads_to: 0.6, w_near: 0.3, w_prop: 0.4, s_telos: 0.5, s_ref: 0.3, s_central: 0.2, resolution: 0.5 }\n").unwrap();
    assert_eq!(boot.lenses.len(), 1);
}

#[test]
fn rejects_unknown_edge_kind() {
    assert!(serde_yaml::from_str::<super::EdgeDef>("from: a\nto: b\nkind: sideways\nweight: 1.0\n").is_err());
}
```

- [ ] **Step 2:** `cargo test -p temper-next deserializes_scenario_and_bootseed` — FAIL (types undefined).

- [ ] **Step 3: Implement the model** — same as the prior plan's `model.rs` (CogmapDef/TelosDef/WorldDef/ProfileDef/EntityDef/ResourceDef/FacetDef/EdgeDef/Step/Expectation/CmpOp), with these deltas:
  - `Scenario` field `pub uses_lenses: Vec<String>` (NOT `lenses: Vec<LensDef>`).
  - Add:
    ```rust
    #[derive(Debug, Deserialize)]
    #[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
    pub struct BootSeed { pub event_types: Vec<String>, pub lenses: Vec<LensDef> }

    #[derive(Debug, Deserialize)]
    #[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
    pub struct LensDef {
        pub name: String,
        pub w_express: f64, pub w_contains: f64, pub w_leads_to: f64, pub w_near: f64, pub w_prop: f64,
        pub s_telos: f64, pub s_ref: f64, pub s_central: f64, pub resolution: f64,
    }
    ```
  - `EdgeKind` reused from `affinity` (gets `Deserialize` in Task 8 — Task 8 must land before this compiles, OR add the derive here and remove from Task 8). **Sequencing note:** do Task 8's EdgeKind-derive step before this task, or fold it in.
  - `FacetDef` as the prior plan (untagged Explicit/Bare), with the ⚠️ schemars-untagged fallback note.

- [ ] **Step 4:** `cargo test -p temper-next deserializes_scenario_and_bootseed rejects_unknown_edge_kind` — PASS.
- [ ] **Step 5: Commit** — `git commit -am "feat(temper-next): scenario + bootseed YAML model"`

---

## Task 8: Enabling fixes to existing code

**Tag:** AMEND (4 fixes, all spec deferred-CR). Disk: `affinity.rs:4-9,68-70,91-99`; `substrate.rs:73-93,127-134`; `write.rs:14,25-33`; `main.rs`.

This is the prior plan's **Tasks 2, 3, 5 combined** — execute those step sequences verbatim:
- [ ] **EdgeKind** — add `serde::Deserialize` (`rename_all="snake_case"`) + gated `JsonSchema`; test `edge_kind_deserializes_snake_case_and_rejects_unknown`.
- [ ] **parse_kind** — make exhaustive/erroring (`near` explicit, unknown → `anyhow::bail!`), update the fallible edge collect in `substrate.rs:61-70`.
- [ ] **label_factor** — remove the `-> 1.0` placeholder + its multiplication in `affinity()` + the placeholder test.
- [ ] **facet reader AMEND** — add `expand_facets(owner, &Value, weight) -> Vec<Facet>` (scalar + array expansion) and replace the `filter_map`/`iter().next()` at `substrate.rs:81-93` with a `flat_map(expand_facets…)`; test `expand_facets_handles_scalar_multikey_and_array`.
- [ ] **explicit emitter** — `materialize_cogmap(pool, cogmap, lens, emitter: Uuid)`; bind `$2` emitter in the event INSERT (`write.rs:25-33`); update `main.rs` to resolve the cogmap's **genesis/steward** emitter honestly:
  ```rust
  // the entity that seeded this cogmap (the bound steward) — a real referent, not "latest event"
  let emitter: Uuid = sqlx::query_scalar!(
      "SELECT emitter_entity_id FROM kb_events \
       WHERE producing_anchor_table='kb_cogmaps' AND producing_anchor_id=$1 \
       ORDER BY occurred_at ASC LIMIT 1", cogmap
  ).fetch_one(&pool).await?;
  ```
  and update `tests/materialize.rs` callers to pass an emitter.

⚠️ **Macro vs runtime:** convert the queries you touch here to `!` macros **except** any `::vector` query. `write.rs`'s centroid/readout UPDATEs and `embed.rs`'s `UPDATE kb_chunks SET embedding=$1::vector` stay **runtime `query()`** (the established pgvector exception — see `search_service`). The emitter SELECT and the edge/facet reads are non-vector → macros.

- [ ] **Run + commit** — `cargo test -p temper-next` (+ `--features artifact-tests materialize` if artifact loaded); `git commit -am "fix(temper-next): exhaustive edge_kind, facet multi-key, explicit emitter, drop label_factor"`

---

## Task 9: Thin loader (`load_scenario` over the SQL functions)

**Tag:** EXTEND. The loader **calls the Layer-1 functions** — no direct substrate inserts (except the tiny `world` identity rows). Returns `Loaded { cogmap, emitter, keys }` with implicit `telos`.

**Files:** `crates/temper-next/src/scenario/loader.rs`

- [ ] **Step 1: Implement** (all `query_scalar!`, non-vector):
```rust
use crate::scenario::model::*;
use anyhow::{Context, Result};
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

pub struct Loaded { pub cogmap: Uuid, pub emitter: Uuid, pub keys: HashMap<String, Uuid> }

pub(crate) fn edge_kind_sql(k: crate::affinity::EdgeKind) -> &'static str {
    use crate::affinity::EdgeKind::*;
    match k { Express => "express", Contains => "contains", LeadsTo => "leads_to", Near => "near" }
}

pub async fn load_scenario(pool: &PgPool, s: &Scenario) -> Result<Loaded> {
    // world identity rows (tiny — direct, not event-projected for M1)
    let mut profiles = HashMap::new();
    for p in &s.world.profiles {
        let id: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_profiles (handle, display_name, system_access) VALUES ($1,$2,$3) RETURNING id",
            p.handle, p.display_name, p.system_access
        ).fetch_one(pool).await?;
        profiles.insert(p.handle.clone(), id);
    }
    let mut entities = HashMap::new();
    for e in &s.world.entities {
        let pid = profiles.get(&e.profile).with_context(|| format!("entity {} → unknown profile {}", e.name, e.profile))?;
        let id: Uuid = sqlx::query_scalar!(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,$3) RETURNING id",
            pid, e.name, serde_json::json!({})
        ).fetch_one(pool).await?;
        entities.insert(e.name.clone(), id);
    }
    let owner = *profiles.get(&s.cogmap.owner).context("cogmap.owner not in world.profiles")?;
    let emitter = *entities.get(&s.cogmap.emitter).context("cogmap.emitter not in world.entities")?;

    // genesis (existing fn) → cogmap + telos
    let cogmap: Uuid = sqlx::query_scalar!(
        "SELECT cogmap_genesis($1,$2,$3,$4,$5,$6)",
        s.name, s.cogmap.telos.title, s.cogmap.telos.statement, &s.cogmap.telos.questions, owner, emitter
    ).fetch_one(pool).await?.context("cogmap_genesis returned null")?;
    let telos: Uuid = sqlx::query_scalar!("SELECT telos_resource_id FROM kb_cogmaps WHERE id=$1", cogmap)
        .fetch_one(pool).await?;

    let mut keys = HashMap::new();
    keys.insert("telos".to_string(), telos);
    for r in &s.resources {
        let title = r.title.clone().unwrap_or_else(|| r.key.clone());
        let rid: Uuid = sqlx::query_scalar!(
            "SELECT resource_create($1,$2,$3,$4,$5,$6,$7)",
            title, r.origin_uri, cogmap, owner, r.body, r.doc_type, emitter
        ).fetch_one(pool).await?.context("resource_create returned null")?;
        keys.insert(r.key.clone(), rid);
        if let Some(f) = &r.facets {
            let values = serde_json::Value::Object(f.values().clone());
            sqlx::query!("SELECT facet_set($1,$2,$3,$4)", rid, values, f.weight(), emitter)
                .execute(pool).await?;
        }
    }
    for e in &s.edges {
        let src = *keys.get(&e.from).with_context(|| format!("edge from unknown key {}", e.from))?;
        let tgt = *keys.get(&e.to).with_context(|| format!("edge to unknown key {}", e.to))?;
        sqlx::query!(
            "SELECT relationship_assert($1,$2,$3::edge_kind,$4,$5,$6,$7)",
            src, tgt, edge_kind_sql(e.kind), e.label, e.weight, cogmap, emitter
        ).execute(pool).await?;
    }
    Ok(Loaded { cogmap, emitter, keys })
}
```
⚠️ `cogmap_genesis`/`resource_create` return `uuid`; sqlx infers `Option<Uuid>` for function-call SELECTs → hence `.context("…null")?`. ⚠️ `&s.cogmap.telos.questions` binds `&Vec<String>` → `text[]`; confirm sqlx accepts the slice form in the macro (use `&s.cogmap.telos.questions[..]` if needed). ⚠️ `$3::edge_kind` cast on a `&str` — the macro needs the enum type known; if it complains, pass the kind as the PG enum via a small newtype or keep this one as runtime `query()`.

- [ ] **Step 2:** `cargo build -p temper-next` (online) — clean.
- [ ] **Step 3: Commit** — `git commit -am "feat(temper-next): thin load_scenario over resource_create/relationship_assert/facet_set"`

---

## Task 10: Loader integration test

**Tag:** EXTEND (test). Mirrors the prior plan's Task 10, but boot-seed first.

**Files:** `crates/temper-next/tests/scenario_load.rs` (`artifact-tests`), `tests/fixtures/minimal_scenario.yaml`

- [ ] **Step 1: Test** — reset `temper_next`, `seed_system`, then `load_scenario`, then read back via `substrate::load`:
```rust
#![cfg(feature = "artifact-tests")]
use temper_next::scenario::{bootseed, loader, model::Scenario};
use temper_next::substrate;

#[tokio::test]
async fn loads_minimal_scenario_into_readable_substrate() {
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();
    let s: Scenario = serde_yaml::from_str(&std::fs::read_to_string("tests/fixtures/minimal_scenario.yaml").unwrap()).unwrap();
    let loaded = loader::load_scenario(&pool, &s).await.unwrap();
    assert!(loaded.keys.contains_key("telos"));
    let sub = substrate::load(&pool, loaded.cogmap, "telos-default").await.unwrap();
    assert!(sub.nodes.len() >= s.resources.len() + 1); // +telos
    assert!(!sub.edges.is_empty());
}
```
The fixture uses `uses_lenses: [telos-default]` (a boot-seeded global lens) so `substrate::load(...,"telos-default")` resolves the global lens.
⚠️ **Isolation:** these tests share the `temper_next` namespace. Add a reset helper (drop+reload `01_schema`+`02_functions`, or `TRUNCATE` the kb_* tables) in a serialized setup. Confirm the pattern `tests/materialize.rs` relies on before inventing one — the existing artifact-tests assume a pre-loaded seed; this suite must own its setup.

- [ ] **Step 2:** `cargo nextest run -p temper-next --features artifact-tests loads_minimal_scenario` — PASS.
- [ ] **Step 3: Commit** — `git commit -am "test(temper-next): boot-seed + load_scenario produce a readable substrate"`

---

## Task 11: Runner — validate, materialize, emit, assert

**Tag:** EXTEND. Spec §"Runner execution semantics". Combine the prior plan's Tasks 11–13 (runner spine + emit_event + expectation eval), with these deltas under the new architecture:
- `emit_event` calls `relationship_assert` (the SQL function), not a raw insert:
  ```rust
  async fn emit_event(pool: &PgPool, loaded: &Loaded, _event_type: &str, edges: &[EdgeDef]) -> Result<()> {
      for e in edges {
          let src = *loaded.keys.get(&e.from).with_context(|| format!("emit edge from unknown key {}", e.from))?;
          let tgt = *loaded.keys.get(&e.to).with_context(|| format!("emit edge to unknown key {}", e.to))?;
          sqlx::query!("SELECT relationship_assert($1,$2,$3::edge_kind,$4,$5,$6,$7)",
              src, tgt, crate::scenario::loader::edge_kind_sql(e.kind), e.label, e.weight, loaded.cogmap, loaded.emitter)
              .execute(pool).await?;
      }
      Ok(())
  }
  ```
  (Each `relationship_assert` emits its own `relationship_asserted` event with the explicit emitter — the S6h mutation. The `event_type` field is informational for M1.)
- Lens validation checks against **boot-seeded + scenario** lenses: validate every lens named in `steps`/`uses_lenses` exists in `kb_cogmap_lenses` (global or this cogmap) via a query, OR against `s.uses_lenses` for the static check plus a DB existence check at first materialize.
- Expectation eval queries are all **non-vector reads → `query!`/`query_scalar!` macros** (region membership, `content_cohesion`, `internal_tension`, `cogmap_staleness`). Use the prior plan's `eval_expectation` bodies, converted from runtime `query()`+`.get()` to macros. `region_of`, `region_count`, `region_size`, `cohesion_order`, `internal_tension`, `reproducible`/`fingerprint_differs` (runner fp cache), `stale` — exactly as the prior plan, macro-ized.
- `materialize` step: `embed::embed_chunks(pool).await?` then `write::materialize_cogmap(pool, loaded.cogmap, lens, loaded.emitter).await?`; maintain `fps`/`prev_fps` per lens (reproducible compares them).

- [ ] **Step 1:** Implement `run_scenario`, `validate_lens_names`, `emit_event`, `eval_expectation` per the above + the prior plan's bodies.
- [ ] **Step 2:** `cargo build -p temper-next && cargo test -p temper-next` — clean + unit tests pass.
- [ ] **Step 3: Commit** — `git commit -am "feat(temper-next): scenario runner (validate/materialize/emit/assert, S6a-h)"`

---

## Task 12: Stand up `.sqlx` offline cache + `prepare-next`

**Tag:** EXTEND (infra). Spec §"`!`-macro prepare ritual".

**Files:** `crates/temper-next/.sqlx/` (generated), `Makefile.toml`

- [ ] **Step 1: Add the cargo-make task** to `Makefile.toml`:
```toml
[tasks.prepare-next]
description = "Regenerate temper-next's per-crate .sqlx cache against the loaded temper_next artifact"
script = '''
for f in 01_schema 02_functions; do psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f schema-artifact/$f.sql; done
DATABASE_URL="${DATABASE_URL}?options=-csearch_path%3Dtemper_next,public" cargo sqlx prepare -p temper-next
'''
```
⚠️ If `DATABASE_URL` already has query params, merge rather than append `?`. Confirm the exact cargo-make script syntax against existing tasks in `Makefile.toml`.

- [ ] **Step 2: Generate + commit the cache:**
```bash
cargo make prepare-next
git add crates/temper-next/.sqlx
```
- [ ] **Step 3: Verify offline build** — `SQLX_OFFLINE=true cargo build -p temper-next` — clean (compiles against the committed cache).
- [ ] **Step 4: Commit** — `git commit -am "build(temper-next): per-crate .sqlx offline cache + prepare-next task"`

---

## Task 13: Author `onboarding-cogmap.yaml`

**Tag:** CONFORM. Transcribe verbatim from `03_seed.sql` (executable grounding). Identical to the prior plan's Task 14 cast table, with **two changes**: (a) no `lenses:` block — use `uses_lenses: [telos-default, telos-default-propheavy]`; (b) the boot-seed must be loaded before this scenario runs.

**Files:** `schema-artifact/scenarios/onboarding-cogmap.yaml`

- [ ] **Step 1: Author the YAML** — use the cast table from the spec / prior plan:
  - Genesis (`03_seed.sql:183-193`): name `onboarding-cogmap`; telos title `Onboarding charter`; statement `Help a new EPD engineer reach first-merge confidence in week one.`; the 3 questions (lines 188-190).
  - World: profile `{ handle: dave, display_name: Dave, system_access: approved }`, entity `{ name: onboarding-agent#1, profile: dave }`.
  - 14 resources (telos implicit): `regulation` (`temper://reg/pair`, doc_type `cogmap_regulation`, body line 210, no facet) + the 13 concepts with origin_uris/prose/facets per the table below (prose copied **byte-for-byte** from the cited lines):

  | key | origin_uri | facet | body lines |
  |-----|-----------|-------|-----------|
  | pair | temper://c/pair | `{ values: { phase: first-week } }` | 263-265 |
  | smallest | temper://c/smallest | `{ values: { phase: first-week } }` | 278-280 |
  | confidence | temper://c/confidence | `{ values: { phase: first-week } }` | 293-295 |
  | staging | temper://c/staging | `{ values: { topic: deployment }, weight: 1.5 }` | 311-313 |
  | flags | temper://c/flags | `{ values: { topic: deployment }, weight: 1.5 }` | 326-328 |
  | rollback | temper://c/rollback | `{ values: { topic: deployment }, weight: 1.5 }` | 341-343 |
  | oncall | temper://c/oncall | `{ values: { topic: deployment }, weight: 1.5 }` | 356-358 |
  | checklist | temper://c/checklist | `{ values: { topic: deployment }, weight: 1.5 }` | 373-375 |
  | bluegreen | temper://c/bluegreen | `{ values: { topic: deployment }, weight: 1.5 }` | 390-392 |
  | bigbang | temper://c/bigbang | `{ values: { topic: deployment }, weight: 1.5 }` | 405-407 |
  | solo | temper://c/solo | none | 422-423 |
  | setup | temper://c/setup | none | 438-439 |
  | firstbuild | temper://c/firstbuild | none | 449-450 |

  - 10 edges: `{telos→regulation, express, operationalized_by}` (line 214-216); `{setup→firstbuild, leads_to, then}` (line 454); the 8 from lines 464-471 (pair→smallest near, pair→confidence near, smallest→pair near, confidence→pair express, staging→flags leads_to, flags→rollback leads_to, rollback→oncall leads_to, bluegreen→bigbang near label contradicts). All weight 1.0.
  - `uses_lenses: [telos-default, telos-default-propheavy]`.
  - `steps:` — the full S6a–h runbook from the spec §"The YAML DSL".

- [ ] **Step 2: Parse test** (ungated): assert `resources.len()==14`, `edges.len()==10`, `uses_lenses.len()==2`.
  Run: `cargo test -p temper-next onboarding_yaml_parses` — PASS.
- [ ] **Step 3: Commit** — `git commit -am "feat(scenarios): onboarding-cogmap.yaml (15-node cast, lenses by reference)"`

---

## Task 14: Roundtrip integration test + 04b verdict cross-check

**Tag:** EXTEND (test). The acceptance gate. Same as the prior plan's Task 15, with `seed_system` first.

**Files:** `crates/temper-next/tests/scenario_roundtrip.rs` (`artifact-tests`)

- [ ] **Step 1: Test:**
```rust
#![cfg(feature = "artifact-tests")]
use temper_next::scenario::{bootseed, model::Scenario, runner};
use temper_next::substrate;

#[tokio::test]
async fn onboarding_scenario_roundtrips_to_s6_verdict() {
    let pool = substrate::connect().await.unwrap();
    // reset temper_next, then boot-seed (event types + global lenses), then run.
    bootseed::seed_system(&pool).await.unwrap();
    let s: Scenario = serde_yaml::from_str(
        &std::fs::read_to_string("../../schema-artifact/scenarios/onboarding-cogmap.yaml").unwrap()).unwrap();
    runner::run_scenario(&pool, &s).await.expect("declarative S6a-h asserts pass");
    let all_pass: bool = sqlx::query_scalar(VERDICT_SQL).fetch_one(&pool).await.unwrap();
    assert!(all_pass, "04b onboarding_s6_verdict all_pass must be true");
}
const VERDICT_SQL: &str = r#"…"#; // transcribe the `v AS (...) SELECT ... all_pass` body from 04b_region_suite.sql VERBATIM
```
(`VERDICT_SQL` is a **runtime** query in a test target — no macro/cache. Transcribe it byte-faithfully from `04b_region_suite.sql`.)

- [ ] **Step 2: Run** (artifact + ONNX):
```bash
for f in 01_schema 02_functions; do psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f schema-artifact/$f.sql; done
cargo nextest run -p temper-next --features artifact-tests onboarding_scenario_roundtrips
```
Expected: PASS — declarative asserts AND `all_pass = true`.

- [ ] **Step 3: Equivalence confidence check** — run `schema-artifact/run_eval.sh` (SQL-seed path) and confirm it still prints ALL S6 PASS. Both seed paths pass the same verdict.
- [ ] **Step 4: Commit** — `git commit -am "test(temper-next): onboarding YAML roundtrips to S6 verdict (nextest-native)"`

---

## Task 15: Schema emission + drift snapshot

**Tag:** EXTEND. Same as the prior plan's Task 16.

**Files:** `crates/temper-next/tests/scenario_schema.rs` (`scenario-schema`), `schema-artifact/scenarios/scenario.schema.json`

- [ ] **Step 1:** drift test emitting `schemars::schema_for!(Scenario)` vs the committed snapshot (`UPDATE_SCHEMA=1` refreshes). Body identical to the prior plan's Task 16.
- [ ] **Step 2:** `UPDATE_SCHEMA=1 cargo test -p temper-next --features scenario-schema scenario_json_schema_matches_snapshot` — writes the snapshot, passes. ⚠️ If `schemars` rejects the untagged `FacetDef`/`CmpOp` rename, apply the Task 7 struct fallback and regenerate.
- [ ] **Step 3:** no-UPDATE run — PASS against committed snapshot.
- [ ] **Step 4: Commit** — `git commit -am "feat(temper-next): emit + snapshot-test scenario JSON Schema"`

---

## Final verification (run inline in the controller session)

- [ ] `cargo make check` — clean (fmt/clippy/machete/TS).
- [ ] `cargo make prepare-next` produces no diff (cache current) — commit if it does.
- [ ] `SQLX_OFFLINE=true cargo build -p temper-next` — offline compile clean.
- [ ] `cargo nextest run -p temper-next` (ungated) — pass.
- [ ] artifact + ONNX, write-path (self-resetting): `cargo nextest run -p temper-next --features artifact-tests` — pass (boot-seed, loader, roundtrip incl. cross-path membership).
- [ ] artifact + ONNX, legacy read-path (load 01+02+03_seed first): `cargo nextest run -p temper-next --features artifact-tests-legacy` — pass (materialize, substrate_read, embed_job). **Never** combine with the write-path feature in one run (resets vs. seed conflict).
- [ ] `cargo test -p temper-next --features scenario-schema` — snapshot passes.
- [ ] `schema-artifact/run_eval.sh` — ALL S6 PASS (SQL path unbroken).
- [ ] Update `temper-next` CLAUDE.md / artifact README with the `prepare-next` ritual + the new SQL functions (keep-CLAUDE.md-current rule).

## Self-review notes (spec coverage)

- Mutation mechanics as SQL functions → Tasks 2–5; boot-seed (event types + global lenses) → Task 6; `!`-macros + per-crate `.sqlx` → Tasks 6/8/9/11 (macros) + Task 12 (cache); pgvector runtime exception honored (Task 8).
- Thin loader over functions → Task 9; runner full S6a–h → Task 11; onboarding YAML (lenses by reference) → Task 13; nextest-native equivalence → Task 14; JSON Schema drift → Task 15.
- Deferred CR folded into M1: parse_kind, label_factor, facet multi-key, explicit emitter, lens-name validation (Tasks 8, 11).
- Out of scope (full 03_seed retirement, access scaffold, ts-rs/OpenAPI, temper-api routing) → absent, correct.
