# Atlas Beat D — Region → resources drill (composition) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Drill a cogmap region (or a shift-selected union of regions) into a two-axis force-graph — the region's facets (ideas) plus the context-homed resources they were `derived_from` (the work), each visibility-gated — rendering ideas as circles clustered at the center and work as document-squares on an outer ring.

**Architecture:** A new cross-home, region-seeded, visibility-gated backend read (the crux — it does NOT fence to cogmap scope, so it reaches context-homed resources) surfaced through a service-direct endpoint. The frontend re-points territory-focus from the retired R3 members-hull to a composition force-graph reusing the shipped `forceNeighborhood` + `NodeChip`, with shape-encodes-axis marks and a radial layout that reinforces the two axes spatially. Union is expressed in the URL as a `+`-joined region-id list.

**Tech Stack:** Rust (sqlx macros, Axum, temper-services), PostgreSQL, ts-rs wire types, SvelteKit 5 runes, d3-force, vitest (node/pure-fn), temper-e2e (`test-db`).

## Global Constraints

- Spec: `docs/superpowers/specs/2026-07-08-atlas-beat-d-region-resources-drill-spec.md`.
- **Branch `jct/atlas-reshape` is HELD** — commit per task locally, **do NOT push or PR**.
- **Visibility predicate reproduced conjunct-for-conjunct** — both edge endpoints ⊆ `resources_visible_to`, `NOT is_folded`, home anchor `anchor_readable_by_profile`. Every read path MUST have a **deny-direction** e2e test. (`feedback_read_gate_must_match_full_canonical_visibility`, `reference_array_agg_scope_null_fall_open_leak`.)
- **No silent truncation** — when a bound clamps the node/region set, surface it (log + return a flag), never drop silently.
- New SQL → regenerate per-crate sqlx caches: `cargo make prepare-services` + `cargo make prepare-e2e`. `#[sqlx::test]` migrations are compile-time embedded → `touch crates/temper-api/src/lib.rs` after adding a migration.
- Migrations are **immutable once shipped**; these are new, unshipped files — editing them in-branch is fine, but a dev-DB reset (`sqlx migrate run` from clean `public`) is required after edits.
- Wire type is the existing `AtlasSubgraph` (`graph_atlas.rs`) — **no new wire type**.
- Reuse existing marks/layout; do not restyle site chrome. Dark-only. Palette from `palette.ts`.
- Controller runs all cargo/DB/vitest gates and commits; implementer subagents write code+tests only (`feedback_sdd_subagents_stall_on_backgrounded_cargo`). Run `cargo fmt` before every commit (`feedback_implementer_subagents_must_run_fmt`).

---

## File Structure

**Backend (Rust / SQL)**
- Create: `migrations/20260708000001_graph_region_composition.sql` — `graph_region_composition_edges` + `graph_atlas_nodes_visible`.
- Modify: `crates/temper-services/src/services/graph_service.rs` — add `region_composition_slice`; (Task 12) delete `territory_slice`.
- Modify: `crates/temper-api/src/handlers/graph.rs` — add `region_composition` handler; (Task 12) delete `territory_slice` handler.
- Modify: `crates/temper-api/src/routes.rs` — add composition route; (Task 12) delete regions/{id}/slice route.
- Create: `tests/e2e/tests/atlas_region_composition_test.rs` — read + deny + union + bound + gate.

