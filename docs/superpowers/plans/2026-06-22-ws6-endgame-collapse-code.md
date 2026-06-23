# WS6 Endgame Collapse — Code Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse the `temper_next`/`public` two-schema split to a single canonical schema in the code: re-home the cognitive-map substrate out of the migration scaffolding, delete the split machinery, de-qualify all SQL, rewrite the two raw-pool leak services onto the substrate shape, and bring the surface-parity acceptance gate green — so the live schema rename (separate operator runbook) ships against ready code.

**Architecture:** Three crates carry the split: `temper-next` (substrate + scaffolding wearing one crate), `temper-api` (the flag/selection dispatch + the leak services), and `temper-mcp` (dispatches through the same selection). The collapse is mostly **deletion + de-qualification**, with two genuine rewrites (`graph_service`, `event_service`). Development runs against the local Docker DB with the connection default pointed at the substrate schema (`temper_next` today; `public` after the live rename), so de-qualified SQL resolves to the substrate exactly as it will in production.

**Tech Stack:** Rust (axum, sqlx, tokio), PostgreSQL 17 (Neon prod) / 18 (local Docker, port 5437), pgvector, cargo-make + cargo-nextest, SQLX_OFFLINE caches per crate.

> **Resume anchor (read first).** Full session context that produced this plan:
> `temper resource show 019ef3e9-f407-7cd1-866a-0ee06448780a` — the decisions, commit
> history, and the migration-state gotchas. **During the migration `temper resource
> list`/`search` are broken** (doctype/context filters don't constrain); `resource show
> <uuid>` works (trailing-UUID resolution). To find other recent docs, query Neon
> directly: `neonctl psql --project-id crimson-fog-23541670 --org-id
> org-wild-snow-32921543 --role-name neondb_owner -- -c "..."` against
> `temper_next.kb_resources` (backend is `next`; prod is live on `temper_next`).

## Global Constraints

- **Service layer owns SQL.** All SQL lives in `temper-api/src/services/`. Writes route through the backend trait; reads (list/show/meta/search/graph/events) stay service-direct. Never inline `sqlx::query!()` in a handler or MCP tool.
- **Typed structs over inline JSON.** No `serde_json::json!()` for known shapes. Define a struct.
- **Shared wire types in `temper-core`** with `ts-rs` derives; never mirror a Rust struct as a hand-written zod/TS type.
- **`#[expect(lint, reason = "...")]`** not `#[allow]`.
- **Atomic cross-crate commits.** The pre-commit hook gates whole-workspace clippy; a change that breaks compilation across `temper-api`/`temper-mcp`/`tests/e2e`/deploy adapters must land as **one** commit (Tasks 6, 8, 9 below).
- **sqlx caches are per-crate.** After changing SQL, regenerate the matching cache: `cargo make prepare-api` (temper-api), `cargo make prepare-next` (temper-next), `cargo make prepare-e2e` (e2e). Never `cargo sqlx prepare --workspace` (clobbers per-crate caches). All `cargo make` tasks force `SQLX_OFFLINE=true`.
- **Dev DB:** `postgresql://temper:temper@localhost:5437/temper_development`. Collapsed-schema dev sets the connection search_path to the substrate (Task 1).
- **No behavior change in Phase 1.** Re-home tasks move code without changing behavior; the existing suite stays green.
- **The live schema DDL (rename/extension-homing/drop) is NOT in this plan** — it is the operator runbook `docs/guides/ws6-endgame-collapse-runbook.md`. This plan delivers the *code* that the runbook's redeploy step ships.

---

## File Structure

**Re-homed (Phase 1):**
- Create `crates/temper-next/src/keys.rs` — the property-tier classifier (`key_fate`, `KeyFate`, `MANAGED_PROPERTY_KEYS`, `is_managed_property_key`), moved out of `synthesis/key_fate.rs`. Live write-path code; not scaffolding.
- Create `crates/temper-next/src/text.rs` — `slugify`, moved out of `synthesis/bootstrap.rs`. Shared by `writes` + `scenario`.
- Create `crates/temper-next/src/parity.rs` — `ReadChunk`/`reconstruct_body`/`new_substrate_chunks` (+ `BodyMismatch`/`ParityReport`/`body_parity_report`), moved out of `synthesis/parity.rs`, next to `readback/`. Retires with `readback/` at shim-exit.

**Deleted (Phases 1 + 3):**
- `crates/temper-next/src/synthesis/` (whole dir, after survivors moved).
- `crates/temper-api/src/backend/{selection.rs, read_selector.rs}` and `services/backend_selection_service.rs`.

**Rewritten (Phases 2–3):**
- `schema-artifact/02_functions.sql` — gains ported `graph_traverse` + `graph_subgraph_nodes`.
- `crates/temper-api/src/services/graph_service.rs`, `event_service.rs`.
- `crates/temper-api/src/backend/db_backend.rs` collapses to *the* backend (was `NextBackend`).
- `crates/temper-next/src/{writes.rs, substrate.rs}`, `temper-api/src/backend/next_backend.rs` — de-qualified, search_path hooks removed.

---

## Phase 1 — Re-home substrate survivors, delete synthesis

No schema dependency; the existing suite stays green throughout. Each task is its own commit.

### Task 1: Establish the collapsed-schema dev environment

**Files:**
- Modify: `Makefile.toml` (add a `db-collapsed` task)
- Create: `docs/guides/ws6-collapsed-dev-env.md`

**Interfaces:**
- Produces: `cargo make db-collapsed` — loads the substrate artifact into the local DB and prints the `DATABASE_URL` to export for collapsed-schema dev.

- [ ] **Step 1: Add the `db-collapsed` cargo-make task**

```toml
# Makefile.toml — append
[tasks.db-collapsed]
description = "Load the substrate artifact into the local DB for collapsed-schema dev (search_path defaulted to temper_next)."
env = { SQLX_OFFLINE = "false" }
script = '''
DB="${DATABASE_URL:-postgresql://temper:temper@localhost:5437/temper_development}"
for f in 00_namespace_reset 01_schema 02_functions; do
  psql "$DB" -v ON_ERROR_STOP=1 -f "schema-artifact/$f.sql" >/dev/null
done
echo "substrate loaded into temper_next."
echo "For collapsed-schema dev, export:"
echo "  export DATABASE_URL=\"${DB}?options=-csearch_path%3Dtemper_next,public\""
'''
```

- [ ] **Step 2: Run it and confirm the substrate loads**

Run: `cargo make db-collapsed`
Expected: `substrate loaded into temper_next.` and the export hint, exit 0.

- [ ] **Step 3: Document the dev loop**

Write `docs/guides/ws6-collapsed-dev-env.md`: one screen explaining that collapsed-schema dev points the connection default at `temper_next` (post-rename this becomes `public`), so de-qualified SQL resolves to the substrate; legacy `public.*` stays present but unreferenced. Include the `export DATABASE_URL=...search_path%3Dtemper_next,public` line and a note that this mirrors the runbook's post-rename state.

- [ ] **Step 4: Commit**

```bash
git add Makefile.toml docs/guides/ws6-collapsed-dev-env.md
git commit -m "WS6 collapse: local collapsed-schema dev env (substrate as connection default)"
```

### Task 2: Re-home `key_fate` → `temper-next/src/keys.rs`

**Files:**
- Create: `crates/temper-next/src/keys.rs`
- Delete: `crates/temper-next/src/synthesis/key_fate.rs`
- Modify: `crates/temper-next/src/lib.rs` (add `pub mod keys;`), `crates/temper-api/src/backend/next_backend.rs:27`, `crates/temper-next/src/readback/mod.rs:29`, `crates/temper-next/src/synthesis/mod.rs:196`

**Interfaces:**
- Produces: `temper_next::keys::{key_fate, KeyFate, is_managed_property_key, MANAGED_PROPERTY_KEYS}` — same items, new path.

- [ ] **Step 1: Move the file verbatim**

```bash
git mv crates/temper-next/src/synthesis/key_fate.rs crates/temper-next/src/keys.rs
```
The module body is unchanged (pure functions, no SQL): `KeyFate` enum, `MANAGED_PROPERTY_KEYS`, `is_managed_property_key`, `key_fate`.

- [ ] **Step 2: Register the module, drop the old declaration**

In `crates/temper-next/src/lib.rs` add `pub mod keys;` (alongside the other `pub mod`s). In `crates/temper-next/src/synthesis/mod.rs` remove `pub mod key_fate;` (line 15).

- [ ] **Step 3: Repoint the three importers**

- `crates/temper-api/src/backend/next_backend.rs:27`: `use temper_next::synthesis::key_fate::{key_fate, KeyFate};` → `use temper_next::keys::{key_fate, KeyFate};`
- `crates/temper-next/src/readback/mod.rs:29`: `use crate::synthesis::key_fate::is_managed_property_key;` → `use crate::keys::is_managed_property_key;`
- `crates/temper-next/src/synthesis/mod.rs:196`: `key_fate::key_fate(key) == key_fate::KeyFate::Property` → `crate::keys::key_fate(key) == crate::keys::KeyFate::Property` (this caller is deleted in Task 5, but must compile now).

- [ ] **Step 4: Verify compile + existing tests**

Run: `cargo make check`
Expected: clean (no unresolved `synthesis::key_fate`).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "WS6 collapse: re-home key_fate out of synthesis/ to temper-next/keys.rs"
```

### Task 3: Re-home `slugify` → `temper-next/src/text.rs`

**Files:**
- Create: `crates/temper-next/src/text.rs`
- Modify: `crates/temper-next/src/lib.rs`, `crates/temper-next/src/synthesis/bootstrap.rs:355` (remove the fn), `crates/temper-next/src/writes.rs:22`, `crates/temper-next/src/scenario/access/loader.rs:136`

**Interfaces:**
- Produces: `temper_next::text::slugify(s: &str) -> String` — same lowercase alphanumeric-or-dash behavior.

- [ ] **Step 1: Write the failing test for the moved function**

Create `crates/temper-next/src/text.rs`:
```rust
//! Pure text helpers shared across the substrate (writes, scenario).

/// Lowercase, alphanumeric-or-dash slug. Moved verbatim from the retired
/// `synthesis::bootstrap::slugify` (the synthesis scaffolding is deleted at collapse).
pub fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slugify_lowercases_and_dashes() {
        assert_eq!(slugify("Hello World!"), "hello-world");
        assert_eq!(slugify("  A  B  "), "a-b");
    }
}
```
> Copy the **exact** body from `synthesis/bootstrap.rs:355` rather than the reconstruction above if it differs; the test pins the contract either way.

- [ ] **Step 2: Run the test to verify it passes (pure move)**

Run: `cargo nextest run -p temper-next text::tests`
Expected: PASS (it is a pure function).

- [ ] **Step 3: Register module, remove the old fn, repoint importers**

- `lib.rs`: add `pub mod text;`
- `synthesis/bootstrap.rs:355`: delete the `slugify` fn; if `bootstrap.rs` itself calls it, replace with `crate::text::slugify`.
- `writes.rs:22`: `use crate::synthesis::bootstrap::slugify;` → `use crate::text::slugify;`
- `scenario/access/loader.rs:136`: `crate::synthesis::bootstrap::slugify(&c.name)` → `crate::text::slugify(&c.name)`

- [ ] **Step 4: Verify**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "WS6 collapse: re-home slugify out of synthesis/ to temper-next/text.rs"
```

