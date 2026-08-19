# Atlas C3.1 Legibility & Reachability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Graph Atlas usable above the node — labelled territories, reachable cogmap-node content, an honest region count, and neighborhoods that actually render their neighbors.

**Architecture:** Two independent phases. **Phase A** fixes the backend reads (bidirectional neighborhood walk + honest degree; a net-new cogmap-scoped R4 stack; removal of the semantically-inverted sub-cluster count). **Phase B** fixes presentation (derive region labels in R2 + truncate; tier-2 label collision; verify pack sizing + empty-territory ghost; payload column nit). Each task is TDD where a test can assert behavior, and render-verified on the `/dev/atlas` harness where it's visual.

**Tech Stack:** Rust (sqlx macro queries, Postgres functions), Axum (temper-api), SvelteKit + Svelte 5 runes + D3 (temper-ui), ts-rs codegen, cargo-nextest, e2e crate (real Axum + Postgres).

**Spec:** `docs/superpowers/specs/2026-07-06-atlas-c31-legibility-reachability-beat-design.md`

## Global Constraints

- **Never edit a shipped migration.** All SQL changes are NEW migration files in `migrations/`. `CREATE OR REPLACE FUNCTION` for unchanged signatures; `DROP FUNCTION` for removals. Additive-only on main.
- **SECURITY INVARIANT (from `migrations/20260703130000_...:10-18`):** every function emitting/counting an access-controlled row reproduces the FULL canonical visibility predicate — resources → `resources_visible_to` (or `resources_in_*_scope ⊆ it`); edges → `NOT is_folded AND anchor_readable_by_profile(home) AND endpoint_readable_by_profile(source) AND endpoint_readable_by_profile(target)`; regions/cogmaps → `cogmap_readable_by_profile`. Gating on a SUBSET leaks private relationships.
- **Access-semantics changes need the e2e access tier.** test-db green is a false signal for A1/A2. Run `cargo make test-e2e`; the harness mints admins via direct `kb_team_members` owner-writes that test-db never exercises.
- **After any SQL change:** `cargo sqlx prepare --workspace -- --all-features` → `cargo make prepare-services` → `cargo make prepare-e2e` (whichever targets the changed macro queries). `cargo make check` is `SQLX_OFFLINE=true` — the honest local probe of committed caches.
- **After any change to a `#[ts(export)]` struct:** `cargo make generate-ts-types` and commit the regenerated `packages/temper-ui/src/lib/types/generated/*.ts` (even unrelated regenerated files ride along).
- **e2e tests spawn the `temper` bin:** nextest rebuilds the lib but not the bin — `cargo build -p temper-cli --bin temper` before running e2e if CLI behavior changed (N/A here, but keep in mind if a test spawns the binary).
- **Every implementer runs `cargo fmt` before committing** — `cargo make check` gates on `cargo fmt --check`.
- **Harness verify** = `cd packages/temper-ui && bun run dev`, open `http://localhost:5173/dev/atlas`, pick the named scenario. Dev-only route (404 outside dev).

---

# Phase A — Reachability (backend reads)

> **Note on sqlx:** the graph service (`crates/temper-services/src/services/graph_service.rs`) uses
> **runtime** `sqlx::query_as::<_, (…)>` / `query_scalar` (turbofish, not the `query!` macros), so
> service edits need **no** `cargo sqlx prepare`. e2e fixtures use runtime `sqlx::query(...)` inserts
> too. Only add a `prepare-*` step if you introduce a `query!`/`query_as!` macro (this plan does not).

## Task 1 (spec A1): Bidirectional neighborhood walk + honest degree

**Why:** `graph_traverse_scoped` seeds outgoing-only (`e.source_id = ANY(p_seed_ids)`) so a node whose
edges are incoming yields zero neighbors; and `graph_atlas_nodes.degree` counts undirected
`edges_visible_to` with no team clamp, so the hover "N edges" over-counts what the walk can reach →
"4 edges, 0 shown." Make the walk bidirectional and clamp degree to team scope so the number is honest.

**Files:**
- Create: `migrations/20260706120000_atlas_bidirectional_walk_honest_degree.sql`
- Test: `tests/e2e/tests/graph_atlas_slice_e2e.rs` (extend — add one test)

**Interfaces:**
- Consumes: existing `graph_traverse_scoped(profile,team,seeds,depth,edge_kinds) RETURNS TABLE(id,source_id,target_id,edge_kind,polarity,label,weight)` and `graph_atlas_nodes(profile,team,ids) RETURNS TABLE(id,title,doc_type,home,degree,first_chunk)` — both bodies rewritten, **signatures/RETURNS unchanged** (so `CREATE OR REPLACE`, no service change).
- Produces: same shapes; `neighborhood_slice` (`graph_service.rs:262`) is untouched.

- [ ] **Step 1: Write the failing test** — a seed whose only edge is *incoming* must return its neighbor, and the seed's returned `degree` must equal the number of distinct neighbors rendered.

