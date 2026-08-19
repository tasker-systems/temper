# Invocation Envelope + Agent-Authorship Metadata Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an event-sourced **invocation envelope** (an agentic-workflow run modeled at the accountability grain) and per-event **agent-authorship metadata** (reasoning + graded-band confidence) to the `temper_next` substrate, proven by artifact tests + replay.

**Architecture:** The envelope is a first-class projected entity (`kb_invocations`) born from the already-seeded `delegated_launch` event and closed by a new `invocation_closed` event; every mutation event done under a run carries a new `kb_events.invocation_id`. Authorship rides in the existing unused `kb_events.metadata` JSONB — so it is **invisible to projections (and thus to affinity math) by construction**, and it survives replay for free because replay restores `kb_events` verbatim. The delegation launch-gate reuses the existing `cogmaps_share_a_team` predicate.

**Tech Stack:** PostgreSQL (plpgsql, the `temper_next` namespace artifact in `schema-artifact/`), Rust (`temper-next` crate: sqlx, serde, schemars), cargo-nextest.

## Global Constraints

- **Source of design truth:** `schema-artifact/01_schema.sql` + `02_functions.sql` are the design-master. Any change here MUST be mirrored by an **append-only forward migration** in `migrations/` so the semantic drift guard (`crates/temper-next/tests/schema_drift.rs::migrations_reconstruct_artifact_schema`) stays green. Migration function bodies are fingerprinted by `pg_get_functiondef` and must be **byte-identical** to the artifact (unqualified names resolving via `SET search_path TO temper_next, public;` — never schema-qualify the body). Do NOT edit the frozen install migration `20260613000001`.
- **sqlx offline cache is per-crate:** after adding/altering any `sqlx::query!`/`query_scalar!`/`query_as!` in `temper-next`, run **`cargo make prepare-next`** (loads the artifact, prepares with `search_path=temper_next`). NEVER `cargo sqlx prepare --workspace` (clobbers per-crate caches). All `cargo make` tasks force `SQLX_OFFLINE=true`.
- **Typed structs over inline JSON** for production data. `serde_json::json!`/inline jsonb is permitted ONLY in SQL-level test fixtures that exercise a DB function directly.
- **Lint suppression:** `#[expect(lint, reason = "...")]`, never `#[allow]`. All public types derive `Debug`.
- **Identity-as-input:** all entity ids (including `invocation_id`) are pre-generated in Rust (`Uuid::now_v7()`) and arrive in the payload; SQL never generates a projected entity id.
- **Test feature + serialization:** the new tests are gated `#![cfg(feature = "artifact-tests")]` and belong to the serial `temper-next-write` nextest group (they own/reset the namespace). Run: `cargo nextest run -p temper-next --features artifact-tests`. Final gate: `cargo make check`.
- **In scope:** the envelope + authorship substrate primitives + their proof. **Out of scope (later plans):** the `temper-agents` TS package; Eve agent definitions / MCP tool surface; trigger delivery (HTTP channel); the scenario-YAML surface for invocations/authorship (the steward-workflow plan owns the triage runbook DSL); threading authorship through non-authored mutations (system/bootstrap acts). The four **authored-act** functions in scope are exactly: `resource_create`, `relationship_assert`, `facet_set`, `relationship_fold` (the steward's concept/edge/facet/fold acts).

---

### Task 1: Artifact schema — `invocation_id` column + `kb_invocations` table

**Files:**
- Modify: `schema-artifact/01_schema.sql` (the `kb_events` CREATE TABLE near lines 287–312; append a new table + indexes after it)
- Test: `crates/temper-next/tests/invocation_envelope.rs` (Create)

**Interfaces:**
- Produces: table `temper_next.kb_invocations(id, opened_by_event_id, status, trigger_kind, originating_cogmap_id, parent_cogmap_id, scoped_entity_id, telos_resource_id, outcome, opened_at, closed_by_event_id, closed_at)`; column `temper_next.kb_events.invocation_id UUID NULL`.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-next/tests/invocation_envelope.rs`:

```rust
#![cfg(feature = "artifact-tests")]
//! Invocation envelope + agent-authorship metadata. Each test resets the artifact (01+02 via psql),
//! boot-seeds the system actor, and exercises the new substrate. Serialized via the
//! `temper-next-write` nextest group (it owns the namespace).

mod common;

use temper_next::substrate;

/// Reset the artifact (01+02), connect, boot-seed the system actor. Standard write-path preamble.
async fn setup() -> sqlx::PgPool {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    temper_next::scenario::bootseed::seed_system(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn schema_has_invocations_table_and_event_column() {
    let pool = setup().await;
    // kb_events.invocation_id exists and is nullable UUID
    let col: Option<String> = sqlx::query_scalar(
        "SELECT data_type FROM information_schema.columns \
         WHERE table_schema='temper_next' AND table_name='kb_events' AND column_name='invocation_id'",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(col.as_deref(), Some("uuid"), "kb_events.invocation_id must be uuid");

    // kb_invocations table exists
    let tbl: Option<String> = sqlx::query_scalar(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema='temper_next' AND table_name='kb_invocations'",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(tbl.as_deref(), Some("kb_invocations"), "kb_invocations table must exist");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(schema_has_invocations_table_and_event_column)'`
Expected: FAIL — `kb_events.invocation_id must be uuid` (column absent) / table absent.

- [ ] **Step 3: Add the column + table to the artifact**

In `schema-artifact/01_schema.sql`, inside the `CREATE TABLE kb_events (...)` add the column right after the `metadata` line:

```sql
    metadata               JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- Coarser-than-correlation grouping: the agentic-invocation this event was emitted under
    -- (NULL for keyboard-holder / system acts). correlation_id stays act-grain; this is run-grain.
    invocation_id          UUID,
```

Then add this index next to the other `kb_events` indexes (after the existing `idx_kb_events_correlation`):

```sql
CREATE INDEX idx_kb_events_invocation  ON kb_events(invocation_id);
```

Immediately after the `kb_events` indexes/trigger block, add the new table:

```sql
-- ── Invocation envelope (accountability-grain model of an agentic-workflow run) ───────────────
-- Projected from `delegated_launch` (open) + `invocation_closed` (close). The runtime owns
-- orchestration (steps/retries/checkpoints); the substrate records only intent, the delegation
-- binding, the telos/scope, and the terminal outcome. `id` is identity-as-input (the run's own id).
CREATE TABLE kb_invocations (
    id                     UUID PRIMARY KEY,
    opened_by_event_id     UUID NOT NULL REFERENCES kb_events(id),
    status                 TEXT NOT NULL DEFAULT 'open'
                               CHECK (status IN ('open','completed','failed','abandoned')),
    trigger_kind           TEXT NOT NULL,
    originating_cogmap_id  UUID NOT NULL REFERENCES kb_cogmaps(id),
    parent_cogmap_id       UUID REFERENCES kb_cogmaps(id),   -- set for delegated launches (gate-checked)
    scoped_entity_id       UUID NOT NULL REFERENCES kb_entities(id),
    telos_resource_id      UUID NOT NULL REFERENCES kb_resources(id),
    outcome                JSONB,                            -- filled at close: {disposition, counts, note}
    opened_at              TIMESTAMPTZ NOT NULL,
    closed_by_event_id     UUID REFERENCES kb_events(id),
    closed_at              TIMESTAMPTZ
);
CREATE INDEX idx_kb_invocations_cogmap ON kb_invocations(originating_cogmap_id);
CREATE INDEX idx_kb_invocations_status ON kb_invocations(status);
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(schema_has_invocations_table_and_event_column)'`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/01_schema.sql crates/temper-next/tests/invocation_envelope.rs
git commit -m "feat(temper-next): kb_invocations table + kb_events.invocation_id column (artifact)"
```

---

### Task 2: Extend `_event_append` with `p_metadata` + `p_invocation`

**Files:**
- Modify: `schema-artifact/02_functions.sql` (`_event_append`, lines 759–777)
- Test: `crates/temper-next/tests/invocation_envelope.rs`

**Interfaces:**
- Consumes: `kb_events.metadata`, `kb_events.invocation_id` (Task 1).
- Produces: `_event_append(p_type_name, p_emitter, p_anchor_table, p_anchor_id, p_payload, p_references DEFAULT '[]', p_correlation DEFAULT NULL, p_payload_version DEFAULT 1, p_metadata DEFAULT '{}', p_invocation DEFAULT NULL)` — two new trailing defaulted params, so all existing positional callers are unaffected.

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-next/tests/invocation_envelope.rs`:

```rust
#[tokio::test]
async fn event_append_persists_metadata_and_invocation() {
    let pool = setup().await;
    let emitter: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE name='system'")
            .fetch_one(&pool).await.unwrap();
    let inv = uuid::Uuid::now_v7();
    // Call _event_append directly with named args for the two new params.
    let ev: uuid::Uuid = sqlx::query_scalar(
        "SELECT _event_append('cogmap_seeded', $1, NULL, NULL, '{}'::jsonb, \
                p_metadata => $2::jsonb, p_invocation => $3)",
    )
    .bind(emitter)
    .bind(serde_json::json!({"reasoning": "SENTINEL"}))
    .bind(inv)
    .fetch_one(&pool).await.unwrap();

    let (meta, got_inv): (serde_json::Value, Option<uuid::Uuid>) = sqlx::query_as(
        "SELECT metadata, invocation_id FROM kb_events WHERE id=$1",
    )
    .bind(ev)
    .fetch_one(&pool).await.unwrap();
    assert_eq!(meta["reasoning"], "SENTINEL");
    assert_eq!(got_inv, Some(inv));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(event_append_persists_metadata_and_invocation)'`
Expected: FAIL — function `_event_append(...)` has no `p_metadata`/`p_invocation` argument.

- [ ] **Step 3: Edit `_event_append`**

Replace the function in `schema-artifact/02_functions.sql` (lines 759–777) with:

```sql
CREATE FUNCTION _event_append(
    p_type_name text, p_emitter uuid, p_anchor_table text, p_anchor_id uuid,
    p_payload jsonb,
    p_references jsonb DEFAULT '[]'::jsonb,
    p_correlation uuid DEFAULT NULL,
    p_payload_version int DEFAULT 1,
    p_metadata jsonb DEFAULT '{}'::jsonb,
    p_invocation uuid DEFAULT NULL
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_et uuid; v_ev uuid := uuid_generate_v7();
BEGIN
    SELECT id INTO v_et FROM kb_event_types WHERE name = p_type_name;
    IF v_et IS NULL THEN RAISE EXCEPTION 'event_type % not seeded', p_type_name; END IF;
    INSERT INTO kb_events (id, event_type_id, emitter_entity_id,
                           producing_anchor_table, producing_anchor_id,
                           payload, "references", payload_version, correlation_id,
                           metadata, invocation_id)
    VALUES (v_ev, v_et, p_emitter, p_anchor_table, p_anchor_id,
            p_payload, p_references, p_payload_version, COALESCE(p_correlation, v_ev),
            p_metadata, p_invocation);
    RETURN v_ev;
END;
$$;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(event_append_persists_metadata_and_invocation)'`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/02_functions.sql crates/temper-next/tests/invocation_envelope.rs
git commit -m "feat(temper-next): _event_append carries metadata + invocation_id"
```

---

### Task 3: `invocation_open` + `_project_delegated_launch` (with delegation gate)

**Files:**
- Modify: `schema-artifact/02_functions.sql` (append after the existing mutation functions, e.g. after `relationship_reweight` ~line 1204)
- Test: `crates/temper-next/tests/invocation_envelope.rs`

**Interfaces:**
- Consumes: `_event_append` (Task 2), `cogmaps_share_a_team(uuid, uuid)` (existing, line 319), `kb_cogmaps.telos_resource_id`.
- Produces: `invocation_open(p_payload jsonb, p_emitter uuid) RETURNS uuid` (returns the invocation id); `_project_delegated_launch(p_event uuid, p_payload jsonb) RETURNS void`. Payload shape: `{invocation_id, trigger_kind, originating_cogmap_id, parent_cogmap_id?, scoped_entity_id}`.

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-next/tests/invocation_envelope.rs` (add `use temper_next::events::{fire, SeedAction, Fired};`, `use temper_next::ids::{ProfileId, EntityId, CogmapId};`, `use temper_next::content::{PreparedBlock, PreparedChunk};`, `use temper_next::ids::{BlockId, ChunkId};`, `use uuid::Uuid;` to the file header, plus the `system_actor`, `one_chunk_block`, and `genesis` helpers shown here):

```rust
async fn system_actor(pool: &sqlx::PgPool) -> (ProfileId, EntityId) {
    let p: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool).await.unwrap();
    let e: Uuid = sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
        .bind(p).fetch_one(pool).await.unwrap();
    (ProfileId::from(p), EntityId::from(e))
}

fn one_chunk_block(content: &str) -> PreparedBlock {
    let mut embedding = vec![0.0_f32; 768];
    embedding[0] = 1.0;
    PreparedBlock {
        block_id: BlockId::from(Uuid::now_v7()),
        seq: 0,
        role: None,
        chunks: vec![PreparedChunk {
            chunk_id: ChunkId::from(Uuid::now_v7()),
            chunk_index: 0,
            content_hash: format!("{:064x}", Uuid::now_v7().as_u128()),
            content: content.to_string(),
            embedding,
            header_path: None,
            heading_depth: None,
        }],
    }
}

/// Genesis a cogmap, returning its id (the telos resource is created inside).
async fn genesis(pool: &sqlx::PgPool, owner: ProfileId, emitter: EntityId, name: &str) -> CogmapId {
    let charter = vec![one_chunk_block("telos charter statement")];
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path TO temper_next, public").execute(&mut *tx).await.unwrap();
    let (cogmap, _telos) = fire(&mut tx, SeedAction::CogmapGenesis {
        name, telos_title: "Telos", charter: &charter, owner, emitter,
    }).await.unwrap().cogmap_genesis().unwrap();
    tx.commit().await.unwrap();
    cogmap
}

#[tokio::test]
async fn invocation_open_projects_open_row() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-a").await;
    let inv = Uuid::now_v7();

    let returned: Uuid = sqlx::query_scalar("SELECT invocation_open($1::jsonb, $2)")
        .bind(serde_json::json!({
            "invocation_id": inv, "trigger_kind": "manual",
            "originating_cogmap_id": cog.uuid(), "scoped_entity_id": emitter.uuid(),
        }))
        .bind(emitter.uuid())
        .fetch_one(&pool).await.unwrap();
    assert_eq!(returned, inv, "invocation_open returns the invocation id");

    let (status, trig, orig, telos_present): (String, String, Uuid, bool) = sqlx::query_as(
        "SELECT status, trigger_kind, originating_cogmap_id, telos_resource_id IS NOT NULL \
         FROM kb_invocations WHERE id=$1",
    ).bind(inv).fetch_one(&pool).await.unwrap();
    assert_eq!(status, "open");
    assert_eq!(trig, "manual");
    assert_eq!(orig, cog.uuid());
    assert!(telos_present, "telos resolved from the cogmap");
}

#[tokio::test]
async fn delegation_gate_blocks_unshared_cogmaps() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let parent = genesis(&pool, owner, emitter, "parent").await;
    let child = genesis(&pool, owner, emitter, "child").await; // no shared team
    let inv = Uuid::now_v7();
    let res = sqlx::query_scalar::<_, Uuid>("SELECT invocation_open($1::jsonb, $2)")
        .bind(serde_json::json!({
            "invocation_id": inv, "trigger_kind": "delegated",
            "originating_cogmap_id": child.uuid(), "parent_cogmap_id": parent.uuid(),
            "scoped_entity_id": emitter.uuid(),
        }))
        .bind(emitter.uuid())
        .fetch_one(&pool).await;
    let err = res.expect_err("delegation gate must reject cogmaps with no shared team");
    assert!(err.to_string().contains("delegation gate"), "got: {err}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(invocation_open_projects_open_row) | test(delegation_gate_blocks_unshared_cogmaps)'`
Expected: FAIL — `function invocation_open(jsonb, uuid) does not exist`.

- [ ] **Step 3: Add the functions**

Append to `schema-artifact/02_functions.sql`:

```sql
-- ── Invocation envelope (open) ───────────────────────────────────────────────
-- Opens an agentic-workflow run: emits `delegated_launch`, projects an `open` kb_invocations row.
-- Delegation contract 1 (the launch gate): a DELEGATED launch (parent_cogmap_id present) is rejected
-- unless the two cogmaps share a team. A top-level launch (no parent) needs no gate. Identity-as-input:
-- the run's id arrives in the payload.
CREATE FUNCTION invocation_open(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_inv uuid := (p_payload->>'invocation_id')::uuid;
        v_orig uuid := (p_payload->>'originating_cogmap_id')::uuid;
        v_parent uuid := (p_payload->>'parent_cogmap_id')::uuid;
        v_ev uuid;
BEGIN
    IF v_parent IS NOT NULL AND NOT cogmaps_share_a_team(v_parent, v_orig) THEN
        RAISE EXCEPTION 'delegation gate: cogmaps % and % share no team', v_parent, v_orig;
    END IF;
    v_ev := _event_append('delegated_launch', p_emitter, 'kb_cogmaps', v_orig, p_payload,
                          p_invocation => v_inv);
    PERFORM _project_delegated_launch(v_ev, p_payload);
    RETURN v_inv;
END;
$$;

CREATE FUNCTION _project_delegated_launch(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    INSERT INTO kb_invocations (id, opened_by_event_id, status, trigger_kind,
        originating_cogmap_id, parent_cogmap_id, scoped_entity_id, telos_resource_id, opened_at)
    SELECT (p_payload->>'invocation_id')::uuid, p_event, 'open', p_payload->>'trigger_kind',
           (p_payload->>'originating_cogmap_id')::uuid, (p_payload->>'parent_cogmap_id')::uuid,
           (p_payload->>'scoped_entity_id')::uuid, c.telos_resource_id, v_occurred
    FROM kb_cogmaps c WHERE c.id = (p_payload->>'originating_cogmap_id')::uuid;
END;
$$;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(invocation_open_projects_open_row) | test(delegation_gate_blocks_unshared_cogmaps)'`
Expected: PASS (both).

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/02_functions.sql crates/temper-next/tests/invocation_envelope.rs
git commit -m "feat(temper-next): invocation_open + delegation launch-gate"
```

---

### Task 4: `invocation_close` + `_project_invocation_closed`

**Files:**
- Modify: `schema-artifact/02_functions.sql` (append after Task 3's functions)
- Modify: `schema-artifact/seeds/system.yaml` (register the new event type — needed before the close event can append)
- Test: `crates/temper-next/tests/invocation_envelope.rs`

**Interfaces:**
- Consumes: `kb_invocations` (Task 1), `_event_append` (Task 2).
- Produces: `invocation_close(p_payload jsonb, p_emitter uuid) RETURNS uuid`; `_project_invocation_closed(p_event uuid, p_payload jsonb) RETURNS void`. Payload: `{invocation_id, disposition, outcome}` where `disposition ∈ {completed, failed, abandoned}`.

- [ ] **Step 1: Register the `invocation_closed` event type**

In `schema-artifact/seeds/system.yaml`, add to the `event_types:` list (after `delegated_launch`):

```yaml
  - invocation_closed
```

(`delegated_launch` is already registered. The close type must be seeded or `_event_append` raises "event_type ... not seeded".)

- [ ] **Step 2: Write the failing test**

Append to `crates/temper-next/tests/invocation_envelope.rs`:

```rust
#[tokio::test]
async fn invocation_close_sets_terminal_status() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-c").await;
    let inv = Uuid::now_v7();
    sqlx::query_scalar::<_, Uuid>("SELECT invocation_open($1::jsonb, $2)")
        .bind(serde_json::json!({
            "invocation_id": inv, "trigger_kind": "manual",
            "originating_cogmap_id": cog.uuid(), "scoped_entity_id": emitter.uuid(),
        }))
        .bind(emitter.uuid()).fetch_one(&pool).await.unwrap();

    sqlx::query_scalar::<_, Uuid>("SELECT invocation_close($1::jsonb, $2)")
        .bind(serde_json::json!({
            "invocation_id": inv, "disposition": "completed",
            "outcome": {"concepts": 3, "edges": 2},
        }))
        .bind(emitter.uuid()).fetch_one(&pool).await.unwrap();

    let (status, outcome, closed): (String, serde_json::Value, bool) = sqlx::query_as(
        "SELECT status, outcome, closed_at IS NOT NULL FROM kb_invocations WHERE id=$1",
    ).bind(inv).fetch_one(&pool).await.unwrap();
    assert_eq!(status, "completed");
    assert_eq!(outcome["concepts"], 3);
    assert!(closed, "closed_at set");
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(invocation_close_sets_terminal_status)'`
Expected: FAIL — `function invocation_close(jsonb, uuid) does not exist`.

- [ ] **Step 4: Add the functions**

Append to `schema-artifact/02_functions.sql`:

```sql
-- ── Invocation envelope (close) ──────────────────────────────────────────────
-- Closes the run with a terminal disposition + outcome counts. `disposition` must be one of the
-- non-open status values (the kb_invocations CHECK enforces it). Emits `invocation_closed`.
CREATE FUNCTION invocation_close(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_inv uuid := (p_payload->>'invocation_id')::uuid;
        v_orig uuid; v_ev uuid;
BEGIN
    SELECT originating_cogmap_id INTO v_orig FROM kb_invocations WHERE id = v_inv;
    IF v_orig IS NULL THEN RAISE EXCEPTION 'invocation_close: unknown invocation %', v_inv; END IF;
    v_ev := _event_append('invocation_closed', p_emitter, 'kb_cogmaps', v_orig, p_payload,
                          p_invocation => v_inv);
    PERFORM _project_invocation_closed(v_ev, p_payload);
    RETURN v_ev;
END;
$$;

CREATE FUNCTION _project_invocation_closed(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    UPDATE kb_invocations
       SET status = p_payload->>'disposition',
           outcome = p_payload->'outcome',
           closed_by_event_id = p_event,
           closed_at = v_occurred
     WHERE id = (p_payload->>'invocation_id')::uuid;
END;
$$;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(invocation_close_sets_terminal_status)'`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add schema-artifact/02_functions.sql schema-artifact/seeds/system.yaml crates/temper-next/tests/invocation_envelope.rs
git commit -m "feat(temper-next): invocation_close + invocation_closed event type"
```

---

### Task 5: Thread `p_metadata` + `p_invocation` through the four authored-act functions

**Files:**
- Modify: `schema-artifact/02_functions.sql` — `resource_create` (~743), `relationship_assert` (~813), `relationship_fold` (~840), `facet_set` (~875)
- Test: `crates/temper-next/tests/invocation_envelope.rs`

**Interfaces:**
- Produces (new signatures, all backward-compatible via defaults):
  - `resource_create(p_payload jsonb, p_content jsonb, p_emitter uuid, p_metadata jsonb DEFAULT '{}', p_invocation uuid DEFAULT NULL) RETURNS uuid`
  - `relationship_assert(p_payload jsonb, p_emitter uuid, p_metadata jsonb DEFAULT '{}', p_invocation uuid DEFAULT NULL) RETURNS uuid`
  - `relationship_fold(p_payload jsonb, p_emitter uuid, p_metadata jsonb DEFAULT '{}', p_invocation uuid DEFAULT NULL) RETURNS uuid`
  - `facet_set(p_payload jsonb, p_emitter uuid, p_metadata jsonb DEFAULT '{}', p_invocation uuid DEFAULT NULL) RETURNS uuid`

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-next/tests/invocation_envelope.rs`:

```rust
#[tokio::test]
async fn authored_resource_create_stamps_metadata_and_invocation_sql() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-auth").await;
    let inv = Uuid::now_v7();
    let res_id = Uuid::now_v7();
    // resource_create with the two new args (named) — minimal payload + empty content sidecar.
    sqlx::query_scalar::<_, Uuid>(
        "SELECT resource_create($1::jsonb, '{}'::jsonb, $2, p_metadata => $3::jsonb, p_invocation => $4)",
    )
    .bind(serde_json::json!({
        "resource_id": res_id, "title": "Concept X", "origin_uri": "temper://x",
        "home": {"table": "kb_cogmaps", "id": cog.uuid()},
        "owner_profile_id": owner.uuid(), "blocks": [],
    }))
    .bind(emitter.uuid())
    .bind(serde_json::json!({"reasoning": "AUTHORSHIP_SENTINEL", "confidence": "probable"}))
    .bind(inv)
    .fetch_one(&pool).await.unwrap();

    let (meta, got_inv): (serde_json::Value, Option<Uuid>) = sqlx::query_as(
        "SELECT metadata, invocation_id FROM kb_events \
         WHERE event_type_id = (SELECT id FROM kb_event_types WHERE name='resource_created')",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(meta["reasoning"], "AUTHORSHIP_SENTINEL");
    assert_eq!(meta["confidence"], "probable");
    assert_eq!(got_inv, Some(inv));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(authored_resource_create_stamps_metadata_and_invocation_sql)'`
Expected: FAIL — `resource_create(...)` has no `p_metadata`/`p_invocation` argument.

- [ ] **Step 3: Edit the four functions**

In `schema-artifact/02_functions.sql`, for **`resource_create`** replace with:

```sql
CREATE FUNCTION resource_create(p_payload jsonb, p_content jsonb, p_emitter uuid,
                                p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('resource_created', p_emitter,
                          p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_resource_created(v_ev, p_payload, p_content);
END;
$$;
```

For **`relationship_assert`** replace with:

```sql
CREATE FUNCTION relationship_assert(p_payload jsonb, p_emitter uuid,
                                    p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('relationship_asserted', p_emitter,
                          p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_relationship_asserted(v_ev, p_payload);
END;
$$;
```

For **`relationship_fold`** replace with:

```sql
CREATE FUNCTION relationship_fold(p_payload jsonb, p_emitter uuid,
                                  p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_edge uuid := (p_payload->>'edge_id')::uuid;
        v_home_tbl text; v_home uuid;
BEGIN
    SELECT home_anchor_table, home_anchor_id INTO v_home_tbl, v_home
        FROM kb_edges WHERE id = v_edge;
    IF v_home IS NULL THEN
        RAISE EXCEPTION 'relationship_fold: edge % not found', v_edge;
    END IF;
    v_ev := _event_append('relationship_folded', p_emitter, v_home_tbl, v_home, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_relationship_folded(v_ev, p_payload);
END;
$$;
```

For **`facet_set`** replace with:

```sql
CREATE FUNCTION facet_set(p_payload jsonb, p_emitter uuid,
                          p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_anchor_tbl text; v_anchor uuid;
        v_owner uuid := (p_payload#>>'{owner,id}')::uuid;
BEGIN
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_owner ORDER BY (anchor_table='kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'facet_set: resource % has no home to anchor the property event', v_owner;
    END IF;
    v_ev := _event_append('property_asserted', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_property_asserted(v_ev, p_payload);
END;
$$;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(authored_resource_create_stamps_metadata_and_invocation_sql)'`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/02_functions.sql crates/temper-next/tests/invocation_envelope.rs
git commit -m "feat(temper-next): thread authorship metadata + invocation_id through authored-act mutations"
```

---

### Task 6: Rust types — `InvocationId`, `ConfidenceBand`, `Disposition`, `AgentAuthorship`, payloads, `EventKind`

**Files:**
- Modify: `crates/temper-next/src/ids.rs` (add `InvocationId` newtype)
- Modify: `crates/temper-next/src/payloads.rs` (add enums + payload structs)
- Modify: `crates/temper-next/src/events.rs` (add `EventKind` variants + names)
- Test: `crates/temper-next/src/payloads.rs` (a `#[cfg(test)]` serde round-trip)

**Interfaces:**
- Produces:
  - `ids::InvocationId` (transparent UUID newtype)
  - `payloads::ConfidenceBand` (`Tentative|Probable|Confident`, serde `snake_case`)
  - `payloads::Disposition` (`Completed|Failed|Abandoned`, serde `snake_case`)
  - `payloads::AgentAuthorship { reasoning: Option<String>, confidence: ConfidenceBand, rationale: Option<String>, persona: Option<String>, model: Option<String> }`
  - `payloads::DelegatedLaunch { invocation_id: InvocationId, trigger_kind: String, originating_cogmap_id: CogmapId, parent_cogmap_id: Option<CogmapId>, scoped_entity_id: EntityId }`
  - `payloads::InvocationClosed { invocation_id: InvocationId, disposition: Disposition, outcome: serde_json::Value }`
  - `EventKind::{DelegatedLaunch, InvocationClosed}` with names `"delegated_launch"`, `"invocation_closed"`

- [ ] **Step 1: Add the `InvocationId` newtype**

In `crates/temper-next/src/ids.rs`, after an existing `id_newtype!(...)` invocation, add:

```rust
id_newtype!(
    /// A `kb_invocations` row (an agentic-workflow run, accountability grain).
    InvocationId
);
```

- [ ] **Step 2: Write the failing test**

In `crates/temper-next/src/payloads.rs`, add at the bottom:

```rust
#[cfg(test)]
mod authorship_tests {
    use super::*;

    #[test]
    fn authorship_serializes_confidence_band() {
        let a = AgentAuthorship {
            reasoning: Some("because X".into()),
            confidence: ConfidenceBand::Probable,
            rationale: None,
            persona: None,
            model: None,
        };
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!(v["confidence"], "probable");
        let back: AgentAuthorship = serde_json::from_value(v).unwrap();
        assert_eq!(back.confidence, ConfidenceBand::Probable);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo nextest run -p temper-next authorship_serializes_confidence_band`
Expected: FAIL — `ConfidenceBand` / `AgentAuthorship` not found.

- [ ] **Step 4: Add the enums + structs**

In `crates/temper-next/src/payloads.rs`, near the other payload definitions (and import `InvocationId`, `EntityId` from `crate::ids` if not already), add:

```rust
/// The agent's SUBJECTIVE self-assessment of an authored act — a graded band, not a false-precision
/// scalar. Ordinal: Tentative < Probable < Confident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceBand {
    Tentative,
    Probable,
    Confident,
}

/// Per-event agent-authorship metadata — rides in `kb_events.metadata`, NOT the payload, so it is
/// invisible to projections (and thus to affinity math) by construction. `reasoning` is required on
/// structural acts at the AGENT layer (the substrate stores whatever is supplied).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct AgentAuthorship {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    pub confidence: ConfidenceBand,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Terminal disposition of an invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Completed,
    Failed,
    Abandoned,
}

/// `delegated_launch` payload — opens an invocation envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct DelegatedLaunch {
    pub invocation_id: InvocationId,
    pub trigger_kind: String,
    pub originating_cogmap_id: CogmapId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_cogmap_id: Option<CogmapId>,
    pub scoped_entity_id: EntityId,
}

/// `invocation_closed` payload — closes an invocation with a terminal outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct InvocationClosed {
    pub invocation_id: InvocationId,
    pub disposition: Disposition,
    #[serde(default)]
    pub outcome: serde_json::Value,
}
```

(If `payloads.rs` does not already `use crate::ids::{... InvocationId, EntityId ...}`, add them to the existing `use` of `crate::ids`.)

- [ ] **Step 5: Add the `EventKind` variants**

In `crates/temper-next/src/events.rs`, add to `enum EventKind` (after `BlockMutated`):

```rust
    DelegatedLaunch,
    InvocationClosed,
```

And to `as_canonical_name`'s match:

```rust
            EventKind::DelegatedLaunch => "delegated_launch",
            EventKind::InvocationClosed => "invocation_closed",
```

- [ ] **Step 6: Run test + build to verify they pass**

Run: `cargo nextest run -p temper-next authorship_serializes_confidence_band`
Expected: PASS.
Run: `cargo build -p temper-next`
Expected: builds clean.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-next/src/ids.rs crates/temper-next/src/payloads.rs crates/temper-next/src/events.rs
git commit -m "feat(temper-next): InvocationId + authorship/launch/close payload types"
```

---

### Task 7: Rust firing — `EventContext`, `fire_with`, `SeedAction::Invocation{Open,Close}`, `Fired::Invocation`

**Files:**
- Modify: `crates/temper-next/src/events.rs` (`SeedAction`, `Fired`, `fire`, add `EventContext` + `fire_with`)
- Test: `crates/temper-next/tests/invocation_envelope.rs`

**Interfaces:**
- Consumes: `payloads::{AgentAuthorship, DelegatedLaunch, InvocationClosed, Disposition}`, `ids::InvocationId` (Task 6).
- Produces:
  - `events::EventContext { authorship: Option<AgentAuthorship>, invocation: Option<InvocationId> }` (derives `Debug, Default`)
  - `events::fire_with(conn: &mut sqlx::PgConnection, action: SeedAction<'_>, ctx: EventContext) -> Result<Fired>`
  - `events::fire(conn, action) -> Result<Fired>` delegates to `fire_with(conn, action, EventContext::default())`
  - `SeedAction::InvocationOpen { invocation: InvocationId, trigger_kind: &str, originating: CogmapId, parent: Option<CogmapId>, scoped_entity: EntityId, emitter: EntityId }`
  - `SeedAction::InvocationClose { invocation: InvocationId, disposition: Disposition, outcome: serde_json::Value, originating: CogmapId, emitter: EntityId }`
  - `Fired::Invocation(InvocationId)` + `Fired::invocation(self) -> Result<InvocationId>`

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-next/tests/invocation_envelope.rs` (add `use temper_next::events::{fire_with, EventContext};` and `use temper_next::payloads::{AgentAuthorship, ConfidenceBand};` to the header):

```rust
#[tokio::test]
async fn fire_with_authorship_stamps_metadata_via_rust_path() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-rust").await;
    let inv = temper_next::ids::InvocationId::from(Uuid::now_v7());

    // Open the invocation through the typed Rust path.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path TO temper_next, public").execute(&mut *tx).await.unwrap();
    let opened = fire(&mut tx, SeedAction::InvocationOpen {
        invocation: inv, trigger_kind: "manual", originating: cog, parent: None,
        scoped_entity: emitter, emitter,
    }).await.unwrap().invocation().unwrap();
    tx.commit().await.unwrap();
    assert_eq!(opened, inv);

    // Author a resource under the invocation with authorship metadata.
    let blocks = vec![one_chunk_block("concept body")];
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path TO temper_next, public").execute(&mut *tx).await.unwrap();
    fire_with(&mut tx, SeedAction::ResourceCreate {
        title: "C", origin_uri: "temper://c",
        home: temper_next::payloads::AnchorRef::cogmap(cog),
        owner, originator: None, blocks: &blocks, doc_type: Some("concept"), emitter,
    }, EventContext {
        authorship: Some(AgentAuthorship {
            reasoning: Some("RUST_SENTINEL".into()), confidence: ConfidenceBand::Confident,
            rationale: None, persona: Some("steward".into()), model: None,
        }),
        invocation: Some(inv),
    }).await.unwrap();
    tx.commit().await.unwrap();

    let (meta, got_inv): (serde_json::Value, Option<Uuid>) = sqlx::query_as(
        "SELECT metadata, invocation_id FROM kb_events \
         WHERE event_type_id=(SELECT id FROM kb_event_types WHERE name='resource_created')",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(meta["reasoning"], "RUST_SENTINEL");
    assert_eq!(meta["confidence"], "confident");
    assert_eq!(got_inv, Some(inv.uuid()));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(fire_with_authorship_stamps_metadata_via_rust_path)'`
Expected: FAIL — `EventContext` / `fire_with` / `SeedAction::InvocationOpen` / `Fired::invocation` not found.

- [ ] **Step 3: Add `SeedAction` variants, `Fired` variant, `EventContext`, `fire_with`**

In `crates/temper-next/src/events.rs`:

Add to `enum SeedAction<'a>` (after the last existing variant):

```rust
    InvocationOpen {
        invocation: InvocationId,
        trigger_kind: &'a str,
        originating: CogmapId,
        parent: Option<CogmapId>,
        scoped_entity: EntityId,
        emitter: EntityId,
    },
    InvocationClose {
        invocation: InvocationId,
        disposition: payloads::Disposition,
        outcome: serde_json::Value,
        originating: CogmapId,
        emitter: EntityId,
    },
```

Add to `enum Fired`:

```rust
    Invocation(InvocationId),
```

Add to `impl Fired` (an extractor mirroring the others):

```rust
    /// Extract the invocation id an `InvocationOpen` fire produced.
    pub fn invocation(self) -> Result<InvocationId> {
        match self {
            Fired::Invocation(id) => Ok(id),
            other => anyhow::bail!("expected Fired::Invocation, got {other:?}"),
        }
    }
```

Add `InvocationId` to the `use crate::ids::{...}` import. Then add the context type + split `fire`:

```rust
/// Per-fire authored-act context: the agent's authorship metadata (→ kb_events.metadata) and the
/// invocation it is acting under (→ kb_events.invocation_id). Default = a keyboard-holder/system act
/// (no authorship, no invocation), so `fire` callers are unchanged.
#[derive(Debug, Default)]
pub struct EventContext {
    pub authorship: Option<payloads::AgentAuthorship>,
    pub invocation: Option<InvocationId>,
}

impl EventContext {
    fn metadata_json(&self) -> Result<serde_json::Value> {
        Ok(match &self.authorship {
            Some(a) => serde_json::to_value(a)?,
            None => serde_json::json!({}),
        })
    }
    fn invocation_uuid(&self) -> Option<Uuid> {
        self.invocation.map(InvocationId::uuid)
    }
}
```

Change the existing `fire` to delegate, then rename its body to `fire_with`:

```rust
pub async fn fire(conn: &mut sqlx::PgConnection, action: SeedAction<'_>) -> Result<Fired> {
    fire_with(conn, action, EventContext::default()).await
}

pub async fn fire_with(
    conn: &mut sqlx::PgConnection,
    action: SeedAction<'_>,
    ctx: EventContext,
) -> Result<Fired> {
    // ... the existing match body, with the four authored-act arms + two new arms changed below ...
}
```

- [ ] **Step 4: Pass `ctx` through the four authored-act arms**

Inside `fire_with`, in the four authored-act arms, bind `ctx` and extend the SQL call. Compute once at the top of `fire_with`:

```rust
    let ctx_meta = ctx.metadata_json()?;
    let ctx_inv = ctx.invocation_uuid();
```

`SeedAction::ResourceCreate { .. }` — change its `sqlx::query_scalar!("SELECT resource_create($1,$2,$3)", payload, sidecar, emitter.uuid())` to:

```rust
        let id = sqlx::query_scalar!(
            "SELECT resource_create($1,$2,$3,$4,$5)",
            serde_json::to_value(&payload)?, sidecar, emitter.uuid(), ctx_meta, ctx_inv,
        )
        .fetch_one(&mut *conn).await?
        .context("resource_create returned null")?;
```

`SeedAction::RelationshipAssert { .. }` — change `"SELECT relationship_assert($1,$2)"` to `"SELECT relationship_assert($1,$2,$3,$4)"` binding `..., ctx_meta, ctx_inv` as `$3,$4`.

`SeedAction::FacetSet { .. }` — change `"SELECT facet_set($1,$2)"` to `"SELECT facet_set($1,$2,$3,$4)"` binding `..., ctx_meta, ctx_inv` as `$3,$4`.

`SeedAction::RelationshipFold { .. }` — change `"SELECT relationship_fold($1,$2)"` to `"SELECT relationship_fold($1,$2,$3,$4)"` binding `..., ctx_meta, ctx_inv` as `$3,$4`.

(The payload-building Rust in each arm is unchanged — only the SQL string + the two extra binds. The other arms ignore `ctx`.)

- [ ] **Step 5: Add the two new arms**

Inside `fire_with`'s match, add:

```rust
        SeedAction::InvocationOpen {
            invocation, trigger_kind, originating, parent, scoped_entity, emitter,
        } => {
            let payload = payloads::DelegatedLaunch {
                invocation_id: invocation,
                trigger_kind: trigger_kind.to_owned(),
                originating_cogmap_id: originating,
                parent_cogmap_id: parent,
                scoped_entity_id: scoped_entity,
            };
            let id = sqlx::query_scalar!(
                "SELECT invocation_open($1,$2)",
                serde_json::to_value(&payload)?, emitter.uuid(),
            )
            .fetch_one(&mut *conn).await?
            .context("invocation_open returned null")?;
            Ok(Fired::Invocation(InvocationId::from(id)))
        }
        SeedAction::InvocationClose {
            invocation, disposition, outcome, originating: _, emitter,
        } => {
            let payload = payloads::InvocationClosed {
                invocation_id: invocation, disposition, outcome,
            };
            sqlx::query_scalar!(
                "SELECT invocation_close($1,$2)",
                serde_json::to_value(&payload)?, emitter.uuid(),
            )
            .fetch_one(&mut *conn).await?;
            Ok(Fired::Invocation(invocation))
        }
```

(`originating` is carried in the variant for symmetry/clarity but `invocation_close` resolves the cogmap from the stored row; bind it as `_` to avoid an unused warning, or drop the field — keep it for caller readability.)

- [ ] **Step 6: Regenerate the sqlx cache**

Run: `cargo make prepare-next`
Expected: updates `crates/temper-next/.sqlx/` with the new/changed queries (`resource_create($1..$5)`, `relationship_assert/$4`, `facet_set/$4`, `relationship_fold/$4`, `invocation_open`, `invocation_close`).

- [ ] **Step 7: Run test to verify it passes**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(fire_with_authorship_stamps_metadata_via_rust_path)'`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-next/src/events.rs crates/temper-next/.sqlx crates/temper-next/tests/invocation_envelope.rs
git commit -m "feat(temper-next): EventContext + fire_with + invocation SeedActions"
```

---

### Task 8: Replay arms + `kb_invocations` in the byte-identical diff

**Files:**
- Modify: `crates/temper-next/src/replay.rs` (`PROJECTION_DUMPS`, the `replay` match)
- Test: `crates/temper-next/tests/invocation_envelope.rs`

**Interfaces:**
- Consumes: `_project_delegated_launch`, `_project_invocation_closed` (Tasks 3–4), `kb_invocations` (Task 1).
- Produces: replay arms for `"delegated_launch"` / `"invocation_closed"`; `kb_invocations` added to `PROJECTION_DUMPS` so it is part of the replay byte-identical proof.

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-next/tests/invocation_envelope.rs` (add `use temper_next::replay;`):

```rust
#[tokio::test]
async fn invocation_and_authorship_survive_replay() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-replay").await;
    let inv = temper_next::ids::InvocationId::from(Uuid::now_v7());

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path TO temper_next, public").execute(&mut *tx).await.unwrap();
    fire(&mut tx, SeedAction::InvocationOpen {
        invocation: inv, trigger_kind: "manual", originating: cog, parent: None,
        scoped_entity: emitter, emitter,
    }).await.unwrap();
    fire(&mut tx, SeedAction::InvocationClose {
        invocation: inv, disposition: temper_next::payloads::Disposition::Completed,
        outcome: serde_json::json!({"concepts": 0}), originating: cog, emitter,
    }).await.unwrap();
    tx.commit().await.unwrap();

    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_artifact();
    let pool2 = substrate::connect().await.unwrap();
    replay::replay(&pool2, &snap).await.unwrap();
    let after = replay::dump_projections(&pool2).await.unwrap();

    let inv_before = before.iter().find(|(t, _)| t == "kb_invocations").map(|(_, v)| v);
    let inv_after = after.iter().find(|(t, _)| t == "kb_invocations").map(|(_, v)| v);
    assert!(inv_before.is_some(), "kb_invocations must be in the projection dump set");
    assert_eq!(inv_before, inv_after, "kb_invocations must replay byte-identically");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(invocation_and_authorship_survive_replay)'`
Expected: FAIL — either `kb_invocations` absent from the dump set (assert), or `replay: no projector for event type delegated_launch` (bail).

- [ ] **Step 3: Add `kb_invocations` to `PROJECTION_DUMPS`**

In `crates/temper-next/src/replay.rs`, add to the `PROJECTION_DUMPS` array (ordered by `id` — `kb_invocations.id` is payload-carried identity-as-input, so it diffs in full):

```rust
    (
        "kb_invocations",
        "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.id), '[]'::jsonb) FROM kb_invocations t",
    ),
```

- [ ] **Step 4: Add the replay match arms**

In the `replay` function's `match name.as_str()` (before the `other => bail!` arm), add:

```rust
            "delegated_launch" => {
                sqlx::query("SELECT _project_delegated_launch($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
            "invocation_closed" => {
                sqlx::query("SELECT _project_invocation_closed($1,$2)")
                    .bind(id)
                    .bind(&payload)
                    .execute(pool)
                    .await?;
            }
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(invocation_and_authorship_survive_replay)'`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-next/src/replay.rs crates/temper-next/tests/invocation_envelope.rs
git commit -m "feat(temper-next): replay projects + diffs kb_invocations"
```

---

### Task 9: Authorship-invisible-to-affinity proof

**Files:**
- Test: `crates/temper-next/tests/invocation_envelope.rs`

**Interfaces:**
- Consumes: everything above. No new production code — this is the load-bearing correctness proof that authorship never reaches an affinity-input table.

- [ ] **Step 1: Write the test**

Append to `crates/temper-next/tests/invocation_envelope.rs`:

```rust
#[tokio::test]
async fn authorship_is_invisible_to_affinity_inputs() {
    let pool = setup().await;
    let (owner, emitter) = system_actor(&pool).await;
    let cog = genesis(&pool, owner, emitter, "map-invis").await;
    let inv = temper_next::ids::InvocationId::from(Uuid::now_v7());

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path TO temper_next, public").execute(&mut *tx).await.unwrap();
    fire(&mut tx, SeedAction::InvocationOpen {
        invocation: inv, trigger_kind: "manual", originating: cog, parent: None,
        scoped_entity: emitter, emitter,
    }).await.unwrap();
    let blocks = vec![one_chunk_block("invisibility body")];
    fire_with(&mut tx, SeedAction::ResourceCreate {
        title: "Z", origin_uri: "temper://z",
        home: temper_next::payloads::AnchorRef::cogmap(cog),
        owner, originator: None, blocks: &blocks, doc_type: Some("concept"), emitter,
    }, EventContext {
        authorship: Some(AgentAuthorship {
            reasoning: Some("INVIS_SENTINEL".into()), confidence: ConfidenceBand::Tentative,
            rationale: Some("INVIS_SENTINEL".into()), persona: None, model: None,
        }),
        invocation: Some(inv),
    }).await.unwrap();
    tx.commit().await.unwrap();

    // Authorship IS in the ledger metadata.
    let in_meta: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events WHERE metadata->>'reasoning' = 'INVIS_SENTINEL'",
    ).fetch_one(&pool).await.unwrap();
    assert!(in_meta >= 1, "authorship must be recorded in kb_events.metadata");

    // Authorship is NOT in ANY affinity-input projection (resources / edges / properties).
    for table in ["kb_resources", "kb_edges", "kb_properties"] {
        let leaked: i64 = sqlx::query_scalar(&format!(
            "SELECT count(*) FROM {table} t WHERE to_jsonb(t)::text LIKE '%INVIS_SENTINEL%'",
        )).fetch_one(&pool).await.unwrap();
        assert_eq!(leaked, 0, "{table} must not contain authorship text (invisible to affinity)");
    }
}
```

- [ ] **Step 2: Run the full new test file**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(authorship_is_invisible_to_affinity_inputs)'`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-next/tests/invocation_envelope.rs
git commit -m "test(temper-next): prove authorship invisible to affinity-input projections"
```

---

### Task 10: Append-only forward migration mirroring the artifact deltas

**Files:**
- Create: `migrations/20260618000001_temper_next_invocation_envelope.sql`
- Test: `crates/temper-next/tests/schema_drift.rs` (existing `migrations_reconstruct_artifact_schema` — no edit, must pass)

**Interfaces:**
- Consumes: the artifact deltas from Tasks 1–5.
- Produces: a forward migration that, applied after the existing lineage, reconstructs the modified artifact (so `namespace_fingerprint` from migrations == from artifact). Function bodies are **byte-identical** to the artifact (unqualified, under `SET search_path`).

- [ ] **Step 1: Run the drift guard to verify it fails**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(migrations_reconstruct_artifact_schema)'`
Expected: FAIL — the artifact now has `kb_invocations` + `kb_events.invocation_id` + the new/changed functions that the migration lineage does not yet produce.

- [ ] **Step 2: Write the migration**

Create `migrations/20260618000001_temper_next_invocation_envelope.sql`:

```sql
-- temper_next — invocation envelope + agent-authorship metadata.
--
-- Append-only to the frozen temper_next lineage (install 20260613000001 + 4c 20260616000001 +
-- can_modify 20260617000001 precede this). The artifact (schema-artifact/01_schema.sql +
-- 02_functions.sql) is the design-master; this is its faithful append. The semantic drift guard
-- (crates/temper-next/tests/schema_drift.rs) proves the lineage reconstructs the artifact, so the
-- function BODIES here are byte-identical to the artifact (unqualified names resolving against the
-- SET search_path below — never schema-qualify the body; that is what pg_get_functiondef fingerprints).
-- Idempotent: ADD COLUMN IF NOT EXISTS / CREATE TABLE IF NOT EXISTS / CREATE OR REPLACE FUNCTION.
SET search_path TO temper_next, public;

ALTER TABLE kb_events ADD COLUMN IF NOT EXISTS invocation_id UUID;
CREATE INDEX IF NOT EXISTS idx_kb_events_invocation ON kb_events(invocation_id);

CREATE TABLE IF NOT EXISTS kb_invocations (
    id                     UUID PRIMARY KEY,
    opened_by_event_id     UUID NOT NULL REFERENCES kb_events(id),
    status                 TEXT NOT NULL DEFAULT 'open'
                               CHECK (status IN ('open','completed','failed','abandoned')),
    trigger_kind           TEXT NOT NULL,
    originating_cogmap_id  UUID NOT NULL REFERENCES kb_cogmaps(id),
    parent_cogmap_id       UUID REFERENCES kb_cogmaps(id),
    scoped_entity_id       UUID NOT NULL REFERENCES kb_entities(id),
    telos_resource_id      UUID NOT NULL REFERENCES kb_resources(id),
    outcome                JSONB,
    opened_at              TIMESTAMPTZ NOT NULL,
    closed_by_event_id     UUID REFERENCES kb_events(id),
    closed_at              TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_kb_invocations_cogmap ON kb_invocations(originating_cogmap_id);
CREATE INDEX IF NOT EXISTS idx_kb_invocations_status ON kb_invocations(status);

-- _event_append (extended) — body byte-identical to schema-artifact/02_functions.sql.
CREATE OR REPLACE FUNCTION _event_append(
    p_type_name text, p_emitter uuid, p_anchor_table text, p_anchor_id uuid,
    p_payload jsonb,
    p_references jsonb DEFAULT '[]'::jsonb,
    p_correlation uuid DEFAULT NULL,
    p_payload_version int DEFAULT 1,
    p_metadata jsonb DEFAULT '{}'::jsonb,
    p_invocation uuid DEFAULT NULL
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_et uuid; v_ev uuid := uuid_generate_v7();
BEGIN
    SELECT id INTO v_et FROM kb_event_types WHERE name = p_type_name;
    IF v_et IS NULL THEN RAISE EXCEPTION 'event_type % not seeded', p_type_name; END IF;
    INSERT INTO kb_events (id, event_type_id, emitter_entity_id,
                           producing_anchor_table, producing_anchor_id,
                           payload, "references", payload_version, correlation_id,
                           metadata, invocation_id)
    VALUES (v_ev, v_et, p_emitter, p_anchor_table, p_anchor_id,
            p_payload, p_references, p_payload_version, COALESCE(p_correlation, v_ev),
            p_metadata, p_invocation);
    RETURN v_ev;
END;
$$;
```

Then **append to the same migration file** the four extended authored-act functions (`resource_create`, `relationship_assert`, `relationship_fold`, `facet_set`) and the two new envelope functions + projections (`invocation_open`, `_project_delegated_launch`, `invocation_close`, `_project_invocation_closed`), each as `CREATE OR REPLACE FUNCTION` with a body **copied verbatim** from `schema-artifact/02_functions.sql` (the exact text from Tasks 3–5). Do not schema-qualify any name in the bodies.

- [ ] **Step 3: Run the drift guard to verify it passes**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(migrations_reconstruct_artifact_schema)'`
Expected: PASS — migration lineage fingerprint == artifact fingerprint.

> If it fails with a function-body mismatch, the migration body text differs from the artifact (whitespace / a stray schema-qualification). Diff the `pg_get_functiondef` output or re-copy the artifact body verbatim.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260618000001_temper_next_invocation_envelope.sql
git commit -m "feat(temper-next): forward migration for invocation envelope + authorship (drift guard green)"
```

---

### Task 11: Full-suite green + cache + final gate

**Files:**
- Possibly Modify: `crates/temper-next/.sqlx/` (regen), `schema-artifact/03_seed.sql` (parity)
- Test: the whole `temper-next` artifact suite + `cargo make check`

- [ ] **Step 1: Parity — register `invocation_closed` in `03_seed.sql`**

For registry parity with `system.yaml` (the legacy read-path seed lists the same vocabulary), add `'invocation_closed'` to the `INSERT INTO kb_event_types (name) VALUES (...)` list in `schema-artifact/03_seed.sql` (the block near lines 30–38, alongside the existing `'delegated_launch'`).

- [ ] **Step 2: Regenerate the sqlx cache (idempotent re-check)**

Run: `cargo make prepare-next`
Expected: no uncommitted drift beyond Task 7's; if anything changed, it is the new query rows.

- [ ] **Step 3: Run the full artifact write-path suite**

Run: `cargo nextest run -p temper-next --features artifact-tests`
Expected: PASS — the whole `temper-next-write` group (existing tests + the new `invocation_envelope` tests + `schema_drift`) green. No reset races (serial group).

- [ ] **Step 4: Run the JSON-Schema snapshot guard**

Run: `cargo nextest run -p temper-next --features scenario-schema`
Expected: PASS — the new payload types are NOT referenced by `Scenario`/`Seed`, so the committed `scenario.schema.json`/`seed.schema.json` snapshots are unchanged. (If this fails, a new type was accidentally wired into the scenario model — revert that; it is out of scope.)

- [ ] **Step 5: Final workspace gate**

Run: `cargo make check`
Expected: PASS — Rust fmt + clippy (`-D warnings`) + docs + machete, against the committed `.sqlx` cache (`SQLX_OFFLINE=true`).

- [ ] **Step 6: Commit**

```bash
git add schema-artifact/03_seed.sql crates/temper-next/.sqlx
git commit -m "chore(temper-next): invocation_closed seed parity + cache; suite green"
```

---

## Self-Review

**1. Spec coverage** (research doc `2026-06-18-agentic-workflows-on-temper-via-vercel-eve`, the "invocation envelope + authorship" thread):
- Invocation envelope = accountability grain (trigger, delegation binding, telos/scope, correlated mutation events, terminal outcome) → Tasks 1, 3, 4 (`kb_invocations` columns + open/close). ✓
- Correlated mutation events under the run → Task 1 (`invocation_id` column) + Tasks 5, 7 (threaded). ✓
- Delegation binding + launch gate (`cogmaps_share_a_team`) → Task 3. ✓
- Agent-authorship metadata: reasoning + graded-band confidence + rationale → Tasks 5–7. ✓
- Invisible to affinity by construction → Tasks 1/5 (metadata column, never projected) + Task 9 (proof). ✓
- Replayable → Task 8 (replay arms + byte-identical diff). ✓
- Runtime owns orchestration, stored as opaque markers, never depended on → satisfied by NOT modeling steps (envelope only); `metadata` is opaque to projections. ✓
- Identity-as-input for the run id → Tasks 3, 6, 7. ✓

**2. Placeholder scan:** No "TBD"/"add error handling"/"similar to Task N". The only "copy verbatim from the artifact" instruction (Task 10, appended function bodies) is deliberate and required by the drift guard's byte-identical fingerprint; the exact bodies are given in Tasks 3–5. ✓

**3. Type consistency:** `InvocationId` (ids.rs) used uniformly across payloads/events/tests; `ConfidenceBand`/`Disposition` serde `snake_case` matches the SQL string assertions (`"probable"`, `"completed"`); `fire_with(conn, action, ctx)` + `EventContext { authorship, invocation }` consistent between Task 7 definition and Tasks 7/9 usage; `Fired::Invocation(InvocationId)` + `.invocation()` consistent; SQL signatures (`resource_create/$5`, `relationship_assert|facet_set|relationship_fold/$4`, `invocation_open|invocation_close/$2`) consistent between Tasks 2–5 (SQL), 7 (Rust binds), and the `prepare-next` cache. ✓

## Execution Handoff

(Filled at handoff time.)