### Task 4: Re-home `parity` helpers → `temper-next/src/parity.rs`

**Files:**
- Create: `crates/temper-next/src/parity.rs` (move from `synthesis/parity.rs`)
- Modify: `crates/temper-next/src/lib.rs`, `crates/temper-next/src/readback/mod.rs:608-609`, `crates/temper-next/src/synthesis/mod.rs:362`

**Interfaces:**
- Produces: `temper_next::parity::{ReadChunk, reconstruct_body, new_substrate_chunks, BodyMismatch, ParityReport, body_parity_report}` — same items, new path. Retires with `readback/` at shim-exit.

- [ ] **Step 1: Move the file**

```bash
git mv crates/temper-next/src/synthesis/parity.rs crates/temper-next/src/parity.rs
```

- [ ] **Step 2: Register, repoint importers**

- `lib.rs`: add `pub mod parity;`
- `synthesis/mod.rs`: remove `pub mod parity;` (line 16); the §8-gate call at `:362` (`parity::body_parity_report()`) → `crate::parity::body_parity_report()` (deleted with synthesis in Task 5, must compile now).
- `readback/mod.rs:608-609`: `crate::synthesis::parity::{new_substrate_chunks, reconstruct_body}` → `crate::parity::{...}`.

- [ ] **Step 3: Verify (parity tests still green)**