**Frontend (TS / Svelte)**
- Modify: `src/lib/components/graph/atlas/marks/NodeChip.svelte` — context → rounded square (already prototyped; add test).
- Modify: `src/lib/graph/atlas/layout/forceNeighborhood.ts` — radial-by-home + cross-home link loosening (already prototyped; add test).
- Create: `src/lib/graph/atlas/layout/forceNeighborhood.radial.test.ts`.
- Modify: `src/lib/graph/atlas/nav.ts` — `territoryIds()` helper (+-joined union), `buildDrillTerritoryUrl` append-on-shift, tier for territory focus.
- Modify: `src/lib/graph/atlas/nav.test.ts` — union parsing/build tests.
- Modify: `src/lib/server/graph-reads.ts` — `regionCompositionPath` + `readRegionComposition`.
- Modify: `src/routes/(app)/graph/[owner]/+page.server.ts` — territory focus → composition read (was R3 slice).
- Modify: `src/lib/components/graph/atlas/AtlasCanvas.svelte` (+ `TierNeighborhood.svelte`) — render territory focus as the composition force-graph.
- Modify: `src/lib/components/graph/atlas/HomeA11yList.svelte` or a new `CompositionA11yList.svelte` — list grouped by axis (Ideas / Sources).
- Delete (Task 12): `TierTerritory.svelte` + R3 wiring + `TerritorySlice` type usage.
- Modify: `static/dev/atlas-fixtures.json` — synthetic `regionDrill` + `regionDrillUnion` scenarios (Task 13).

---

## Phase 1 — Backend: the cross-home composition read

### Task 1: SQL — `graph_region_composition_edges` + `graph_atlas_nodes_visible`

**Files:**
- Create: `migrations/20260708000001_graph_region_composition.sql`

**Interfaces:**
- Produces:
  - `graph_region_composition_edges(p_profile uuid, p_region_ids uuid[], p_depth int) RETURNS TABLE(id uuid, source_id uuid, target_id uuid, edge_kind edge_kind, polarity edge_polarity, label text, weight double precision)`
  - `graph_atlas_nodes_visible(p_profile uuid, p_ids uuid[]) RETURNS TABLE(id uuid, title text, doc_type text, home text, degree int, first_chunk text)`

- [ ] **Step 1: Write the migration.** Model on `migrations/20260706120200_atlas_cogmap_neighborhood.sql`, but seed from region members and DROP the cogmap-scope fence — gate each endpoint through `resources_visible_to` directly.

```sql
-- Beat D: region → resources COMPOSITION read. Unlike graph_traverse_cogmap_scoped,
-- the walk is NOT fenced to a cogmap's homed resources — it is seeded by region
-- members (facets) and follows visible edges out to context-homed resources (the
-- builder axis). Full edge-visibility predicate reproduced conjunct-for-conjunct:
-- both endpoints in resources_visible_to, NOT is_folded, home anchor readable.

CREATE FUNCTION graph_region_composition_edges(
    p_profile    uuid,
    p_region_ids uuid[],
    p_depth      int
) RETURNS TABLE(
    id uuid, source_id uuid, target_id uuid, edge_kind edge_kind,
    polarity edge_polarity, label text, weight double precision
) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE
    vis AS (SELECT resource_id AS id FROM resources_visible_to(p_profile)),
    seeds AS (  -- region members that are visible to the caller
        SELECT DISTINCT m.member_id AS id
        FROM kb_cogmap_region_members m
        JOIN vis v ON v.id = m.member_id
        WHERE m.region_id = ANY(p_region_ids)
    ),
    reached AS (
        SELECT id AS node_id, 0 AS depth FROM seeds
        UNION
        SELECT CASE WHEN e.source_id = r.node_id THEN e.target_id ELSE e.source_id END, r.depth + 1
        FROM reached r
        JOIN kb_edges e ON (e.source_id = r.node_id OR e.target_id = r.node_id)
        JOIN vis vs ON vs.id = e.source_id
        JOIN vis vt ON vt.id = e.target_id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND NOT e.is_folded
          AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
          AND r.depth < LEAST(p_depth, 3)
    )
    SELECT DISTINCT e.id, e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, e.weight
    FROM kb_edges e
    JOIN reached rs ON rs.node_id = e.source_id
    JOIN reached rt ON rt.node_id = e.target_id
    JOIN vis vs ON vs.id = e.source_id
    JOIN vis vt ON vt.id = e.target_id
    WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
      AND NOT e.is_folded
      AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id);
$$;

CREATE FUNCTION graph_atlas_nodes_visible(p_profile uuid, p_ids uuid[])
RETURNS TABLE(id uuid, title text, doc_type text, home text, degree int, first_chunk text)
LANGUAGE sql STABLE AS $$
    WITH vis AS (SELECT resource_id AS id FROM resources_visible_to(p_profile)),
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
    JOIN vis v ON v.id = ids.id           -- deny-as-absence: unseen ids drop out
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
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true;
$$;
```

