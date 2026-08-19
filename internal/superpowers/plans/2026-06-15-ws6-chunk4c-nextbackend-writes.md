# WS6 chunk 4c — NextBackend writes + Backend-trait growth: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make resource + relationship writes answerable from `temper_next` behind `flag=next` (gated OFF), proven by round-trip equivalence with the legacy backend at the §9 floor.

**Architecture:** New event-sourced mutation functions in the `temper_next` artifact (`schema-artifact/01_schema.sql`+`02_functions.sql`) + their typed `events::fire` arms; a temper-next `writes` module composing them; a feature-gated `NextBackend` (temper-api) whose write methods delegate to it via natural-key identity resolution; the `Backend` trait grown to carry relationship/edge writes so the 4a selector dispatches them.

**Tech Stack:** Rust (temper-next, temper-api, temper-core), PostgreSQL 18 + pgvector (the `temper_next` namespace), sqlx (`!`-macros target `temper_next`; regenerate with `cargo make prepare-next`), nextest.

**Spec:** `docs/superpowers/specs/2026-06-15-ws6-chunk4c-nextbackend-writes-design.md`. Read it — this plan is an index + sequence + grounding over it, not a replacement.

**Load-bearing invariant (spec, adjudication §0/§3, carried verbatim):** *"all writes through atomic SQL mutation functions that emit + project in one transaction"* and *"replay is the same code path as normal operation"* — every new function uses the `_event_append` / `_project_*` split, never a direct projection write.

**Standing rules for this branch:**
- Any temper-api build with `next-backend` MUST set `SQLX_OFFLINE=true` (temper-next macros target the `temper_next` namespace, unvalidatable live).
- After changing any temper-next SQL or artifact functions, run **`cargo make prepare-next`** (per-crate cache; never `--workspace`).
- `git checkout HEAD -- <path>` to restore from HEAD (not `git checkout <path>`, which restores the index).
- Branch: `jct/ws6-chunk4-gate-decomposition` (continues 4a+4b; all 4× ship as ONE PR — no PR at the end of this plan).

---

## File structure

**Artifact (design-master; regenerated installs derive from these):**
- `schema-artifact/01_schema.sql` — add the edge-uniqueness partial index.
- `schema-artifact/02_functions.sql` — `resource_delete`/`_project_resource_deleted`, `resource_update`/`_project_resource_updated`, `resource_rehome`/`_project_resource_rehomed`, `relationship_retype`/`_project_relationship_retyped`, `relationship_reweight`/`_project_relationship_reweighted`; modify `_project_relationship_asserted` to be idempotent.
- `schema-artifact/seeds/system.yaml` — add the five new event-type names to the registry.
- `schema-artifact/03_seed.sql` — add `resource_rehomed` (the other four already registered there).

**temper-next:**
- `crates/temper-next/src/payloads.rs` — `ResourceDeleted`, `ResourceUpdated`, `ResourceRehomed`, `RelationshipRetyped`, `RelationshipReweighted` payload structs.
- `crates/temper-next/src/events.rs` — `EventKind` variants + `SeedAction` variants + `fire` arms for each.
- `crates/temper-next/src/writes.rs` *(new)* — typed write ops NextBackend calls (`create_resource`, `update_resource`, `delete_resource`, `assert/retype/reweight/fold_relationship`, identity-resolution helpers).
- `crates/temper-next/src/lib.rs` — `pub mod writes;`.
- `crates/temper-next/tests/write_path_mutations.rs` *(new)* — artifact-tests for the new functions (`temper-next-write` group).

**temper-api:**
- `crates/temper-api/src/backend/next_backend.rs` — implement the write methods; identity resolution.
- `crates/temper-core/src/operations/backend.rs` — grow the `Backend` trait (4 relationship methods).
- `crates/temper-api/src/backend/db_backend.rs` — move the 4 concrete relationship methods into `impl Backend`.
- `crates/temper-api/src/backend/selection.rs` + the `require_legacy_backend` relationship sites — repoint to `select_backend`.
- `tests/e2e/tests/backend_write_path_next.rs` *(new)* — round-trip equivalence e2e.

---

## SLICE 1 — substrate mutation functions

> Grounding note before starting: open `crates/temper-next/tests/` and read one existing write-path test (e.g. the ledger/replay test in the `temper-next-write` group) to copy its **reset-to-01+02 + `bootseed::seed_system` preamble** verbatim — every test in this slice owns the namespace the same way. Confirm the group wiring in `.config/nextest.toml`.

### Task 1.1: register the five new event-type names

**Files:**
- Modify: `schema-artifact/seeds/system.yaml` (the `event_types:` list)
- Modify: `schema-artifact/03_seed.sql` (add `resource_rehomed` only)
- Modify: `crates/temper-next/src/events.rs` (`EventKind` enum + `as_canonical_name`)

- [ ] **Step 1: Add names to `system.yaml`.** Under `event_types:`, add (the four already in `03_seed` but absent here, plus the new one):

```yaml
  - resource_updated
  - resource_deleted
  - resource_rehomed
  - relationship_retyped
  - relationship_reweighted
```

- [ ] **Step 2: Add `resource_rehomed` to `03_seed.sql`.** In the `INSERT INTO kb_event_types (name) VALUES` block (`03_seed.sql:30-37`), append `('resource_rehomed')` to the list (the other four are already present).