Run: `cargo make check && cargo nextest run -p temper-next parity`
Expected: clean; parity unit tests pass.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "WS6 collapse: re-home parity helpers out of synthesis/ next to readback"
```

### Task 5: Delete `synthesis/` + the `Synthesize` subcommand

**Files:**
- Delete: `crates/temper-next/src/synthesis/` (`bootstrap.rs`, `mod.rs`, `source.rs` — `key_fate.rs`/`parity.rs` already moved)
- Modify: `crates/temper-next/src/lib.rs` (drop `pub mod synthesis;`), `crates/temper-next/src/main.rs` (drop `Synthesize`), delete synthesis-only tests

**Interfaces:**
- Consumes: nothing external still references `synthesis::*` (Tasks 2–4 moved every survivor).
- Produces: `temper-next` binary with only the `Materialize` subcommand.

- [ ] **Step 1: Confirm no external references remain**

Run: `rg -n "synthesis::|mod synthesis|Synthesize|bootseed::" crates/ tests/`
Expected: only hits inside `crates/temper-next/src/synthesis/`, `main.rs`'s `Synthesize` arm, and synthesis-only tests. If anything else appears, re-home it before deleting (do not proceed).

- [ ] **Step 2: Delete the directory + module**

```bash
git rm -r crates/temper-next/src/synthesis
```
In `lib.rs` remove `pub mod synthesis;`.

- [ ] **Step 3: Remove the `Synthesize` subcommand**

In `crates/temper-next/src/main.rs`: delete the `Synthesize { limit }` variant from the `Cmd` enum (lines ~16-19), its match arm (lines ~54-60), and the `synthesis::{self, RunOpts}` import (line 3). Keep `Materialize`.

- [ ] **Step 4: Drop now-dead `bootseed::system_event_type_names`**

Run: `rg -n "system_event_type_names" crates/temper-next`
If its only definition+caller were inside synthesis, the symbol is now gone with the dir. If a `bootseed` module elsewhere still defines it with no caller, delete the dead fn (and the module if it is now empty). Confirm with `cargo make check` (dead-code lint).

- [ ] **Step 5: Retire synthesis-only tests**

Run: `rg -l "synthesis::run|RunOpts|Synthesize" crates/temper-next/tests`
Delete any test file that exercises the synthesis run path specifically. **Keep** `parity_reads.rs`/`corpus_parity_reads.rs` if they exercise `readback`/`parity` (now re-homed) rather than synthesis — repoint their imports to `crate::parity` if needed.

- [ ] **Step 6: Verify**

Run: `cargo make check && cargo nextest run -p temper-next`
Expected: clean; no `synthesis` references; binary builds with `Materialize` only.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "WS6 collapse: delete synthesis/ scaffolding + the Synthesize subcommand"
```

