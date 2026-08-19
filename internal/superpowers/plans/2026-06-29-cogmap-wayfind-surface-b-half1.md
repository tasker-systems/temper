# Cogmap Wayfinding Surface B — Half 1 Implementation Plan (Beats 0 + 1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a cognitive map a first-class resource home: add a generalized scope-id filter to `unified_search`, and let `temper resource create --cogmap <ref>` home a resource in a map and `temper search --cogmap <ref>` search that map — gated, visibility-respecting, with cold-start safety.

**Architecture:** The FTS+vector+graph blend (`unified_search`) is scope-agnostic; Surface B only changes how the bounding `resource_id` set is established. We (Beat 0) add a generalized `p_scope_ids uuid[]` corpus filter to `unified_search` parallel to the existing `p_context_id` filter, then (Beat 1) thread a polymorphic home anchor through the create command so resources can be cogmap-homed, and resolve `--cogmap` search into that scope-id set. Cogmap refs resolve client-side by trailing-UUID (`parse_ref`) — no server slug lookup. The producer write gate is a named seam `cogmap_authorable_by_profile` delegating to `cogmap_readable_by_profile` (team-cogmap membership) pending the cogmap-arc RBAC.

**Tech Stack:** Rust (sqlx, axum, clap, rmcp/MCP), PostgreSQL 17/18 + pgvector, sqlx migrations.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-06-29-search-substrate-beat3-surface-b-wayfinding-design.md`. This plan implements §3 (Half 1) + the §6 `p_scope_ids` additive filter. **Half 2 (`--wayfind`/`--lens`/`--regions`, §4) and `--cogmap` on `edge assert` are OUT OF SCOPE here** (see "Deferred" below).
- **Migrations are immutable once shipped.** Never edit `20260624*`–`20260629000003*`. Add NEW migration files only. Next free names: `20260629000004_*`, `20260629000005_*`.
- **`--all-features` for every build/clippy/check.** `cargo make check` runs `SQLX_OFFLINE=true` against committed `.sqlx/` caches.
- **After changing any macro SQL or function schema:** regenerate caches — `cargo sqlx prepare --workspace -- --all-features`, and for test-target queries `cargo make prepare-api`. `unified_search` is called via **runtime `query_as`** (the `::vector` cast forbids the macro), so its signature change needs NO workspace-cache entry — but new `query_scalar!`/`query!` macro calls do.
- **Auth before writes** (CLAUDE.md): the `--cogmap` create gate runs *before* any home-row write.
- **Surfaces dispatch through the backend via one operations command** — do not inline persistence in handlers/actions.
- **Typed structs / parse-don't-validate**: the home choice is a `HomeAnchor` enum (exactly one home), never a placeholder `ContextId` plus a flag.
- **Test tiers:** cogmap/access-semantics tests live in `temper-api/tests` (test-db, mirroring `reconcile_cogmap_test.rs` + `search_context_ref_test.rs`) and `temper-substrate/tests` (artifact-tests). Per `feedback_access_semantics_changes_need_e2e_tier`, also run e2e before pushing. Substrate tests that spawn nothing run under `cargo make test-artifacts`.

## Deferred (explicitly NOT in this plan)

- **`--wayfind` / `--lens` / `--regions`** (the region-salience funnel, spec §4) → Beat 2, next task.
- **`--cogmap` on `edge assert`** → deferred. Rationale: edges have no `kb_resource_homes` row (homes are per-resource), `edge assert` has **no `--context` flag today** to be symmetric with, and per the owner's ruling edge creation is governed by *visibility* (team-cogmap reach), not a separate authorial gate — so there is no Beat-1 home-row or gate semantics to implement. Provenance-only act-correlation can be added later if wanted.
- **The exact producer write RBAC** — `cogmap_authorable_by_profile` is a labeled seam delegating to membership; the cogmap-arc RBAC tightens it later (see memory `project_authorial_rbac_undefined_contexts_cogmaps`).

---

## File Structure

**New files:**
- `migrations/20260629000004_search_scope_ids.sql` — DROP+CREATE `unified_search` with added `p_scope_ids uuid[]`.
- `migrations/20260629000005_cogmap_home_authz_and_scope.sql` — `cogmap_authorable_by_profile(profile,cogmap)::bool` + `cogmap_scope_ids(principal,cogmap) RETURNS SETOF uuid`.
- `crates/temper-api/tests/cogmap_home_test.rs` — surface tests: `--cogmap` create homes correctly, Surface-A excludes it, deny→403/zero.

**Modified files (by task):**
- Beat 0: `crates/temper-substrate/src/readback/mod.rs` (`UnifiedSearchQuery`, `unified_search`), `crates/temper-substrate/tests/search_surface_a.rs` (scope-filter test).
- Beat 1B: `crates/temper-substrate/src/writes.rs` (`CreateParams.home`), `crates/temper-api/src/backend/db_backend.rs:862`, all `CreateParams {` test sites.
- Beat 1C: `crates/temper-core/src/types/ids.rs` or a new `home.rs` (`HomeAnchor`), `crates/temper-workflow/src/operations/commands.rs` (`CreateResource.home`), `crates/temper-api/src/backend/db_backend.rs` (map), all `CreateResource {` construction sites.
- Beat 1D: `crates/temper-core/src/types/ingest.rs` (`home_cogmap_id`), `crates/temper-api/src/handlers/ingest.rs` (branch+gate), `crates/temper-cli/src/cli.rs` + `commands/resource.rs` + `cloud_backend/translators.rs` (`--cogmap`), `crates/temper-mcp/src/tools/resources.rs` (`CreateResourceInput.cogmap`).
- Beat 1E: `crates/temper-core/src/types/api.rs` (`SearchParams.cogmap_id`), `crates/temper-api/src/backend/substrate_read.rs` (`search_select` branch), `crates/temper-cli/src/cli.rs` + `actions/search.rs` + `commands/search_cmd.rs` + `main.rs` (`--cogmap`).

---

## Task A — Beat 0: generalized `p_scope_ids` corpus filter on `unified_search`

**Files:**
- Create: `migrations/20260629000004_search_scope_ids.sql`
- Modify: `crates/temper-substrate/src/readback/mod.rs` (`UnifiedSearchQuery` ~1078-1090, `unified_search` ~1095-1116)
- Test: `crates/temper-substrate/tests/search_surface_a.rs`

**Interfaces:**
- Produces: SQL `unified_search(...)` gains a trailing `p_scope_ids uuid[]` (param 12). When `NULL`, behavior is identical to today. When non-NULL, the corpus is additionally restricted to `c.id = ANY(p_scope_ids)`.
- Produces (Rust): `UnifiedSearchQuery` gains `pub scope_ids: Option<&'a [Uuid]>`; `readback::unified_search` binds it as `$12::uuid[]`.

- [ ] **Step 1: Write the failing test** — append to `crates/temper-substrate/tests/search_surface_a.rs` a test that creates two context-homed resources sharing a distinctive FTS term, then calls `unified_search` with `scope_ids = Some(&[id_a])` and asserts only `id_a` returns. Model the setup on the existing tests in that file (they already build `CreateParams` + call `readback::unified_search`). The new call passes `scope_ids: Some(&[id_a])`; assert the result vec's `resource_id`s contain `id_a` and not `id_b`.

```rust
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn scope_ids_restricts_corpus(pool: PgPool) {
    // ... existing-pattern setup: profile, context, two resources A and B
    //     both bodies contain the term "zscopeword" ...
    let hits = readback::unified_search(
        &pool,
        UnifiedSearchQuery {
            principal: profile.uuid(),
            query: Some("zscopeword"),
            embedding: None,
            seed_ids: &[],
            depth: 1,
            edge_types: &[],
            context_id: None,
            doc_type: None,
            graph_expand: false,
            limit: 50,
            offset: 0,
            scope_ids: Some(&[id_a.uuid()]),
        },
    )
    .await
    .unwrap();
    let ids: Vec<_> = hits.iter().map(|h| h.resource_id).collect();
    assert!(ids.contains(&id_a.uuid()), "in-scope A should be present");
    assert!(!ids.contains(&id_b.uuid()), "out-of-scope B must be filtered");
}
```

- [ ] **Step 2: Run it — expect compile failure** (no `scope_ids` field yet).

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-substrate --features artifact-tests scope_ids_restricts_corpus`
Expected: FAIL — `UnifiedSearchQuery` has no field `scope_ids`.

- [ ] **Step 3: Write the migration** `migrations/20260629000004_search_scope_ids.sql`. Copy the **entire current** `unified_search` body from `migrations/20260626000002_search_beat2_surface_a.sql:90-146` verbatim, then (a) `DROP FUNCTION` the old 11-arg signature first, (b) add `p_scope_ids uuid[]` as the final parameter, (c) add one predicate to the `corpus` CTE.

```sql
-- Beat 3 / Surface B: generalized scope-id corpus filter (spec §6).
-- Additive to the dormant p_context_id filter: restrict the corpus to an explicit id set.
DROP FUNCTION IF EXISTS unified_search(uuid, text, vector, uuid[], int, text[], uuid, text, boolean, int, int);

CREATE FUNCTION unified_search(
  p_principal uuid, p_query text, p_emb vector, p_seed_ids uuid[], p_depth int,
  p_edge_types text[], p_context_id uuid, p_doc_type text, p_graph_expand boolean,
  p_limit int, p_offset int, p_scope_ids uuid[])
RETURNS TABLE (resource_id uuid, fts_score real, vector_score real, graph_score real, combined_score real)
LANGUAGE sql STABLE AS $$
  WITH
  k AS (SELECT 1.0::float8 AS w_fts, 1.0::float8 AS w_vec, 0.5::float8 AS w_graph,
               0.5::float8 AS gamma, 100 AS vector_k, 20 AS auto_seed_n),
  -- ... (paste fts, vec, blend0, seeds, graph, cand CTEs verbatim from the Beat-2 source) ...
  corpus AS (   -- context/doc_type/scope candidate-corpus filter
    SELECT c.id FROM cand c
     WHERE (p_context_id IS NULL OR EXISTS (
             SELECT 1 FROM kb_resource_homes h
              WHERE h.resource_id = c.id AND h.anchor_table = 'kb_contexts' AND h.anchor_id = p_context_id))
       AND (p_scope_ids IS NULL OR c.id = ANY(p_scope_ids))
       AND (p_doc_type IS NULL OR EXISTS (
             SELECT 1 FROM kb_properties p
              WHERE p.owner_table = 'kb_resources' AND p.owner_id = c.id
                AND p.property_key = 'doc_type' AND NOT p.is_folded
                AND p.property_value #>> '{}' = p_doc_type))
  )
  -- ... (paste scored CTE + final SELECT/ORDER/LIMIT verbatim) ...
$$;
```

> **Implementer note:** open the Beat-2 source file and copy every CTE between `k` and the final `SELECT` exactly; only `corpus` changes (one added line) and the signature gains the trailing param. Do not paraphrase the FTS/vec/graph/blend/scored CTEs — transcribe them.

- [ ] **Step 4: Add the `scope_ids` field + bind** in `crates/temper-substrate/src/readback/mod.rs`.

In `UnifiedSearchQuery<'a>` (after `offset`): `pub scope_ids: Option<&'a [Uuid]>,`

In `unified_search`, extend the SQL string and bind list:
```rust
    let hits = sqlx::query_as::<_, ScoredHit>(
        "SELECT resource_id, fts_score, vector_score, graph_score, combined_score
           FROM unified_search($1, $2, $3::vector, $4::uuid[], $5, $6::text[], $7, $8, $9, $10::int, $11::int, $12::uuid[])",
    )
    // ... existing binds $1..$11 unchanged ...
    .bind(q.scope_ids)   // $12 — Option<&[Uuid]> binds to uuid[] / NULL
    .fetch_all(pool)
    .await?;
```
Update **every** `UnifiedSearchQuery { … }` construction site (the substrate readback callers and `substrate_read::search_select`) to add `scope_ids: None`. Grep: `rg 'UnifiedSearchQuery \{' crates/`.

- [ ] **Step 5: Run the new test + the existing surface-A tests — expect PASS.**

Run: `cargo make prepare-api` is NOT needed (runtime query_as). Then:
`DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo nextest run -p temper-substrate --features artifact-tests search`
Expected: the new `scope_ids_restricts_corpus` PASSES and all pre-existing `unified_search`/surface-A tests still PASS.

- [ ] **Step 6: Commit**

```bash
git add migrations/20260629000004_search_scope_ids.sql crates/temper-substrate/src/readback/mod.rs crates/temper-substrate/tests/search_surface_a.rs crates/temper-api/src/backend/substrate_read.rs
git commit -m "Surface B Beat 0: add p_scope_ids corpus filter to unified_search"
```

---

## Task B — Beat 1B: generalize `CreateParams.home` to a polymorphic `AnchorRef`

**Files:**
- Modify: `crates/temper-substrate/src/writes.rs` (`CreateParams` ~95-110, `create_resource_with` line 138)
- Modify: `crates/temper-api/src/backend/db_backend.rs:862` (live `CreateParams` site)
- Modify all `CreateParams {` test sites (grep below)
- Test: `crates/temper-substrate/tests/search_index.rs` (add cogmap-home assertion)

**Interfaces:**
- Produces: `CreateParams.home` becomes `pub home: AnchorRef` (from `crate::payloads::AnchorRef`). Callers construct `AnchorRef::context(ctx)` or `AnchorRef::cogmap(cogmap_id)`. The projector already writes `anchor_table`/`anchor_id` from the payload, so no SQL change.

- [ ] **Step 1: Write the failing test** — add to `crates/temper-substrate/tests/search_index.rs` a test that creates a resource with a cogmap home and asserts the home row.

```rust
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn create_resource_homes_in_cogmap(pool: PgPool) {
    // genesis a cogmap (reuse the existing cogmap fixture helper in this test file /
    // KernelCreateParams pattern) -> cogmap_id; a profile -> owner.
    let id = writes::create_resource(
        &pool,
        writes::CreateParams {
            title: "concept",
            origin_uri: "",
            body: "body text",
            doc_type: "note",
            home: temper_substrate::payloads::AnchorRef::cogmap(cogmap_id),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
        },
    )
    .await
    .unwrap();
    let (table, anchor): (String, Uuid) = sqlx::query_as(
        "SELECT anchor_table, anchor_id FROM kb_resource_homes WHERE resource_id = $1",
    )
    .bind(id.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(table, "kb_cogmaps");
    assert_eq!(anchor, cogmap_id.uuid());
}
```

- [ ] **Step 2: Run it — expect compile failure** (`home` is `ContextId`, not `AnchorRef`).

Run: `cargo nextest run -p temper-substrate --features artifact-tests create_resource_homes_in_cogmap`
Expected: FAIL — mismatched types / `home` expects `ContextId`.

- [ ] **Step 3: Change the field + the wrap.** In `writes.rs`:
  - `CreateParams.home`: `pub home: ContextId,` → `pub home: AnchorRef,` (ensure `use crate::payloads::AnchorRef;` is in scope).
  - `create_resource_with` line 138: `home: AnchorRef::context(p.home),` → `home: p.home,`.

- [ ] **Step 4: Update every `CreateParams {` construction site** to wrap the context in `AnchorRef::context(...)`. Mechanical transform: `home: <expr>,` → `home: AnchorRef::context(<expr>),`. Sites (grep `rg 'CreateParams \{' crates/ -l`, exclude `KernelCreateParams`):
  - `crates/temper-api/src/backend/db_backend.rs:862` (live — wraps `home` built from the cmd; this becomes the mapping point in Task C, but for now wrap with `AnchorRef::context`).
  - `crates/temper-substrate/tests/search_index.rs` (×6), `search_surface_a.rs`, `act_authorship_projection_invisibility.rs` (×2), `write_path_mutations.rs`.
  Add the needed `use temper_substrate::payloads::AnchorRef;` import to each test file.

- [ ] **Step 5: Run substrate tests — expect PASS.**

Run: `cargo nextest run -p temper-substrate --features artifact-tests`
Expected: the new test PASSES; all pre-existing substrate tests still PASS.

- [ ] **Step 6: `cargo fmt` then commit**

```bash
cargo fmt
git add crates/temper-substrate/src/writes.rs crates/temper-api/src/backend/db_backend.rs crates/temper-substrate/tests/
git commit -m "Surface B Beat 1B: generalize CreateParams.home to polymorphic AnchorRef"
```

---

## Task C — Beat 1C: `HomeAnchor` on the `CreateResource` command + backend mapping

**Files:**
- Create or modify: `crates/temper-core/src/types/home.rs` (new) + `mod` wiring in `crates/temper-core/src/types/mod.rs`
- Modify: `crates/temper-workflow/src/operations/commands.rs` (`CreateResource.context` line 31 → `home`)
- Modify: `crates/temper-api/src/backend/db_backend.rs` (~787, ~862: map `HomeAnchor` → `AnchorRef`)
- Modify every `CreateResource {` construction site (grep below)

**Interfaces:**
- Produces: `temper_core::types::home::HomeAnchor` — `enum HomeAnchor { Context(ContextId), Cogmap(CogmapId) }`, `Debug, Clone, PartialEq, Serialize, Deserialize`.
- Produces: `CreateResource.context: ContextId` → `pub home: HomeAnchor`.
- Consumes (Task D): the ingest handler builds `HomeAnchor::Cogmap(..)` when `home_cogmap_id` is set, else `HomeAnchor::Context(..)`.

- [ ] **Step 1: Write the failing test** — unit test in `crates/temper-core/src/types/home.rs` round-tripping the enum (parse-don't-validate seam), plus a `cargo make check` gate that proves all construction sites compile after the rename.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ids::{CogmapId, ContextId};

    #[test]
    fn home_anchor_serde_roundtrip() {
        let c = HomeAnchor::Context(ContextId::new());
        let j = serde_json::to_string(&c).unwrap();
        assert_eq!(c, serde_json::from_str(&j).unwrap());
        let m = HomeAnchor::Cogmap(CogmapId::new());
        let j = serde_json::to_string(&m).unwrap();
        assert_eq!(m, serde_json::from_str(&j).unwrap());
    }
}
```

- [ ] **Step 2: Run it — expect failure** (module doesn't exist).

Run: `cargo nextest run -p temper-core home_anchor_serde_roundtrip`
Expected: FAIL — unresolved module `home`.

- [ ] **Step 3: Define `HomeAnchor`** in `crates/temper-core/src/types/home.rs`:

```rust
//! The home of a resource: exactly one of a context or a cognitive map.
//! Parse-don't-validate: surfaces resolve a ref into one variant before
//! building a `CreateResource` command — never a placeholder id plus a flag.

use serde::{Deserialize, Serialize};

use crate::types::ids::{CogmapId, ContextId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HomeAnchor {
    Context(ContextId),
    Cogmap(CogmapId),
}
```
Wire it: add `pub mod home;` to `crates/temper-core/src/types/mod.rs`.

- [ ] **Step 4: Rename the command field.** In `crates/temper-workflow/src/operations/commands.rs:26-31`, change `pub context: ContextId,` to `pub home: temper_core::types::home::HomeAnchor,` (keep the doc comment, updated to describe the polymorphic home). Ensure `HomeAnchor` is imported.

- [ ] **Step 5: Map in `db_backend`.** At `crates/temper-api/src/backend/db_backend.rs:787`, replace `let home = cmd.context;` with a match producing an `AnchorRef`:

```rust
let home = match cmd.home {
    HomeAnchor::Context(c) => AnchorRef::context(c),
    HomeAnchor::Cogmap(m) => AnchorRef::cogmap(m),
};
```
At line ~862 the `CreateParams { home, .. }` now consumes this `AnchorRef` directly (drop the `AnchorRef::context(...)` wrap added in Task B for this site). Add imports for `HomeAnchor` and `AnchorRef`.

- [ ] **Step 6: Update every `CreateResource {` construction site** — `context: <expr>` → `home: HomeAnchor::Context(<expr>)`. Sites (grep `rg 'CreateResource \{' crates/`):
  - `crates/temper-cli/src/commands/resource.rs:237` (CLI placeholder `ContextId::new()` → `HomeAnchor::Context(ContextId::new())`).
  - `crates/temper-cli/src/cloud_backend/backend.rs:512`, `crates/temper-cli/src/cloud_backend/translators.rs:234` (test).
  - `crates/temper-mcp/src/tools/resources.rs:372`.
  - `crates/temper-api/src/handlers/ingest.rs:62`, `crates/temper-api/src/handlers/resources.rs:167`.
  - `crates/temper-workflow/src/operations/actions.rs:730` (test), `crates/temper-workflow/src/operations/commands.rs:285` (test/Default site).
  (The scenario `Step::CreateResource` in `temper-substrate/src/scenario/{runner,model}.rs` is a different enum — leave it.)

- [ ] **Step 7: Whole-workspace check — expect PASS.**

Run: `cargo make check` (fmt + clippy `-D warnings` + machete, all-features). Per `feedback_atomic_commits_for_type_refactors`, the whole workspace must be green in one shot.
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-core/src/types/home.rs crates/temper-core/src/types/mod.rs crates/temper-workflow/src/operations/commands.rs crates/temper-api/src/backend/db_backend.rs crates/temper-cli crates/temper-mcp
git commit -m "Surface B Beat 1C: HomeAnchor enum on CreateResource + backend mapping"
```

---

## Task D — Beat 1D: cogmap home wire + `--cogmap` create surface + write gate

**Files:**
- Create: `migrations/20260629000005_cogmap_home_authz_and_scope.sql` (both functions; `cogmap_scope_ids` consumed in Task E)
- Modify: `crates/temper-core/src/types/ingest.rs` (`IngestPayload.home_cogmap_id`)
- Modify: `crates/temper-api/src/handlers/ingest.rs` (branch + gate)
- Modify: `crates/temper-cli/src/cli.rs:276-319`, `crates/temper-cli/src/commands/resource.rs` (192-302, `CreateResourceArgs` 172-189), `crates/temper-cli/src/cloud_backend/translators.rs` (set `home_cogmap_id`)
- Modify: `crates/temper-mcp/src/tools/resources.rs` (`CreateResourceInput.cogmap` 22-54, resolve 315-319)
- Test: `crates/temper-api/tests/cogmap_home_test.rs` (new)

**Interfaces:**
- Produces (SQL): `cogmap_authorable_by_profile(p_profile uuid, p_cogmap uuid) RETURNS boolean` (delegates to `cogmap_readable_by_profile`). `cogmap_scope_ids(p_principal uuid, p_cogmap uuid) RETURNS SETOF uuid` — cogmap-homed ids the principal can see, gated by `cogmap_readable_by_profile` (deny → zero rows).
- Produces (wire): `IngestPayload.home_cogmap_id: Option<Uuid>` (`#[serde(default)]`). When `Some`, the home is that cogmap and `context_ref` is ignored; when `None`, existing context behavior.
- Consumes: `HomeAnchor` (Task C), `cogmap_readable_by_profile` (existing, `canonical_functions.sql:259-267`).

- [ ] **Step 1: Write the migration** `migrations/20260629000005_cogmap_home_authz_and_scope.sql`:

```sql
-- Surface B Beat 1: producer write seam + single-map search scope.

-- Authorial RBAC seam. Today team-cogmap membership confers write; the cogmap-arc
-- RBAC tightens this WITHOUT touching call sites. (memory: project_authorial_rbac_undefined_contexts_cogmaps)
CREATE FUNCTION cogmap_authorable_by_profile(p_profile uuid, p_cogmap uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT cogmap_readable_by_profile(p_profile, p_cogmap);
$$;

-- Single-map search scope: resources homed in the cogmap that the principal can see,
-- gated by map readability (deny -> zero rows -> empty corpus, never an error).
CREATE FUNCTION cogmap_scope_ids(p_principal uuid, p_cogmap uuid)
RETURNS SETOF uuid LANGUAGE sql STABLE AS $$
    SELECT h.resource_id
    FROM kb_resource_homes h
    WHERE h.anchor_table = 'kb_cogmaps'
      AND h.anchor_id = p_cogmap
      AND cogmap_readable_by_profile(p_principal, p_cogmap)
      AND h.resource_id IN (SELECT resource_id FROM resources_visible_to(p_principal));
$$;
```

- [ ] **Step 2: Write the failing surface test** `crates/temper-api/tests/cogmap_home_test.rs` (gated `#![cfg(feature = "test-db")]`, `#[sqlx::test(migrator = "temper_api::MIGRATOR")]`). Model setup on `reconcile_cogmap_test.rs` (cogmap genesis + team membership via direct SQL) and `search_context_ref_test.rs` (POST helpers). Three tests:

```rust
// 1. create --cogmap homes the resource in the map (anchor_table='kb_cogmaps').
async fn create_cogmap_homed_resource_writes_cogmap_home() { /* POST /api/ingest with home_cogmap_id=Some(map); assert kb_resource_homes row table='kb_cogmaps' */ }
// 2. a cogmap-homed resource is invisible to Surface-A context search of the owner's context.
async fn cogmap_homed_resource_invisible_to_context_search() { /* create context resource + cogmap resource sharing FTS term; search --context @me/x returns only the context one */ }
// 3. a principal who cannot read the map gets 403 on --cogmap create (auth before writes).
async fn create_into_unreadable_cogmap_is_forbidden() { /* second profile not in the map's team; POST home_cogmap_id=map; assert 403 and NO kb_resource_homes row written */ }
```
Each test seeds `kb_team_cogmaps` + `kb_team_members` directly (the reconcile-test pattern) so membership is real.

- [ ] **Step 3: Run — expect failure** (`home_cogmap_id` field missing / handler ignores it).

Run: `DATABASE_URL=… cargo nextest run -p temper-api --features test-db --test cogmap_home_test`
Expected: FAIL (compile: no `home_cogmap_id`; or behavior: row homed in a context).

- [ ] **Step 4: Add the wire field.** In `crates/temper-core/src/types/ingest.rs` after `context_ref` (line 20):
```rust
    /// When set, the resource is homed in this cognitive map (`anchor_table='kb_cogmaps'`)
    /// and `context_ref` is ignored. Resolved client-side (cogmap refs are trailing-UUID-only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_cogmap_id: Option<uuid::Uuid>,
```

- [ ] **Step 5: Branch + gate in the ingest handler.** In `crates/temper-api/src/handlers/ingest.rs:29-80`, before resolving `context_ref`, branch on `payload.home_cogmap_id`:

```rust
let home = match payload.home_cogmap_id {
    Some(map) => {
        let cogmap = CogmapId::from(map);
        // Auth before writes: producer gate (seam → membership).
        let ok: bool = sqlx::query_scalar!(
            "SELECT cogmap_authorable_by_profile($1, $2)", profile_id.uuid(), map
        ).fetch_one(&pool).await?.unwrap_or(false);
        if !ok { return Err(ApiError::Forbidden); }
        HomeAnchor::Cogmap(cogmap)
    }
    None => {
        // existing context_ref parse + resolve_context_ref path
        HomeAnchor::Context(resolved_context_id)
    }
};
let cmd = CreateResource { home, /* ...rest unchanged... */ };
```
(Match the handler's actual error type / pool accessor / profile extractor — read the file. `ApiError::Forbidden` mirrors `resolve_context_ref`'s Forbidden.)

- [ ] **Step 6: CLI `--cogmap` on create.** In `crates/temper-cli/src/cli.rs:276-319` add `#[arg(long)] cogmap: Option<String>,`. In `crates/temper-cli/src/commands/resource.rs`: add `cogmap: Option<&'a str>` to `CreateResourceArgs` (172-189); in `create()` (192-302) enforce mutual exclusion (exactly one of `--context`/`--cogmap`; error otherwise — replace the unconditional `require_context` with a check), resolve `--cogmap` via `temper_workflow::operations::parse_ref(cogmap)?.0` to a `CogmapId`, and set it on the ingest path. Thread `home_cogmap_id` through `cloud_backend/translators.rs` `cmd_to_ingest_payload` (~61-112) — set `home_cogmap_id` from the cmd's home when it's `Cogmap`. (When `--cogmap` is used, send `context_ref` as empty string; the server branches on `home_cogmap_id` first.)

- [ ] **Step 7: MCP create input.** In `crates/temper-mcp/src/tools/resources.rs` add `pub cogmap: Option<String>` to `CreateResourceInput` (22-54); in the handler (296-409) resolve via `parse_ref` and set `home_cogmap_id` symmetric to the CLI. (String field — no enum, so no `schemars(inline)` concern.)

- [ ] **Step 8: Regenerate sqlx caches** (new `query_scalar!` macro calls):

Run: `cargo sqlx prepare --workspace -- --all-features && cargo make prepare-api`

- [ ] **Step 9: Run the surface tests — expect PASS.**

Run: `DATABASE_URL=… cargo nextest run -p temper-api --features test-db --test cogmap_home_test`
Expected: all three PASS.

- [ ] **Step 10: `cargo fmt` then commit**

```bash
cargo fmt
git add migrations/20260629000005_cogmap_home_authz_and_scope.sql crates/temper-core/src/types/ingest.rs crates/temper-api crates/temper-cli crates/temper-mcp
git commit -m "Surface B Beat 1D: --cogmap home on create (wire + gate + CLI + MCP)"
```

---

## Task E — Beat 1E: `--cogmap` search = single-map direct scope

**Files:**
- Modify: `crates/temper-core/src/types/api.rs` (`SearchParams.cogmap_id` 40-92 + `Default` 76-92)
- Modify: `crates/temper-api/src/backend/substrate_read.rs` (`search_select` 325-373)
- Modify: `crates/temper-cli/src/cli.rs:222-249`, `crates/temper-cli/src/main.rs:459-490`, `crates/temper-cli/src/actions/search.rs` (`CliSearchArgs` 25-36, `build_search_params` 39-60), `crates/temper-cli/src/commands/search_cmd.rs:15-25`
- Test: `crates/temper-api/tests/cogmap_home_test.rs` (extend)

**Interfaces:**
- Consumes: `cogmap_scope_ids` (Task D migration), `unified_search` `p_scope_ids` (Task A).
- Produces (wire): `SearchParams.cogmap_id: Option<Uuid>` (`#[serde(default)]`) — resolved client-side from `--cogmap <ref>`.

- [ ] **Step 1: Write the failing test** — extend `cogmap_home_test.rs`:

```rust
// search --cogmap returns the map's homed resource and ranks it; non-member -> zero rows.
async fn cogmap_search_scopes_to_map() {
    // create a cogmap-homed resource with FTS term "zmapword"; member searches {cogmap_id:Some(map), query:"zmapword"} -> contains it.
}
async fn cogmap_search_denied_for_non_member_returns_zero() {
    // second profile not in the map's team searches the same -> 200 with empty results (not error).
}
```

- [ ] **Step 2: Run — expect failure** (no `cogmap_id` field / not scoped).

Run: `DATABASE_URL=… cargo nextest run -p temper-api --features test-db --test cogmap_home_test cogmap_search`
Expected: FAIL.

- [ ] **Step 3: Add the wire field.** In `crates/temper-core/src/types/api.rs`, add to `SearchParams` (after `context_ref`):
```rust
    /// Single-map scope (Surface B). Resolved client-side (cogmap refs are trailing-UUID-only).
    /// Mutually exclusive with `context_ref`. When set, the corpus is the map's homed
    /// participants the principal can see.
    #[serde(default)]
    pub cogmap_id: Option<Uuid>,
```
Add `cogmap_id: None` to the `Default` impl (76-92).

- [ ] **Step 4: Resolve scope in `search_select`.** In `crates/temper-api/src/backend/substrate_read.rs:325-373`, after the existing `context_ref` resolution, when `params.cogmap_id` is `Some(map)`, fetch the scope ids and pass them; reject combining with `context_ref` (`ApiError::BadRequest`):

```rust
let scope_ids: Option<Vec<Uuid>> = match params.cogmap_id {
    Some(map) => {
        if context_id.is_some() {
            return Err(ApiError::BadRequest("context_ref and cogmap_id are mutually exclusive".into()));
        }
        Some(
            sqlx::query_scalar!("SELECT cogmap_scope_ids($1, $2)", principal.uuid(), map)
                .fetch_all(&pool).await?
                .into_iter().flatten().collect(),
        )
    }
    None => None,
};
// build UnifiedSearchQuery with scope_ids: scope_ids.as_deref()
```
(Note: an empty `Some(vec![])` correctly yields zero rows via the `c.id = ANY('{}')` predicate — the deny case. Confirm `cogmap_scope_ids` returns `Option<Uuid>` rows from the macro and `.flatten()` is right; adjust to the macro's actual nullability.)

- [ ] **Step 5: CLI `--cogmap` on search.** `cli.rs:222-249` add `#[arg(long)] cogmap: Option<String>,`; thread through `main.rs:459-490` into `CliSearchArgs` (add `cogmap: Option<&'a str>` at `search.rs:25-36`) and `search_cmd.rs:15-25` re-bundle; in `build_search_params` (39-60) map `args.cogmap` via `parse_ref(..)?.0.uuid()` into `cogmap_id`.

- [ ] **Step 6: Regenerate caches** (new macro):

Run: `cargo sqlx prepare --workspace -- --all-features && cargo make prepare-api`

- [ ] **Step 7: Run — expect PASS.**

Run: `DATABASE_URL=… cargo nextest run -p temper-api --features test-db --test cogmap_home_test`
Expected: all tests PASS.

- [ ] **Step 8: `cargo fmt` then commit**

```bash
cargo fmt
git add crates/temper-core/src/types/api.rs crates/temper-api/src/backend/substrate_read.rs crates/temper-cli
git commit -m "Surface B Beat 1E: --cogmap single-map search scope"
```

---

## Final: consolidated review + verification + PR

- [ ] **Consolidated spec/code review** (deferred per `feedback_subagent_review_cadence`): review all five tasks against spec §3, §6, §7, §9 (the Half-1 ACs) and the code-quality lens in one pass.
- [ ] **Full verification** (evidence before claims, per superpowers:verification-before-completion):
  - `cargo make check` (fmt + clippy -D warnings + machete, all-features) — PASS.
  - `cargo build -p temper-cli --bin temper` then reinstall (`cargo install --path crates/temper-cli`) — per `feedback_reinstall_temper_after_cli_merge` / `feedback_nextest_does_not_rebuild_spawned_temper_bin`.
  - `cargo make test-artifacts` (cogmap/ONNX substrate tier) — PASS.
  - `cargo nextest run -p temper-api --features test-db --test cogmap_home_test --test search_context_ref_test` — PASS.
  - **e2e tier** (`cargo make test-e2e`) per `feedback_access_semantics_changes_need_e2e_tier` — the deny→403/zero gating is access-semantics; test-db alone is a false signal.
  - Grep no bare-context regressions: `cargo make test-e2e-embed` if any wire/ingest fixtures touched (`feedback_local_test_e2e_green_false_signal_for_embed`).
- [ ] **Push + open PR** (per `feedback_always_push_pr_never_merge_local`; merge `origin/main` first per `feedback_merge_main_before_pushing_pr`). Title: `Surface B Half 1: cogmap-as-home (--cogmap create + search) + p_scope_ids`.
- [ ] **Queue Beat 2** — create the next task (`--wayfind` region-salience funnel, spec §4) in `@me/temper`, and save the session note.

---

## Self-Review

**Spec coverage (Half 1, §3/§6/§9):**
- §6 `p_scope_ids` additive filter → Task A. ✅
- §3 `--cogmap` as resource home (create writes `anchor_table='kb_cogmaps'`) → Tasks B/C/D. ✅
- §3 producer write gate (route through a producer-side check before write) → Task D `cogmap_authorable_by_profile`, auth-before-write in handler. ✅
- §5/§9 `--cogmap` single-map scope = the direct homed-participant set (the degenerate/cold-start path) → Task E `cogmap_scope_ids`. A region-less map is irrelevant here because `--cogmap` never touches regions. ✅
- §7 gating at every stage (map admission + member visibility) → `cogmap_scope_ids` composes `cogmap_readable_by_profile` ∧ `resources_visible_to`; `unified_search` re-gates inside each candidate fn. ✅
- §9 AC "invisible to Surface A, visible to `--cogmap`; deny→zero rows" → Task D test 2 + Task E tests. ✅
- §9 AC "green under test-artifacts" → substrate tests (A, B) run there; api tests run under test-db (where cogmap tests already live). ✅
- **Edge assert `--cogmap`** → consciously Deferred (see rationale). The §9 "create/edge writes a home row" AC is satisfied by *create* (edges have no home row); the edge half is not a home-row producer.
- **Half 2 (§4 wayfind)** → out of scope, Beat 2. ✅

**Type consistency:** `HomeAnchor::{Context,Cogmap}` (Task C) ↔ `AnchorRef::{context,cogmap}` (Task B) ↔ `home_cogmap_id`/`cogmap_id` wire fields (D/E). `cogmap_scope_ids` (D) consumed by `search_select` → `p_scope_ids` (A). `scope_ids: Option<&[Uuid]>` field name consistent A↔E. ✅

**Placeholder scan:** SQL CTE bodies in Task A Step 3 are explicitly marked "paste verbatim from the Beat-2 source" rather than paraphrased — the implementer transcribes the unchanged CTEs; only `corpus` and the signature change. No TODO/TBD. ✅