- [ ] **Step 2: Apply to dev DB.** `export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development` then `sqlx migrate run --source migrations`. Expected: `Applied 20260708000001/migrate graph region composition`.
- [ ] **Step 3: Smoke-check in psql** against the seeded/synthetic DB (or the real region on a dev copy): `SELECT count(*) FROM graph_region_composition_edges('<profile>'::uuid, ARRAY['<region>']::uuid[], 1);` returns > 0.
- [ ] **Step 4: Commit.** `git add migrations/20260708000001_graph_region_composition.sql && cargo fmt && git commit -m "feat(atlas): Beat D — graph_region_composition SQL (cross-home region read)"`.

### Task 2: Service — `region_composition_slice`

**Files:**
- Modify: `crates/temper-services/src/services/graph_service.rs` (model on `cogmap_neighborhood_slice` L261 and `territory_slice` L445 entry gate)

**Interfaces:**
- Consumes: `graph_region_composition_edges`, `graph_atlas_nodes_visible` (Task 1); `AtlasSubgraph`/`AtlasNode`/`AtlasEdge`/`NodeHome` (graph_atlas).
- Produces: `pub async fn region_composition_slice(pool: &PgPool, profile_id: ProfileId, region_ids: &[Uuid], depth: i32) -> ApiResult<AtlasSubgraph>`