---

## Phase 2 — Additive prep (lands before the flip, breaks nothing)

### Task 6: Port `graph_traverse` + `graph_subgraph_nodes` into the substrate

**Files:**
- Modify: `schema-artifact/02_functions.sql` (add both functions)
- Test: `crates/temper-next/tests/graph_functions_test.rs` (new; gated `artifact-tests`)

**Interfaces:**
- Produces (SQL): `graph_traverse(p_profile uuid, p_seed_ids uuid[], p_depth int)` and `graph_subgraph_nodes(p_profile uuid, p_context_name varchar, p_aggregator_types text[], p_depth int)` resolving against the substrate, with the SAME output columns `graph_service` already binds.

These are **additive** — new functions in the substrate; nothing references them until Task 8. The legacy `public` copies are untouched.

- [ ] **Step 1: Write the failing test (function exists + returns substrate rows)**

Create `crates/temper-next/tests/graph_functions_test.rs`:
```rust
#![cfg(feature = "artifact-tests")]
//! graph_traverse / graph_subgraph_nodes resolve against the substrate.
//! Owns the temper_next namespace (resets 01+02, seeds a tiny context+two edged resources).
// ... harness: reset namespace, seed a context with two resources + one edge between them ...

#[sqlx::test]
async fn subgraph_nodes_returns_seeded_resources(pool: sqlx::PgPool) {
    // seed via resource_create + relationship_assert (02_functions mutation fns)
    // then:
    let rows = sqlx::query("SELECT resource_id, slug, doc_type, edge_count FROM graph_subgraph_nodes($1,$2,$3::text[],$4::int)")
        .bind(profile_id).bind("temper").bind(&["concept".to_string()]).bind(2_i32)
        .fetch_all(&pool).await.expect("subgraph_nodes runs");
    assert!(!rows.is_empty(), "seeded aggregator resource is returned");
}
```

- [ ] **Step 2: Run it, verify it fails (function absent)**

Run: `cargo nextest run -p temper-next --features artifact-tests subgraph_nodes_returns_seeded_resources`
Expected: FAIL — `function graph_subgraph_nodes(...) does not exist`.

- [ ] **Step 3: Add the ported functions to `02_functions.sql`**

Append, re-expressed onto the substrate (translations: context via `kb_resource_homes`→`kb_contexts`; doc_type/session/stage via `kb_properties`; first_chunk via `kb_chunks`→`kb_chunk_content`→`kb_content_blocks`; edges via `kb_edges` with `source_table/target_table='kb_resources'`; slug derived from title; stage property key = `temper-stage`):

