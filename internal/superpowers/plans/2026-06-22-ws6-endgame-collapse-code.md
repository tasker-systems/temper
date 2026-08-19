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
- **Atomic cross-crate commits.** The pre-commit hook gates whole-workspace clippy; a change that breaks compilation across `temper-api`/`temper-mcp`/`tests/e2e`/deploy adapters must land as **one** commit (Tasks 7 and 8 below; the flip's de-qualified SQL + sqlx caches are part of that same Task-8 commit).
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

**Grafted / dark-launched (Phase 2.5):**
- `schema-artifact/01_schema.sql` — gains the identity/infra layer (kb_profiles email/preferences, 3 enums, 7 infra tables); `02_functions.sql` gains `has_system_access`/`is_system_admin` (Task A).
- `crates/temper-api/src/services/{profile,access,context,edge}_service.rs` — gain `#[cfg(feature="next-backend")]` `*_next` ports over qualified `temper_next.*` (Tasks B/C/D/E). Legacy bodies stay live until the flip.
- `crates/temper-core/src/operations/actions.rs` — receives the moved pure create-guards; `crates/temper-api/src/backend/next_backend.rs` — guards wired into `create_resource`; `crates/temper-next/src/readback/mod.rs` — substrate `find_by_body_hash` (Task F).
- `crates/temper-api/src/backend/read_selector.rs` — gains `list_meta_select`; the 3 bypass surfaces routed through the selector (Task G).

**Deleted (Phases 1 + 3):**
- `crates/temper-next/src/synthesis/` (whole dir, after survivors moved).
- `crates/temper-api/src/backend/selection.rs`, `services/backend_selection_service.rs`. (`read_selector.rs` is NOT deleted — collapsed in place to the substrate read dispatcher; it wraps the surviving `readback` path.)
- `crates/temper-api/src/services/{sync,doc_type,search,relationship,meta,ingest}_service.rs` + the legacy bodies of `{resource,profile,access,context,edge}_service.rs` (the audit RETIRE/PORT dispositions; their callers repointed by Phase 2.5 + T8.7-8).
- `crates/temper-core/src/types/sync.rs`.

**Rewritten (Phases 2–3):**
- `schema-artifact/02_functions.sql` — gains ported `graph_traverse` + `graph_subgraph_nodes` (T6).
- `crates/temper-api/src/services/graph_service.rs`, `event_service.rs` (T8).
- `crates/temper-api/src/backend/db_backend.rs` collapses to *the* backend (was `NextBackend`).
- `crates/temper-next/src/{writes.rs, substrate.rs, readback/mod.rs}`, `temper-api/src/backend/next_backend.rs` — de-qualified, search_path hooks removed (T8).
- `crates/temper-core/src/types/{profile.rs, resource.rs}` — `Profile` reshape + doc-type-by-name wire (T8.9).

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

> **⚠️ RE-SEQUENCED (2026-06-23, discovered at execution): this task runs LAST, as Phase 3 Task 10 — not here in Phase 1.**
> Deleting `synthesis/` removes `temper_next::synthesis::run`, but the e2e split tests still call it
> (`surface_parity_next.rs:107`, `backend_{read,write}_path_next.rs`). Because `cargo make check` runs
> `clippy --workspace --all-targets --all-features` and `tests/e2e` is a workspace member, `--all-features`
> enables `next-backend` on temper-e2e and **compiles those synthesis callers** — so deleting synthesis here
> fails the gate. The last caller (`surface_parity_next.rs`) is only de-synthesized in **Task 9** (the parity
> gate — its `synthesis::run` seed → direct `DbBackend` write). So this deletion is the FINAL code step,
> as **Task 10** after the Task 9 parity gate, when no caller remains (consistent with the line-26 atomicity
> rule: a change that breaks `tests/e2e` compilation lands coincident with the e2e cleanup). Phase 1 is
> therefore **Tasks 1–4 only**. See the re-stated step at the end of Phase 3.
>
> **Also folded in (execution adjudication):** the plan's step-5 "keep `parity_reads.rs`" is conditional on
> it exercising readback/parity rather than synthesis — it does NOT (its header: "asserts identical output …
> over the synthesized prod-shape fixture"; every test calls `common::seed_and_synthesize` → `synthesis::run`).
> So `parity_reads.rs` **retires with synthesis**; durable read-parity coverage stays in `corpus_parity_reads.rs`
> (clean, KEEP) + the Task-10 `surface_parity_next.rs` 8-surface gate. `tests/synthesis_source.rs` is a third
> synthesis-only test file (tests `synthesis::source`) that also retires. Prune now-dead `common/mod.rs`
> helpers (`seed_and_synthesize`, `seed_prod_shape_fixture`) only if uncalled by surviving tests.

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

## Phase 2.5 — Additive service-layer ports (chunks A–G, dark-launch)

> **Why this phase exists.** The original Task 8 assumed only `graph`/`event` were "raw-pool
> leak services." Execution proved the **entire** legacy service layer (identity/access/context/
> sync/edge/resource/meta/search/ingest/doc_type/relationship) still targets the legacy `public`
> shape and backs live surfaces. The service-layer audit
> (`docs/superpowers/specs/2026-06-23-ws6-service-layer-collapse-audit.md`) re-scoped the flip into
> **additive prep chunks A–G** (this phase) **then a shrunken atomic flip** (Task 8). Read the audit's
> "Disposition map" + "Resolved product decisions" before executing — every disposition (PORT / RETIRE /
> GRAFT-SATISFIED) and the 5 product decisions are cited there.

**Dark-launch model (the chosen port mechanism).** Each chunk lands the **ported substrate code**
without disturbing the live legacy path:
- New substrate Rust fns are **`#[cfg(feature = "next-backend")]`** and query **qualified `temper_next.*`**
  (mirroring `next_backend.rs` / `read_selector.rs::next_impl`). The flip de-qualifies them.
- Where a ported fn must return an existing temper-core type that the substrate can't fully populate
  (e.g. `Profile.slug`/`avatar_url`), it **synthesizes the soon-to-be-dropped fields** from substrate
  data — the same §9-non-invariant discipline `read_selector`'s next arms already use. The **type
  reshape is deferred to the flip** (a breaking change that lands with legacy deletion).
- The **legacy call site stays live**; the chunk does NOT rewire middleware/handlers. The flip swaps
  call sites. So a ported `*_next` fn is "dark" (present, compiled, sqlx-verified, unit-tested under
  `next-backend`) but unreached in production until the flip.
- **Per-chunk cache discipline:** a chunk that adds `next-backend`-gated temper-api queries must regen
  `crates/temper-api/.sqlx` **in the same commit** (`cargo make prepare-api` against the collapsed dev
  DB) — pre-commit `cargo make check` runs `SQLX_OFFLINE=true`, so a stale cache fails the commit.
  Confirm `prepare-api` enables `next-backend` (it must, since chunk-4 next arms are already cached);
  if it does not, fix the task to pass `--features next-backend` and note it.
- **Pure-SQL chunks** (A; the graph port T6 was the template) are tested in the `temper-next`
  `artifact-tests` harness and regen `crates/temper-next/.sqlx` via `cargo make prepare-next`.

Each chunk is one independently-reviewable, green commit. They have **no ordering dependency on each
other except**: B/C/D/E all need A's grafted tables/functions present in the artifact first.

> **CRITICAL — artifact changes MUST be paired with a `migrations/` forward migration.** Discovered at
> execution (2026-06-23): `temper-api`'s `#[sqlx::test]` integration tests + `cargo sqlx prepare` resolve
> `temper_next` from the **`migrations/` install chain** (`20260613000001_install_temper_next.sql`
> [FROZEN, generated by `tools/gen-install-migration.sh`] + append-only forward migrations), **NOT** from
> `schema-artifact/`. So a chunk that grafts tables/functions into the artifact (Task A) is **invisible to
> temper-api** — B/C macros referencing the graft won't resolve — until the same DDL lands as an
> append-only forward migration. This is also what the `schema_drift::migrations_reconstruct_artifact_schema`
> guard enforces. The reconciliation commit (`3cff564`) added `migrations/20260623000001_temper_next_artifact_graft.sql`
> covering BOTH un-migrated deltas (T6's graph functions + Task A's graft), restoring `schema_drift` GREEN.
> **Rule for any future artifact-touching chunk:** mirror its DDL into a new append-only forward migration
> (the `20260617000001_temper_next_can_modify.sql` pattern — `SET search_path`, function bodies
> byte-identical + unqualified for the `pg_get_functiondef` fingerprint), keep `schema_drift` green, and
> NEVER edit the frozen install migration. `migrations/` itself is retired later (Task 8 Step 10 removes
> boot `migrate!`; Step 1 repoints the e2e harness to the artifact) — until then it is the temper-api
> schema source and must track the artifact.

### Task A: Graft the identity/infra layer into the local artifact

The substrate kernel (`schema-artifact/01_schema.sql`) deliberately omits the operational/identity
tables ("out of scope for this artifact", `01_schema.sql:25-27`). The ported access/profile/context
services need them present locally so their macros resolve and `prepare-api` passes. The live-cutover
graft DDL already exists at `docs/superpowers/specs/2026-06-22-ws6-canonical-layer-draft.sql` — this
task folds its **DDL half** into the artifact (data carry-over stays runbook-only).

**Files:**
- Modify: `schema-artifact/01_schema.sql` — add `email`/`preferences` to the `kb_profiles` CREATE
  (`:71-77`); add 3 enums (`:40-60` block); add 7 infra tables after their FK deps.
- Modify: `schema-artifact/02_functions.sql` — add `has_system_access` + `is_system_admin` (the
  `kb_teams.is_active` predicate **dropped** — that column does not exist in the substrate).
- **Add: `migrations/20260623000001_temper_next_artifact_graft.sql`** — the paired append-only forward
  migration (see the CRITICAL preamble note). Without it the graft is invisible to temper-api (so B/C
  fail). It also covered T6's then-un-migrated graph functions, restoring `schema_drift` green. DONE in
  reconciliation commit `3cff564`.
- Test: `crates/temper-next/tests/identity_graft_test.rs` (new; `#![cfg(feature = "artifact-tests")]`,
  owns the namespace per the `temper-next-write` group convention).

**Interfaces:**
- Produces (SQL): substrate `kb_profiles` gains `email VARCHAR(256)` + `preferences JSONB NOT NULL
  DEFAULT '{}'`; tables `kb_profile_auth_links`, `kb_system_settings`, `kb_join_requests`,
  `kb_team_invitations`, `kb_transfers`, `kb_blob_files`, `kb_ingestion_records`; enums
  `join_request_status`/`invitation_status`/`transfer_status`; functions
  `has_system_access(uuid)->bool` and `is_system_admin(uuid)->bool`.

- [ ] **Step 1: Write the failing test (tables + functions resolve)**

Create `crates/temper-next/tests/identity_graft_test.rs`:
```rust
#![cfg(feature = "artifact-tests")]
//! The grafted identity/infra layer resolves against the substrate. Owns the temper_next namespace
//! (resets 01+02). Asserts the 7 tables exist, kb_profiles carries email/preferences, and the two
//! system-access functions evaluate (open access_mode → has_system_access true for any profile).

#[sqlx::test]
async fn identity_graft_resolves(pool: sqlx::PgPool) {
    // reset namespace to 01+02 (see the temper-next-write harness helper)
    // seed kb_system_settings (id=1, access_mode='open') + one kb_profile
    let has = sqlx::query_scalar::<_, bool>("SELECT has_system_access($1)")
        .bind(profile_id).fetch_one(&pool).await.expect("has_system_access runs");
    assert!(has, "open access_mode grants access");
    // assert email/preferences columns + each grafted table is queryable
    sqlx::query("SELECT email, preferences FROM kb_profiles LIMIT 1").execute(&pool).await.unwrap();
    sqlx::query("SELECT 1 FROM kb_profile_auth_links LIMIT 1").execute(&pool).await.unwrap();
}
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cargo nextest run -p temper-next --features artifact-tests identity_graft_resolves`
Expected: FAIL — `column "email" does not exist` / `relation "kb_profile_auth_links" does not exist`.

- [ ] **Step 3: Fold the graft DDL into `01_schema.sql`**

Add `email VARCHAR(256)` and `preferences JSONB NOT NULL DEFAULT '{}'::jsonb` to the `kb_profiles`
CREATE TABLE (`:71-77`). In the enum block (`:40-60`) add:
```sql
CREATE TYPE join_request_status AS ENUM ('pending', 'approved', 'rejected', 'withdrawn');
CREATE TYPE invitation_status   AS ENUM ('pending', 'accepted', 'declined', 'expired');
CREATE TYPE transfer_status     AS ENUM ('pending', 'accepted', 'declined', 'cancelled');
```
After `kb_resources`/`kb_profiles`/`kb_teams` are defined, append the 7 tables **verbatim from
`2026-06-22-ws6-canonical-layer-draft.sql:69-174`** (`kb_profile_auth_links`, `kb_system_settings`,
`kb_join_requests`, `kb_team_invitations`, `kb_transfers`, `kb_blob_files`, `kb_ingestion_records`).
Keep the `LEGACY.`-carryover comments out — only the CREATE TABLE DDL belongs here.

- [ ] **Step 4: Add the two functions to `02_functions.sql` (is_active dropped)**

Append (ported from `migrations/20260407000001_system_access_gate.sql:50-90`, the `t.is_active = true`
predicate removed since the substrate `kb_teams` has no `is_active` column):
```sql
CREATE FUNCTION has_system_access(p_profile_id UUID) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    WITH settings AS (SELECT access_mode, gating_team_slug FROM kb_system_settings LIMIT 1)
    SELECT CASE
        WHEN settings.access_mode = 'open' THEN true
        WHEN settings.access_mode = 'invite_only' THEN EXISTS (
            SELECT 1 FROM kb_team_members tm JOIN kb_teams t ON t.id = tm.team_id
             WHERE tm.profile_id = p_profile_id AND t.slug = settings.gating_team_slug)
        ELSE false
    END FROM settings
$$;

CREATE FUNCTION is_system_admin(p_profile_id UUID) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    WITH settings AS (SELECT gating_team_slug FROM kb_system_settings LIMIT 1)
    SELECT EXISTS (
        SELECT 1 FROM kb_team_members tm JOIN kb_teams t ON t.id = tm.team_id
         WHERE tm.profile_id = p_profile_id AND t.slug = settings.gating_team_slug
           AND tm.role = 'owner')
    FROM settings
$$;
```

- [ ] **Step 5: Run the test, verify it passes; regen the cache**

Run: `cargo nextest run -p temper-next --features artifact-tests identity_graft_resolves` → PASS.
Then `cargo make prepare-next` (the artifact changed; no `?`-macro change yet, but keep the cache current).

- [ ] **Step 6: Commit**

```bash
git add schema-artifact/01_schema.sql schema-artifact/02_functions.sql \
  crates/temper-next/tests/identity_graft_test.rs crates/temper-next/.sqlx
git commit -m "WS6 collapse (A): graft the identity/infra layer into the local artifact"
```

### Task B: Dark-launch the substrate profile path

`profile_service` resolves auth identity (`resolve_from_claims` in auth middleware), reads
(`get_by_id`), and updates. The substrate `kb_profiles` is `(id, handle, display_name, system_access,
created)` + grafted `email`/`preferences` — legacy `slug`→`handle`, and `avatar_url`/`vault_config`/
`is_active`/`updated` are gone. We dark-launch `*_next` variants that return the **existing**
`temper_core::Profile` shape, synthesizing the dropped fields (`slug = handle`, `avatar_url = None`,
`vault_config = {}`, `is_active = true`, `updated = created`). The `Profile` reshape + the
middleware/handler call-site swap are deferred to the flip.

**Files:**
- Modify: `crates/temper-api/src/services/profile_service.rs` — add `#[cfg(feature = "next-backend")]`
  fns `resolve_from_claims_next`, `get_by_id_next`, `update_next`, and `generate_profile_handle`
  (mirrors `generate_profile_slug:29-75`, querying `temper_next.kb_profiles.handle`).
- Test: `crates/temper-api/tests/profile_next_test.rs` (new; `#![cfg(all(feature = "test-db", feature
  = "next-backend"))]`).

**Interfaces:**
- Consumes: A's grafted `kb_profiles.email`/`preferences`, `kb_profile_auth_links`; substrate
  `kb_contexts(owner_table, owner_id, slug, name)`.
- Produces: `resolve_from_claims_next(pool, &AuthClaims) -> ApiResult<Profile>`,
  `get_by_id_next(pool, Uuid) -> ApiResult<Profile>`, `update_next(pool, id, display_name,
  preferences, vault_config) -> ApiResult<Profile>` — all returning the current `Profile` shape with
  synthesized dropped fields. (The flip renames these onto the canonical names + drops the synthesis.)

- [ ] **Step 1: Write the failing test**

Create `crates/temper-api/tests/profile_next_test.rs`: a fresh `AuthClaims` resolves to a new
`temper_next` profile + a `'default'` context (owner_table='kb_profiles', owner_id=profile, slug from
display_name); `get_by_id_next` reads it back with `slug == handle` and `is_active == true`.
```rust
#![cfg(all(feature = "test-db", feature = "next-backend"))]
#[sqlx::test]
async fn resolve_then_get_roundtrips_over_substrate(pool: sqlx::PgPool) {
    let claims = test_claims("auth0|abc", "Ada Lovelace", "ada@x.dev");
    let p = profile_service::resolve_from_claims_next(&pool, &claims).await.expect("resolve");
    assert_eq!(p.slug, p.handle_or_synth()); // slug synthesized = handle
    let got = profile_service::get_by_id_next(&pool, p.id.into()).await.expect("get");
    assert_eq!(got.display_name, "Ada Lovelace");
    assert!(got.is_active);
}
```

- [ ] **Step 2: Run it, verify it fails (fns absent)**

Run: `cargo nextest run -p temper-api --features test-db,next-backend resolve_then_get_roundtrips_over_substrate`
Expected: FAIL — `no function named resolve_from_claims_next`.

- [ ] **Step 3: Implement the `_next` fns (qualified `temper_next.*`, synthesized Profile)**

Translate `resolve_from_claims:85-195` onto the substrate: the `kb_profile_auth_links` lookup/insert is
unchanged (table grafted by A); the `kb_profiles` INSERT drops `avatar_url`/`vault_config`/`is_active`/
`updated` and writes `handle` (via `generate_profile_handle`) + `email`/`preferences`; the
`kb_contexts` auto-insert becomes:
```rust
sqlx::query!(
    r#"INSERT INTO temper_next.kb_contexts (id, owner_table, owner_id, slug, name)
       VALUES ($1, 'kb_profiles', $2, 'default', 'default')
       ON CONFLICT (owner_table, owner_id, slug) DO NOTHING"#,
    Uuid::now_v7(), profile_id,
).execute(pool).await?;
```
Every `SELECT` maps to a `Profile` with `slug: row.handle.clone()`, `avatar_url: None`,
`vault_config: serde_json::json!({})`, `is_active: true`, `updated: row.created`. Qualify all tables
`temper_next.*`; for unqualified helper calls (none needed here) use the `SET LOCAL search_path` txn
pattern from `read_selector.rs::next_impl::list:233-244`. `generate_profile_handle` mirrors
`generate_profile_slug` but selects `WHERE handle = $1`.

- [ ] **Step 4: Run the test green; regen the cache**

Run the Step-2 command → PASS. Then `cargo make prepare-api` (captures the new `next-backend` macros).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/services/profile_service.rs \
  crates/temper-api/tests/profile_next_test.rs crates/temper-api/.sqlx
git commit -m "WS6 collapse (B): dark-launch the substrate profile path"
```

### Task C: Dark-launch the substrate access path

`access_service` gates system access (`has_system_access`/`is_system_admin` — now resolvable via A),
and runs the join-request lifecycle. Three substrate deltas: drop the `kb_teams.is_active` predicate
(`:96`); the `kb_team_members` INSERT (`:302-314`) drops `id`/`joined_at`/`invited_by_profile_id`
(absent in the substrate `(team_id, profile_id, role, created)` shape); and `emit_join_request_event`
(`:378-398`) ports from the legacy `resolve_event_type` + direct `kb_events` INSERT onto the substrate
`_event_append` (`02_functions.sql:761-783`). The join-request event types must be seeded.

**Files:**
- Modify: `crates/temper-api/src/services/access_service.rs` — add `#[cfg(feature = "next-backend")]`
  `*_next` variants of `create_join_request`, `withdraw_request`, `review_request`, `get_own_request`,
  `list_pending_requests`, plus a `next`-shaped `emit_join_request_event_next`. (`has_system_access`/
  `is_system_admin`/`get_system_settings`/`get_public_settings`/`get_entitlements` call the SQL
  functions or read `kb_system_settings` — graft-satisfied by A, so they need **no `_next` variant**;
  confirm each compiles unchanged against the grafted artifact.)
- Modify: the substrate event-type seed (the `bootseed`/`system_event_type_names` list — grep
  `crates/temper-next/src/scenario/bootseed.rs` and `schema-artifact/`) to include
  `join_request.submitted` / `.withdrawn` / `.approved` / `.rejected`.
- Test: `crates/temper-api/tests/access_next_test.rs` (new; `next-backend` + `test-db`).

**Interfaces:**
- Consumes: A's `kb_system_settings`/`kb_join_requests` + `has_system_access`/`is_system_admin`;
  substrate `kb_team_members(team_id, profile_id, role, created)`, `_event_append`.
- Produces: `create_join_request_next(pool, CreateJoinRequestParams) -> ApiResult<JoinRequest>` (and
  the sibling `*_next` fns), each emitting via `_event_append('join_request.<x>', profile_id,
  'kb_profiles', profile_id, payload)`.

- [ ] **Step 1: Write the failing test**

`access_next_test.rs`: seed `kb_system_settings` (invite_only, gating team) + a gating team; a
`create_join_request_next` inserts a `pending` `kb_join_requests` row AND appends a
`join_request.submitted` event (assert a `kb_events` row with that type exists); `review_request_next`
approve inserts a `kb_team_members(team_id, profile_id, role='watcher')` row (no `id`/`joined_at`).
```rust
#![cfg(all(feature = "test-db", feature = "next-backend"))]
#[sqlx::test]
async fn join_request_submit_emits_substrate_event(pool: sqlx::PgPool) { /* … */ }
```

- [ ] **Step 2: Run it, verify it fails (event type unseeded / fn absent)**

Run: `cargo nextest run -p temper-api --features test-db,next-backend join_request_submit_emits_substrate_event`
Expected: FAIL — `no function named create_join_request_next` (and, once added but unseeded,
`event_type join_request.submitted not seeded`).

- [ ] **Step 3: Seed the join-request event types**

Add the four `join_request.*` names to the substrate system event-type seed list (the same list
`_event_append` validates against). Re-run the artifact reset so they're present.

- [ ] **Step 4: Implement the `*_next` variants**

Mirror each legacy fn; the only body changes: drop `AND is_active = true` from the gating-team lookup;
the `kb_team_members` INSERT becomes `INSERT INTO temper_next.kb_team_members (team_id, profile_id,
role) VALUES ($1, $2, 'watcher') ON CONFLICT (team_id, profile_id) DO NOTHING`; `emit_join_request_event_next`
becomes:
```rust
sqlx::query_scalar!(
    "SELECT _event_append($1, $2, 'kb_profiles', $2, $3)",
    event_type, profile_id, payload_json,
).fetch_one(pool).await?;   // emitter == anchor == the requesting profile
```
Qualify all tables `temper_next.*`. (`invited_by_profile_id` reviewer attribution is dropped — it
survives on `kb_join_requests.reviewed_by_profile_id` + the approval event payload.)

- [ ] **Step 5: Run green; regen cache; commit**

Step-1 test PASS → `cargo make prepare-api` → commit:
```bash
git add crates/temper-api/src/services/access_service.rs crates/temper-api/tests/access_next_test.rs \
  crates/temper-next/src/scenario/bootseed.rs crates/temper-api/.sqlx
git commit -m "WS6 collapse (C): dark-launch the substrate access path"
```

### Task D: Dark-launch the substrate context path

`context_service` lists/resolves/creates contexts. Substrate deltas: there is **no
`contexts_visible_to`** function — derive visibility inline (owner OR `kb_team_contexts` share);
resource counts come from `kb_resource_homes` (not `kb_resources.kb_context_id`); the CREATE writes
`owner_table`/`owner_id`/`slug` (slug NOT NULL, generated from name) and **drops the context-created
event** (product decision 5 — contexts are infra).

**Files:**
- Modify: `crates/temper-api/src/services/context_service.rs` — add `#[cfg(feature = "next-backend")]`
  `list_visible_next`, `get_visible_next`, `resolve_by_name_next`, `create_next` (and
  `resolve_name_by_id_next` if a caller needs it — `resolve_name_by_id:90` has no visibility gate, so
  its substrate form is a plain `SELECT name FROM temper_next.kb_contexts WHERE id=$1`).
- Test: `crates/temper-api/tests/context_next_test.rs` (new; `next-backend` + `test-db`).

**Interfaces:**
- Consumes: substrate `kb_contexts(owner_table, owner_id, slug, name, created)`, `kb_team_contexts`,
  `kb_resource_homes(anchor_table, anchor_id)`.
- Produces: `list_visible_next(pool, ProfileId) -> ApiResult<Vec<ContextRowWithCounts>>`,
  `get_visible_next`, `resolve_by_name_next`, `create_next(pool, ProfileId, name) ->
  ApiResult<ContextRow>`. (`ContextRow`/`ContextRowWithCounts` keep their current shape; the substrate
  has no `updated` — synthesize `updated = created` if the struct carries it.)

- [ ] **Step 1: Write the failing test**

`context_next_test.rs`: `create_next` inserts a context with a generated slug; `list_visible_next`
returns it for the owner with `resource_count = 0`; after homing one resource via
`kb_resource_homes(anchor_table='kb_contexts', anchor_id)`, the count is 1; a non-owner without a
`kb_team_contexts` share does not see it.

- [ ] **Step 2: Run it, verify it fails (fns absent)**

Run: `cargo nextest run -p temper-api --features test-db,next-backend context_next`
Expected: FAIL — `no function named create_next`.

- [ ] **Step 3: Implement the `*_next` variants with the inline visibility predicate**

The shared visibility predicate replacing `contexts_visible_to($1)`:
```sql
WHERE (c.owner_table = 'kb_profiles' AND c.owner_id = $1)
   OR EXISTS (SELECT 1 FROM temper_next.kb_team_contexts tc
                JOIN temper_next.kb_team_members tm ON tm.team_id = tc.team_id
               WHERE tc.context_id = c.id AND tm.profile_id = $1)
```
`list_visible_next` counts via `LEFT JOIN temper_next.kb_resource_homes rh ON rh.anchor_table =
'kb_contexts' AND rh.anchor_id = c.id` (`COUNT(rh.resource_id)`). `create_next`:
```rust
sqlx::query_as!(ContextRow,
    r#"INSERT INTO temper_next.kb_contexts (id, owner_table, owner_id, slug, name)
       VALUES ($1, 'kb_profiles', $2, $3, $4)
       RETURNING id, name, owner_table AS kb_owner_table, owner_id AS "kb_owner_id!", created, created AS updated"#,
    Uuid::now_v7(), profile_id, slug_from_name(name), name,
).fetch_one(pool).await?   // NO event emission (decision 5)
```
Generate `slug` from `name` with a simple lowercase/dash helper (or reuse the
`generate_profile_handle` shape for uniqueness against `(owner_table, owner_id, slug)`).

- [ ] **Step 4: Run green; regen cache; commit**

Step-1 PASS → `cargo make prepare-api` → commit:
```bash
git add crates/temper-api/src/services/context_service.rs crates/temper-api/tests/context_next_test.rs \
  crates/temper-api/.sqlx
git commit -m "WS6 collapse (D): dark-launch the substrate context path"
```

### Task E: Dark-launch `list_resource_edges` over `kb_edges`

The one live edge READ (`GET /api/resources/{id}/edges`, `edge_service::list_resource_edges:598-657`)
calls the legacy-only `graph_resource_edges`/`peer_slug`. Port it onto substrate `kb_edges` +
`edges_visible_to($1)` (`02_functions.sql:301-309`). `peer_slug` is §7-dissolved — derive it from the
peer title via the substrate `sluggify` helper. `direction` is derived from which endpoint is the
queried resource.

**Files:**
- Modify: `crates/temper-api/src/services/edge_service.rs` — add `#[cfg(feature = "next-backend")]`
  `list_resource_edges_next(pool, profile_id, resource_id) -> ApiResult<Vec<GraphEdgeRow>>`.
- Test: `crates/temper-api/tests/edge_read_next_test.rs` (new; `next-backend` + `test-db`).

**Interfaces:**
- Consumes: substrate `kb_edges(source_table, source_id, target_table, target_id, edge_kind, polarity,
  label, weight, is_folded, created)`, `edges_visible_to(uuid)->TABLE(edge_id)`,
  `resources_visible_to(uuid)` (1-arg substrate form).
- Produces: `list_resource_edges_next` returning the **unchanged** `temper_core::types::graph::
  GraphEdgeRow` (`graph.rs:262-273`).

- [ ] **Step 1: Write the failing test**

`edge_read_next_test.rs`: seed a context + two resources + one `kb_edges` row between them; assert
`list_resource_edges_next` returns one `GraphEdgeRow` whose `peer_resource_id` is the other resource,
`direction` reflects source/target, `peer_slug == sluggify(peer_title)`; a not-visible resource id
returns `ApiError::NotFound`.

- [ ] **Step 2: Run it, verify it fails (fn absent)**

Run: `cargo nextest run -p temper-api --features test-db,next-backend edge_read_next`
Expected: FAIL — `no function named list_resource_edges_next`.

- [ ] **Step 3: Implement over `kb_edges` + `edges_visible_to`**

```rust
let rows = sqlx::query!(
    r#"
    SELECT e.id AS "edge_id!: Uuid",
           (CASE WHEN e.source_id = $2 THEN e.target_id ELSE e.source_id END) AS "peer_resource_id!: Uuid",
           peer.title                  AS "peer_title!: String",
           temper_next.sluggify(peer.title) AS "peer_slug!: String",
           e.edge_kind                 AS "edge_kind!: EdgeKind",
           e.polarity                  AS "polarity!: Polarity",
           e.label                     AS "label: String",
           (CASE WHEN e.source_id = $2 THEN 'outgoing' ELSE 'incoming' END) AS "direction!: String",
           e.weight                    AS "weight!: f64",
           e.created                   AS "created!: chrono::DateTime<chrono::Utc>"
      FROM temper_next.kb_edges e
      JOIN temper_next.edges_visible_to($1) v ON v.edge_id = e.id
      JOIN temper_next.kb_resources peer
        ON peer.id = (CASE WHEN e.source_id = $2 THEN e.target_id ELSE e.source_id END)
     WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
       AND (e.source_id = $2 OR e.target_id = $2)
    "#,
    profile_id, resource_id,
).fetch_all(pool).await?;
```
Gate the resource itself first: `EXISTS(SELECT 1 FROM temper_next.resources_visible_to($1) WHERE
resource_id = $2)` → `ApiError::NotFound` if false (matches the legacy 404). `label` is nullable →
`label.unwrap_or_default()`. **Verify** the `direction` string vocabulary the UI expects against the
legacy `graph_resource_edges` (`migrations/20260522100002_edges_as_projection.sql:257`) and the
`sluggify` helper name (`02_functions.sql` — `sluggify` vs `slugify`); adjust the literals to match.

- [ ] **Step 4: Run green; regen cache; commit**

Step-1 PASS → `cargo make prepare-api` → commit:
```bash
git add crates/temper-api/src/services/edge_service.rs crates/temper-api/tests/edge_read_next_test.rs \
  crates/temper-api/.sqlx
git commit -m "WS6 collapse (E): dark-launch list_resource_edges over kb_edges"
```

### Task F: Lift create-time guards into `NextBackend::create_resource`

`NextBackend::create_resource` (`next_backend.rs:209-256`) today calls `writes::create_resource` with
**zero** validation/defaults/dedup — the legacy `ingest_service::ingest` (`:420-590`) ran
`strip_system_managed_fields`→`apply_defaults_value`→`ensure_managed_identity_keys`→
`validate_managed_meta`→`find_by_body_hash`. Product decision 3: **lift** those guards into the
substrate create path.

**Decomposition refinement (2026-06-23, for the dark-launch model):** do NOT move the pure helpers to
`temper-core` in this chunk. The legacy `strip_system_managed_fields:100` + `validate_managed_meta:692`
still live in `ingest_service` (which exists until the flip), so `NextBackend::create_resource` simply
**calls them where they are** (`ingest_service::strip_system_managed_fields`/`validate_managed_meta`) plus
the temper-core defaults (`apply_defaults_value`/`ensure_managed_identity_keys`, already in
`operations/actions.rs`) plus a NEW substrate `find_by_body_hash`. This keeps F purely additive (no
helper relocation, no `IngestError` move, no `ingest_service` behavior change). **The flip (Task 8 Step
8) owns moving the two surviving pure helpers to a permanent home when it deletes `ingest_service`** —
see the Step-8 note. Body-hash dedup gets a substrate form (the legacy `find_by_body_hash:213` joins the
dead `kb_resource_manifests`; the substrate keys on `kb_resources.body_hash`).

**Files:**
- Add: `crates/temper-next/src/readback/mod.rs` `find_by_body_hash(pool, principal, body_hash) ->
  Result<Option<Uuid>>` over `temper_next.kb_resources.body_hash` gated by `resources_visible_to`
  (mirror the existing readback fns' search_path discipline). NEW temper-next macro → regen
  `crates/temper-next/.sqlx` (`cargo make prepare-next`, F-exclusive — no sibling touches temper-next).
- Modify: `crates/temper-api/src/backend/next_backend.rs` — in `create_resource`, between meta
  extraction (`:233`) and `writes::create_resource` (`:235`), run strip→defaults→ensure-keys→validate
  (calling the existing `ingest_service` + `temper_core` helpers) and the `find_by_body_hash` dedup
  pre-check (return the existing row's id on a hit). Keep `cmd.doctype` (the name) as the doc-type key.
- Test: `crates/temper-api/tests/create_guards_next_test.rs` (new; `next-backend` + `test-db`).

**Interfaces:**
- Consumes: `ingest_service::{strip_system_managed_fields, validate_managed_meta}`,
  `temper_core::operations::{apply_defaults_value, ensure_managed_identity_keys}`,
  `temper_next::readback::find_by_body_hash`.
- Produces: `NextBackend::create_resource` now rejects invalid managed_meta (the existing typed
  validation error), applies doc-type defaults (e.g. task→`temper-stage: backlog`), and dedups on
  body_hash. (`ingest_service` is untouched — same legacy behavior.)

- [ ] **Step 1: Write the failing tests**

`create_guards_next_test.rs`: (a) a `task` create with no `temper-stage` comes back with
`temper-stage: backlog` (default applied); (b) a managed_meta violating the task schema is rejected
(not silently written); (c) creating the same body twice returns the first resource id (dedup).

- [ ] **Step 2: Run them, verify they fail**

Run: `cargo nextest run -p temper-api --features test-db,next-backend create_guards_next`
Expected: FAIL — defaults not applied / invalid meta accepted / duplicate created.

- [ ] **Step 3: Add the substrate dedup; wire the existing guards into `create_resource`**

Add `find_by_body_hash` to `temper-next/readback` (see the plan's substrate dedup SQL in the original
Task F Step 4). Then in `NextBackend::create_resource`, after extracting managed/open meta, call (in
order): `ingest_service::strip_system_managed_fields` → `apply_defaults_value(&cmd.doctype, &mut
managed)` → `ensure_managed_identity_keys(&mut managed, &cmd.title, slug)` →
`ingest_service::validate_managed_meta(...)` (propagate its typed error) → `find_by_body_hash` (return
the existing id on hit) → the existing `writes::create_resource`. Do NOT move or modify the
`ingest_service` helpers.

- [ ] **Step 4: Add the substrate dedup + wire the guards into `create_resource`**

`find_by_body_hash` over the substrate:
```rust
let dup: Option<Uuid> = sqlx::query_scalar!(
    r#"SELECT r.id FROM temper_next.kb_resources r
        JOIN temper_next.resources_visible_to($1) v ON v.resource_id = r.id
       WHERE r.body_hash = $2 AND r.is_active LIMIT 1"#,
    principal, body_hash,
).fetch_optional(pool).await?;
```
In `create_resource`, after extracting managed/open meta: `strip_system_managed_fields` → `apply_defaults_value(&cmd.doctype, &mut managed)` → `ensure_managed_identity_keys(&mut managed, &cmd.title, slug)` → `validate_managed_meta(...)` (return the typed error) → `find_by_body_hash` (return the existing row on hit) → then the existing `writes::create_resource` call.

- [ ] **Step 5: Run green; regen caches; commit**

Step-1 tests PASS + ingest regression green → `cargo make prepare-api && cargo make prepare-next` →
commit:
```bash
git add crates/temper-core/src/operations/actions.rs crates/temper-api/src/services/ingest_service.rs \
  crates/temper-next/src/readback/mod.rs crates/temper-api/src/backend/next_backend.rs \
  crates/temper-api/tests/create_guards_next_test.rs crates/temper-api/.sqlx crates/temper-next/.sqlx
git commit -m "WS6 collapse (F): lift create-time guards into the substrate create path"
```

### Task G: Route the three `read_selector`-bypass surfaces through the selector

Three live reads call legacy services **directly**, bypassing `read_selector` (audit "Architecture
finding"): MCP resources-protocol (`temper-mcp/src/resources.rs` → `resource_service::{list_visible,
get_content,get_visible}`), HTTP `?meta_only=true` (`handlers/resources.rs:74-76` →
`resource_service::list_visible_meta`, **no selector arm**), and MCP `enrich_resources`
(`tools/resources.rs:236` → `meta_service::get_meta_batch`). Route all three through `read_selector`
(flag-gated). Pre-flip default `BackendSelection::Legacy` makes behavior identical (the Legacy arm
calls the same legacy fn); the `next-backend` arm newly makes them substrate-capable. The meta-list
path needs a **new selector arm**. (The doc-type-by-UUID→by-name **wire** change is breaking across
senders, so it lands in the flip, not here — see Task 8.)

**Files:**
- Modify: `crates/temper-api/src/backend/read_selector.rs` — add `list_meta_select(selection, pool,
  profile_id, params) -> ApiResult<ResourceMetaListResponse>` (Legacy → `list_visible_meta`; Next →
  a `next_impl::list_meta` projecting `readback::enriched_list` to the meta-list shape).
- Modify: `crates/temper-api/src/handlers/resources.rs:74-76` — call `read_selector::list_meta_select(
  state.backend_selection, …)`.
- Modify: `crates/temper-mcp/src/resources.rs` — route `list_resources`/`read_resource` through
  `read_selector::{list_select, show_select, get_content_select}` with `state.backend_selection`.
- Modify: `crates/temper-mcp/src/tools/resources.rs:236` — route `enrich_resources` through
  `read_selector::get_meta_select` per id (or a batched `get_meta_batch_select` arm).
- Test: extend `tests/e2e/tests/surface_parity_next.rs` (or a focused `next-backend` test) to cover
  the meta-only list + MCP resource-protocol read over the Next arm.

**Interfaces:**
- Consumes: existing `read_selector` arms + the new `list_meta_select`.
- Produces: the three bypass surfaces now dispatch through `read_selector` (flag-respecting); the flip
  collapses the selector to substrate-only.

- [ ] **Step 1: Write the failing test (meta-only list over the Next arm)**

Add to the parity suite: with `BackendSelection::Next`, `list_meta_select` returns the schema-only
resource's managed/open tiers; assert the MCP resource-protocol `read_resource` resolves the same body
through `get_content_select`.

- [ ] **Step 2: Run it, verify it fails (no `list_meta_select`)**

Run: `cargo nextest run -p temper-e2e --features test-db,next-backend -E 'test(meta_only_list_next)'`
Expected: FAIL — `no function named list_meta_select`.

- [ ] **Step 3: Add `list_meta_select` + its Next arm; repoint the three surfaces**

`next_impl::list_meta` maps `readback::enriched_list(pool, principal, None, None)` rows to
`ResourceMetaListResponse` (managed/open already carried per row). Add the Legacy arm =
`resource_service::list_visible_meta`. Repoint the HTTP handler + the two MCP surfaces to the selector
fns, threading `state.backend_selection`.

- [ ] **Step 4: Run green; regen caches; commit**

Step-1 PASS → `cargo make prepare-api && cargo make prepare-e2e` → commit:
```bash
git add crates/temper-api/src/backend/read_selector.rs crates/temper-api/src/handlers/resources.rs \
  crates/temper-mcp/src/resources.rs crates/temper-mcp/src/tools/resources.rs \
  tests/e2e/tests/surface_parity_next.rs crates/temper-api/.sqlx tests/e2e/.sqlx
git commit -m "WS6 collapse (G): route the read_selector-bypass surfaces through the selector"
```

---

## Phase 3 — The shrunken atomic collapse flip

> **Shrunk by Phase 2.5.** Chunks A–G landed every substrate port dark (compiled, sqlx-verified,
> tested under `next-backend`). The flip is now **mechanical**: point the connection at the substrate,
> de-qualify all `temper_next.*` SQL + drop the search_path hooks, **swap each call site from the
> legacy fn to its `*_next` port**, delete the now-dead legacy services + split machinery, reshape the
> deferred types, remove the feature/flag/`migrate!`, and **regenerate every sqlx cache in this same
> commit** (de-qualified queries invalidate the caches; pre-commit `cargo make check` is `SQLX_OFFLINE`
> — they must move together). The old standalone "regenerate caches" task is **absorbed here**.

Everything in this task **must land together** — the workspace compiles and tests pass only when the code uniformly assumes one schema. It is **one commit** (Task 8), including the sqlx-cache regen (de-qualified queries invalidate the caches; the pre-commit `cargo make check` is `SQLX_OFFLINE`, so a cache from a separate commit would fail). Its steps are ordered. With Phase 2.5's ports already landed dark, the flip's substantive work is **swapping call sites legacy→`*_next`, deleting the now-dead legacy services, reshaping the deferred types, and stripping the split machinery** — not writing new substrate logic.

### Task 8: Collapse to a single backend + single schema

**Files (delete):**
- Split machinery: `backend/selection.rs`, `services/backend_selection_service.rs`. (NOT `read_selector.rs` — collapsed in place per Step 6; it wraps the surviving `readback`/`*_select` read path.)
- Retired legacy services (per the audit Disposition map — their callers are repointed to the `*_next` ports or the `readback` path by this task + Phase 2.5): `services/sync_service.rs`, `services/doc_type_service.rs`, `services/search_service.rs`, `services/relationship_service.rs`, `services/meta_service.rs`, `services/ingest_service.rs`, and the **legacy bodies** (now-dead halves) of `resource_service.rs` (write fns `update`/`delete`/`check_can_modify` + the legacy read fns), `profile_service.rs` (legacy `resolve_from_claims`/`get_by_id`/`update`/`generate_profile_slug`), `access_service.rs` (legacy join-request fns), `context_service.rs` (legacy fns), `edge_service.rs` (legacy `list_resource_edges` + the retired derivation fns `extract_and_upsert_edges`/`reconcile_edges`/`extract_declarations_from_resource` — product decision 1). `relationship_service::reproject_pending_for_resource` retires with frontmatter→edge derivation (decision 1; its sole caller `ingest_service` retires).
- `temper-core/types/sync.rs` (sync wire types) + the 3 sync routes.

**Files (modify):** `backend/mod.rs`, `backend/db_backend.rs`, `backend/next_backend.rs` (→ renamed `DbBackend`), `backend/read_selector.rs` (collapse in place), `state.rs`, `handlers/{resources,meta,edges,ingest,search,profiles,contexts,access}.rs` (swap call sites to the ported fns), `middleware/{auth,system_access}.rs` (profile/access call-site swaps), `services/{graph_service,event_service}.rs`, `services/{profile,access,context,edge}_service.rs` (de-qualify the `*_next` fns + drop the `_next` suffix as they become the only impl), `main.rs`, `api/mcp.rs`, `api/axum.rs`, `crates/temper-mcp/src/{resources.rs,tools/{resources,relationships,search}.rs}`, `crates/temper-next/src/{writes.rs,substrate.rs,readback/mod.rs}`, `crates/temper-core/src/types/{profile.rs,resource.rs}` (deferred reshapes), `crates/temper-events/src/{types/scope.rs,ledger.rs}`, `Cargo.toml` + `crates/temper-api/Cargo.toml` + `tests/e2e/Cargo.toml` (drop `next-backend` feature), all three `.sqlx` caches, `tests/e2e/...`.

**Interfaces:**
- Consumes: every Phase 2.5 `*_next` port (profile/access/context/edge/guards/meta-list arm), `NextBackend` (becomes THE backend), `temper_next::{keys,text,parity}` (Phase 1), `graph_subgraph_nodes`/`graph_traverse` (Task 6), the substrate `kb_events`/`kb_edges` shapes, the moved create-guards (Task F).
- Produces: `DbBackend` = the single backend (renamed from `NextBackend`); one schema everywhere; all SQL unqualified against the substrate default; the legacy `public`-shape service layer gone; `Profile`/`ResourceCreateRequest` reshaped.

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

- Delete `backend/selection.rs`, `services/backend_selection_service.rs`. **Do NOT delete `backend/read_selector.rs`** — collapse it in place to the substrate read dispatcher (Step 6 correction): drop the `selection` param + `Legacy` arms + the `next-backend` gate on `next_impl`; it keeps wrapping `readback`.
- In `backend/db_backend.rs`: replace the old legacy `DbBackend` body with `NextBackend`'s impl (the substrate write path is now the only backend). Simplest mechanical route: rename `NextBackend` → `DbBackend` in `next_backend.rs`, delete the old `db_backend.rs`, rename the file. `DbBackend::new(pool, profile_id)` is the constructor.
- `backend/mod.rs`: drop `pub use selection::*`, `pub use next_backend::NextBackend`, the `read_selector`/`selection` modules; export only `DbBackend`.
- `state.rs`: delete the `backend_selection` field (line 159) + `with_backend_selection` (line 178).

- [ ] **Step 6: Repoint every handler + MCP call site to the single backend**

For each of the 23 call sites (handlers: resources.rs:79,115,204,277,317,144; search.rs:26; meta.rs:33,76; edges.rs:77,125,172,219; ingest.rs:67,133 — MCP: resources.rs:346,413,419,424,467,565,624; relationships.rs:119,155,190,225; search.rs:15):
- `select_backend(state.backend_selection, &state.pool, pid, device, surface)` → `DbBackend::new(state.pool.clone(), pid)`.
- `read_selector::list_select(state.backend_selection, &pool, pid, params)` → `read_selector::list_select(&pool, pid, params)` — **drop only the `selection` arg**, keep the call. **CORRECTED (2026-06-23, discovered at execution — AMEND of the design's "delete read_selector.rs"):** the original Step-6 text said "route reads to the legacy service directly (`resource_service::list_visible` etc.)" — that is **WRONG**. Those legacy read services query projections that DO NOT EXIST in the substrate (`vault_resources_browse`, `kb_resource_manifests`, `kb_current_chunks`, `unified_search()`/`graph_search()` — all verified absent from `schema-artifact/`). The substrate read path is the **`readback`** module (the design spec: *"readback is de-qualified, NOT deleted at collapse … collapse makes readback resolve to the one schema; shim-exit removes it"*), which `read_selector`'s `next_impl` already wraps. So **collapse `read_selector.rs` IN PLACE** instead of deleting it: remove the `selection: BackendSelection` param + the `BackendSelection::Legacy` match arms, **un-feature-gate** the `next_impl` (readback) bodies (the `next-backend` feature is being removed), and have handlers/MCP call the now-param-less `*_select` fns (`list_select`/`show_select`/`get_content_select`/`get_meta_select`/`search_select`/`list_enriched_select`). The genuinely-deleted split machinery is the *dual-dispatch + flag*: `selection.rs`, `backend_selection_service.rs`, the `BackendSelection` enum, `state.backend_selection`, `with_backend_selection`, and the `Legacy`-arm dispatch — NOT the readback read path. Rename `read_selector.rs` → an honest name (e.g. `reads.rs`) if cheap, else keep the filename with an updated module doc (it is now the substrate read dispatcher, not a selector). Re-implementing list/show/content/meta/search onto substrate base tables is NOT in scope — that would reinvent `readback`.

- [ ] **Step 7: Swap the service-direct call sites to the ported `*_next` fns + drop the legacy bodies**

Phase 2.5 dark-launched the ports; now make them the *only* impl. For each ported service, de-qualify
its `*_next` fns (drop the `temper_next.` prefixes + the `SET LOCAL search_path` txns — Step 2's
discipline) and **rename them onto the canonical names** (the legacy fn deletes, the `_next` fn takes
its name), then repoint callers:
- **profile** (`profile_service`): swap `middleware/auth.rs:127` + `mcp/service.rs:63`
  (`resolve_from_claims`), `handlers/profiles.rs:33` (`get_by_id`), `:67` (`update`) onto the ported
  bodies; delete legacy `resolve_from_claims`/`get_by_id`/`update`/`generate_profile_slug`.
- **access** (`access_service`): swap `handlers/access.rs` (`create_join_request:52`/`get_own_request:61`/
  `withdraw_request:70`/`list_pending_requests:95`/`review_request:119`) + `middleware/system_access.rs`
  onto the ported bodies; delete the legacy join-request fns + the legacy `emit_join_request_event`.
- **context** (`context_service`): swap `handlers/contexts.rs:27/48/69`, `ingest_service:427`
  (now in the retiring ingest — see Step 8), and `read_selector.rs:121` (`resolve_by_name` in the
  collapsed `list_enriched`) onto the ported bodies; delete legacy fns.
- **edge** (`edge_service`): swap `handlers/edges.rs:40` onto `list_resource_edges_next`; delete the
  legacy `list_resource_edges`.

- [ ] **Step 8: Retire the now-dead legacy services**

With every caller repointed (Step 7 + Phase 2.5 G + the backend collapse), delete the services the
audit dispositioned RETIRE:
- **Before deleting `ingest_service`:** move the two pure helpers `strip_system_managed_fields` + `validate_managed_meta` (+ `ValidateParams`) to a permanent home — `temper-core/src/operations/actions.rs` (returning a temper-core-native validation error; the schema-validation machinery already lives in temper-core), or a surviving service. Repoint `DbBackend::create_resource` (the calls Task F added) to the new home. This is the helper-move Task F deferred (see Task F's decomposition note).
- `git rm` `services/{sync_service,doc_type_service,search_service,relationship_service,meta_service,ingest_service}.rs`; drop their `mod` lines in `services/mod.rs`.
- `resource_service.rs`: delete the write fns (`update`/`delete`/`check_can_modify`) + the legacy read fns now superseded by `readback`/the `*_select` arms; keep only what a surviving caller still needs (grep to confirm none remain — `get_visible`'s 8 sites route through `show_select`).
- `edge_service.rs`: delete `extract_and_upsert_edges`/`reconcile_edges`/`extract_declarations_from_resource` (decision 1, frontmatter→edge derivation retired) + `relationship_service::reproject_pending_for_resource` (retires with its sole caller `ingest_service`).
- Drop the 3 sync routes + `temper-core/types/sync.rs`; re-home the lone sync e2e audit test (or delete if it only exercised the dead manifest path).
- doc-type: re-route the MCP `doc_types` tool's `list_all` to enumerate the temper-core schemas (`crates/temper-core/schemas/*.schema.json`) instead of `SELECT … FROM kb_doc_types`.
- Heavy test surface (`edge_ingest_test`, `relationship_projection_test`, `resource_update_reconcile_edges_test`, `audit_test`, `tests/e2e/mcp_*`): rewrite to the substrate path or delete with their service.

- [ ] **Step 9: Reshape the deferred types + the doc-type-by-name wire change**

These are breaking type changes that could not land additively in Phase 2.5 — they land here with the
legacy deletion:
- **`Profile`** (`temper-core/src/types/profile.rs:16-31`): rename `slug`→`handle`; drop `avatar_url`/`vault_config`/`is_active`/`updated`. Drop the synthesized-field fills in the (now-canonical) profile fns. `cargo make generate-ts-types`.
- **doc-type wire** (`ResourceCreateRequest`, `temper-core/src/types/resource.rs`): replace `kb_doc_type_id: Uuid` with `doc_type: String` (the substrate stores doc-type as a property name; `NextBackend`/`DbBackend` create already passes `cmd.doctype` through). Update `handlers/resources.rs:181-183` to take the name directly (delete `resolve_doc_type_name_by_id`); delete `read_selector.rs:128-129`'s `resolve_doc_type` name→UUID step (the collapsed `list_enriched` filters by name via `readback::enriched_list`). Update `temper-client` + the CLI `resource create` + the temper-ui caller to send `doc_type`. `cargo make generate-ts-types`.

- [ ] **Step 10: Remove the boot-time `migrate!` + the 3 startup flag reads**

- `main.rs`: delete `sqlx::migrate!("../../migrations").run(&pool)...` (lines 27-30) and the `backend_selection_service::read` block (lines 34-37); construct `AppState::new(pool, jwks_store, config)` without `with_backend_selection`.
- `api/mcp.rs` + `api/axum.rs`: delete their `backend_selection_service::read` + `with_backend_selection` blocks (mcp.rs:34,38; axum.rs:41,44).

- [ ] **Step 11: Drop the `next-backend` feature + retire split tests**

- Root `Cargo.toml`: `temper-api` features → drop `next-backend` (keep `ingest-pipeline`). `temper-api/Cargo.toml`: remove the `next-backend` feature def (temper-next becomes an unconditional dep — add `temper-api` to the public-schema CI jobs' `--exclude` list per `[[project_workspace_feature_unification_ort]]`'s `--exclude temper-cloud` precedent). `tests/e2e/Cargo.toml`: remove the `next-backend` feature.
- Delete `tests/e2e/tests/{backend_read_path_next,backend_write_path_next,backend_selection_gate}.rs` (they test the split). Keep `surface_parity_next.rs` (amended in Task 9) and `mcp_round_trip_test.rs` (de-split its assertions).
- **Adjudicate the `schema_drift` guard + `migrations/`.** Through Phase 2.5 each artifact change was paired with a forward migration, so `schema_drift` is GREEN going into the flip (see the CRITICAL preamble note). Now that Step 10 removed the boot `migrate!` and Step 1 repointed the e2e harness to load the artifact directly, decide `migrations/`'s fate: if it is fully dead post-collapse (the live cutover/runbook no longer drives Neon via this lineage), **retire** `migrations/` + `schema_drift::migrations_reconstruct_artifact_schema` (the `migrations→artifact` invariant dissolves by design); if the runbook (Phase 4) still ships the live schema via `migrations/`, KEEP both and ensure any flip-time artifact change carries its forward migration. Record the decision in the commit message; cross-check the runbook before deleting.

- [ ] **Step 12: temper-events `kb_scopes`/`porosity` retirement**

- `crates/temper-events/src/types/scope.rs`: delete the `Porosity` enum + `Scope` struct (the `porosity` type + `kb_scopes` are dropped in the substrate).
- `crates/temper-events/src/ledger.rs:49-58`: remove the `kb_scopes` EXISTS validation + the `UnknownScope` error path (events anchor via `producing_anchor`, not `scope_id`). Adjust the `LedgerWrite` shape if it carries `scope_id`.

- [ ] **Step 13: Regenerate every sqlx cache, then verify the whole workspace (absorbs the old Task 9)**

De-qualified queries + the type reshapes invalidate all three caches, so regen them **in this commit**
(separately-committed caches would fail the pre-commit `SQLX_OFFLINE` check). First repoint
`Makefile.toml`'s `prepare-next` comment: it keeps `search_path%3Dtemper_next,public` for local dev
(the substrate until the live rename), with a note that the live deploy resolves against `public`. Then:
```bash
cargo make db-collapsed
export DATABASE_URL="postgresql://temper:temper@localhost:5437/temper_development?options=-csearch_path%3Dtemper_next,public"
cargo make prepare-next && cargo make prepare-api && cargo make prepare-e2e
cargo make check
```
Expected: `cargo make check` clean under `SQLX_OFFLINE=true` — no `select_backend`/`read_selector`'s
`selection` arg/`backend_selection`/`temper_next.`/`kb_resource_edges`/`kb_scopes`/`kb_doc_types`
references anywhere; the committed caches satisfy every macro.

- [ ] **Step 14: Commit (atomic — code + all three caches together)**

```bash
git add -A && git commit -m "WS6 collapse: flip to one schema — swap call sites to the substrate ports, retire the legacy service layer, de-qualify SQL, reshape types, drop the flag/migrate!, regen caches"
```

### Task 9: Bring the surface-parity gate green (eight surfaces) + cursor test

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

### Task 10: Delete `synthesis/` + the `Synthesize` subcommand (re-sequenced from Phase 1 Task 5)

Runs here because only now — after Task 9 amended `surface_parity_next.rs` off `synthesis::run`, and Task 8 deleted `backend_{read,write}_path_next.rs` — does **no** caller of `temper_next::synthesis::run` remain, so the `--workspace --all-features` clippy gate stays green when the module is deleted. Execute the original Task 5 steps (lines ~247–282) plus the adjudication folded into the Task 5 callout:

- [ ] **Step 1:** `rg -n "synthesis::|mod synthesis|Synthesize \{|RunOpts|seed_and_synthesize" crates/ tests/` — every hit is now a deletion target (only unrelated prose "Synthesize" comments may remain).
- [ ] **Step 2:** `git rm -r crates/temper-next/src/synthesis`; drop `pub mod synthesis;` from `lib.rs`.
- [ ] **Step 3:** remove the `Synthesize { limit }` variant + match arm + `synthesis::{self, RunOpts}` import from `main.rs` (keep `Materialize`).
- [ ] **Step 4:** delete the now-dead `scenario::bootseed::system_event_type_names` fn **only if** still uncalled — **reconcile with chunk C**: if chunk C seeded the `join_request.*` event types by extending this list (rather than the artifact SQL seed), and production/CI seeds event types through it, it is NOT dead. Confirm the authoritative post-collapse event-type seed home (artifact SQL vs this fn) and keep whichever production uses; keep the rest of `bootseed` regardless.
- [ ] **Step 5:** delete synthesis-only test files: `tests/synthesis.rs`, `tests/synthesis_bootstrap.rs`, `tests/synthesis_source.rs`, `tests/parity_reads.rs` (synthesis-output validation — retires with synthesis). KEEP `tests/corpus_parity_reads.rs`. Prune `common/mod.rs` helpers (`seed_and_synthesize`/`seed_prod_shape_fixture`) only if uncalled by surviving tests.
- [ ] **Step 6:** fix the two intra-doc links that referenced `crate::synthesis::source` (`readback/mod.rs:676`, `writes.rs:10`) — reword (the link target is gone).
- [ ] **Step 7:** `cargo make check` clean + `cargo nextest run -p temper-next` no failures → commit `WS6 collapse: delete synthesis/ scaffolding + the Synthesize subcommand`.

---

## Phase 4 — Handoff to the live cutover

The code now assumes one schema and is green against the local collapsed schema. The **live schema rename, extension/uuid homing, graft, promote, and `public_legacy` drop** are the operator runbook `docs/guides/ws6-endgame-collapse-runbook.md` (separate artifact). Its redeploy step ships exactly the code this plan produced; its acceptance gate is the eight-surface `surface_parity_next` run over the live schema.

- [ ] **Final step: confirm the full suite green + push the branch**

Run: `cargo make test-all` (collapsed `DATABASE_URL` exported)
Expected: green. Then push the branch and open the PR per `finishing-a-development-branch`.

---

## Self-Review notes

- **Spec coverage (audit dispositions → tasks):** identity graft + missing functions → **A**; profile/access/context/edge ports → **B/C/D/E** (dark-launched), de-qualified + call-sites-swapped + legacy-deleted in **T8.7-8**; create-guards lift → **F**; read-selector bypasses → **G**; doc-type-by-name wire + `Profile` reshape → **T8.9**; split machinery + de-qualify + search_path hooks → **T8.2/5/6/10/11**; graph/event rewrites → **T6/T7/T8.3-4**; temper-events scopes/porosity → **T8.12**; sqlx caches → **T8.13** (absorbed the old standalone cache task); parity gate → **T9**; synthesis deletion → **T10**. The 3 DDL decisions (extension homing, drop-gate, the rename itself) are **runbook**, not code — correctly out of this plan.
- **Additive-then-flip safety:** every Phase 2.5 chunk is its own green commit that does not disturb the live legacy path (dark-launch); the flip (T8) is one atomic cross-crate commit (the pre-commit hook gates whole-workspace clippy, and de-qualified queries require their caches in the same commit). T7 is likewise single-commit cross-crate.
- **Known verify-then-act steps** (legitimate, not placeholders): bootseed dead-code reconciled with chunk C (T10.4), `sluggify` helper name + edge `direction` vocabulary (E.3), `kb_team_members` columns (T8.4 / C.4), `prepare-api` enabling `next-backend` (Phase 2.5 preamble), the surviving-caller grep before deleting `resource_service` legacy fns (T8.8) — each is a grep+confirm because the exact current symbol must be checked at execution time.
- **Type-consistency:** ported fns are introduced as `*_next` (B/C/D/E) returning the **current** temper-core shapes (synthesizing soon-dropped fields), then de-suffixed onto the canonical names with the type reshape in T8.7/T8.9 — no name collides across tasks.