Append to `tests/e2e/tests/graph_atlas_slice_e2e.rs` (reuse the file's existing helpers: `common::setup`, `provision_profile`, `create_team`, `add_member`, `create_resource`, `grant_read_to_team`, `assert_edge`, `slice`):

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn incoming_edge_neighbor_is_reachable_and_degree_is_honest(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let profile = provision_profile(&app, &app.token).await;
    let team = create_team(&pool, "bidir-team").await;
    add_member(&pool, team, profile).await;

    // seed S and neighbor N, both in team scope; the ONLY edge is N -> S (incoming to S).
    let seed = create_resource(&pool, "seed").await;
    let nbr = create_resource(&pool, "neighbor").await;
    grant_read_to_team(&pool, team, seed).await;
    grant_read_to_team(&pool, team, nbr).await;
    assert_edge(&pool, nbr, seed).await; // source=nbr, target=seed

    let body = serde_json::json!({ "seeds": [seed], "depth": 2, "edge_kinds": [] });
    let res = slice(&app, &app.token, team, body).await;
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let sub: temper_core::types::graph_atlas::AtlasSubgraph = res.json().await.unwrap();

    // BEFORE the fix: nodes == [seed] only. AFTER: neighbor is reachable via the incoming edge.
    assert!(sub.nodes.iter().any(|n| n.id == nbr), "incoming-edge neighbor must be reachable");
    // degree honesty: the seed's reported degree equals the distinct neighbors actually rendered.
    let seed_node = sub.nodes.iter().find(|n| n.id == seed).expect("seed node present");
    let rendered_neighbors = sub.nodes.iter().filter(|n| n.id != seed).count() as i32;
    assert_eq!(seed_node.degree, rendered_neighbors, "hover degree must equal rendered neighbor count");
}
```

- [ ] **Step 2: Run the test, verify it fails**

Run: `cd tests/e2e && cargo test --test graph_atlas_slice_e2e incoming_edge_neighbor -- --nocapture`
(Single-target `cargo test` bypasses the nextest `--list` hang on fresh e2e binaries; needs Docker Postgres up: `cargo make docker-up`.)
Expected: FAIL — the neighbor is absent (outgoing-only walk) and/or degree != rendered count.

- [ ] **Step 3: Write the migration** — bidirectional node-frontier walk + team-clamped degree.

Create `migrations/20260706120000_atlas_bidirectional_walk_honest_degree.sql`:

```sql
-- A1: make the neighborhood walk bidirectional and the node degree honest.
--
-- Body-only changes; RETURNS TABLE shapes are unchanged from their current defs
-- (graph_traverse_scoped: 20260704000009; graph_atlas_nodes: 20260706000001), so
-- CREATE OR REPLACE is valid. Shipped migrations stay immutable.
--
-- SECURITY INVARIANT preserved conjunct-for-conjunct: every edge still requires
-- both endpoints in team scope (⊆ resources_visible_to), NOT is_folded, and the
-- edge's home anchor readable. Widening direction does NOT widen the visibility
-- set — team scope is the security boundary and is unchanged.

CREATE OR REPLACE FUNCTION graph_traverse_scoped(
    p_profile     uuid,
    p_team        uuid,
    p_seed_ids    uuid[],
    p_depth       int,
    p_edge_kinds  edge_kind[]
) RETURNS TABLE(
    id uuid, source_id uuid, target_id uuid, edge_kind edge_kind,
    polarity edge_polarity, label text, weight double precision
) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE scope AS (
        SELECT resource_id AS id FROM resources_in_team_scope(p_profile, p_team)
    ),
    -- BFS over the frontier NODE set, following in-scope visible edges in EITHER
    -- direction; the opposite endpoint becomes the next frontier. UNION dedups.
    reached AS (
        SELECT unnest(p_seed_ids) AS node_id, 0 AS depth
        UNION
        SELECT CASE WHEN e.source_id = r.node_id THEN e.target_id ELSE e.source_id END, r.depth + 1
        FROM reached r
        JOIN kb_edges e
          ON (e.source_id = r.node_id OR e.target_id = r.node_id)
        JOIN scope ss ON ss.id = e.source_id
        JOIN scope st ON st.id = e.target_id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND NOT e.is_folded
          AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
          AND r.depth < LEAST(p_depth, 10)
          AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
               OR e.edge_kind = ANY(p_edge_kinds))
    )
    -- Return every visible in-scope edge whose BOTH endpoints were reached.
    SELECT DISTINCT e.id, e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, e.weight
    FROM kb_edges e
    JOIN reached rs ON rs.node_id = e.source_id
    JOIN reached rt ON rt.node_id = e.target_id
    JOIN scope ss ON ss.id = e.source_id
    JOIN scope st ON st.id = e.target_id
    WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
      AND NOT e.is_folded
      AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
      AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
           OR e.edge_kind = ANY(p_edge_kinds));
$$;