```sql
-- Ported from migrations/20260522100002_edges_as_projection.sql (legacy public shape),
-- re-expressed onto the substrate. resources_visible_to is the 1-arg substrate form.
CREATE OR REPLACE FUNCTION graph_traverse(p_profile uuid, p_seed_ids uuid[], p_depth int)
RETURNS TABLE (resource_id uuid, source_id uuid, target_id uuid,
               edge_kind edge_kind, polarity edge_polarity, label text, depth int)
LANGUAGE sql STABLE AS $$
  WITH RECURSIVE visible AS (SELECT rv.resource_id AS id FROM resources_visible_to(p_profile) rv),
  walk AS (
    SELECT e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, 1 AS depth
      FROM kb_edges e
     WHERE e.source_table='kb_resources' AND e.target_table='kb_resources'
       AND e.source_id = ANY(p_seed_ids) AND NOT e.is_folded
       AND e.source_id IN (SELECT id FROM visible) AND e.target_id IN (SELECT id FROM visible)
    UNION
    SELECT e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, w.depth+1
      FROM kb_edges e JOIN walk w ON e.source_id = w.target_id
     WHERE e.source_table='kb_resources' AND e.target_table='kb_resources'
       AND NOT e.is_folded AND w.depth < p_depth
       AND e.target_id IN (SELECT id FROM visible)
  )
  SELECT w.target_id, w.source_id, w.target_id, w.edge_kind, w.polarity, w.label, w.depth FROM walk w;
$$;

CREATE OR REPLACE FUNCTION graph_subgraph_nodes(
  p_profile uuid, p_context_name varchar, p_aggregator_types text[], p_depth int)
RETURNS TABLE (resource_id uuid, slug varchar, title text, doc_type varchar,
               edge_count int, session_count int, first_chunk text, stage_raw text)
LANGUAGE sql STABLE AS $$
  WITH ctx AS (SELECT id FROM kb_contexts WHERE name = p_context_name),
  doc AS (  -- doc_type property per resource
    SELECT p.owner_id AS rid, p.property_value #>> '{}' AS dt
      FROM kb_properties p
     WHERE p.owner_table='kb_resources' AND p.property_key='doc_type' AND NOT p.is_folded),
  seeds AS (
    SELECT r.id
      FROM kb_resources r
      JOIN kb_resource_homes h ON h.resource_id=r.id AND h.anchor_table='kb_contexts'
      JOIN ctx ON ctx.id = h.anchor_id
      JOIN doc ON doc.rid = r.id
     WHERE r.is_active AND doc.dt = ANY(p_aggregator_types)),
  walked AS (
    SELECT DISTINCT t.resource_id AS id
      FROM graph_traverse(p_profile, ARRAY(SELECT id FROM seeds), p_depth) t
    UNION SELECT id FROM seeds),
  nodes AS (
    SELECT r.id, doc.dt AS doc_type, r.title FROM kb_resources r
      JOIN walked w ON w.id=r.id JOIN doc ON doc.rid=r.id
     WHERE r.is_active AND doc.dt <> 'session')  -- sessions are not nodes
  SELECT
    n.id,
    sluggify(n.title)::varchar AS slug,           -- slug retired in substrate; derive from title
    n.title,
    n.doc_type::varchar,
    (SELECT count(*)::int FROM kb_edges e
       WHERE NOT e.is_folded AND e.source_table='kb_resources' AND e.target_table='kb_resources'
         AND (e.source_id=n.id OR e.target_id=n.id)) AS edge_count,
    0::int AS session_count,                       -- session adjacency: 0 until re-modelled (see note)
    (SELECT cc.content FROM kb_chunks ch
       JOIN kb_content_blocks b ON b.id=ch.block_id
       JOIN kb_chunk_content cc ON cc.chunk_id=ch.id
      WHERE ch.resource_id=n.id AND ch.is_current AND NOT b.is_folded
      ORDER BY b.seq, ch.chunk_index LIMIT 1) AS first_chunk,
    (SELECT sp.property_value #>> '{}' FROM kb_properties sp
      WHERE sp.owner_table='kb_resources' AND sp.owner_id=n.id
        AND sp.property_key='temper-stage' AND NOT sp.is_folded LIMIT 1) AS stage_raw
  FROM nodes n;
$$;
```
> **`sluggify`** is the substrate's existing SQL slug helper if present (grep `02_functions.sql` for `sluggify`/`slugify`); if absent, inline a `lower(regexp_replace(...))`. **`session_count`** is set to 0 (the legacy session-adjacency join depended on `kb_doc_types`); re-modelling session adjacency onto properties is a follow-up (note it in the function comment) — `GraphNode.session_count` stays valid (0), so the UI degrades gracefully.

- [ ] **Step 4: Run the test, verify it passes**

Run: `cargo nextest run -p temper-next --features artifact-tests subgraph_nodes_returns_seeded_resources`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/02_functions.sql crates/temper-next/tests/graph_functions_test.rs
git commit -m "WS6 collapse: port graph_traverse + graph_subgraph_nodes onto the substrate"
```

### Task 7: Deprecate the `/api/events?` list query (atomic cross-crate removal)

**Files:**
- Modify: `crates/temper-api/src/services/event_service.rs` (remove `list_visible`), `handlers/events.rs` (remove `list`), `routes.rs:86` (remove route), `openapi.rs`
- Modify: `crates/temper-core/src/types/api.rs` (remove `EventListParams`, `EventRow`; keep `EventCursorResponse`)
- Modify: `crates/temper-mcp/src/...` (remove the `list_events` tool + registration), `crates/temper-client/src/events.rs` (remove `list`)
- Delete/reduce: `tests/e2e/tests/events_test.rs`; the events assertion in `mcp_round_trip_test.rs`
- Modify: any `temper events`-list CLI command (grep)

**Interfaces:**
- Consumes: nothing — this is pure removal of an unused feed.
- Produces: only the cursor remains on the events surface.

This is one atomic commit (removing `EventListParams`/`EventRow` breaks every consumer simultaneously).

- [ ] **Step 1: Enumerate every consumer**

Run: `rg -n "EventListParams|EventRow|list_visible|/api/events\"|list_events|events::list" crates/ tests/ api/`
Record the full set — every hit is a deletion target. Confirm no hit is on the cursor path (`latest_event_id_for_context`, `EventCursorResponse`, `/api/events/{...}/cursor`).

- [ ] **Step 2: Remove the API + core surface**

- `event_service.rs`: delete `list_visible` (and its four query variants) + the `pub use ... EventListParams, EventRow` (line 6 → keep only what the cursor needs). Keep `latest_event_id_for_context`.
- `handlers/events.rs`: delete `list` (line 22) + its `use ... EventListParams, EventRow`. Keep `cursor`.
- `routes.rs`: delete `.route("/api/events", get(handlers::events::list))` (line 86). Keep the cursor route (line 88).
- `openapi.rs`: remove `EventListParams`/`EventRow` from the components + the `list` path.
- `temper-core/src/types/api.rs`: delete `EventListParams`, `EventRow`. Keep `EventCursorResponse`. Regenerate TS types (Step 5).

- [ ] **Step 3: Remove the MCP + client + CLI consumers**

- temper-mcp: delete the `list_events` tool fn + its `#[tool]`/registration in the service; remove the `temper_core::types::api::EventListParams` parameter import.
- `temper-client/src/events.rs`: delete the `list` method; keep the cursor call. Drop the now-unused imports.
- CLI: if `rg` found a `temper events`/list command, delete the command + its action; keep any cursor/sync usage.