- [ ] **Step 3: Add `EventKind` variants.** In `events.rs:37-46` add `ResourceUpdated, ResourceDeleted, ResourceRehomed, RelationshipRetyped, RelationshipReweighted`; in `as_canonical_name` (`events.rs:50-61`) map each to its snake_case name.

- [ ] **Step 4: Build temper-next.** Run: `SQLX_OFFLINE=true cargo build -p temper-next`
  Expected: compiles (the new `EventKind` variants are unused until their `SeedAction` arms land in later tasks — that's fine; `EventKind` has no exhaustive external match).

- [ ] **Step 5: Commit.**

```bash
git add schema-artifact/seeds/system.yaml schema-artifact/03_seed.sql crates/temper-next/src/events.rs
git commit -m "WS6 4c: register resource_updated/deleted/rehomed + relationship_retyped/reweighted event types"
```

### Task 1.2: edge-uniqueness index + idempotent `relationship_assert`

**Files:**
- Modify: `schema-artifact/01_schema.sql` (after the `kb_edges` table, `:424`)
- Modify: `schema-artifact/02_functions.sql` (`_project_relationship_asserted`, `:745-768`)
- Test: `crates/temper-next/tests/write_path_mutations.rs`

- [ ] **Step 1: Write the failing artifact-test.** Create `write_path_mutations.rs` with the standard write-path preamble (reset namespace to 01+02, `bootseed::seed_system`, `#![cfg(feature = "artifact-tests")]`). First test:

```rust
// Re-asserting the same active (src,tgt,kind,label) updates the existing edge's weight
// rather than creating a duplicate active edge (spec: no-duplicate-active-edge invariant).
#[sqlx::test]
async fn reassert_active_edge_is_idempotent(pool: PgPool) {
    let (ctx, owner, emitter, a, b) = seed_two_resources(&pool).await; // helper: two homed resources + actors
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path TO temper_next, public").execute(&mut *tx).await.unwrap();

    let e1 = fire(&mut tx, assert_action(a, b, "leads_to", Some("operationalized_by"), 1.0, ctx, emitter))
        .await.unwrap().relationship().unwrap();
    let e2 = fire(&mut tx, assert_action(a, b, "leads_to", Some("operationalized_by"), 2.0, ctx, emitter))
        .await.unwrap().relationship().unwrap();
    tx.commit().await.unwrap();

    assert_eq!(e1, e2, "re-assert must return the SAME edge id");
    let (count, weight): (i64, f64) = sqlx::query_as(
        "SELECT count(*), max(weight) FROM temper_next.kb_edges \
         WHERE source_id=$1 AND target_id=$2 AND NOT is_folded")
        .bind(a.uuid()).bind(b.uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(count, 1, "exactly one active edge");
    assert_eq!(weight, 2.0, "weight updated to the re-asserted value");
}
```

- [ ] **Step 2: Run, verify it fails.** Run: `cargo nextest run -p temper-next --features artifact-tests reassert_active_edge_is_idempotent`
  Expected: FAIL — two distinct edge ids / count=2 (no uniqueness yet).

- [ ] **Step 3: Add the partial unique index** in `01_schema.sql` immediately after the `kb_edges` `CREATE TABLE` (`:424`):

```sql
-- No duplicate ACTIVE edge for the same declared relationship (spec 4c). NULLS NOT DISTINCT so a
-- NULL label still collides (PG15+). Folded rows are exempt — a folded edge can be superseded.
CREATE UNIQUE INDEX uq_kb_edges_active
    ON kb_edges (source_id, target_id, edge_kind, label) NULLS NOT DISTINCT
    WHERE NOT is_folded;
```

- [ ] **Step 4: Make `_project_relationship_asserted` idempotent** (`02_functions.sql:745-768`). Replace the bare `INSERT … VALUES (…)` with an upsert that returns the surviving row id:

```sql
    INSERT INTO kb_edges (id, source_table, source_id, target_table, target_id,
                          edge_kind, polarity, label, weight,
                          home_anchor_table, home_anchor_id,
                          asserted_by_event_id, last_event_id, created)
    VALUES (v_edge, …)                                  -- (unchanged VALUES list)
    ON CONFLICT (source_id, target_id, edge_kind, label) WHERE NOT is_folded
        DO UPDATE SET weight = EXCLUDED.weight, last_event_id = EXCLUDED.last_event_id
    RETURNING id INTO v_edge;
    RETURN v_edge;
```

Note: `v_edge` is re-bound from `RETURNING`, so on a conflict it becomes the **existing** edge id (the stable identity), which is what `fire` returns to the caller. Declare it `DECLARE v_edge uuid;` and set the payload id into the VALUES (not as the initializer) so the `RETURNING` rebind is the source of truth.

- [ ] **Step 5: Regenerate the cache + run the test.** Run: `cargo make prepare-next && cargo nextest run -p temper-next --features artifact-tests reassert_active_edge_is_idempotent`
  Expected: PASS.

- [ ] **Step 6: Guard the fold-then-reassert case** (a folded edge must NOT block a fresh active assert):

```rust
#[sqlx::test]
async fn reassert_after_fold_creates_fresh_edge(pool: PgPool) {
    // assert → fold → assert the same (src,tgt,kind,label): the partial index excludes the folded
    // row, so the second assert succeeds as a NEW active edge (distinct id from the folded one).
    // … fire assert (e1) → fire RelationshipFold{edge:e1} → fire assert (e2); assert e1 != e2,
    //   and exactly one active + one folded edge exist.
}
```
  Run the same command for this test; Expected: PASS.

- [ ] **Step 7: Commit.**

```bash
git add schema-artifact/01_schema.sql schema-artifact/02_functions.sql crates/temper-next/.sqlx crates/temper-next/tests/write_path_mutations.rs
git commit -m "WS6 4c: edge-uniqueness invariant — partial unique index + idempotent relationship_assert"
```

### Task 1.3: `resource_delete` (soft-delete)

**Files:**
- Modify: `schema-artifact/02_functions.sql`
- Modify: `crates/temper-next/src/payloads.rs`, `crates/temper-next/src/events.rs`
- Test: `crates/temper-next/tests/write_path_mutations.rs`

- [ ] **Step 1: Write the failing test.**

```rust
#[sqlx::test]
async fn resource_delete_sets_inactive(pool: PgPool) {
    let (ctx, owner, emitter, r) = seed_one_resource(&pool).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path TO temper_next, public").execute(&mut *tx).await.unwrap();
    fire(&mut tx, SeedAction::ResourceDelete { resource: r, emitter }).await.unwrap();
    tx.commit().await.unwrap();
    let active: bool = sqlx::query_scalar("SELECT is_active FROM temper_next.kb_resources WHERE id=$1")
        .bind(r.uuid()).fetch_one(&pool).await.unwrap();
    assert!(!active);
}
```

- [ ] **Step 2: Run, verify it fails** (no `SeedAction::ResourceDelete`). Run: `cargo nextest run -p temper-next --features artifact-tests resource_delete_sets_inactive` — Expected: FAIL (does not compile).

- [ ] **Step 3: Add the payload** (`payloads.rs`):

```rust
/// Soft-delete a resource (WS6 4c). Identity-only — projection flips `is_active`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDeleted {
    pub resource_id: ResourceId,
}
```

- [ ] **Step 4: Add the SQL** (`02_functions.sql`, mirror `relationship_fold`'s home-from-target pattern — here the resource's own home is the envelope, like `facet_set`):

```sql
CREATE FUNCTION _project_resource_deleted(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_resource uuid := (p_payload->>'resource_id')::uuid;
BEGIN
    UPDATE kb_resources SET is_active = false, updated = (SELECT occurred_at FROM kb_events WHERE id = p_event)
        WHERE id = v_resource;
    IF NOT FOUND THEN RAISE EXCEPTION 'resource_delete: resource % not found', v_resource; END IF;
    RETURN v_resource;
END;
$$;

CREATE FUNCTION resource_delete(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_resource uuid := (p_payload->>'resource_id')::uuid;
        v_anchor_tbl text; v_anchor uuid;
BEGIN
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table='kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN RAISE EXCEPTION 'resource_delete: resource % has no home', v_resource; END IF;
    v_ev := _event_append('resource_deleted', p_emitter, v_anchor_tbl, v_anchor, p_payload);
    RETURN _project_resource_deleted(v_ev, p_payload);
END;
$$;
```

- [ ] **Step 5: Add the `SeedAction` + `fire` arm + `event_type`.** In `events.rs`: variant `ResourceDelete { resource: ResourceId, emitter: EntityId }`; `event_type` → `EventKind::ResourceDeleted`; `fire` arm:

```rust
SeedAction::ResourceDelete { resource, emitter } => {
    let payload = payloads::ResourceDeleted { resource_id: resource };
    let id = sqlx::query_scalar!("SELECT resource_delete($1,$2)",
        serde_json::to_value(&payload)?, emitter.uuid())
        .fetch_one(&mut *conn).await?.context("resource_delete returned null")?;
    Ok(Fired::Resource(ResourceId::from(id)))
}
```

- [ ] **Step 6: prepare + run.** Run: `cargo make prepare-next && cargo nextest run -p temper-next --features artifact-tests resource_delete_sets_inactive` — Expected: PASS.

- [ ] **Step 7: Replay-parity test.** Add a test asserting that replaying the `resource_deleted` event (re-running `_project_resource_deleted` from the ledger) reproduces `is_active=false` — mirror the existing replay test's structure. Run it; Expected: PASS.

- [ ] **Step 8: Commit.**

```bash
git add schema-artifact/02_functions.sql crates/temper-next/src/{payloads.rs,events.rs} crates/temper-next/.sqlx crates/temper-next/tests/write_path_mutations.rs
git commit -m "WS6 4c: resource_delete soft-delete mutation function"
```

### Task 1.4: `resource_update` (title / origin_uri)

**Files:** as Task 1.3.

- [ ] **Step 1: Failing test** — fire `ResourceUpdate { resource, title: Some("New"), origin_uri: None, emitter }`, assert `kb_resources.title = 'New'` and `origin_uri` unchanged.

- [ ] **Step 2: Run, verify FAIL.**

- [ ] **Step 3: Payload** (`payloads.rs`): `ResourceUpdated { resource_id, #[serde(skip_serializing_if="Option::is_none")] title: Option<String>, #[serde(skip_serializing_if="Option::is_none")] origin_uri: Option<String> }`.

- [ ] **Step 4: SQL** (COALESCE keeps unset fields; envelope = the resource's home, as Task 1.3):

```sql
CREATE FUNCTION _project_resource_updated(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_resource uuid := (p_payload->>'resource_id')::uuid;
BEGIN
    UPDATE kb_resources SET
        title      = COALESCE(p_payload->>'title', title),
        origin_uri = COALESCE(p_payload->>'origin_uri', origin_uri),
        updated    = (SELECT occurred_at FROM kb_events WHERE id = p_event)
        WHERE id = v_resource;
    IF NOT FOUND THEN RAISE EXCEPTION 'resource_update: resource % not found', v_resource; END IF;
    RETURN v_resource;
END;
$$;
-- resource_update(p_payload, p_emitter): home-from-resource envelope (copy resource_delete's body,
-- swap the event name to 'resource_updated' and the projection call to _project_resource_updated).
```
Write the `resource_update` wrapper in full, identical to `resource_delete`'s wrapper except the event name `'resource_updated'` and the projection call.

- [ ] **Step 5: `SeedAction::ResourceUpdate { resource, title: Option<&str>, origin_uri: Option<&str>, emitter }` + fire arm** (build `ResourceUpdated`, `SELECT resource_update($1,$2)`, return `Fired::Resource`).

- [ ] **Step 6: prepare + run** — Expected: PASS.

- [ ] **Step 7: Replay-parity test** for `resource_updated`. Run; Expected: PASS.

- [ ] **Step 8: Commit** — `"WS6 4c: resource_update mutation function (title/origin_uri)"`.

### Task 1.5: `resource_rehome` (context move)

**Files:** as Task 1.3. The home table is `kb_resource_homes` (`01_schema.sql:222`, `resource_id` UNIQUE).

- [ ] **Step 1: Failing test** — seed a resource in context A and a second context B; fire `ResourceRehome { resource, home: AnchorRef::context(B), emitter }`; assert `kb_resource_homes.anchor_id = B`.

- [ ] **Step 2: Run, verify FAIL.**

- [ ] **Step 3: Payload**: `ResourceRehomed { resource_id, home: AnchorRef }` (reuse the existing `payloads::AnchorRef`, `payloads.rs:48`).

- [ ] **Step 4: SQL** — the envelope is the DESTINATION home (where the resource now lives); update the home row's anchor:

```sql
CREATE FUNCTION _project_resource_rehomed(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_resource uuid := (p_payload->>'resource_id')::uuid;
BEGIN
    UPDATE kb_resource_homes SET
        anchor_table = p_payload#>>'{home,table}',
        anchor_id    = (p_payload#>>'{home,id}')::uuid
        WHERE resource_id = v_resource;
    IF NOT FOUND THEN RAISE EXCEPTION 'resource_rehome: resource % has no home', v_resource; END IF;
    RETURN v_resource;
END;
$$;

CREATE FUNCTION resource_rehome(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    -- envelope = the destination anchor (post-move home), taken from the payload.
    v_ev := _event_append('resource_rehomed', p_emitter,
                          p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid, p_payload);
    RETURN _project_resource_rehomed(v_ev, p_payload);
END;
$$;
```

- [ ] **Step 5: `SeedAction::ResourceRehome { resource, home: AnchorRef, emitter }` + fire arm.**

- [ ] **Step 6: prepare + run** — Expected: PASS.

- [ ] **Step 7: Replay-parity test** for `resource_rehomed`. Run; Expected: PASS.

- [ ] **Step 8: Commit** — `"WS6 4c: resource_rehome mutation function (context move)"`.

### Task 1.6: `relationship_retype` (edge_kind / polarity)

**Files:** as Task 1.3. Mirror `relationship_fold` (`02_functions.sql:781-810`) — home read from the edge.

- [ ] **Step 1: Failing test** — assert an edge `leads_to/forward`; fire `RelationshipRetype { edge, kind: EdgeKind::Contains, polarity: EdgePolarity::Forward, emitter }`; assert `kb_edges.edge_kind = 'contains'`.

- [ ] **Step 2: Run, verify FAIL.**

- [ ] **Step 3: Payload**: `RelationshipRetyped { edge_id, edge_kind: EdgeKind, polarity: EdgePolarity }` (reuse `affinity::EdgeKind`'s serialize and `payloads::EdgePolarity`, `payloads.rs:79`).

- [ ] **Step 4: SQL** (copy `relationship_fold`'s home-read wrapper; the projection sets kind+polarity):

```sql
CREATE FUNCTION _project_relationship_retyped(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_edge uuid := (p_payload->>'edge_id')::uuid;
BEGIN
    UPDATE kb_edges SET
        edge_kind = (p_payload->>'edge_kind')::edge_kind,
        polarity  = (p_payload->>'polarity')::edge_polarity,
        last_event_id = p_event
        WHERE id = v_edge;
    IF NOT FOUND THEN RAISE EXCEPTION 'relationship_retype: edge % not found', v_edge; END IF;
    RETURN v_edge;
END;
$$;
-- relationship_retype(p_payload, p_emitter): copy relationship_fold's body verbatim, swapping the
-- event name to 'relationship_retyped' and the projection call to _project_relationship_retyped.
```
Write the `relationship_retype` wrapper in full (the home-from-edge `SELECT … INTO`, `_event_append('relationship_retyped', …)`, projection call).

- [ ] **Step 5: `SeedAction::RelationshipRetype { edge: EdgeId, kind: EdgeKind, polarity: EdgePolarity, emitter }` + fire arm** → `Fired::Relationship`.

- [ ] **Step 6: prepare + run** — Expected: PASS.

- [ ] **Step 7: Replay-parity test** for `relationship_retyped`. Run; Expected: PASS.

- [ ] **Step 8: Commit** — `"WS6 4c: relationship_retype mutation function"`.

### Task 1.7: `relationship_reweight` (weight)

**Files:** as Task 1.6.

- [ ] **Step 1: Failing test** — assert an edge `weight=1.0`; fire `RelationshipReweight { edge, weight: 3.5, emitter }`; assert `kb_edges.weight = 3.5`.

- [ ] **Step 2: Run, verify FAIL.**

- [ ] **Step 3: Payload**: `RelationshipReweighted { edge_id, weight: f64 }`.

- [ ] **Step 4: SQL** — `_project_relationship_reweighted` sets `weight = (p_payload->>'weight')::double precision, last_event_id = p_event`; `relationship_reweight` wrapper copies `relationship_fold`'s body with event name `'relationship_reweighted'`. Write both in full.

- [ ] **Step 5: `SeedAction::RelationshipReweight { edge: EdgeId, weight: f64, emitter }` + fire arm** → `Fired::Relationship`.

- [ ] **Step 6: prepare + run** — Expected: PASS.

- [ ] **Step 7: Replay-parity test** for `relationship_reweighted`. Run; Expected: PASS.

- [ ] **Step 8: Slice-1 regression + commit.** Run: `cargo nextest run -p temper-next --features artifact-tests` (the whole `temper-next-write` group) — Expected: all PASS.

```bash
git add schema-artifact/02_functions.sql crates/temper-next/src/{payloads.rs,events.rs} crates/temper-next/.sqlx crates/temper-next/tests/write_path_mutations.rs
git commit -m "WS6 4c: relationship_reweight mutation function + slice-1 write-path suite green"
```

---

## SLICE 2 — temper-next typed write ops (`writes` module)

The composition layer NextBackend calls. All ops open one tx, `SET LOCAL search_path TO temper_next, public`, fire through `events::fire`, and return typed ids. Mirror `synthesis::run`'s tx discipline (`synthesis/mod.rs:91-94`).

### Task 2.1: identity-resolution helpers

**Files:**
- Create: `crates/temper-next/src/writes.rs`
- Modify: `crates/temper-next/src/lib.rs` (`pub mod writes;`)
- Test: `crates/temper-next/tests/write_path_mutations.rs`

- [ ] **Step 1: Failing test** — after `bootseed::seed_system` + a synthesized profile/entity/context fixture, call `writes::resolve_profile(&pool, prod_profile_id)` and assert it returns the temper_next profile whose `handle` = the production slug. (Seed a `temper_next.kb_profiles` row with a known handle + a matching `public.kb_profiles` slug.)

- [ ] **Step 2: Run, verify FAIL** (no `writes` module).

- [ ] **Step 3: Implement the three resolvers** (runtime `sqlx::query` with explicit `temper_next.`/`public.` qualification — the `synthesis::source` precedent, `source.rs:1-6`):

```rust
//! Typed write composition over the temper_next mutation functions (WS6 4c). Identity is resolved by
//! natural key (handle / entity-name / context-slug) — the same keys synthesis writes by — so no
//! old→new id-map table is needed. All SQL is runtime-qualified to dodge the offline-cache namespace.
use anyhow::{Context, Result};
use sqlx::{PgPool, Row};
use uuid::Uuid;
use crate::ids::{ContextId, EntityId, ProfileId};
use crate::synthesis::bootstrap::slugify;

/// production profile id → temper_next profile id, by `handle` (= production `kb_profiles.slug`).
pub async fn resolve_profile(pool: &PgPool, prod_profile: Uuid) -> Result<ProfileId> {
    let slug: String = sqlx::query("SELECT slug FROM public.kb_profiles WHERE id=$1")
        .bind(prod_profile).fetch_one(pool).await?.get("slug");
    let id: Uuid = sqlx::query("SELECT id FROM temper_next.kb_profiles WHERE handle=$1")
        .bind(&slug).fetch_one(pool).await
        .with_context(|| format!("no temper_next profile for handle {slug:?} (substrate not synthesized?)"))?
        .get("id");
    Ok(ProfileId::from(id))
}

/// the per-surface emitter entity `pete@<surface>` for a profile.
pub async fn resolve_emitter(pool: &PgPool, profile: ProfileId, surface: &str) -> Result<EntityId> {
    let name = format!("pete@{surface}");
    let id: Uuid = sqlx::query("SELECT id FROM temper_next.kb_entities WHERE profile_id=$1 AND name=$2")
        .bind(profile.uuid()).bind(&name).fetch_one(pool).await
        .with_context(|| format!("no emitter entity {name:?}"))?.get("id");
    Ok(EntityId::from(id))
}

/// home context by (owner profile, slugify(name)). Owner-scoped per §2.
pub async fn resolve_context(pool: &PgPool, owner: ProfileId, name: &str) -> Result<ContextId> {
    let slug = slugify(name);
    let id: Uuid = sqlx::query(
        "SELECT id FROM temper_next.kb_contexts WHERE owner_table='kb_profiles' AND owner_id=$1 AND slug=$2")
        .bind(owner.uuid()).bind(&slug).fetch_one(pool).await
        .with_context(|| format!("no context {slug:?} for owner"))?.get("id");
    Ok(ContextId::from(id))
}
```
(Make `synthesis::bootstrap::slugify` `pub` — it's currently `pub(crate)`, already crate-visible, so re-export or keep `pub(crate)` and call within-crate; `writes` is in-crate so `pub(crate)` is fine.)

- [ ] **Step 4: Run** — Expected: PASS.

- [ ] **Step 5: Commit** — `"WS6 4c: temper-next writes module — natural-key identity resolution"`.

### Task 2.2: `create_resource` op

**Files:** `crates/temper-next/src/writes.rs`, test file.

- [ ] **Step 1: Failing test** — `writes::create_resource(&pool, params)` with a title/body/doc_type/context/owner; assert a `kb_resources` row exists with that title and `readback::resource_row(new_id)` reconstructs the §9 fields, and `resource_body_text(new_id)` equals the input body.

- [ ] **Step 2: Run, verify FAIL.**

- [ ] **Step 3: Implement** (params struct — >5 fields; `content::prepare_blocks` for the body; fire `ResourceCreate`; then `PropertyAssert` per property key):

```rust
pub struct CreateParams<'a> {
    pub title: &'a str,
    pub origin_uri: &'a str,
    pub body: &'a str,
    pub doc_type: &'a str,
    pub home: ContextId,
    pub owner: ProfileId,
    pub originator: ProfileId,
    pub emitter: EntityId,
    /// (key, value) property pairs (managed §7-Property-fated + open keys), each fired as PropertyAssert.
    pub properties: &'a [(String, serde_json::Value)],
}

pub async fn create_resource(pool: &PgPool, p: CreateParams<'_>) -> Result<ResourceId> {
    let blocks = crate::content::prepare_blocks(&[(None, p.body)])?;
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL search_path TO temper_next, public").execute(&mut *tx).await?;
    let new_id = crate::events::fire(&mut tx, crate::events::SeedAction::ResourceCreate {
        title: p.title, origin_uri: p.origin_uri,
        home: crate::payloads::AnchorRef::context(p.home),
        owner: p.owner, originator: Some(p.originator),
        blocks: &blocks, doc_type: Some(p.doc_type), emitter: p.emitter,
    }).await?.resource()?;
    for (k, v) in p.properties {
        crate::events::fire(&mut tx, crate::events::SeedAction::PropertyAssert {
            resource: new_id, key: k, value: v, weight: 1.0, emitter: p.emitter,
        }).await?;
    }
    tx.commit().await?;
    Ok(new_id)
}
```

- [ ] **Step 4: Run** — Expected: PASS.

- [ ] **Step 5: Commit** — `"WS6 4c: writes::create_resource"`.

### Task 2.3: `update_resource` + `delete_resource` ops

**Files:** `writes.rs`, test file.

- [ ] **Step 1: Failing tests** — (a) `update_resource` with a new body revises the block (`resource_body_text` changes) and a property change lands via `facet_set`; a title change lands via `resource_update`; a `context_to` lands via `resource_rehome`. (b) `delete_resource` flips `is_active`.

- [ ] **Step 2: Run, verify FAIL.**

- [ ] **Step 3: Implement** `update_resource(pool, UpdateParams)` — partial: for `body` present, find the resource's current block id (`SELECT id FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq LIMIT 1` — confirm column names against `01_schema.sql`) and fire `BlockMutate`; for each property pair fire `PropertyAssert`; for `title`/`origin_uri` fire `ResourceUpdate`; for `context_to` resolve the destination context and fire `ResourceRehome`. `delete_resource(pool, resource, emitter)` fires `ResourceDelete`. (All inside one tx with the search_path set.)

- [ ] **Step 4: Run** — Expected: PASS.

- [ ] **Step 5: Commit** — `"WS6 4c: writes::update_resource + delete_resource"`.

### Task 2.4: relationship ops

**Files:** `writes.rs`, test file.

- [ ] **Step 1: Failing tests** — `assert_relationship` returns an edge id (and is idempotent on re-assert, Task 1.2); `retype/reweight/fold_relationship` mutate it.

- [ ] **Step 2: Run, verify FAIL.**

- [ ] **Step 3: Implement** four thin ops wrapping `SeedAction::{RelationshipAssert, RelationshipRetype, RelationshipReweight, RelationshipFold}` (one tx each, search_path set). `assert` takes resolved `src`/`tgt` `ResourceId`s + kind/polarity/label/weight + home `ContextId`; retype/reweight/fold take the `EdgeId`.

- [ ] **Step 4: Run** — Expected: PASS.

- [ ] **Step 5: Commit** — `"WS6 4c: writes relationship ops (assert/retype/reweight/fold)"`.

---

## SLICE 3 — NextBackend resource writes + round-trip e2e

> `NextBackend` is feature-gated (`next-backend`); all builds here use `SQLX_OFFLINE=true`.

### Task 3.1: `create_resource` through NextBackend

**Files:**
- Modify: `crates/temper-api/src/backend/next_backend.rs` (replace the `create_resource` stub, `:116-123`)
- Test: `tests/e2e/tests/backend_write_path_next.rs` *(new)*

- [ ] **Step 1: Write the failing round-trip e2e.** New e2e under `#![cfg(all(feature = "test-db", feature = "next-backend"))]`, mirroring 4b's `backend_read_path_next.rs` spawn/synthesize harness. Test:

```rust
// Create the SAME logical resource through legacy (public) and through next (temper_next); the
// returned ResourceRow matches at the §9 invariant floor + body-text parity.
#[sqlx::test]
async fn create_roundtrip_matches_at_floor(pool: PgPool) {
    synthesize(&pool).await;                 // bring temper_next up from public (4b harness helper)
    let cmd = sample_create();               // title/body/doctype/context/managed_meta
    let legacy = legacy_backend(&pool).create_resource(cmd.clone()).await.unwrap().value;
    let next   = next_backend(&pool).create_resource(cmd).await.unwrap().value;
    assert_floor_eq(&legacy, &next);         // origin_uri/title/is_active/context_name/doc_type_name/stage/mode/effort/seq
    assert_eq!(body_text(&pool_public, legacy.id).await, body_text_next(&pool, next.id).await);
}
```
(`assert_floor_eq` = the §9 invariant subset; non-invariants — re-minted ids, slug/hashes None, timestamps, `@me` owner_handle, `body_hash` — are excluded, per the 4b parity-floor amendment.)

- [ ] **Step 2: Run, verify FAIL.** Run: `SQLX_OFFLINE=true cargo nextest run -p temper-e2e --features test-db,next-backend create_roundtrip_matches_at_floor` — Expected: FAIL (`NotImplemented`).

- [ ] **Step 3: Implement `NextBackend::create_resource`.** Translate `CreateResource` (`commands.rs:26-52`) → `writes::CreateParams`: resolve owner+originator (`writes::resolve_profile(self.profile_id)`), emitter (`resolve_emitter(.., cmd.origin.as_str())`), home (`resolve_context(owner, &cmd.context)`); split `cmd.managed_meta` through `synthesis::key_fate` (Property-fated only) + `cmd.open_meta` (all) into the `properties` slice; `doc_type = &cmd.doctype`. Call `writes::create_resource`, then return `reconstruct_resource_row(&self.pool, new_id.uuid())` wrapped in `CommandOutput::new`.

- [ ] **Step 4: Run** — Expected: PASS.

- [ ] **Step 5: `cargo make check`** (the offline probe of the committed caches). Expected: clean.

- [ ] **Step 6: Commit** — `"WS6 4c: NextBackend::create_resource + round-trip e2e"`.

### Task 3.2: `update_resource` + `delete_resource` through NextBackend

**Files:** `next_backend.rs` (stubs at `:134-150`), `backend_write_path_next.rs`.

- [ ] **Step 1: Failing round-trip e2e** for update (body + stage + title) and delete — create through both backends, apply the same update/delete through each, read back, assert §9-floor + body-text parity (update) / `is_active=false` (delete).

- [ ] **Step 2: Run, verify FAIL.**

- [ ] **Step 3: Implement** `NextBackend::update_resource` (translate `UpdateResource` → `writes::UpdateParams`: resolve the new id via `resolve_new_id`, map body/managed_meta/open_meta/`move_to.context_to`; ignore `move_to.type_to`? — no: map `type_to` to a `doc_type` property change) and `delete_resource` (`resolve_new_id` → `writes::delete_resource`). Return `reconstruct_resource_row` / `CommandOutput::new(())`.

- [ ] **Step 4: Run** — Expected: PASS.

- [ ] **Step 5: `cargo make check`** — clean.

- [ ] **Step 6: Commit** — `"WS6 4c: NextBackend update/delete + round-trip e2e"`.

---

## SLICE 4 — Backend-trait growth + relationship dispatch

### Task 4.1: grow the `Backend` trait

**Files:**
- Modify: `crates/temper-core/src/operations/backend.rs:44-72`
- Modify: `crates/temper-api/src/backend/db_backend.rs` (move the 4 concrete methods into `impl Backend`)

- [ ] **Step 1: Add the four methods to the trait** (`backend.rs`), returning `CommandOutput<Uuid>` (the backend-opaque edge handle):

```rust
async fn assert_relationship(&self, cmd: AssertRelationship) -> Result<CommandOutput<Uuid>, TemperError>;
async fn retype_relationship(&self, cmd: RetypeRelationship) -> Result<CommandOutput<Uuid>, TemperError>;
async fn reweight_relationship(&self, cmd: ReweightRelationship) -> Result<CommandOutput<Uuid>, TemperError>;
async fn fold_relationship(&self, cmd: FoldRelationship) -> Result<CommandOutput<Uuid>, TemperError>;
```
(Import the command structs — `commands.rs:119-153` — into `backend.rs`.)

- [ ] **Step 2: Move DbBackend's concrete methods into `impl Backend`** (`db_backend.rs:98-415` → into the `#[async_trait] impl Backend` block, `:418`). Signatures are already identical; this is a relocation, zero behavior change.

- [ ] **Step 3: Build + existing suites.** Run: `cargo build -p temper-api` then `cargo nextest run -p temper-api --features test-db` — Expected: PASS (the object-safety test + existing relationship tests still green).

- [ ] **Step 4: Commit** — `"WS6 4c: grow Backend trait with relationship methods; DbBackend satisfies them"`.

### Task 4.2: NextBackend relationship methods

**Files:** `next_backend.rs`, `backend_write_path_next.rs`.

- [ ] **Step 1: Failing edge round-trip e2e** — assert/retype/reweight/fold an edge through both backends (create two resources + an edge through each), read back graph neighbors, assert edge state (kind/polarity/label/weight/is_folded) matches ordering-invariant.

- [ ] **Step 2: Run, verify FAIL.**

- [ ] **Step 3: Implement the four `NextBackend` methods.** `assert_relationship`: resolve `cmd.source` to a new id (`resolve_new_id`) + the target slug to a new id (within temper_next — resolve the target resource by origin_uri/slug; for the round-trip test the target is addressable) + home context + emitter; call `writes::assert_relationship`; return `CommandOutput::new(edge_id)`. `retype/reweight/fold`: the incoming `cmd.correlation_id` IS the next-backend edge id (backend-opaque handle) — pass it straight to the matching `writes` op.

- [ ] **Step 4: Run** — Expected: PASS.

- [ ] **Step 5: Commit** — `"WS6 4c: NextBackend relationship methods + edge round-trip e2e"`.

### Task 4.3: repoint the relationship surface sites

**Files:**
- Modify: the 4a `require_legacy_backend` relationship sites (grep: `git grep -n require_legacy_backend`) in `handlers/edges.rs` + the MCP relationship tools.
- Modify: `crates/temper-api/src/backend/selection.rs` if a dispatch helper is needed.

- [ ] **Step 1: Repoint** each relationship call site from `require_legacy_backend(...)` to `select_backend(...)` (they refused `next` only because no NextBackend write existed; now it does), dispatching the command through the boxed `dyn Backend`.

- [ ] **Step 2: flag=legacy regression** — Run: `cargo nextest run -p temper-api --features test-db` and `SQLX_OFFLINE=true cargo nextest run -p temper-e2e --features test-db` — Expected: byte-identical to pre-4c (legacy arm unchanged).

- [ ] **Step 3: flag=next gate e2e** — extend the 4a gate test so a relationship op under flag=next now reaches NextBackend (no longer a clean refusal). Run under `test-db,next-backend`; Expected: PASS.

- [ ] **Step 4: Commit** — `"WS6 4c: repoint relationship surface sites to select_backend dispatch"`.

---

## SLICE 5 — verification + status

### Task 5.1: full verification

- [ ] **Step 1: temper-next suites.** Run: `cargo nextest run -p temper-next --features artifact-tests` then `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests,next-backend` — Expected: all PASS.

- [ ] **Step 2: api + e2e (both arms).** Run: `cargo nextest run -p temper-api --features test-db`; `SQLX_OFFLINE=true cargo nextest run -p temper-e2e --features test-db,next-backend` — Expected: all PASS.

- [ ] **Step 3: `cargo make check`** — Expected: clean (the honest offline probe; confirms committed `.sqlx` caches).

- [ ] **Step 4: (optional) production-fidelity rehearsal.** If a write path's behavior looks data-shape-dependent, use the Neon MCP tool to export production into a local branch, run `temper-next synthesize`, and re-run the round-trip e2e against the real corpus. Not a gate; a tool.

### Task 5.2: update goal status

- [ ] **Step 1: Update the goal record** `substrate-kernel-to-cognitive-map` WS6 chunk-4 line: 4c BUILT+PROVEN; remaining = §5 client shared-types (incl. the `correlation_id`→neutral-handle rename) → ONE PR → chunk-5 flip (gated on 4c + WS2 access-scoping). Use `temper resource update`.

- [ ] **Step 2: Save a session note** capturing the slice outcomes + any gotchas.

---

## Self-review (completed against the spec)

- **Spec coverage:** Component 1 → Tasks 1.1–1.7; Component 2 → Slice 2; Component 3 → Task 2.1 + Slice 3 translation; Component 4 → Slice 4; Component 5 (proof) → the round-trip e2es in Slices 3–4 + replay-parity per substrate function. The edge-uniqueness invariant → Task 1.2. `move` → Tasks 1.5 (rehome) + 2.3/3.2 (type_to via property). All settled decisions map to a task.
- **Named deferrals (not in scope, per spec):** `by_uri` + MCP `get_resource`/`list_resources` enrichment; §5 shared-type changes incl. the `correlation_id` rename; the flip + WS2 access-scoping.
- **Type consistency:** `SeedAction::{ResourceDelete, ResourceUpdate, ResourceRehome, RelationshipRetype, RelationshipReweight}` and payloads `{ResourceDeleted, ResourceUpdated, ResourceRehomed, RelationshipRetyped, RelationshipReweighted}` used consistently across slices; `writes::{resolve_profile, resolve_emitter, resolve_context, create_resource, update_resource, delete_resource, assert/retype/reweight/fold_relationship}` referenced consistently.
- **Open grounding to confirm at execution (flagged honestly, not assumed):** the exact `temper-next-write` test-harness reset/seed preamble + nextest group wiring (Slice 1 preamble); the content-block column names for the update body-revise block lookup (`01_schema.sql`, Task 2.3); the `backend_read_path_next.rs` spawn/synthesize helpers reused by the write e2e (Slice 3). Each is a read-before-write step, not an invention.