- [ ] **Step 1: Write the service fn.**
  1. Reject empty `region_ids` → `ApiError::BadRequest`.
  2. **Entry gate (deny-as-absence), one query:** every region must exist, `NOT is_folded`, `cogmap_readable_by_profile(profile, reg.cogmap_id)`. Count matches; if `count < region_ids.len()` → `ApiError::NotFound`. (Reproduce `territory_slice`'s gate over the id array via `= ANY($1)`.)
  3. Clamp `region_ids` to the regions-per-union cap (`const MAX_UNION_REGIONS: usize = 6`); `tracing::warn!` if truncated.
  4. edges = `query_as::<_,(Uuid,Uuid,Uuid,EdgeKind,Polarity,Option<String>,f64)>` over `graph_region_composition_edges($1,$2,$3)` binding `profile, &region_ids, depth.min(3)`; map to `AtlasEdge` (same mapping as `cogmap_neighborhood_slice`).
  5. Also fetch seed member ids (so an isolated facet with no edges still renders): `graph_region_members` per region OR add seeds to the node-id set from a `kb_cogmap_region_members` query bound by `= ANY`. node_ids = seeds ∪ edge endpoints; apply `const NODE_CAP: usize = 120` (warn if truncated).
  6. nodes = `query_as` over `graph_atlas_nodes_visible($1,$2)` binding `profile, &node_ids`; map to `AtlasNode` (home `"cogmap"`→`NodeHome::Cogmap` else `Context`; `salience: None`; `excerpt` via `compute_excerpt`).
  7. `Ok(AtlasSubgraph { nodes, edges })`.
- [ ] **Step 2: Compile.** Controller: `cargo make prepare-services` then `env DATABASE_URL=... cargo nextest run -p temper-services --features test-db region_composition 2>/dev/null || true` (unit compile check; behavior covered by e2e in Task 4). Expected: compiles; sqlx cache regenerated.
- [ ] **Step 3: Commit.** `cargo fmt && git add -A crates/temper-services && git commit -m "feat(atlas): Beat D — region_composition_slice service (entry gate + bounds)"`.

### Task 3: Endpoint + handler + route

**Files:**
- Modify: `crates/temper-api/src/handlers/graph.rs` (model on `territory_slice` handler L147 + `cogmap_neighborhood_slice` L84)
- Modify: `crates/temper-api/src/routes.rs` (near L97 regions/slice route)

**Interfaces:**
- Produces: `GET /api/graph/regions/composition?ids=<csv>&depth=<n>` → `Json<AtlasSubgraph>`.

- [ ] **Step 1: Handler.** Query params struct `{ ids: String, depth: Option<i32> }` (typed extractor, not inline). Split `ids` on `,` → `Vec<Uuid>` (parse; malformed → `BadRequest`). Resolve `profile_id` from auth extension (same as `territory_slice`). Call `graph_service::region_composition_slice(&state.pool, profile_id, &ids, depth.unwrap_or(1))`. Return `Json`.
- [ ] **Step 2: Route.** Add `.route("/api/graph/regions/composition", get(handlers::graph::region_composition))` **before** the `/api/graph/regions/{region_id}/slice` param route (static beats param). 
- [ ] **Step 3: Compile.** Controller: `cargo make check` (offline; validates sqlx caches). Expected: green.
- [ ] **Step 4: Commit.** `cargo fmt && git add -A crates/temper-api && git commit -m "feat(atlas): Beat D — GET /api/graph/regions/composition"`.

### Task 4: e2e — read, deny-direction, union, bound, gate

**Files:**
- Create: `tests/e2e/tests/atlas_region_composition_test.rs` (model on the retired-team-scope tests + `bind_cogmap_e2e.rs` fixtures; use the JWKS harness in `tests/e2e/tests/common/`)

- [ ] **Step 1: Write failing tests** (`#[cfg(all(test, feature = "test-db"))]`, `#[sqlx::test]` or the spawn-server harness — match sibling atlas e2e). Cases:
  1. **read:** seed a cogmap with a region whose facet `derived_from` a context-homed resource → composition returns the facet (home cogmap) + the context resource (home context) + the edge.
  2. **deny-direction:** a second profile who can see the facet but NOT the context-homed neighbor (homed in a context they lack) → gets the facet, NOT the neighbor, and no edge to it.
  3. **union:** two regions → seeds unioned, both cores present.
  4. **entry-gate deny:** region in an unreadable cogmap → `404`.
  5. **bound:** > `MAX_UNION_REGIONS` region ids → clamped (assert node set matches first 6; the warn is not asserted).
- [ ] **Step 2: Run, verify fail.** `cargo make prepare-e2e` then `cargo make test-e2e -- atlas_region_composition`. Expected: FAIL (fns/endpoint under test, but assertions on data). (Controller runs; `feedback_nextest_does_not_rebuild_spawned_temper_bin` — `cargo build -p temper-cli --bin temper` first if the harness spawns the CLI.)
- [ ] **Step 3: Make green** (fixes land in Tasks 1–3; iterate here). Expected: PASS.
- [ ] **Step 4: Commit.** `cargo fmt && git add -A tests/e2e && git commit -m "test(atlas): Beat D — region composition e2e (read + deny + union + gate + bound)"`.

---

## Phase 2 — Frontend rendering (already prototyped; formalize + test)

### Task 5: `NodeChip` — context-resource as rounded square

**Files:**
- Modify: `src/lib/components/graph/atlas/marks/NodeChip.svelte` (the `{:else}` (context) branch → `<rect rx={Math.max(2, r*0.32)}>` filled `color`, stroke `CANVAS_BG` 1.5 — **already applied in the working tree; keep, drop the "prototype" comment**).
- Create/Modify test: `src/lib/graph/atlas/marks/nodeChip.shape.test.ts` (pure-fn — extract the shape decision if needed, or assert via a tiny render helper; match existing vitest node-env pure-fn style — no jsdom).

- [ ] **Step 1: Extract testable helper.** If not trivially testable, add `export function nodeMarkKind(home): 'circle' | 'square'` to `nav.ts` or a `marks` util and use it in `NodeChip`. Test: `home==='cogmap' → 'circle'`, `home==='context' → 'square'`.
- [ ] **Step 2: Run/fail → implement → pass.** `bun run test -- nodeChip`. 
- [ ] **Step 3: Commit.** `git add -A src/lib && git commit -m "feat(atlas): Beat D — context-resource marks render as document-squares"`.

### Task 6: `forceNeighborhood` — radial-by-home spatial reinforcement

**Files:**
- Modify: `src/lib/graph/atlas/layout/forceNeighborhood.ts` (radial + cross-home link loosening — **already applied; keep, drop "prototype" comment**).
- Create: `src/lib/graph/atlas/layout/forceNeighborhood.radial.test.ts`.

- [ ] **Step 1: Failing test.** Build a subgraph with 2 cogmap facets (linked) + 2 context nodes (each linked to a facet), run `forceNeighborhood`, assert deterministically: mean radius of `home==='context'` nodes from center > mean radius of `home==='cogmap'` nodes. (Deterministic — ring-init, no random.)
- [ ] **Step 2: Run/fail → keep impl → pass.** `bun run test -- forceNeighborhood`.
- [ ] **Step 3: Commit.** `git add -A src/lib && git commit -m "feat(atlas): Beat D — radial layout separates idea-core from source-ring"`.

---

## Phase 3 — Nav, entry, union, a11y

### Task 7: Nav — union `+`-joined territory ids

**Files:**
- Modify: `src/lib/graph/atlas/nav.ts`, `src/lib/graph/atlas/nav.test.ts`

**Interfaces:**
- Produces: `territoryIds(focus: Focus): string[]` (splits a territory focus id on `+`; `[]` for non-territory); `buildDrillTerritoryUrl(base, regionId, { add?: boolean })` (add=true appends `+regionId` to the current leaf territory token; else replaces focus with `territory:regionId`).

- [ ] **Step 1: Failing tests.** `territoryIds({kind:'territory', id:'A+B'}) === ['A','B']`; `parseFocus` of `?focus=territory:A+B` → `{kind:'territory', id:'A+B'}` (token parser must allow `+` in id — it already keeps the whole post-`:` string, verify); `buildDrillTerritoryUrl(url, 'B', {add:true})` on `?focus=territory:A` → `?focus=territory:A+B`; without add → `?focus=territory:B`. `deriveTier({kind:'territory'})` stays `1`.
- [ ] **Step 2: Run/fail → implement → pass.** `bun run test -- nav`.
- [ ] **Step 3: Commit.** `git commit -m "feat(atlas): Beat D — union territory ids in the URI frame (+-joined)"`.

### Task 8: Read wrapper — `readRegionComposition`

**Files:**
- Modify: `src/lib/server/graph-reads.ts`

**Interfaces:**
- Produces: `regionCompositionPath(ids: string[], depth?: number): string` (`/api/graph/regions/composition?ids=<csv>&depth=<n>`); `readRegionComposition(token, ids, depth?): Promise<AtlasSubgraph>` (via `apiGet`).

- [ ] **Step 1: Add path builder + reader** (mirror `readRegionSlice` L42). Add a pure-fn test for `regionCompositionPath(['A','B'],1)` in `graph-reads.paths.test.ts`.
- [ ] **Step 2: Run/pass.** `bun run test -- graph-reads`.
- [ ] **Step 3: Commit.** `git commit -m "feat(atlas): Beat D — region composition read wrapper"`.

### Task 9: Page load — territory focus → composition

**Files:**
- Modify: `src/routes/(app)/graph/[owner]/+page.server.ts`

- [ ] **Step 1:** Where the load currently branches on `focus.kind === 'territory'` and calls `readRegionSlice` (R3), replace with `readRegionComposition(token, territoryIds(focus), 1)` and populate `neighborhood` (the `AtlasSubgraph`) instead of `slice`. Set `crumbTerritory` label to the region (or "N regions" when union). Keep `selection`/`trail` wiring as for node focus.
- [ ] **Step 2:** Controller: `bun run check` (svelte-check) — expect green; then browser-verify in `/dev/atlas` is covered by Task 13 fixtures.
- [ ] **Step 3: Commit.** `git commit -m "feat(atlas): Beat D — territory focus loads the composition graph"`.

### Task 10: Render — territory focus as force-graph

**Files:**
- Modify: `src/lib/components/graph/atlas/AtlasCanvas.svelte` (+ `TierNeighborhood.svelte` if the tier dispatch keys on `tier`/focus kind)

- [ ] **Step 1:** Make the canvas render the `neighborhood` force-graph (via `forceNeighborhood` + `TierNeighborhood`/marks) for `focus.kind === 'territory'` as well as `'node'`. Seeds passed to `forceNeighborhood` = `territoryIds(focus)`'s members are unknown client-side, so seed-ring on the seeds is optional; pass `[]` or the focused ids as available. Ensure the old `TierTerritory` (R3 hull) is no longer rendered for territory focus.
- [ ] **Step 2:** `bun run check` + `bun run test`. Green.
- [ ] **Step 3: Commit.** `git commit -m "feat(atlas): Beat D — territory drill renders the two-axis force-graph"`.

### Task 11: a11y — list grouped by axis

**Files:**
- Create: `src/lib/components/graph/atlas/CompositionA11yList.svelte` (model on `HomeA11yList.svelte`)

- [ ] **Step 1:** Render (for the `svg role=img` field) a visually-hidden list with two groups — **Ideas** (home cogmap) and **Sources** (home context) — each item a link + doc-type + degree. Add a pure-fn `groupByAxis(nodes)` with a test.
- [ ] **Step 2:** `bun run test -- a11y` + `bun run check`. Green.
- [ ] **Step 3: Commit.** `git commit -m "feat(atlas): Beat D — a11y list groups the drill by Ideas / Sources"`.

---

## Phase 4 — Teardown & harness

### Task 12: Retire R3 `TierTerritory` / `territory_slice`

**Files:** delete `TierTerritory.svelte`; remove `territory_slice` (service + handler + route + `regionSlicePath`/`readRegionSlice`); drop `TerritorySlice` usage from `AtlasViewData` if now unused.

- [ ] **Step 1: Audit both ends** (`feedback_plan_gate_audit_both_ends`): grep `territory_slice`, `TerritorySlice`, `regionSlicePath`, `readRegionSlice`, `TierTerritory`, `/regions/{region_id}/slice` — confirm the only consumers are the ones being repointed by Tasks 9–10.
- [ ] **Step 2:** Delete them. Keep `graph_region_members` (still used by the composition seeds). Do NOT drop the SQL fn in this migration (shipped-object DROP is a separate additive migration later — `feedback_drop_function_non_additive_breaks_deploy_skew`); just stop calling `territory_slice`'s SQL.
- [ ] **Step 3:** `cargo make check` + `bun run check` + full home/graph e2e. Green.
- [ ] **Step 4: Commit.** `git commit -m "refactor(atlas): Beat D — retire R3 TierTerritory hull (superseded by composition drill)"`.

### Task 13: Synthetic harness scenarios

**Files:** Modify `static/dev/atlas-fixtures.json` — add personal-data-free `regionDrill` (facets + context squares) + `regionDrillUnion` scenarios (synthetic ids/titles, matching the `AtlasViewData` shape with `tier:2 / focus:territory`). Update `dev/atlas/README.md` (drop the retired `teamPanorama` name-check while here).

- [ ] **Step 1:** Add scenarios; `bun run test -- fixtures` (the fixtures schema test) green.
- [ ] **Step 2: Commit.** `git commit -m "test(atlas): Beat D — synthetic region-drill harness scenarios"`.

---

## Self-review notes

- **Spec coverage:** §3 rendering → T5/T6; §3.3 union → T7; §4 read → T1/T2; §4.4 visibility deny → T4; §5 nav/entry → T7–T10; §6 bounds → T2 (NODE_CAP/MAX_UNION_REGIONS) + T4; §7 teardown → T12; §9 a11y → T11, harness → T13. Covered.
- **Type consistency:** `AtlasSubgraph`/`AtlasNode`/`AtlasEdge`/`NodeHome` reused throughout; `region_composition_slice` signature is the single source; `territoryIds`/`regionCompositionPath` names used consistently in T7/T8/T9.
- **Verified symbols:** all SQL fns (`resources_visible_to`, `anchor_readable_by_profile`, `edges_visible_to`, `cogmap_readable_by_profile`, `kb_cogmap_region_members`, `graph_region_members`) exist in migrations; `parseFocusToken` already preserves the full post-`:` id (so `+` survives); `NodeChip` `home` prop + `filled` derive exist; `forceNeighborhood` signature confirmed.