- [ ] **Step 4: Remove/reduce the tests**

- Delete `tests/e2e/tests/events_test.rs` (it drives the list feed) **or** reduce it to a cursor-only test.
- In `mcp_round_trip_test.rs`, delete the `list_events` assertion block.

- [ ] **Step 5: Regenerate TS types + verify whole workspace**

Run: `cargo make generate-ts-types && cargo make check`
Expected: clean across all crates (no `EventListParams`/`EventRow` references anywhere).

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "WS6 collapse: deprecate unused /api/events? list query + its consumer chain"
```

---

## Phase 3 — The atomic collapse flip

The split machinery deletion, de-qualification, search_path-hook removal, the two leak-service rewrites onto the substrate-default connection, the boot-`migrate!` removal, and the test-harness repoint **must land together** — the workspace compiles and tests pass only when the code uniformly assumes one schema. This is the largest task; its steps are ordered but it is **one commit** (Task 8). Tasks 9–10 follow as cache regen + gate.

### Task 8: Collapse to a single backend + single schema

**Files (delete):** `backend/selection.rs`, `backend/read_selector.rs`, `services/backend_selection_service.rs`
**Files (modify):** `backend/mod.rs`, `backend/db_backend.rs`, `backend/next_backend.rs`, `state.rs`, `handlers/{resources,meta,edges,ingest,search}.rs`, `services/{graph_service,event_service}.rs`, `main.rs`, `api/mcp.rs`, `api/axum.rs`, `crates/temper-mcp/src/tools/{resources,relationships,search}.rs`, `crates/temper-next/src/{writes.rs,substrate.rs}`, `Cargo.toml` (drop `next-backend` feature), `crates/temper-api/Cargo.toml`, `tests/e2e/...`

**Interfaces:**
- Consumes: `NextBackend` (becomes THE backend), `temper_next::keys`/`text`/`parity` (Phase 1), `graph_subgraph_nodes`/`graph_traverse` (Task 6), the substrate `kb_events`/`kb_edges` shapes.
- Produces: `DbBackend` = the single backend (renamed from `NextBackend`); handlers + MCP tools call it directly; all SQL unqualified against the substrate default.

- [ ] **Step 1: Point the test harness + dev connection at the substrate**

The e2e/`test-db` harness provisions via `migrations/` (builds legacy `public`). Repoint it to build the **substrate** and default search_path to it: in the e2e common harness, after pool creation run the artifact (`00_namespace_reset`+`01_schema`+`02_functions`) and set `search_path=temper_next,public`. (Mirrors `cargo make db-collapsed`.) This is what lets every downstream step's tests run green.

- [ ] **Step 2: De-qualify the substrate SQL + drop the search_path hooks**

- `writes.rs`: delete line 83 `SET LOCAL search_path TO temper_next, public`; change `:36,:52,:66` `temper_next.kb_*` → unqualified `kb_*`. Delete the `:30` `public.kb_profiles` prod-bridge (the write-freeze + single schema make it moot) — read the profile unqualified.
- `substrate.rs:18-22`: delete the `.after_connect(SET search_path...)` builder; connect plainly (the dev `DATABASE_URL` carries the search_path).
- `next_backend.rs`: remove its `SET LOCAL search_path` (`:172`) and de-qualify any `temper_next.`-prefixed SQL.
- `readback/mod.rs`: remove its search_path lines + de-qualify the 53 `temper_next.` refs.

- [ ] **Step 3: Rewrite `graph_service` Query 2 onto `kb_edges`**

In `graph_service.rs` replace the `kb_resource_edges` query (lines 185-201) with:
```rust
let edge_records = sqlx::query!(
    r#"
    SELECT source_id AS "source!: Uuid", target_id AS "target!: Uuid",
           edge_kind AS "edge_kind!: EdgeKind", polarity AS "polarity!: Polarity",
           label AS "label: String"
      FROM kb_edges
     WHERE source_table = 'kb_resources' AND target_table = 'kb_resources'
       AND source_id = ANY($1::uuid[]) AND target_id = ANY($1::uuid[])
       AND NOT is_folded
    "#,
    &node_ids,
).fetch_all(pool).await?;
```
`label` is now nullable — in the map, `label: rec.label.unwrap_or_default()`. Query 1 (`graph_subgraph_nodes`) is unchanged in Rust (it calls the ported function from Task 6).

- [ ] **Step 4: Rewrite `event_service` to the cursor only**

In `event_service.rs`, `latest_event_id_for_context` becomes (per the event-service spec):
```rust
pub async fn latest_event_id_for_context(
    pool: &PgPool, profile_id: Uuid, kb_context_id: Uuid,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"
        SELECT e.id
          FROM kb_events e
         WHERE e.producing_anchor_table = 'kb_contexts'
           AND e.producing_anchor_id = $2
           AND EXISTS (                                   -- context-ownership gate
             SELECT 1 FROM kb_contexts c
              WHERE c.id = $2 AND (
                (c.owner_table='kb_profiles' AND c.owner_id = $1)
                OR (c.owner_table='kb_teams' AND c.owner_id IN
                     (SELECT team_id FROM kb_team_members WHERE profile_id = $1))))
         ORDER BY e.occurred_at DESC
         LIMIT 1
        "#,
        profile_id, kb_context_id,
    ).fetch_optional(pool).await?;
    Ok(id.flatten())
}
```
> Verify `kb_team_members(team_id, profile_id)` column names against `01_schema.sql`; adjust if the membership table differs.

- [ ] **Step 5: Collapse the backend — `NextBackend` becomes `DbBackend`**

- Delete `backend/selection.rs`, `backend/read_selector.rs`, `services/backend_selection_service.rs`.
- In `backend/db_backend.rs`: replace the old legacy `DbBackend` body with `NextBackend`'s impl (the substrate write path is now the only backend). Simplest mechanical route: rename `NextBackend` → `DbBackend` in `next_backend.rs`, delete the old `db_backend.rs`, rename the file. `DbBackend::new(pool, profile_id)` is the constructor.
- `backend/mod.rs`: drop `pub use selection::*`, `pub use next_backend::NextBackend`, the `read_selector`/`selection` modules; export only `DbBackend`.
- `state.rs`: delete the `backend_selection` field (line 159) + `with_backend_selection` (line 178).

- [ ] **Step 6: Repoint every handler + MCP call site to the single backend**

For each of the 23 call sites (handlers: resources.rs:79,115,204,277,317,144; search.rs:26; meta.rs:33,76; edges.rs:77,125,172,219; ingest.rs:67,133 — MCP: resources.rs:346,413,419,424,467,565,624; relationships.rs:119,155,190,225; search.rs:15):
- `select_backend(state.backend_selection, &state.pool, pid, device, surface)` → `DbBackend::new(state.pool.clone(), pid)`.
- `read_selector::list_select(state.backend_selection, &pool, pid, params)` → the direct service call `resource_service::n(&pool, pid, params)` (and likewise `show_select`→`show`, `get_content_select`→content, `get_meta_select`→`n_meta`, `search_select`→search, `list_enriched_select`→the enriched list). The `read_selector` indirection is gone; call the service directly (reads are service-direct per the constraints).

- [ ] **Step 7: Remove the boot-time `migrate!` + the 3 startup flag reads**

- `main.rs`: delete `sqlx::migrate!("../../migrations").run(&pool)...` (lines 27-30) and the `backend_selection_service::read` block (lines 34-37); construct `AppState::new(pool, jwks_store, config)` without `with_backend_selection`.
- `api/mcp.rs` + `api/axum.rs`: delete their `backend_selection_service::read` + `with_backend_selection` blocks (mcp.rs:34,38; axum.rs:41,44).

- [ ] **Step 8: Drop the `next-backend` feature + retire split tests**

- Root `Cargo.toml`: `temper-api` features → drop `next-backend` (keep `ingest-pipeline`). `temper-api/Cargo.toml`: remove the `next-backend` feature def (temper-next becomes an unconditional dep — add `temper-api` to the public-schema CI jobs' `--exclude` list per `[[project_temper_next_unconditional_dep_ci_exclusion]]`). `tests/e2e/Cargo.toml`: remove the `next-backend` feature.
- Delete `tests/e2e/tests/{backend_read_path_next,backend_write_path_next,backend_selection_gate}.rs` (they test the split). Keep `surface_parity_next.rs` (amended in Task 10) and `mcp_round_trip_test.rs` (de-split its assertions).

- [ ] **Step 9: temper-events `kb_scopes`/`porosity` retirement**

- `crates/temper-events/src/types/scope.rs`: delete the `Porosity` enum + `Scope` struct (the `porosity` type + `kb_scopes` are dropped in the substrate).
- `crates/temper-events/src/ledger.rs:49-58`: remove the `kb_scopes` EXISTS validation + the `UnknownScope` error path (events anchor via `producing_anchor`, not `scope_id`). Adjust the `LedgerWrite` shape if it carries `scope_id`.

- [ ] **Step 10: Verify the whole workspace compiles against the collapsed schema**

Run: `cargo make db-collapsed && export DATABASE_URL="postgresql://temper:temper@localhost:5437/temper_development?options=-csearch_path%3Dtemper_next,public" && cargo make check`
Expected: clean — no `select_backend`/`read_selector`/`backend_selection`/`temper_next.`/`kb_resource_edges`/`kb_scopes` references anywhere.

- [ ] **Step 11: Commit (atomic)**

```bash
git add -A && git commit -m "WS6 collapse: delete split machinery, de-qualify SQL, rewrite graph/event services onto substrate, remove boot migrate!"
```

### Task 9: Regenerate every sqlx cache against the single schema

**Files:** `crates/temper-api/.sqlx`, `crates/temper-next/.sqlx`, `tests/e2e/.sqlx`, `Makefile.toml` (`prepare-next` repoint)

- [ ] **Step 1: Repoint `prepare-next`** — in `Makefile.toml` the `prepare-next` task hardcodes `search_path%3Dtemper_next,public`. Post-collapse all crates resolve against the one schema; keep `temper_next` for local dev (it is the substrate until the live rename) but add a comment that the live deploy resolves against `public`. (CI runs `SQLX_OFFLINE=true`, so the committed caches are authoritative.)

- [ ] **Step 2: Regenerate all three caches**

Run (with the collapsed `DATABASE_URL` exported):
```bash
cargo make prepare-next && cargo make prepare-api && cargo make prepare-e2e
```
Expected: each writes its per-crate `.sqlx`; `git status` shows cache churn.

- [ ] **Step 3: Verify offline build matches**

Run: `cargo make check`
Expected: clean with `SQLX_OFFLINE=true` (the committed caches satisfy every macro).

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "WS6 collapse: regenerate sqlx caches against the single schema"
```