CREATE OR REPLACE FUNCTION graph_atlas_nodes(
    p_profile uuid, p_team uuid, p_ids uuid[]
) RETURNS TABLE(id uuid, title text, doc_type text, home text, degree int, first_chunk text)
LANGUAGE sql STABLE AS $$
    WITH scope AS (
        SELECT resource_id AS id FROM resources_in_team_scope(p_profile, p_team)
    ),
    ids AS (SELECT DISTINCT unnest(p_ids) AS id),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table = 'kb_resources' AND p.property_key = 'doc_type' AND NOT p.is_folded
    )
    SELECT r.id, r.title, d.dt AS doc_type, h.home,
           COALESCE(deg.degree, 0) AS degree,
           (SELECT cc.content FROM kb_chunks ch
              JOIN kb_content_blocks b ON b.id = ch.block_id
              JOIN kb_chunk_content cc ON cc.chunk_id = ch.id
             WHERE ch.resource_id = r.id AND ch.is_current AND NOT b.is_folded
             ORDER BY b.seq, ch.chunk_index LIMIT 1) AS first_chunk
    FROM ids
    JOIN scope s   ON s.id = ids.id
    JOIN kb_resources r ON r.id = ids.id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN LATERAL (
        SELECT CASE WHEN bool_or(h2.anchor_table = 'kb_cogmaps') THEN 'cogmap' ELSE 'context' END AS home
        FROM kb_resource_homes h2 WHERE h2.resource_id = r.id
    ) h ON true
    -- HONEST DEGREE: clamp both endpoints to team scope so the count equals what the
    -- (team-scoped) walk can render. Was: edges_visible_to only, which over-counted
    -- cross-scope / cross-team edges the walk can never show.
    LEFT JOIN LATERAL (
        SELECT count(*)::int AS degree
        FROM kb_edges e
        JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
        JOIN scope se  ON se.id  = e.source_id
        JOIN scope se2 ON se2.id = e.target_id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true;
$$;
```

- [ ] **Step 4: Apply the migration and run the test, verify it passes**

Run: `sqlx migrate run` (dev DB on 5437; `cargo make` tasks force `SQLX_OFFLINE`, so apply directly), then
`cd tests/e2e && cargo test --test graph_atlas_slice_e2e incoming_edge_neighbor -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Run the full slice e2e file (no regressions)**

Run: `cd tests/e2e && cargo test --test graph_atlas_slice_e2e`
Expected: all PASS (existing member-OK / outsider-404 / empty-BAD_REQUEST still hold — the visibility set is unchanged).

- [ ] **Step 6: Commit**

```bash
git add migrations/20260706120000_atlas_bidirectional_walk_honest_degree.sql tests/e2e/tests/graph_atlas_slice_e2e.rs
git commit -m "fix(atlas): bidirectional neighborhood walk + team-honest degree (A1/#4)"
```

---

## Task 2 (spec A3): Drop the inverted "sub-clusters" badge

**Why:** `kb_cogmap_regions.component_id` proves components are the region's *parent* grain, so
"sub-clusters of a region" is inverted and un-fixable; and `graph_region_components` returns every
component in the whole cogmap/lens (no `reg.id` tie). Remove the concept from R3.

**Files:**
- Create: `migrations/20260706120100_drop_graph_region_components.sql`
- Modify: `crates/temper-core/src/types/graph_territory.rs` (remove `components` field + `Component` struct)
- Modify: `crates/temper-services/src/services/graph_service.rs:636-645,669` (stop selecting/assembling components)
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/TierTerritory.svelte:37-40` (remove badge)
- Regenerate: `packages/temper-ui/src/lib/types/generated/graph_territory.ts`

**Interfaces:**
- Produces: `TerritorySlice { region_id, label, members }` (no `components`); `Component` struct removed.

- [ ] **Step 1: Update the failing test expectation** — the R3 SQL test should no longer expect a component count. In `tests/e2e/tests/graph_territory_slice_e2e.rs` (or `_sql_test.rs`), find the assertion referencing `components` and change it to assert `slice.members.len()` matches the visible member fixtures. If no such assertion exists, add:

```rust
// A3: the slice no longer carries a components array; members are the only interior grain.
assert_eq!(slice.members.len(), expected_visible_members);
```

Run: `cd tests/e2e && cargo test --test graph_territory_slice_e2e` → Expected: FAIL to COMPILE (field `components` still referenced elsewhere) — that compile error is the failing state driving the removal.

- [ ] **Step 2: Remove the wire field + struct (audit both ends)**

First grep for every `Component` reference so the removal is complete (the type is registered in the
OpenAPI doc and re-exported, not only defined):

Run: `rg -n '\bComponent\b' crates/ packages/temper-ui/src/lib/types/ | rg -v RegionMember`
Expected refs to clear: the struct def + `TerritorySlice.components` field (`graph_territory.rs`), the
service assembly (`graph_service.rs`), any `pub use ...Component` re-export in `graph_territory`'s mod,
and any `components(schemas(... Component ...))` entry in the utoipa OpenAPI registration (search
`rg -n 'Component' crates/temper-api/src -g '!*.md'`). Then:
- delete the `Component` struct (`graph_territory.rs` ~98-107) and the `pub components: Vec<Component>,`
  field from `TerritorySlice` (~119);
- remove any `Component` re-export and any utoipa `schemas(... Component ...)` list entry.

(`cargo make check` will fail to compile on any missed reference — that is the backstop, but grep first
so the OpenAPI-registration case doesn't surprise the build.)

- [ ] **Step 3: Update the service**

In `crates/temper-services/src/services/graph_service.rs`, delete the `components` query block (the `let components: Vec<Component> = sqlx::query_as...graph_region_components...collect();` at ~636-645) and remove `components` from the returned `Ok(TerritorySlice { region_id, label, components, members })` → `Ok(TerritorySlice { region_id, label, members })`. Remove the now-unused `Component` import.

- [ ] **Step 4: Drop the SQL function**

Create `migrations/20260706120100_drop_graph_region_components.sql`:

```sql
-- A3: components are a region's PARENT grain (kb_cogmap_regions.component_id), not
-- sub-clusters of it, and this fn returned all cogmap/lens components (no reg.id tie).
-- The R3 slice no longer surfaces components. Drop the dead function.
DROP FUNCTION IF EXISTS graph_region_components(uuid, uuid);
```

- [ ] **Step 5: Remove the UI badge**

In `packages/temper-ui/src/lib/components/graph/atlas/TierTerritory.svelte`, delete the badge `<g>` block (lines 37-40, the `{slice.components.length} sub-clusters` group). Leave the `slice.members` packing untouched.

- [ ] **Step 6: Regenerate TS types + apply migration + verify**

Run:
```bash
cargo make generate-ts-types           # updates generated/graph_territory.ts (no more Component/components)
sqlx migrate run
cd tests/e2e && cargo test --test graph_territory_slice_e2e
```
Expected: PASS. Then `cd packages/temper-ui && bun run check` → Expected: no TS errors (TierTerritory no longer references `slice.components`).

- [ ] **Step 7: Commit**

```bash
git add migrations/20260706120100_drop_graph_region_components.sql \
        crates/temper-core/src/types/graph_territory.rs \
        crates/temper-services/src/services/graph_service.rs \
        packages/temper-ui/src/lib/components/graph/atlas/TierTerritory.svelte \
        packages/temper-ui/src/lib/types/generated/graph_territory.ts
git commit -m "refactor(atlas): drop inverted region sub-cluster count (A3/#3)"
```

---

## Task 3 (spec A2, SQL): Cogmap-scoped scope + traverse + node functions

**Why:** no cogmap equivalent of the team R4 stack exists, so cogmap nodes dead-end. Add the SQL
siblings that scope to cogmap membership instead of team scope.

**Files:**
- Create: `migrations/20260706120200_atlas_cogmap_neighborhood.sql`
- Test: `tests/e2e/tests/graph_atlas_cogmap_slice_sql_test.rs` (new)

**Interfaces:**
- Produces:
  - `resources_in_cogmap_scope(p_profile uuid, p_cogmap uuid) RETURNS TABLE(resource_id uuid)`
  - `graph_traverse_cogmap_scoped(p_profile,p_cogmap,p_seed_ids,p_depth,p_edge_kinds) RETURNS TABLE(id,source_id,target_id,edge_kind,polarity,label,weight)` — same shape as `graph_traverse_scoped`.
  - `graph_atlas_nodes_cogmap(p_profile,p_cogmap,p_ids) RETURNS TABLE(id,title,doc_type,home,degree,first_chunk)` — same shape as `graph_atlas_nodes`.

- [ ] **Step 1: Write the failing SQL test** — a cogmap-homed seed with an in-cogmap neighbor returns the neighbor; a resource NOT visible to the profile is excluded.

Create `tests/e2e/tests/graph_atlas_cogmap_slice_sql_test.rs` (mirror the fixtures in `graph_atlas_slice_e2e.rs`, but home resources in a cogmap and use `SELECT * FROM graph_traverse_cogmap_scoped(...)` directly against `pool`):

```rust
#![cfg(feature = "test-db")]
mod common;

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cogmap_walk_reaches_in_cogmap_neighbor(pool: sqlx::PgPool) {
    // fixtures: create profile, a readable cogmap, two resources homed in it with an edge,
    // grant readability. (Use the cogmap-readability + kb_resource_homes(anchor='kb_cogmaps')
    // fixtures — see graph_cogmap_panorama_e2e.rs for the readable-cogmap setup.)
    // ...build `profile`, `cogmap`, `seed`, `nbr`, edge nbr->seed, both cogmap-homed + visible...

    let rows: Vec<(uuid::Uuid, uuid::Uuid, uuid::Uuid)> = sqlx::query_as(
        "SELECT id, source_id, target_id FROM graph_traverse_cogmap_scoped($1,$2,$3,$4,$5)",
    )
    .bind(profile).bind(cogmap).bind(&vec![seed]).bind(2_i32)
    .bind(Vec::<String>::new()) // empty edge_kinds => all kinds
    .fetch_all(&pool).await.unwrap();

    assert!(rows.iter().any(|(_, s, t)| *s == nbr && *t == seed),
        "incoming in-cogmap edge must be reachable");
}
```

Run: `cd tests/e2e && cargo test --test graph_atlas_cogmap_slice_sql_test` → Expected: FAIL (function `graph_traverse_cogmap_scoped` does not exist).

- [ ] **Step 2: Write the migration**

Create `migrations/20260706120200_atlas_cogmap_neighborhood.sql`:

```sql
-- A2: cogmap-scoped R4 neighborhood stack — the cogmap-door analog of the team stack.
-- Scope = resources homed in THIS cogmap that are visible to the profile. Entry gate
-- (cogmap_readable_by_profile) is enforced in the service. Full edge-visibility
-- predicate reproduced conjunct-for-conjunct (both endpoints in cogmap scope ⊆
-- resources_visible_to, NOT is_folded, home anchor readable).

CREATE FUNCTION resources_in_cogmap_scope(p_profile uuid, p_cogmap uuid)
RETURNS TABLE(resource_id uuid) LANGUAGE sql STABLE AS $$
    SELECT DISTINCT h.resource_id
    FROM kb_resource_homes h
    JOIN resources_visible_to(p_profile) v ON v.resource_id = h.resource_id
    WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = p_cogmap;
$$;

CREATE FUNCTION graph_traverse_cogmap_scoped(
    p_profile     uuid,
    p_cogmap      uuid,
    p_seed_ids    uuid[],
    p_depth       int,
    p_edge_kinds  edge_kind[]
) RETURNS TABLE(
    id uuid, source_id uuid, target_id uuid, edge_kind edge_kind,
    polarity edge_polarity, label text, weight double precision
) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE scope AS (
        SELECT resource_id AS id FROM resources_in_cogmap_scope(p_profile, p_cogmap)
    ),
    reached AS (
        SELECT unnest(p_seed_ids) AS node_id, 0 AS depth
        UNION
        SELECT CASE WHEN e.source_id = r.node_id THEN e.target_id ELSE e.source_id END, r.depth + 1
        FROM reached r
        JOIN kb_edges e
          ON (e.source_id = r.node_id OR e.target_id = r.node_id)
        JOIN scope ss ON ss.id = e.source_id
        JOIN scope st ON st.id = e.target_id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND NOT e.is_folded
          AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
          AND r.depth < LEAST(p_depth, 10)
          AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
               OR e.edge_kind = ANY(p_edge_kinds))
    )
    SELECT DISTINCT e.id, e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, e.weight
    FROM kb_edges e
    JOIN reached rs ON rs.node_id = e.source_id
    JOIN reached rt ON rt.node_id = e.target_id
    JOIN scope ss ON ss.id = e.source_id
    JOIN scope st ON st.id = e.target_id
    WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
      AND NOT e.is_folded
      AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
      AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
           OR e.edge_kind = ANY(p_edge_kinds));