### Task 10: Bring the surface-parity gate green (eight surfaces) + cursor test

**Files:** `tests/e2e/tests/surface_parity_next.rs`, `tests/e2e/tests/events_cursor_test.rs` (new)

**Interfaces:**
- Consumes: the collapsed stack (Task 8).
- Produces: the collapse acceptance gate, un-`#[ignore]`d and green.

- [ ] **Step 1: Amend the parity gate to eight surfaces**

In `surface_parity_next.rs`: remove surface (9) (the `GET /api/events?resource_id=` assertion). Remove the `#[ignore]` attribute and the `next-backend` cfg gate (it is now `#![cfg(feature = "test-db")]`). The remaining eight surfaces (list, list `--meta-only`, show, show `--meta-only`, show `--edges`, content, search, graph) must all resolve the schema-only resource.

- [ ] **Step 2: Run the gate, expect GREEN**

Run: `cargo nextest run -p temper-e2e --features test-db -E 'test(all_read_surfaces_resolve)'`
Expected: PASS — every surface resolves the resource against the one schema.

- [ ] **Step 3: Write the cursor test**

Create `tests/e2e/tests/events_cursor_test.rs`: a `DbBackend` write into a context the caller owns bumps that context's `latest_event_id`; an un-owned context returns `None`; a cogmap-homed write does **not** bump a context cursor.

- [ ] **Step 4: Run it green**

Run: `cargo nextest run -p temper-e2e --features test-db -E 'test(events_cursor)'`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "WS6 collapse: surface-parity gate green (8 surfaces) + cursor test"
```

---

## Phase 4 — Handoff to the live cutover

The code now assumes one schema and is green against the local collapsed schema. The **live schema rename, extension/uuid homing, graft, promote, and `public_legacy` drop** are the operator runbook `docs/guides/ws6-endgame-collapse-runbook.md` (separate artifact). Its redeploy step ships exactly the code this plan produced; its acceptance gate is the eight-surface `surface_parity_next` run over the live schema.

- [ ] **Final step: confirm the full suite green + push the branch**

Run: `cargo make test-all` (collapsed `DATABASE_URL` exported)
Expected: green. Then push the branch and open the PR per `finishing-a-development-branch`.

---

## Self-Review notes

- **Spec coverage:** every endgame coincident-change bullet maps to a task — split machinery (T8), boot `migrate!` (T8.7), search_path hooks + de-qualify (T8.2), graph/event rewrites (T6/T7/T8.3-4), temper-mcp (T8.6), e2e split tests + feature gate (T8.8), temper-events scopes/porosity (T8.9), sqlx caches (T9). The 3 DDL decisions (extension homing, drop-gate, the rename itself) are **runbook**, not code — correctly out of this plan.
- **Atomicity:** T7 and T8 are flagged single-commit cross-crate changes (the pre-commit hook gates whole-workspace clippy).
- **Known verify-then-act steps** (legitimate, not placeholders): bootseed dead-code (T5.4), `sluggify` helper presence (T6.3), `kb_team_members` columns (T8.4), the CLI events command (T7.3) — each is a grep+confirm because the exact current symbol must be checked at execution time.