$$;

CREATE FUNCTION graph_atlas_nodes_cogmap(
    p_profile uuid, p_cogmap uuid, p_ids uuid[]
) RETURNS TABLE(id uuid, title text, doc_type text, home text, degree int, first_chunk text)
LANGUAGE sql STABLE AS $$
    WITH scope AS (
        SELECT resource_id AS id FROM resources_in_cogmap_scope(p_profile, p_cogmap)
    ),
    ids AS (SELECT DISTINCT unnest(p_ids) AS id),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table = 'kb_resources' AND p.property_key = 'doc_type' AND NOT p.is_folded
    )
    SELECT r.id, r.title, d.dt AS doc_type, h.home,
           COALESCE(deg.degree, 0) AS degree,
           (SELECT cc.content FROM kb_chunks ch
              JOIN kb_content_blocks b ON b.id = ch.block_id
              JOIN kb_chunk_content cc ON cc.chunk_id = ch.id
             WHERE ch.resource_id = r.id AND ch.is_current AND NOT b.is_folded
             ORDER BY b.seq, ch.chunk_index LIMIT 1) AS first_chunk
    FROM ids
    JOIN scope s   ON s.id = ids.id
    JOIN kb_resources r ON r.id = ids.id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN LATERAL (
        SELECT CASE WHEN bool_or(h2.anchor_table = 'kb_cogmaps') THEN 'cogmap' ELSE 'context' END AS home
        FROM kb_resource_homes h2 WHERE h2.resource_id = r.id
    ) h ON true
    LEFT JOIN LATERAL (
        SELECT count(*)::int AS degree
        FROM kb_edges e
        JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
        JOIN scope se  ON se.id  = e.source_id
        JOIN scope se2 ON se2.id = e.target_id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true;
$$;
```

- [ ] **Step 3: Apply + verify the SQL test passes**

Run: `sqlx migrate run` then `cd tests/e2e && cargo test --test graph_atlas_cogmap_slice_sql_test`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260706120200_atlas_cogmap_neighborhood.sql tests/e2e/tests/graph_atlas_cogmap_slice_sql_test.rs
git commit -m "feat(atlas): cogmap-scoped neighborhood SQL (A2 part 1/3)"
```

---

## Task 4 (spec A2, service + API): `cogmap_neighborhood_slice` endpoint + access-tier e2e

**Why:** wire the new SQL into a service (with the cogmap-readability gate before any read) and an HTTP
route mirroring the team slice, and prove it denies outsiders.

**Files:**
- Modify: `crates/temper-services/src/services/graph_service.rs` (add `cogmap_neighborhood_slice`)
- Modify: `crates/temper-api/src/handlers/graph.rs` (add handler)
- Modify: `crates/temper-api/src/routes.rs` (register `/api/cogmaps/{id}/graph/slice`)
- Test: `tests/e2e/tests/graph_atlas_cogmap_slice_e2e.rs` (new, HTTP, access-tier)

**Interfaces:**
- Consumes: `resources_in_cogmap_scope`, `graph_traverse_cogmap_scoped`, `graph_atlas_nodes_cogmap` (Task 3); `SliceRequest`, `AtlasSubgraph`, `AtlasNode`, `AtlasEdge` (`temper-core::types::graph_atlas`).
- Produces: `graph_service::cogmap_neighborhood_slice(pool, profile_id, cogmap_id, req) -> ApiResult<AtlasSubgraph>`; `POST /api/cogmaps/{id}/graph/slice`.

- [ ] **Step 1: Write the failing access-tier test** — reader gets OK + reachable neighbor; outsider gets 404; empty seeds → 400.

Create `tests/e2e/tests/graph_atlas_cogmap_slice_e2e.rs`, mirroring `graph_atlas_slice_e2e.rs`'s structure (its `slice(...)` helper, `provision_profile`, `common::generate_test_jwt` outsider) but POSTing `/api/cogmaps/{cogmap}/graph/slice` and using cogmap-readability fixtures (from `graph_cogmap_panorama_e2e.rs`):

```rust
#![cfg(feature = "test-db")]
mod common;
// ... helpers: provision_profile, create_readable_cogmap, home_resource_in_cogmap,
//     grant_cogmap_read, assert_edge, cogmap_slice(app, token, cogmap, body) ...

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reader_reaches_cogmap_neighborhood_outsider_denied(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let reader = provision_profile(&app, &app.token).await;
    let cogmap = create_readable_cogmap(&pool, reader, "cm").await;
    let seed = home_resource_in_cogmap(&pool, cogmap, "seed").await;
    let nbr  = home_resource_in_cogmap(&pool, cogmap, "nbr").await;
    assert_edge(&pool, nbr, seed).await; // incoming to seed

    // reader: OK + neighbor reachable
    let body = serde_json::json!({ "seeds": [seed], "depth": 2, "edge_kinds": [] });
    let ok = cogmap_slice(&app, &app.token, cogmap, body.clone()).await;
    assert_eq!(ok.status(), reqwest::StatusCode::OK);
    let sub: temper_core::types::graph_atlas::AtlasSubgraph = ok.json().await.unwrap();
    assert!(sub.nodes.iter().any(|n| n.id == nbr));

    // outsider (non-reader of the cogmap): deny-as-absence 404
    let outsider_jwt = common::generate_test_jwt("cm-slice-outsider", "out@test.example.com");
    provision_profile(&app, &outsider_jwt).await;
    let denied = cogmap_slice(&app, &outsider_jwt, cogmap, body.clone()).await;
    assert_eq!(denied.status(), reqwest::StatusCode::NOT_FOUND);

    // empty seeds → 400
    let empty = cogmap_slice(&app, &app.token, cogmap, serde_json::json!({ "seeds": [], "depth": 2, "edge_kinds": [] })).await;
    assert_eq!(empty.status(), reqwest::StatusCode::BAD_REQUEST);
}
```

Run: `cd tests/e2e && cargo test --test graph_atlas_cogmap_slice_e2e` → Expected: FAIL (route not found / 404 for reader too).

- [ ] **Step 2: Add the service function** — gate first, then read.

In `crates/temper-services/src/services/graph_service.rs`, after `neighborhood_slice`, add (mirror it exactly, swapping the team gate for the cogmap gate and the SQL fn names):

```rust
/// R4 cogmap-scoped neighborhood slice: the cogmap-door analog of `neighborhood_slice`.
/// Gate = cogmap_readable_by_profile (deny-as-absence); scope = resources homed in the cogmap.
pub async fn cogmap_neighborhood_slice(
    pool: &PgPool,
    profile_id: ProfileId,
    cogmap_id: Uuid,
    req: SliceRequest,
) -> ApiResult<AtlasSubgraph> {
    if req.seeds.is_empty() {
        return Err(ApiError::BadRequest("seeds must be non-empty".into()));
    }
    let readable: bool = sqlx::query_scalar("SELECT cogmap_readable_by_profile($1, $2)")
        .bind(profile_id.as_uuid())
        .bind(cogmap_id)
        .fetch_one(pool)
        .await?;
    if !readable {
        return Err(ApiError::NotFound);
    }
    let depth = req.depth.min(MAX_DEPTH) as i32;

    let edge_rows = sqlx::query_as::<_, (Uuid, Uuid, Uuid, EdgeKind, Polarity, Option<String>, f64)>(
        "SELECT id, source_id, target_id, edge_kind, polarity, label, weight \
         FROM graph_traverse_cogmap_scoped($1,$2,$3,$4,$5)",
    )
    .bind(profile_id.as_uuid())
    .bind(cogmap_id)
    .bind(&req.seeds)
    .bind(depth)
    .bind(&req.edge_kinds)
    .fetch_all(pool)
    .await?;

    let edges: Vec<AtlasEdge> = edge_rows
        .iter()
        .map(|(id, source, target, edge_kind, polarity, label, weight)| AtlasEdge {
            id: *id,
            source: *source,
            target: *target,
            edge_kind: *edge_kind,
            polarity: *polarity,
            label: label.clone(),
            weight: *weight,
        })
        .collect();

    let mut node_ids: Vec<Uuid> = req.seeds.clone();
    for (_, s, t, ..) in &edge_rows {
        node_ids.push(*s);
        node_ids.push(*t);
    }

    let node_rows = sqlx::query_as::<_, (Uuid, String, Option<String>, String, i32, Option<String>)>(
        "SELECT id, title, doc_type, home, degree, first_chunk \
         FROM graph_atlas_nodes_cogmap($1,$2,$3)",
    )
    .bind(profile_id.as_uuid())
    .bind(cogmap_id)
    .bind(&node_ids)
    .fetch_all(pool)
    .await?;

    let nodes: Vec<AtlasNode> = node_rows
        .into_iter()
        .map(|(id, title, doc_type, home, degree, first_chunk)| AtlasNode {
            id,
            title,
            doc_type,
            home: if home == "cogmap" { NodeHome::Cogmap } else { NodeHome::Context },
            degree,
            salience: None,
            excerpt: first_chunk.as_deref().and_then(compute_excerpt),
        })
        .collect();

    Ok(AtlasSubgraph { nodes, edges })
}
```

> Match the exact `AtlasEdge`/`AtlasNode` field construction used by `neighborhood_slice` (`graph_service.rs:299-348`) — if the real code differs from the sketch above (field names, `compute_excerpt` import path, enum variant names), copy from that function verbatim.

- [ ] **Step 3: Add the handler** — thin pass-through, mirror `neighborhood_slice` handler.

In `crates/temper-api/src/handlers/graph.rs`, after `neighborhood_slice`:

```rust
/// POST /api/cogmaps/{id}/graph/slice — R4 cogmap-scoped neighborhood slice.
#[utoipa::path(
    post,
    path = "/api/cogmaps/{id}/graph/slice",
    tag = "Graph",
    params(("id" = Uuid, Path, description = "Cogmap id to scope the slice to")),
    request_body = SliceRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Cogmap neighborhood slice", body = AtlasSubgraph),
        (status = 400, description = "Empty seed set"),
        (status = 404, description = "Cogmap not readable by this profile")
    )
)]
pub async fn cogmap_neighborhood_slice(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cogmap_id): Path<Uuid>,
    Json(req): Json<SliceRequest>,
) -> ApiResult<Json<AtlasSubgraph>> {
    graph_service::cogmap_neighborhood_slice(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        cogmap_id,
        req,
    )
    .await
    .map(Json)
}
```

- [ ] **Step 4: Register the route**

In `crates/temper-api/src/routes.rs`, near the existing team slice registration (`:92-95`), add:

```rust
.route(
    "/api/cogmaps/{id}/graph/slice",
    post(handlers::graph::cogmap_neighborhood_slice),
)
```

- [ ] **Step 5: Apply migration (if not already) + run the access-tier test**

Run: `cd tests/e2e && cargo test --test graph_atlas_cogmap_slice_e2e`
Expected: PASS (reader OK + neighbor reachable; outsider 404; empty 400).

- [ ] **Step 6: Full check + commit**

Run: `cargo make check` (fmt + clippy + docs; `cargo fmt` first). Then:
```bash
git add crates/temper-services/src/services/graph_service.rs \
        crates/temper-api/src/handlers/graph.rs \
        crates/temper-api/src/routes.rs \
        tests/e2e/tests/graph_atlas_cogmap_slice_e2e.rs
git commit -m "feat(atlas): cogmap neighborhood service + endpoint + access-tier e2e (A2 part 2/3)"
```

---

## Task 5 (spec A2, UI wiring): cogmap-node neighborhood in the loader + TrailRail parity

**Why:** the cogmap loader branch hard-codes `neighborhood: null` and omits selection/trail/resourceRow,
so even the new endpoint wouldn't render. Wire it and add rail parity so a cogmap node opens like a team node.

**Files:**
- Modify: `packages/temper-ui/src/lib/server/graph-reads.ts` (add `readCogmapNeighborhood` + path)
- Modify: `packages/temper-ui/src/routes/(app)/graph/[owner]/+page.server.ts` (cogmap branch:87-125)

**Interfaces:**
- Consumes: `POST /api/cogmaps/{id}/graph/slice` (Task 4); existing `readTrail`, `readResourceRow`, `selectedElement`, `NEIGHBORHOOD_DEPTH`.
- Produces: `readCogmapNeighborhood(token, cogmapId, req: SliceRequest): Promise<AtlasSubgraph>`.

- [ ] **Step 1: Add the client read**

In `packages/temper-ui/src/lib/server/graph-reads.ts`, add a path builder next to `neighborhoodSlicePath` (line 31) and a reader next to `readNeighborhood` (line 64):

```ts
const cogmapNeighborhoodSlicePath = (cogmapId: string) => `/api/cogmaps/${cogmapId}/graph/slice`;

export const readCogmapNeighborhood = (
	token: string,
	cogmapId: string,
	req: SliceRequest
): Promise<AtlasSubgraph> => apiPost<AtlasSubgraph>(cogmapNeighborhoodSlicePath(cogmapId), token, req);
```

- [ ] **Step 2: Wire the cogmap loader branch**

In `packages/temper-ui/src/routes/(app)/graph/[owner]/+page.server.ts`, import `readCogmapNeighborhood` (add to the existing `$lib/server/graph-reads` import block, lines 17-26) and `EdgeKind` is already imported. In the `if (cogmapId) { ... }` branch (87-125), replace the three-item `Promise.all` (93-97) and the `neighborhood: null` / selection block so it reads the neighborhood at Tier 2 and populates the rail:

```ts
	const [territories, slice, home, neighborhood] = await Promise.all([
		tier === 0 ? readCogmapPanorama(token, cogmapId) : Promise.resolve(null),
		tier === 1 && focus.kind === 'territory' ? sliceOrPanorama(token, focus.id, url) : Promise.resolve(null),
		readAtlasHome(token),
		tier === 2 && focus.kind === 'node'
			? readCogmapNeighborhood(token, cogmapId, {
					seeds: [focus.id],
					depth: NEIGHBORHOOD_DEPTH,
					edge_kinds: [] as EdgeKind[]
				})
			: Promise.resolve(null)
	]);
	const cogmapName = home.cogmaps.find((c) => c.id === cogmapId)?.name ?? 'Cognitive map';
	const crumbTerritory = territorySeg
		? await crumbTerritoryLabel(token, territorySeg.id, slice)
		: null;

	// TrailRail parity: R5 trail + resource row are profile-scoped (not team-scoped),
	// so the same selection block as the team branch works inside a cogmap door.
	const selection = selectedElement(focus, url);
	const trail =
		selection.kind === 'edge'
			? await readTrail(token, 'edge', selection.id)
			: selection.kind === 'node'
				? await readTrail(token, 'node', selection.id)
				: null;
	const resourceRow = selection.kind === 'node' ? await readResourceRow(token, selection.id) : null;
```

Then in that branch's returned object (105-124), replace `neighborhood: null`, `selection: { kind: 'none' as const }`, `trail: null`, `resourceRow: null` with `neighborhood`, `selection`, `trail`, `resourceRow`. (Ensure `selectedElement` and `readResourceRow` are imported — `selectedElement` is already in the nav import at 5-14; add `readResourceRow` to the graph-reads import if not present.)

- [ ] **Step 3: Typecheck**

Run: `cd packages/temper-ui && bun run check`
Expected: no errors.

- [ ] **Step 4: Harness verify** — cogmap node now opens a populated neighborhood + rail.

Because the harness fixtures drive from captured page-load output, capture a cogmap-node scenario if the committed bundle lacks one (see `dev/atlas/README.md`; add a `cogmapNodeNeighborhood` grab `cogmap=<COGMAP>&focus=node:<NODE>`), or verify live against prod post-deploy. With `bun run dev` + `http://localhost:5173/dev/atlas`, confirm a cogmap-scoped node renders neighbors + edges (not the "not available in cogmap view yet" text) and the TrailRail shows excerpt/history.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/server/graph-reads.ts \
        packages/temper-ui/src/routes/\(app\)/graph/\[owner\]/+page.server.ts
git commit -m "feat(atlas): cogmap nodes drillable — loader wiring + rail parity (A2 part 3/3)"
```

---

# Phase B — Legibility (frontend + one R2 tweak)

> Depends on Task 2 (A3) for the shrunk `TerritorySlice` shape. B1's R2 change is self-contained here.

## Task 6 (spec B1): Derive region labels in R2 + truncate in the mark

**Why:** region territories inherit `kb_cogmap_regions.label`, usually NULL, and `TerritoryCircle` has
no fallback → blank circles. Derive a representative label in R2 (preferring a stored/steward label),
and truncate long labels in the mark.

**Files:**
- Create: `migrations/20260706120300_region_territory_derived_label.sql`
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/marks/TerritoryCircle.svelte`
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/TierPanorama.svelte` (pass `memberCount`)
- Test: `tests/e2e/tests/graph_territory_overview_sql_test.rs` (extend)

**Interfaces:**
- Consumes: existing `graph_region_territories(profile,team,lens) RETURNS TABLE(region_id,cogmap_id,label,member_count,salience)` — body-only change, shape unchanged (`CREATE OR REPLACE`).
- Produces: `label` = `COALESCE(reg.label, <top visible member title>)`.

- [ ] **Step 1: Write the failing SQL test** — an unlabeled region with a visible member surfaces that member's title as its label; a private member's title never leaks.

Extend `tests/e2e/tests/graph_territory_overview_sql_test.rs` (mirror its region fixtures):

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unlabeled_region_derives_top_visible_member_title(pool: sqlx::PgPool) {
    // fixtures: profile+team, a readable cogmap with a region whose label IS NULL,
    // a visible member "Alpha topic" (affinity 0.9) and a NON-visible member "secret" (affinity 0.99).
    // ...build them, then:
    let rows: Vec<(uuid::Uuid, Option<String>)> = sqlx::query_as(
        "SELECT region_id, label FROM graph_region_territories($1,$2,$3)",
    )
    .bind(profile).bind(team).bind(lens)
    .fetch_all(&pool).await.unwrap();
    let (_, label) = rows.iter().find(|(rid, _)| *rid == region).unwrap();
    assert_eq!(label.as_deref(), Some("Alpha topic"),
        "derives the top VISIBLE member's title, never the higher-affinity private one");
}
```

Run: `cd tests/e2e && cargo test --test graph_territory_overview_sql_test unlabeled_region_derives` → Expected: FAIL (label is NULL).

- [ ] **Step 2: Write the migration**

Create `migrations/20260706120300_region_territory_derived_label.sql`:

```sql
-- B1: an unlabeled region falls back to its top VISIBLE member's title, so the
-- panorama is legible. Prefers a stored/steward label when present (so a future
-- steward-naming arc needs no read change). The representative respects
-- resources_visible_to — a private member's title is never surfaced as a label.
-- Body-only change; RETURNS TABLE shape unchanged → CREATE OR REPLACE.

CREATE OR REPLACE FUNCTION graph_region_territories(
    p_profile uuid, p_team uuid, p_lens uuid
) RETURNS TABLE(region_id uuid, cogmap_id uuid, label text, member_count int, salience double precision)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.cogmap_id,
           COALESCE(reg.label, rep.title) AS label,
           reg.member_count, reg.salience
    FROM kb_cogmap_regions reg
    JOIN kb_team_cogmaps tc ON tc.cogmap_id = reg.cogmap_id
    JOIN team_ancestors(p_team) a ON a.team_id = tc.team_id
    LEFT JOIN LATERAL (
        SELECT r.title
        FROM kb_cogmap_region_members m
        JOIN resources_visible_to(p_profile) v ON v.resource_id = m.member_id
        JOIN kb_resources r ON r.id = m.member_id AND r.is_active
        WHERE m.region_id = reg.id AND m.member_table = 'kb_resources'
        ORDER BY m.affinity DESC NULLS LAST
        LIMIT 1
    ) rep ON true
    WHERE NOT reg.is_folded
      AND reg.lens_id = p_lens
      AND cogmap_readable_by_profile(p_profile, reg.cogmap_id);
$$;
```

- [ ] **Step 3: Apply + verify the SQL test passes**

Run: `sqlx migrate run` then `cd tests/e2e && cargo test --test graph_territory_overview_sql_test unlabeled_region_derives`
Expected: PASS.

- [ ] **Step 4: Truncate + member-count fallback in the mark**

In `packages/temper-ui/src/lib/components/graph/atlas/marks/TerritoryCircle.svelte`: import the shared truncator and add a `memberCount` prop for the last-resort fallback (a region with no *visible* members yields a null derived label). Replace the `displayLabel` derivation (line 21) and wrap the label text in truncation:

```svelte
	import { truncateLabel } from '$lib/graph/atlas/labels';
	// ...add to Props: memberCount?: number;
	// ...destructure: let { x, y, r, kind, label, memberCount = 0, onEnter, ghost = false }: Props = $props();

	const baseLabel = $derived(label ?? (memberCount > 0 ? `Region · ${memberCount}` : null));
	const displayLabel = $derived(
		ghost && baseLabel ? `${baseLabel} · empty` : baseLabel
	);
	// radius-proportional char budget so a long derived title fits the circle
	const shownLabel = $derived(displayLabel ? truncateLabel(displayLabel, Math.max(6, Math.floor(r / 4))) : null);
```

Then in the `<text>` block (currently `{displayLabel}` at line 57) render `{shownLabel}` and gate `{#if shownLabel}`. Keep `aria-label={displayLabel ?? kind}` (full untruncated label for a11y).

- [ ] **Step 5: Pass `memberCount` from the panorama**

In `packages/temper-ui/src/lib/components/graph/atlas/TierPanorama.svelte`, where dense territories render `<TerritoryCircle label={t.label} ... />` (~line 75-85), add `memberCount={t.member_count}`.

- [ ] **Step 6: Typecheck + harness verify**

Run: `cd packages/temper-ui && bun run check` (expect no errors), then `bun run dev` and open `/dev/atlas`
scenario **teamPanorama** — the previously-blank region circles now carry (truncated) labels; the
`TEMPER` context territory still labels; no blank non-empty circles remain.

- [ ] **Step 7: Commit**

```bash
git add migrations/20260706120300_region_territory_derived_label.sql \
        packages/temper-ui/src/lib/components/graph/atlas/marks/TerritoryCircle.svelte \
        packages/temper-ui/src/lib/components/graph/atlas/TierPanorama.svelte
git commit -m "feat(atlas): derive + truncate region territory labels (B1/#1)"
```

---

## Task 7 (spec B2/B3/B4/B5): Legibility polish + verification pass

**Why:** close the remaining legibility siblings — the payload column collision (concrete fix), and
verify the already-partly-built pieces (pack sizing, tier-2 label gating, empty-territory ghost).

**Files:**
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/TrailRail.svelte` (B5, payload CSS)
- Verify-only: `packTerritories.ts` (B2), `marks/NodeChip.svelte` (B3), `marks/TerritoryCircle.svelte` (B4)

- [ ] **Step 1 (B5): Fix the payload key/value collision** — stack key above value so long keys never overrun.

In `packages/temper-ui/src/lib/components/graph/atlas/TrailRail.svelte`, the `.ev-payload > div` rule
(~line 294) uses `grid-template-columns: 90px 1fr`; a key like `originator_profile_id` overruns the
90px column into the value. Change the payload rows to a stacked layout (key line, value line):

```css
	.ev-payload > div {
		display: block;            /* was: grid; grid-template-columns: 90px 1fr; */
		margin-bottom: 6px;
	}
	.ev-payload dt {
		/* key on its own line — no fixed column to overrun */
		font-size: 10px;
		letter-spacing: 0.08em;
		opacity: 0.6;
		margin-bottom: 1px;
	}
	.ev-payload dd {
		margin: 0;
		word-break: break-word;
	}
```

- [ ] **Step 2 (B5): Harness verify the payload**

Run: `cd packages/temper-ui && bun run dev`, open `/dev/atlas` scenario **nodeSelected**, expand the
"Resource created" history row → keys (`originator_profile_id`, `owner_profile_id`) sit on their own
line above their values, no overlap.

- [ ] **Step 3 (B2): Verify pack sizing** — confirm regions size by `salience`, contexts/cogmaps by
`member_count`, and that Task 6's labels make the packed panorama legible. Inspect
`packages/temper-ui/src/lib/graph/atlas/layout/packTerritories.ts` (the `.sum(...)` at ~36-42 already
does this) and eyeball **teamPanorama** + **cogmapPanorama** on the harness. No code change expected;
if a size branch reads wrong, note it — do not add force-weighting (Rejected in the spec).

- [ ] **Step 4 (B3): Verify tier-2 label legibility** — open **nodeNeighborhood** on the harness (a
dense-ish neighborhood; capture one via `dev/atlas/README.md` if the committed fixture is sparse).
`NodeChip.svelte:69` already truncates (`truncateLabel(title, 22)`), offsets below (`y + r + 13`), and
gates to the top-5 anchors + hides on hover. **If** labels still overlap, apply the minimal tune:
reduce the anchor count in `TierNeighborhood.svelte:23` (`labelAnchors(graph.nodes, seedId, 5)` → `3`)
and/or shrink the char budget to `18`. Only change if overlap is observed.

- [ ] **Step 5 (B4): Verify empty-territory ghost** — confirm an empty (zero-member) context/region
renders via the existing `ghost` path (`TerritoryCircle.svelte:12-21`, de-emphasized + "· empty"),
not a bare circle. On **teamPanorama**, an empty `TEMPER`-style context should read as a ghost. No code
change expected (ghost already exists); if a zero-member territory renders solid, wire `ghost` at its
call site in `TierPanorama.svelte`.

- [ ] **Step 6: Full check + commit**

Run: `cd packages/temper-ui && bun run check` and `bun run test` (expect green). Then:
```bash
git add packages/temper-ui/src/lib/components/graph/atlas/TrailRail.svelte
# plus any tune files touched in Steps 4-5
git commit -m "polish(atlas): payload column + tier-2 legibility verification (B2-B5)"
```

---

## Final: consolidated review

After all tasks land (per subagent-review-cadence — defer review to the end, not per-task):

1. **Consolidated code + spec review** (opus) — verify each spec item (#1-#4, G1/G2/L3, payload) is
   met; **critically re-check the SQL visibility predicates conjunct-for-conjunct** against the shipped
   originals (the new cogmap walk + the widened team walk must not add any visibility surface — diff
   each new/changed function against the SECURITY INVARIANT), and confirm the access-tier deny-direction
   tests actually exercise the outsider path.
2. **Gates:** `cargo make check` green; `cargo make test-e2e` green (the access tier — test-db alone is
   a false signal for A1/A2). If any fixture touches the embed path, `cargo make test-e2e-embed`.
3. **Prod browser-verify post-merge** (Auth0 previews can't authenticate): cogmap node opens a
   neighborhood; team node with only incoming edges renders neighbors + honest count; region
   territories are labelled; region header shows no phantom sub-cluster count.
4. **PR sequencing** (spec): PR-A = Tasks 1-2; PR-B = Tasks 3-5 (cogmap stack, own access review);
   PR-C = Tasks 6-7 (legibility).
