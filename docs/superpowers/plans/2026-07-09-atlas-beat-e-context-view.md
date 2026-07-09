# Atlas Beat E — Context View Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the legacy Cytoscape context graph with an Atlas-native, tiered context view — goal containers at Tier 0 with a residual tray, the Beat-D force drill at Tier 1 — reachable from one URL by both the left-nav "Graph" link and the Atlas build circle.

**Architecture:** A new `?context=<slug>` door on `/graph/[owner]`, mirroring the existing `?cogmap=<id>` door. Tier 0 renders container territories (goal-rooted, edge-derived) plus residual buckets derived from a group-by key. Tier 1 reuses `TierNeighborhood`/`forceNeighborhood` with the radial inverted via a `coreHome` parameter — the mark shapes never change. Backend adds four SQL functions and one service; the node half reuses the already-generic `graph_atlas_nodes_visible`.

**Tech Stack:** Rust (Axum, sqlx, utoipa), PostgreSQL 17/18, SvelteKit 5 (runes), d3-force, TypeScript, vitest, cargo-nextest, ts-rs.

**Spec:** [`docs/superpowers/specs/2026-07-09-atlas-beat-e-context-view-design.md`](../specs/2026-07-09-atlas-beat-e-context-view-design.md) — read §3 (D1–D10) before starting.

## Global Constraints

- **Branch:** `jct/atlas-beat-e`. Already carries `ad324b09` (kind-agnostic `TierPanorama` + `log1p` ramp), `4e0e83d6` (context tint `#7dbae8` + temperature-as-axis), `eb66e34e` (spec), and a merge of `origin/main`. Do not redo these.
- **Migrations must number above `20260709000005`.** A sibling session landed that file today. This plan uses `20260709000010` and `20260709000011`.
- **Never edit an applied migration** — not even a comment. It breaks the prod sqlx checksum.
- **No `sqlx::query!()` inline in any surface** (handler, MCP tool, CLI action). SQL lives in the persistence layer; services compose it.
- **Typed structs, never `serde_json::json!()`** for data with a known shape.
- **Auth before writes/reads.** Every query scopes through `resources_visible_to` / `anchor_readable_by_profile`.
- **Shared wire types live in `temper-core`** with `ts-rs` derives. Never hand-write a TS mirror.
- **`--all-features`** for builds and clippy. Use `#[expect(lint, reason = "...")]`, never `#[allow]`.
- **All public types implement `Debug`.**
- **After changing SQL:** `cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-services`, then `cargo make prepare-api`. Per-crate last. Prune orphaned `.sqlx` files.
- **`cargo fmt` before every commit** — `cargo make check` gates on `fmt --check` (exit 105).
- **The visual language is invariant:** `nodeMarkShape(home)` — circles are cogmap nodes, rounded-squares are context resources. In every view. Never flip it.
- **Container walks filter on no edge label and no direction.** See spec §2, finding 1: the `parent_of` → `advances` backfill inverts direction and changes `edge_kind`. A label-filtered walk breaks the day it lands.
- **`temper` is on PATH.** Never `cargo run` it. Rebuild before e2e: `cargo build -p temper-cli --bin temper`.
- Run the dev harness with `cd packages/temper-ui && bun run dev` → `localhost:5173/dev/atlas`.

---

## File Structure

**Create:**

| path | responsibility |
|---|---|
| `migrations/20260709000010_graph_context_reads.sql` | The four context-graph read functions |
| `migrations/20260709000011_atlas_nodes_visible_stage.sql` | `DROP`+`CREATE` `graph_atlas_nodes_visible` widened with `stage` |
| `crates/temper-core/src/types/graph_context.rs` | `ContextPanorama`, `ResidualGroups`, `ResidualBucket`, `GroupKeyMeta` |
| `crates/temper-services/src/services/context_graph_service.rs` | Composes the SQL into panorama + composition reads |
| `crates/temper-api/tests/context_graph_test.rs` | `test-db` integration, incl. the deny direction |
| `packages/temper-ui/src/lib/graph/atlas/residualTray.ts` | Pure tray layout/model |
| `packages/temper-ui/src/lib/graph/atlas/residualTray.test.ts` | Tray model tests |
| `packages/temper-ui/src/lib/components/graph/atlas/ResidualTray.svelte` | Tray mark |
| `tests/e2e/tests/context_view_door_test.rs` | Door round-trip |

**Modify:**

| path | change |
|---|---|
| `crates/temper-core/src/types/graph_atlas.rs:30` | `AtlasNode.stage: Option<String>` |
| `crates/temper-core/src/types/mod.rs` | `pub mod graph_context;` |
| `crates/temper-services/src/services/graph_service.rs:539` | node query gains `stage` |
| `crates/temper-services/src/services/mod.rs` | `pub mod context_graph_service;` |
| `crates/temper-api/src/handlers/graph.rs` | two handlers; later, delete `get_subgraph` |
| `crates/temper-api/src/routes.rs:95-107` | two routes; later, delete the subgraph route |
| `crates/temper-api/src/openapi.rs` | paths + schemas |
| `packages/temper-ui/src/lib/graph/atlas/nav.ts` | `?context`, `Focus` container/bucket, builders |
| `packages/temper-ui/src/lib/graph/atlas/layout/forceNeighborhood.ts:76-95` | `coreHome` param |
| `packages/temper-ui/src/lib/components/graph/atlas/AtlasCanvas.svelte:65` | dispatch |
| `packages/temper-ui/src/lib/components/graph/atlas/TierPanorama.svelte` | render tray |
| `packages/temper-ui/src/lib/components/graph/atlas/TierNeighborhood.svelte:22` | pass `coreHome` |
| `packages/temper-ui/src/lib/components/graph/atlas/TierHome.svelte:149` | route via `vault-url` |
| `packages/temper-ui/src/lib/components/graph/atlas/HomeA11yList.svelte:92` | **bug:** drops the context slug |
| `packages/temper-ui/src/lib/vault-url.ts:17` | `contextGraphHref` → the door |
| `packages/temper-ui/src/lib/server/graph-reads.ts` | two wrappers + path builders |
| `packages/temper-ui/src/routes/(app)/graph/[owner]/+page.server.ts` | `?context` branch |
| `packages/temper-ui/src/lib/graph/atlas/crumbModel.ts` | context/container/bucket crumbs |
| `packages/temper-ui/src/routes/dev/atlas/README.md` | new scenarios |
| `packages/temper-ui/static/dev/atlas-fixtures.json` | sanitized `contextPanorama`, `contextDrill` |
| `packages/temper-ui/src/lib/graph/atlas/fixtures.test.ts` | pin new scenarios |

**Delete (Task 12 only, after the new door is live):** the legacy route, `KnowledgeGraph.svelte`, `ResourcePeek.svelte`, `GraphLegend.svelte`, `ModeToggle.svelte`, `ContextWatermark.svelte`, `src/lib/graph/{elements,derive,styling,layout,tiers,adjacency,peek,trail,navigation}.ts` + tests, `get_subgraph`, `aggregator_subgraph` & friends, `crates/temper-api/tests/graph_subgraph_test.rs`, and the three cytoscape deps.

---

## Task 1: Wire types — `ContextPanorama` + `AtlasNode.stage`

**Files:**
- Create: `crates/temper-core/src/types/graph_context.rs`
- Modify: `crates/temper-core/src/types/mod.rs`, `crates/temper-core/src/types/graph_atlas.rs`
- Test: `crates/temper-core/src/types/graph_context.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces: `ContextPanorama { containers: Vec<Territory>, residual: ResidualGroups, group_keys: Vec<GroupKeyMeta> }`, `ResidualGroups { group_key: String, buckets: Vec<ResidualBucket> }`, `ResidualBucket { value: String, count: i32 }`, `GroupKeyMeta { key: String, distinct_values: i32, coverage: i32 }`, and `AtlasNode.stage: Option<String>`.

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-core/src/types/graph_context.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn residual_bucket_round_trips() {
        let g = ResidualGroups {
            group_key: "doc_type".into(),
            buckets: vec![ResidualBucket { value: "session".into(), count: 395 }],
        };
        let json = serde_json::to_string(&g).expect("serialize");
        assert_eq!(json, r#"{"group_key":"doc_type","buckets":[{"value":"session","count":395}]}"#);
        let back: ResidualGroups = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, g);
    }

    #[test]
    fn empty_residual_serializes_as_empty_array_not_null() {
        // A well-edged context has NO residuals; the tray must render "nothing",
        // never crash on a null (spec D2: the tray shrinks to nothing).
        let g = ResidualGroups { group_key: "doc_type".into(), buckets: vec![] };
        assert_eq!(serde_json::to_string(&g).unwrap(), r#"{"group_key":"doc_type","buckets":[]}"#);
    }
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p temper-core --all-features graph_context`
Expected: FAIL — `file not found for module` / `cannot find type ResidualGroups`.

- [ ] **Step 3: Write the types**

Create `crates/temper-core/src/types/graph_context.rs` (above the test module):

```rust
//! Beat E — the context panorama wire types.
//!
//! A context panorama is the builder-axis sibling of the cogmap panorama: container
//! territories (goal-rooted, edge-derived) plus the residue that reaches no container.
//!
//! Residual buckets are DERIVED from a group-by key, never enumerated. `group_key`
//! defaults to `doc_type` (itself just a `kb_properties` row), so grouping by `stage`,
//! a facet, or a keyword needs no schema change. This is why there is no
//! `WHERE doc_type <> 'session'` anywhere in the read path — sessions are a bucket the
//! data produced, not a rule the designer wrote.

use serde::{Deserialize, Serialize};

use super::graph_territory::Territory;

/// One residual bucket: a distinct value of the group key, and how many otherwise
/// uncontained resources carry it.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_context.ts"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResidualBucket {
    pub value: String,
    pub count: i32,
}

/// The residue of a context, grouped. `buckets` is empty (never null) when every
/// resource reaches a container — the healthy steady state.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_context.ts"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResidualGroups {
    pub group_key: String,
    pub buckets: Vec<ResidualBucket>,
}

/// A group key the caller could have grouped by, with how much of the context it covers.
/// Lets the UI offer alternatives without the server assuming which one matters.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_context.ts"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct GroupKeyMeta {
    pub key: String,
    pub distinct_values: i32,
    pub coverage: i32,
}

/// Tier-0 of the context door.
///
/// `containers` carry `TerritoryKind::Context` — NOT a new variant. `kind` selects the
/// tint, and tint encodes the AXIS (spec D6): a goal container sits on the builder axis,
/// so it is `Context`-tinted even though it is rooted at a goal. `label` is the goal title.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_context.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ContextPanorama {
    pub containers: Vec<Territory>,
    pub residual: ResidualGroups,
    pub group_keys: Vec<GroupKeyMeta>,
}
```

Add to `crates/temper-core/src/types/mod.rs`, in alphabetical position among the other `pub mod` lines:

```rust
pub mod graph_context;
```

In `crates/temper-core/src/types/graph_atlas.rs`, add a field to `AtlasNode` immediately after `excerpt`:

```rust
    /// Workflow stage (`backlog`/`in-progress`/`done`/`cancelled`) for doc-types that
    /// carry one — tasks, chiefly. `None` for every other doc-type and for reads that
    /// do not source it. Ported from the legacy subgraph's `stage_raw` (spec D8): stage
    /// is load-bearing on a builder surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
```

- [ ] **Step 4: Run the test**

Run: `cargo test -p temper-core --all-features graph_context`
Expected: PASS (2 tests). The workspace will not yet build — `graph_service.rs` constructs `AtlasNode` without `stage`. Fix it now:

In `crates/temper-services/src/services/graph_service.rs`, every `AtlasNode { ... }` literal gains `stage: None` for the moment (Task 3 wires the real value). Find them with:

```bash
grep -n "AtlasNode {" crates/temper-services/src/services/graph_service.rs
```

- [ ] **Step 5: Regenerate TS types and verify**

```bash
cargo make generate-ts-types
git status --short packages/temper-ui/src/lib/types/generated/
```
Expected: `graph_context.ts` created; `graph_atlas.ts` modified with `stage?: string`.
Commit **all** regenerated files, even ones unrelated to this change.

- [ ] **Step 6: Full check + commit**

```bash
cargo fmt --all
cargo make check
git add crates/temper-core packages/temper-ui/src/lib/types/generated crates/temper-services/src/services/graph_service.rs
git commit -m "feat(atlas): ContextPanorama wire types + AtlasNode.stage"
```

---

## Task 2: SQL — the four context-graph read functions

**Files:**
- Create: `migrations/20260709000010_graph_context_reads.sql`
- Test: `crates/temper-api/tests/context_graph_test.rs`

**Interfaces:**
- Produces (callable from Task 4):
  - `graph_context_containers(p_profile uuid, p_context_id uuid, p_container_types text[], p_depth int) → TABLE(id uuid, label text, member_count int)`
  - `graph_context_residual_counts(p_profile uuid, p_context_id uuid, p_group_key text, p_container_types text[], p_depth int) → TABLE(group_value text, member_count int)`
  - `graph_context_residual_members(p_profile uuid, p_context_id uuid, p_group_key text, p_group_value text, p_container_types text[], p_depth int) → TABLE(id uuid)`
  - `graph_context_composition_edges(p_profile uuid, p_seed_ids uuid[], p_depth int) → TABLE(id uuid, source_id uuid, target_id uuid, edge_kind edge_kind, polarity edge_polarity, label text, weight double precision)`

> **Read first:** `migrations/20260708000002_graph_region_composition.sql`. It is the model. Reproduce its edge-visibility predicate **conjunct-for-conjunct**: both endpoints in `resources_visible_to`, `NOT is_folded`, and `anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)`. A read gate that enforces a *subset* of the canonical visibility predicate is a leak.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-api/tests/context_graph_test.rs`:

```rust
//! Beat E — integration tests for the context-graph SQL reads.
#![cfg(all(test, feature = "test-db"))]

use sqlx::PgPool;
use uuid::Uuid;

mod common;
use common::{seed_context_with_goal_and_tasks, seed_profile};

#[sqlx::test]
async fn containers_count_members_regardless_of_edge_label_or_direction(pool: PgPool) {
    // The parent_of -> advances backfill (20260709000005) REVERSES direction and changes
    // edge_kind. Member counts must be invariant across both representations, or every
    // territory silently empties the day that migration lands. (spec §2, finding 1)
    let (profile, ctx, goal) = seed_context_with_goal_and_tasks(&pool, 3).await;

    let rows: Vec<(Uuid, Option<String>, i32)> = sqlx::query_as(
        "SELECT id, label, member_count FROM graph_context_containers($1, $2, $3, $4)",
    )
    .bind(profile)
    .bind(ctx)
    .bind(&["goal"][..])
    .bind(2_i32)
    .fetch_all(&pool)
    .await
    .expect("containers");

    assert_eq!(rows.len(), 1, "one goal container");
    assert_eq!(rows[0].0, goal);
    assert_eq!(rows[0].2, 3, "three tasks reachable at depth 2");
}

#[sqlx::test]
async fn containers_deny_direction_invisible_resources_are_absent(pool: PgPool) {
    // Deny direction: a resource visible to A but not B must not appear in B's counts,
    // and its EXISTENCE must not leak through the member_count either.
    let (owner, ctx, goal) = seed_context_with_goal_and_tasks(&pool, 3).await;
    let stranger = seed_profile(&pool, "stranger").await;

    let rows: Vec<(Uuid, Option<String>, i32)> = sqlx::query_as(
        "SELECT id, label, member_count FROM graph_context_containers($1, $2, $3, $4)",
    )
    .bind(stranger)
    .bind(ctx)
    .bind(&["goal"][..])
    .bind(2_i32)
    .fetch_all(&pool)
    .await
    .expect("containers");

    assert!(rows.is_empty(), "stranger sees no containers, not empty ones");
    let _ = (owner, goal);
}

#[sqlx::test]
async fn residual_counts_group_by_doc_type_and_exclude_contained(pool: PgPool) {
    let (profile, ctx, _goal) = seed_context_with_goal_and_tasks(&pool, 3).await;

    let rows: Vec<(String, i32)> = sqlx::query_as(
        "SELECT group_value, member_count FROM graph_context_residual_counts($1, $2, $3, $4, $5)",
    )
    .bind(profile)
    .bind(ctx)
    .bind("doc_type")
    .bind(&["goal"][..])
    .bind(2_i32)
    .fetch_all(&pool)
    .await
    .expect("residual counts");

    // The 3 tasks reach the goal, so nothing is residual.
    assert!(rows.is_empty(), "contained tasks are not residual, got {rows:?}");
}
```

Add to `crates/temper-api/tests/common/mod.rs` (create the helpers if absent — model them on the existing seeds in that module):

```rust
/// Seeds a profile-owned context holding one `goal` and `n` `task` resources, each linked
/// goal --parent_of--> task. Returns `(profile_id, context_id, goal_id)`.
pub async fn seed_context_with_goal_and_tasks(
    pool: &PgPool,
    n: usize,
) -> (Uuid, Uuid, Uuid) { /* mirror the seeds used by graph_subgraph_test.rs */ }

/// Seeds a bare profile with no access to anything.
pub async fn seed_profile(pool: &PgPool, handle: &str) -> Uuid { /* ... */ }
```

- [ ] **Step 2: Run and watch it fail**

```bash
cargo make docker-up
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo nextest run -p temper-api --features test-db --test context_graph_test
```
Expected: FAIL — `function graph_context_containers(...) does not exist`.

- [ ] **Step 3: Write the migration**

Create `migrations/20260709000010_graph_context_reads.sql`:

```sql
-- Beat E: the context-graph reads. Builder-axis sibling of graph_region_composition
-- (20260708000002), from which the edge-visibility predicate is reproduced
-- conjunct-for-conjunct: both endpoints in resources_visible_to, NOT is_folded, and the
-- edge's home anchor readable.
--
-- INVARIANT: the container walk filters on NO edge label and NO direction. Goal->task
-- membership is recorded as `parent_of` (contains, goal->task) historically and as
-- `advances` (leads_to, task->goal) currently; migration 20260709000005 converts the
-- former to the latter. A label- or direction-filtered walk empties every territory the
-- day that backfill lands. Undirected + label-blind makes member counts invariant.

-- Container territories: resources of `p_container_types` homed in the context, sized by
-- how many distinct resources they reach within p_depth over VISIBLE internal edges.
CREATE FUNCTION graph_context_containers(
    p_profile         uuid,
    p_context_id      uuid,
    p_container_types text[],
    p_depth           int
) RETURNS TABLE(id uuid, label text, member_count int) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE
    vis AS (SELECT resource_id AS id FROM resources_visible_to(p_profile)),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
          FROM kb_properties p
         WHERE p.owner_table = 'kb_resources' AND p.property_key = 'doc_type'
           AND NOT p.is_folded
    ),
    ctx AS (  -- context-homed, active, visible. deny-as-absence via the vis join.
        SELECT r.id, r.title, d.dt
          FROM kb_resources r
          JOIN kb_resource_homes h ON h.resource_id = r.id
                                  AND h.anchor_table = 'kb_contexts'
                                  AND h.anchor_id = p_context_id
          JOIN vis v ON v.id = r.id
          LEFT JOIN doc d ON d.rid = r.id
         WHERE r.is_active
    ),
    ie AS (  -- internal edges, both endpoints visible + in-context, edge home readable
        SELECT e.source_id, e.target_id
          FROM kb_edges e
          JOIN ctx s ON s.id = e.source_id
          JOIN ctx t ON t.id = e.target_id
         WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
           AND NOT e.is_folded
           AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
    ),
    containers AS (SELECT c.id, c.title FROM ctx c WHERE c.dt = ANY(p_container_types)),
    reached AS (
        SELECT c.id AS root, c.id AS node_id, 0 AS depth FROM containers c
        UNION
        SELECT r.root,
               CASE WHEN ie.source_id = r.node_id THEN ie.target_id ELSE ie.source_id END,
               r.depth + 1
          FROM reached r
          JOIN ie ON (ie.source_id = r.node_id OR ie.target_id = r.node_id)
         WHERE r.depth < LEAST(p_depth, 3)
    )
    SELECT c.id,
           c.title AS label,
           (SELECT count(DISTINCT rr.node_id)::int - 1 FROM reached rr WHERE rr.root = c.id)
      FROM containers c;
$$;

-- Residual = context-homed + visible, reaching NO container. Grouped by an arbitrary
-- kb_properties key, so the bucket set is derived from data, never enumerated.
CREATE FUNCTION graph_context_residual_counts(
    p_profile         uuid,
    p_context_id      uuid,
    p_group_key       text,
    p_container_types text[],
    p_depth           int
) RETURNS TABLE(group_value text, member_count int) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE
    vis AS (SELECT resource_id AS id FROM resources_visible_to(p_profile)),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
          FROM kb_properties p
         WHERE p.owner_table = 'kb_resources' AND p.property_key = 'doc_type'
           AND NOT p.is_folded
    ),
    ctx AS (
        SELECT r.id, d.dt
          FROM kb_resources r
          JOIN kb_resource_homes h ON h.resource_id = r.id
                                  AND h.anchor_table = 'kb_contexts'
                                  AND h.anchor_id = p_context_id
          JOIN vis v ON v.id = r.id
          LEFT JOIN doc d ON d.rid = r.id
         WHERE r.is_active
    ),
    ie AS (
        SELECT e.source_id, e.target_id
          FROM kb_edges e
          JOIN ctx s ON s.id = e.source_id
          JOIN ctx t ON t.id = e.target_id
         WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
           AND NOT e.is_folded
           AND anchor_readable_by_profile(p_profile, e.home_anchor_table, e.home_anchor_id)
    ),
    reached AS (
        SELECT c.id AS node_id, 0 AS depth FROM ctx c WHERE c.dt = ANY(p_container_types)
        UNION
        SELECT CASE WHEN ie.source_id = r.node_id THEN ie.target_id ELSE ie.source_id END,
               r.depth + 1
          FROM reached r
          JOIN ie ON (ie.source_id = r.node_id OR ie.target_id = r.node_id)
         WHERE r.depth < LEAST(p_depth, 3)
    ),
    grp AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS gv
          FROM kb_properties p
         WHERE p.owner_table = 'kb_resources' AND p.property_key = p_group_key
           AND NOT p.is_folded
    )
    SELECT COALESCE(g.gv, '(none)') AS group_value, count(*)::int
      FROM ctx c
      LEFT JOIN grp g ON g.rid = c.id
     WHERE c.id NOT IN (SELECT node_id FROM reached)
     GROUP BY 1
     ORDER BY 2 DESC;
$$;

-- The ids behind one residual bucket — the seeds for its drill.
CREATE FUNCTION graph_context_residual_members(
    p_profile         uuid,
    p_context_id      uuid,
    p_group_key       text,
    p_group_value     text,
    p_container_types text[],
    p_depth           int
) RETURNS TABLE(id uuid) LANGUAGE sql STABLE AS $$
    SELECT r.id
      FROM kb_resources r
      JOIN kb_resource_homes h ON h.resource_id = r.id
                              AND h.anchor_table = 'kb_contexts'
                              AND h.anchor_id = p_context_id
      JOIN resources_visible_to(p_profile) v ON v.resource_id = r.id
      LEFT JOIN kb_properties p ON p.owner_table = 'kb_resources' AND p.owner_id = r.id
                               AND p.property_key = p_group_key AND NOT p.is_folded
     WHERE r.is_active
       AND COALESCE(p.property_value #>> '{}', '(none)') = p_group_value
       AND r.id NOT IN (
            SELECT rc.id FROM graph_context_containers(p_profile, p_context_id,
                                                       p_container_types, p_depth) rc
            UNION
            SELECT e.target_id FROM kb_edges e
              JOIN graph_context_containers(p_profile, p_context_id,
                                            p_container_types, p_depth) c2 ON c2.id = e.source_id
             WHERE NOT e.is_folded
            UNION
            SELECT e.source_id FROM kb_edges e
              JOIN graph_context_containers(p_profile, p_context_id,
                                            p_container_types, p_depth) c3 ON c3.id = e.target_id
             WHERE NOT e.is_folded
       );
$$;

-- Composition edges from an arbitrary visible seed set. NOT fenced to the context: the
-- walk follows visible edges out to cogmap-homed resources, which is what makes "the work
-- + the ideas distilled from it" one graph. Mirrors graph_region_composition_edges.
CREATE FUNCTION graph_context_composition_edges(
    p_profile  uuid,
    p_seed_ids uuid[],
    p_depth    int
) RETURNS TABLE(
    id uuid, source_id uuid, target_id uuid, edge_kind edge_kind,
    polarity edge_polarity, label text, weight double precision
) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE
    vis AS (SELECT resource_id AS id FROM resources_visible_to(p_profile)),
    seeds AS (SELECT DISTINCT s.id FROM unnest(p_seed_ids) s(id) JOIN vis v ON v.id = s.id),
    reached AS (
        SELECT id AS node_id, 0 AS depth FROM seeds
        UNION
        SELECT CASE WHEN e.source_id = r.node_id THEN e.target_id ELSE e.source_id END,
               r.depth + 1
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
```

- [ ] **Step 4: Apply and run the tests**

```bash
sqlx migrate run
cargo nextest run -p temper-api --features test-db --test context_graph_test
```
Expected: PASS (3 tests). If `containers_count_members...` reports `member_count` off by one, the `- 1` (excluding the root) is wrong — fix the SQL, not the test.

- [ ] **Step 5: Add the backfill-invariance test**

Append to `crates/temper-api/tests/context_graph_test.rs`:

```rust
#[sqlx::test]
async fn container_counts_survive_the_parent_of_to_advances_conversion(pool: PgPool) {
    let (profile, ctx, goal) = seed_context_with_goal_and_tasks(&pool, 3).await;

    let count = |p: &PgPool| async move {
        sqlx::query_scalar::<_, i32>(
            "SELECT member_count FROM graph_context_containers($1,$2,$3,$4)",
        )
        .bind(profile).bind(ctx).bind(&["goal"][..]).bind(2_i32)
        .fetch_one(p).await.expect("count")
    };
    let before = count(&pool).await;

    // Run the sibling session's conversion: parent_of goal->task becomes advances task->goal.
    sqlx::query("SELECT backfill_goal_parent_of_to_advances()")
        .execute(&pool).await.expect("backfill");

    assert_eq!(count(&pool).await, before, "undirected, label-blind walk is invariant");
    let _ = goal;
}
```

- [ ] **Step 6: Run, then commit**

```bash
cargo nextest run -p temper-api --features test-db --test context_graph_test
cargo fmt --all
git add migrations/20260709000010_graph_context_reads.sql crates/temper-api/tests
git commit -m "feat(sql): context-graph reads — containers, residual counts/members, composition edges"
```

---

## Task 3: Migration — widen `graph_atlas_nodes_visible` with `stage`

**Files:**
- Create: `migrations/20260709000011_atlas_nodes_visible_stage.sql`
- Modify: `crates/temper-services/src/services/graph_service.rs:536-560`

**Interfaces:**
- Consumes: `AtlasNode.stage` (Task 1).
- Produces: `graph_atlas_nodes_visible(p_profile uuid, p_ids uuid[]) → TABLE(id, title, doc_type, home, degree, first_chunk, stage)`.

> A shipped SQL function whose `RETURNS TABLE` changes needs a **new migration** doing `DROP` then `CREATE` — you cannot widen it in place, and you must never edit `20260708000002`.

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-api/tests/context_graph_test.rs`:

```rust
#[sqlx::test]
async fn atlas_nodes_visible_reports_task_stage(pool: PgPool) {
    let (profile, _ctx, _goal) = seed_context_with_goal_and_tasks(&pool, 1).await;
    let task_id: Uuid = sqlx::query_scalar(
        "SELECT owner_id FROM kb_properties WHERE property_key='doc_type'
           AND property_value #>> '{}' = 'task' LIMIT 1",
    ).fetch_one(&pool).await.expect("a task");

    sqlx::query("SELECT set_resource_stage($1, 'in-progress')")
        .bind(task_id).execute(&pool).await.ok(); // helper may not exist; set via kb_properties if so

    let stage: Option<String> = sqlx::query_scalar(
        "SELECT stage FROM graph_atlas_nodes_visible($1, $2)",
    )
    .bind(profile)
    .bind(&[task_id][..])
    .fetch_one(&pool).await.expect("node row");

    assert_eq!(stage.as_deref(), Some("in-progress"));
}
```

- [ ] **Step 2: Run and watch it fail**

Run: `cargo nextest run -p temper-api --features test-db --test context_graph_test atlas_nodes_visible_reports_task_stage`
Expected: FAIL — `column "stage" does not exist`.

- [ ] **Step 3: Write the migration**

Create `migrations/20260709000011_atlas_nodes_visible_stage.sql`. Copy the body of `graph_atlas_nodes_visible` verbatim from `migrations/20260708000002_graph_region_composition.sql`, then add the `stage` column. The signature changes, so `DROP` first:

```sql
-- Beat E (spec D8): widen graph_atlas_nodes_visible with `stage`. The legacy subgraph
-- returned `stage_raw`; AtlasNode did not carry it, so retiring that surface would have
-- dropped the task-stage signal from a builder view. RETURNS TABLE changed => DROP+CREATE
-- in a NEW migration (20260708000002 is applied and immutable).

DROP FUNCTION IF EXISTS graph_atlas_nodes_visible(uuid, uuid[]);

CREATE FUNCTION graph_atlas_nodes_visible(p_profile uuid, p_ids uuid[])
RETURNS TABLE(id uuid, title text, doc_type text, home text, degree int,
              first_chunk text, stage text)
LANGUAGE sql STABLE AS $$
    WITH vis AS (SELECT resource_id AS id FROM resources_visible_to(p_profile)),
    ids AS (SELECT DISTINCT unnest(p_ids) AS id),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table = 'kb_resources' AND p.property_key = 'doc_type' AND NOT p.is_folded
    ),
    stg AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS st
        FROM kb_properties p
        WHERE p.owner_table = 'kb_resources' AND p.property_key = 'stage' AND NOT p.is_folded
    )
    SELECT r.id, r.title, d.dt AS doc_type, h.home,
           COALESCE(deg.degree, 0) AS degree,
           (SELECT cc.content FROM kb_chunks ch
              JOIN kb_content_blocks b ON b.id = ch.block_id
              JOIN kb_chunk_content cc ON cc.chunk_id = ch.id
             WHERE ch.resource_id = r.id AND ch.is_current AND NOT b.is_folded
             ORDER BY b.seq, ch.chunk_index LIMIT 1) AS first_chunk,
           s.st AS stage
    FROM ids
    JOIN vis v ON v.id = ids.id           -- deny-as-absence: unseen ids drop out
    JOIN kb_resources r ON r.id = ids.id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN stg s ON s.rid = r.id
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

- [ ] **Step 4: Update the one Rust call site**

`crates/temper-services/src/services/graph_service.rs:536` — extend the tuple type and the `SELECT`, and set `stage` on the constructed `AtlasNode`:

```rust
        _,
        (Uuid, String, Option<String>, String, i32, Option<String>, Option<String>),
    >(
        "SELECT id, title, doc_type, home, degree, first_chunk, stage FROM graph_atlas_nodes_visible($1, $2)",
    )
```

and in the `.map(...)` closure, bind the seventh element and pass it through as `stage`.

- [ ] **Step 5: Apply, regenerate caches, run**

```bash
sqlx migrate run
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make prepare-api
cargo nextest run -p temper-api --features test-db --test context_graph_test
```
Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && cargo make check
git add migrations crates .sqlx crates/temper-services/.sqlx crates/temper-api/.sqlx
git commit -m "feat(sql): widen graph_atlas_nodes_visible with stage (DROP+CREATE)"
```

---

## Task 4: `context_graph_service`

**Files:**
- Create: `crates/temper-services/src/services/context_graph_service.rs`
- Modify: `crates/temper-services/src/services/mod.rs`

**Interfaces:**
- Consumes: Task 1's types; Task 2 + 3's SQL.
- Produces:
  - `context_panorama(pool: &PgPool, profile_id: ProfileId, context_id: ContextId, group_key: &str, container_types: &[String], depth: i32) -> ApiResult<ContextPanorama>`
  - `context_composition(pool: &PgPool, profile_id: ProfileId, seeds: &[Uuid], depth: i32) -> ApiResult<AtlasSubgraph>`
  - `residual_member_ids(pool: &PgPool, profile_id: ProfileId, context_id: ContextId, group_key: &str, group_value: &str, container_types: &[String], depth: i32) -> ApiResult<Vec<Uuid>>`

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-api/tests/context_graph_test.rs`:

```rust
use temper_services::services::context_graph_service;
use temper_core::ids::{ContextId, ProfileId};

#[sqlx::test]
async fn panorama_returns_containers_and_empty_residual(pool: PgPool) {
    let (profile, ctx, goal) = seed_context_with_goal_and_tasks(&pool, 3).await;
    let p = context_graph_service::context_panorama(
        &pool, ProfileId::from(profile), ContextId::from(ctx),
        "doc_type", &["goal".to_string()], 2,
    ).await.expect("panorama");

    assert_eq!(p.containers.len(), 1);
    assert_eq!(p.containers[0].id, goal);
    assert_eq!(p.containers[0].member_count, 3);
    assert_eq!(p.residual.group_key, "doc_type");
    assert!(p.residual.buckets.is_empty(), "tray empties on well-edged data");
}

#[sqlx::test]
async fn composition_from_a_container_includes_its_members(pool: PgPool) {
    let (profile, _ctx, goal) = seed_context_with_goal_and_tasks(&pool, 3).await;
    let sg = context_graph_service::context_composition(
        &pool, ProfileId::from(profile), &[goal], 1,
    ).await.expect("composition");

    assert_eq!(sg.nodes.len(), 4, "goal + 3 tasks");
    assert_eq!(sg.edges.len(), 3);
}
```

- [ ] **Step 2: Run and watch it fail**

Run: `cargo nextest run -p temper-api --features test-db --test context_graph_test panorama_returns`
Expected: FAIL — `unresolved import context_graph_service`.

- [ ] **Step 3: Write the service**

Create `crates/temper-services/src/services/context_graph_service.rs`. Mirror `graph_service::region_composition_slice` for the node-hydration half (it already calls `graph_atlas_nodes_visible` and maps `home`/`excerpt`); extract that mapping into a shared helper rather than copy-pasting it — duplication across service boundaries is a bug waiting to happen.

```rust
//! Beat E — the context door's reads. Persistence lives in SQL (20260709000010);
//! this module composes it. No `sqlx::query!()` ever appears in a handler.

use sqlx::PgPool;
use temper_core::ids::{ContextId, ProfileId};
use temper_core::types::graph_atlas::AtlasSubgraph;
use temper_core::types::graph_context::{ContextPanorama, GroupKeyMeta, ResidualBucket, ResidualGroups};
use temper_core::types::graph_territory::{Territory, TerritoryKind};
use uuid::Uuid;

use crate::ApiResult;

/// Bound the drill so a residual bucket of 395 sessions cannot produce a hairball.
/// Not a silent truncation: the caller reports what was dropped.
const MAX_SEEDS: usize = 250;

pub async fn context_panorama(
    pool: &PgPool,
    profile_id: ProfileId,
    context_id: ContextId,
    group_key: &str,
    container_types: &[String],
    depth: i32,
) -> ApiResult<ContextPanorama> {
    let containers: Vec<Territory> = sqlx::query_as::<_, (Uuid, Option<String>, i32)>(
        "SELECT id, label, member_count FROM graph_context_containers($1, $2, $3, $4)",
    )
    .bind(profile_id.as_uuid())
    .bind(*context_id)
    .bind(container_types)
    .bind(depth)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(id, label, member_count)| Territory {
        id,
        // Tint encodes the AXIS, not container-ness (spec D6). A goal container sits on
        // the builder axis, so it is Context-tinted.
        kind: TerritoryKind::Context,
        label,
        member_count,
        salience: None,
        coherence: None,
        anchor_id: *context_id,
    })
    .collect();

    let buckets: Vec<ResidualBucket> = sqlx::query_as::<_, (String, i32)>(
        "SELECT group_value, member_count FROM graph_context_residual_counts($1,$2,$3,$4,$5)",
    )
    .bind(profile_id.as_uuid())
    .bind(*context_id)
    .bind(group_key)
    .bind(container_types)
    .bind(depth)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(value, count)| ResidualBucket { value, count })
    .collect();

    let group_keys = available_group_keys(pool, profile_id, context_id).await?;

    Ok(ContextPanorama {
        containers,
        residual: ResidualGroups { group_key: group_key.to_string(), buckets },
        group_keys,
    })
}

/// What else the caller could group by, and how much of the context each key covers.
async fn available_group_keys(
    pool: &PgPool,
    profile_id: ProfileId,
    context_id: ContextId,
) -> ApiResult<Vec<GroupKeyMeta>> {
    Ok(sqlx::query_as::<_, (String, i32, i32)>(
        r#"
        SELECT p.property_key,
               count(DISTINCT p.property_value #>> '{}')::int AS distinct_values,
               count(DISTINCT p.owner_id)::int               AS coverage
          FROM kb_properties p
          JOIN kb_resource_homes h ON h.resource_id = p.owner_id
                                  AND h.anchor_table = 'kb_contexts' AND h.anchor_id = $2
          JOIN resources_visible_to($1) v ON v.resource_id = p.owner_id
         WHERE p.owner_table = 'kb_resources' AND NOT p.is_folded
         GROUP BY 1 HAVING count(DISTINCT p.property_value #>> '{}') BETWEEN 2 AND 24
         ORDER BY 3 DESC
        "#,
    )
    .bind(profile_id.as_uuid())
    .bind(*context_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(key, distinct_values, coverage)| GroupKeyMeta { key, distinct_values, coverage })
    .collect())
}

pub async fn context_composition(
    pool: &PgPool,
    profile_id: ProfileId,
    seeds: &[Uuid],
    depth: i32,
) -> ApiResult<AtlasSubgraph> {
    if seeds.is_empty() {
        return Err(crate::ApiError::BadRequest("seeds must be non-empty".into()));
    }
    let bounded = &seeds[..seeds.len().min(MAX_SEEDS)];
    // edges → node id set → hydrate. Same two-step as region_composition_slice.
    todo_hydrate(pool, profile_id, bounded, depth).await
}
```

Replace `todo_hydrate` with the real body: call `graph_context_composition_edges`, collect the distinct endpoint ids **plus the seeds** (so an isolated seed still renders), then hydrate with the shared `graph_atlas_nodes_visible` helper. Do not leave the name `todo_hydrate` in the tree.

Add `pub mod context_graph_service;` to `crates/temper-services/src/services/mod.rs`.

- [ ] **Step 4: Run the tests**

```bash
cargo nextest run -p temper-api --features test-db --test context_graph_test
```
Expected: PASS (7 tests).

- [ ] **Step 5: Regenerate caches and commit**

```bash
cargo sqlx prepare --workspace -- --all-features && cargo make prepare-services && cargo make prepare-api
cargo fmt --all && cargo make check
git add crates .sqlx
git commit -m "feat(services): context_graph_service — panorama + composition"
```

---

## Task 5: API handlers + routes + OpenAPI

**Files:**
- Modify: `crates/temper-api/src/handlers/graph.rs`, `crates/temper-api/src/routes.rs`, `crates/temper-api/src/openapi.rs`

**Interfaces:**
- Consumes: `context_graph_service` (Task 4).
- Produces: `GET /api/graph/contexts/panorama?context_ref=&group_by=` → `ContextPanorama`; `GET /api/graph/contexts/composition?context_ref=&container=|group=&depth=` → `AtlasSubgraph`.

> The context arrives as a **query param, not a path segment** — a decorated ref is `owner/slug` and contains a slash. Mirror `get_subgraph` (`handlers/graph.rs:45-68`): `parse_context_ref` → `resolve_context_ref`, which already yields `NotFound` on a miss and `Forbidden` on team non-membership without leaking existence.

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-api/tests/context_graph_test.rs` an axum-level test in the style of the existing handler tests in this crate (find one with `grep -rn "create_app" crates/temper-api/tests | head -3`):

```rust
#[sqlx::test]
async fn panorama_endpoint_404s_for_an_invisible_context(pool: PgPool) {
    let (_owner, ctx, _goal) = seed_context_with_goal_and_tasks(&pool, 1).await;
    let stranger_token = /* mint a JWT for a profile with no access; see fixtures */;

    let res = request_get(
        &pool,
        &format!("/api/graph/contexts/panorama?context_ref={ctx}&group_by=doc_type"),
        &stranger_token,
    ).await;

    // Deny-as-absence: never 403 "exists but forbidden" for a profile-owned context.
    assert_eq!(res.status(), 404);
}
```

- [ ] **Step 2: Run and watch it fail**

Run: `cargo nextest run -p temper-api --features test-db --test context_graph_test panorama_endpoint`
Expected: FAIL — 404 from the router (route absent), or compile error.

- [ ] **Step 3: Add the handlers**

In `crates/temper-api/src/handlers/graph.rs`:

```rust
/// Query for `GET /api/graph/contexts/panorama`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ContextPanoramaQuery {
    /// Decorated context ref (`@me/temper`, `+team/slug`) or a bare UUID.
    pub context_ref: String,
    /// Property key the residual tray groups by. Defaults to `doc_type`.
    pub group_by: Option<String>,
    /// Doc-types treated as containers. Defaults to `goal` (spec D4: a parameter, not a constant).
    pub container_types: Option<String>,
    pub depth: Option<i32>,
}

/// Query for `GET /api/graph/contexts/composition`. Exactly one of `container` / `group` is required.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ContextCompositionQuery {
    pub context_ref: String,
    /// Container resource id to drill.
    pub container: Option<Uuid>,
    /// Residual bucket to drill, as `<group_key>:<group_value>`.
    pub group: Option<String>,
    pub container_types: Option<String>,
    pub depth: Option<i32>,
}
```

Both handlers: `parse_context_ref` → `resolve_context_ref` → service. `container_types` splits on `,` and defaults to `vec!["goal".to_string()]`; `group_by` defaults to `"doc_type"`. For `composition`, reject when neither or both of `container`/`group` are supplied (`ApiError::BadRequest`) — parse, don't validate: decode into an enum first.

```rust
enum CompositionTarget { Container(Uuid), Bucket { key: String, value: String } }
```

- [ ] **Step 4: Register routes + OpenAPI**

In `routes.rs`, beside the existing graph routes (~line 101):

```rust
        .route("/api/graph/contexts/panorama", get(handlers::graph::context_panorama))
        .route("/api/graph/contexts/composition", get(handlers::graph::context_composition))
```

In `openapi.rs`, add both paths and the `ContextPanorama`/`ResidualGroups`/`ResidualBucket`/`GroupKeyMeta` schemas, and update the path-count assertion (~`openapi.rs:236`).

- [ ] **Step 5: Run and commit**

```bash
cargo nextest run -p temper-api --features test-db --test context_graph_test
cargo fmt --all && cargo make check
git add crates && git commit -m "feat(api): context panorama + composition endpoints"
```

---

## Task 6: `nav.ts` — the `?context` door and its focus tokens

**Files:**
- Modify: `packages/temper-ui/src/lib/graph/atlas/nav.ts`
- Test: `packages/temper-ui/src/lib/graph/atlas/nav.test.ts`

**Interfaces:**
- Produces: `parseContextScope(url): string | null`; `Focus` gains `{kind:'container'; id:string}` and `{kind:'bucket'; groupKey:string; value:string}`; `buildContextUrl(base, slug)`, `buildDrillContainerUrl(base, id)`, `buildDrillBucketUrl(base, groupKey, value)`; `deriveTier` maps both new kinds to `1`.

> A container id is a **resource** uuid. Do **not** reuse the `territory:` token: `territoryIds()` splits territory tokens on `~` for the Beat-D region union, and region ids are ephemeral. Separate token kinds keep the two addressing schemes from colliding.

- [ ] **Step 1: Write the failing tests**

Append to `packages/temper-ui/src/lib/graph/atlas/nav.test.ts`:

```ts
import { buildContextUrl, buildDrillBucketUrl, buildDrillContainerUrl, deriveTier, parseContextScope, parseFocus } from './nav';

const u = (s: string) => new URL(`http://x${s}`);

describe('context door', () => {
	it('reads the ?context scope', () => {
		expect(parseContextScope(u('/graph/@me?context=temper'))).toBe('temper');
		expect(parseContextScope(u('/graph/@me'))).toBeNull();
	});

	it('entering a context clears any focus', () => {
		expect(buildContextUrl(u('/graph/@me?focus=node:abc'), 'temper')).toBe('/graph/@me?context=temper');
	});

	it('parses a container focus and puts it on tier 1', () => {
		const f = parseFocus(u('/graph/@me?context=temper&focus=container:9f2e').searchParams);
		expect(f).toEqual({ kind: 'container', id: '9f2e' });
		expect(deriveTier(f)).toBe(1);
	});

	it('parses a bucket focus and puts it on tier 1', () => {
		const f = parseFocus(u('/graph/@me?context=temper&focus=bucket:doc_type:session').searchParams);
		expect(f).toEqual({ kind: 'bucket', groupKey: 'doc_type', value: 'session' });
		expect(deriveTier(f)).toBe(1);
	});

	it('round-trips a bucket value containing a colon', () => {
		const url = buildDrillBucketUrl(u('/graph/@me?context=temper'), 'stage', 'in:progress');
		expect(parseFocus(new URL(`http://x${url}`).searchParams)).toEqual({
			kind: 'bucket', groupKey: 'stage', value: 'in:progress'
		});
	});

	it('builds a container drill', () => {
		expect(buildDrillContainerUrl(u('/graph/@me?context=temper'), '9f2e'))
			.toBe('/graph/@me?context=temper&focus=container%3A9f2e');
	});
});
```

- [ ] **Step 2: Run and watch it fail**

Run: `cd packages/temper-ui && bun run test --run nav`
Expected: FAIL — `parseContextScope is not a function`.

- [ ] **Step 3: Implement**

In `nav.ts`, extend `Focus` and `parseFocusToken`. The bucket token is `bucket:<groupKey>:<value>` — split into **at most 3 parts** so a value containing `:` survives:

```ts
export type Focus =
	| { kind: 'none' }
	| { kind: 'territory'; id: string }
	| { kind: 'node'; id: string }
	| { kind: 'container'; id: string }
	| { kind: 'bucket'; groupKey: string; value: string };

function parseFocusToken(tok: string): Focus | null {
	const [kind, ...rest] = tok.split(':');
	if (kind === 'bucket') {
		const groupKey = rest[0];
		const value = rest.slice(1).join(':'); // values may contain ':'
		return groupKey && value ? { kind: 'bucket', groupKey, value } : null;
	}
	const id = rest.join(':');
	if (id && (kind === 'territory' || kind === 'node' || kind === 'container')) {
		return { kind, id } as Focus;
	}
	return null;
}

export function parseContextScope(url: URL): string | null {
	return url.searchParams.get('context');
}

/** Enter a context door: set context, clear focus (re-scope resets to Tier 0). */
export function buildContextUrl(base: URL, slug: string): string {
	return withParams(base, (p) => {
		p.set('context', slug);
		p.delete('focus');
		p.delete('cogmap');
	});
}

export function buildDrillContainerUrl(base: URL, id: string): string {
	return withParams(base, (p) => p.set('focus', `container:${id}`));
}

export function buildDrillBucketUrl(base: URL, groupKey: string, value: string): string {
	return withParams(base, (p) => p.set('focus', `bucket:${groupKey}:${value}`));
}
```

Extend `deriveTier`:

```ts
		case 'territory':
		case 'container':
		case 'bucket':
			return 1;
```

- [ ] **Step 4: Run**

Run: `bun run test --run nav`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cd packages/temper-ui && bun run check
git add packages/temper-ui/src/lib/graph/atlas/nav.ts packages/temper-ui/src/lib/graph/atlas/nav.test.ts
git commit -m "feat(atlas): ?context door + container/bucket focus tokens"
```

---

## Task 7: `forceNeighborhood` — the `coreHome` radial

**Files:**
- Modify: `packages/temper-ui/src/lib/graph/atlas/layout/forceNeighborhood.ts:76-95`, `packages/temper-ui/src/lib/components/graph/atlas/TierNeighborhood.svelte:22`
- Test: `packages/temper-ui/src/lib/graph/atlas/layout/forceNeighborhood.radial.test.ts`

**Interfaces:**
- Produces: `forceNeighborhood(subgraph, seeds, { width, height, coreHome })` where `coreHome: NodeHome` defaults to `'cogmap'` (Beat D's shipped behaviour).

> **Shapes do not move.** `nodeMarkShape(home)` stays keyed on `home`. Only the radius keys on `coreHome`. A reviewer who sees a change to `marks.ts` in this task should reject it.

- [ ] **Step 1: Write the failing test**

Append to `forceNeighborhood.radial.test.ts` (mirror the existing `contextMean > facetMean` assertion):

```ts
it('inverts the radial when coreHome is context', () => {
	const laid = forceNeighborhood(mixedSubgraph, [], { width: 1040, height: 620, coreHome: 'context' });
	const mean = (home: string) => {
		const rs = laid.nodes.filter((n) => n.home === home)
			.map((n) => Math.hypot(n.x - 520, n.y - 310));
		return rs.reduce((a, b) => a + b, 0) / rs.length;
	};
	// Context resources are the SUBJECT: they hold the core; cogmap distillations ring them.
	expect(mean('cogmap')).toBeGreaterThan(mean('context'));
});

it('defaults to coreHome cogmap — Beat D behaviour is unchanged', () => {
	const laid = forceNeighborhood(mixedSubgraph, [], { width: 1040, height: 620 });
	const mean = (home: string) => {
		const rs = laid.nodes.filter((n) => n.home === home)
			.map((n) => Math.hypot(n.x - 520, n.y - 310));
		return rs.reduce((a, b) => a + b, 0) / rs.length;
	};
	expect(mean('context')).toBeGreaterThan(mean('cogmap'));
});
```

- [ ] **Step 2: Run and watch it fail**

Run: `bun run test --run forceNeighborhood`
Expected: FAIL on the first new test (`coreHome` ignored).

- [ ] **Step 3: Implement**

In `forceNeighborhood.ts`, add `coreHome` to the options and key `forceRadial` on it:

```ts
export interface ForceOptions {
	width: number;
	height: number;
	/**
	 * Which home is the SUBJECT of this view: its nodes hold the core, the other home rings
	 * them. Beat D's region drill distils ideas FROM sources, so cogmap facets are the core
	 * (the default). Beat E's context view inverts it: the work is the subject.
	 *
	 * This is the composition, not the visual language. Mark SHAPE stays keyed on `home`
	 * (`marks.ts`) so a circle is always a cogmap node and a rounded-square always a context
	 * resource, in every view.
	 */
	coreHome?: NodeHome;
}
```

then, in the force setup, replace the hard-coded `home === 'context'` test:

```ts
	const core = opts.coreHome ?? 'cogmap';
	.force('radial', forceRadial<ForceNode>(
		(d) => (d.home === core ? rInner : rOuter),
		cx, cy
	).strength(0.6))
```

Keep the link distance/strength `home`-crossing logic exactly as it is — it depends on *whether* two nodes differ in home, not on which is central.

In `TierNeighborhood.svelte`, add a `coreHome` prop (default `'cogmap'`) and pass it into `forceNeighborhood`.

- [ ] **Step 4: Run**

Run: `bun run test --run forceNeighborhood`
Expected: PASS, including the unchanged Beat-D default.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas/layout packages/temper-ui/src/lib/components/graph/atlas/TierNeighborhood.svelte
git commit -m "feat(atlas): coreHome radial — invert composition, never the mark shapes"
```

---

## Task 8: The residual tray

**Files:**
- Create: `packages/temper-ui/src/lib/graph/atlas/residualTray.ts`, `residualTray.test.ts`, `packages/temper-ui/src/lib/components/graph/atlas/ResidualTray.svelte`
- Modify: `packages/temper-ui/src/lib/components/graph/atlas/TierPanorama.svelte`

**Interfaces:**
- Produces: `trayModel(buckets: ResidualBucket[], width: number): TrayCell[]` where `TrayCell = { value: string; count: number; x: number; width: number }`; and `territoryWeight(t: { salience: number | null; member_count: number }): number`, **exported from `labels.ts`**.

> **Open question, settle here:** the spec (§7) leans toward the tray living in the **page chrome** rather than inside the `AtlasCanvas` SVG — a doorway should not pan away under the camera. Implement it in chrome. If it reads badly in `/dev/atlas`, say so in the review rather than silently moving it.

- [ ] **Step 0: Extract `territoryWeight` so it can be tested**

`ad324b09` introduced `territoryWeight` as a local `const` inside `TierPanorama.svelte` — it is unreachable from a test. Move it to `src/lib/graph/atlas/labels.ts` and export it; `TierPanorama` imports it. Then add to `labels.test.ts`:

```ts
import { territoryWeight } from './labels';

describe('territoryWeight', () => {
	it('uses a region salience verbatim — regions skip the log ramp', () => {
		expect(territoryWeight({ salience: 0.5, member_count: 99 })).toBe(0.5);
	});

	it('log1p-compresses a raw member_count', () => {
		// member counts are heavy-tailed; the raw ratio pinned ordinary goals to the floor.
		expect(territoryWeight({ salience: null, member_count: 4 })).toBe(Math.log1p(4));
	});

	it('maps an empty container to 0 so it still ghost-renders', () => {
		expect(territoryWeight({ salience: null, member_count: 0 })).toBe(0);
	});

	it('a null-salience region with members takes the log branch (behaviour change in ad324b09)', () => {
		expect(territoryWeight({ salience: null, member_count: 7 })).toBe(Math.log1p(7));
	});
});
```

Run `bun run test --run labels` — expect FAIL (`territoryWeight` not exported), then extract and re-run to PASS.

- [ ] **Step 1: Write the failing tray test**

Create `residualTray.test.ts`:

```ts
import { describe, expect, it } from 'vitest';
import { trayModel } from './residualTray';

describe('trayModel', () => {
	it('is empty for a well-edged context — the tray vanishes', () => {
		expect(trayModel([], 900)).toEqual([]);
	});

	it('sizes cells by share but never below a legible minimum', () => {
		const cells = trayModel(
			[{ value: 'session', count: 395 }, { value: 'decision', count: 9 }],
			900
		);
		expect(cells).toHaveLength(2);
		expect(cells[0].width).toBeGreaterThan(cells[1].width);
		expect(cells[1].width).toBeGreaterThanOrEqual(118);
		expect(cells[0].x).toBe(0);
		expect(cells[1].x).toBe(cells[0].width);
	});

	it('orders by count descending regardless of input order', () => {
		const cells = trayModel(
			[{ value: 'decision', count: 9 }, { value: 'session', count: 395 }],
			900
		);
		expect(cells.map((c) => c.value)).toEqual(['session', 'decision']);
	});
});
```

- [ ] **Step 2: Run and watch it fail**

Run: `bun run test --run residualTray`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement**

Create `residualTray.ts`:

```ts
// residualTray.ts
/**
 * The residual tray: a doorway to what reaches no container, NOT a landmark of the work.
 *
 * Residuals are deliberately kept OUT of the force field. In-field they capture the label
 * gate (4 of 10 on real data) and, worse, `intensityOf` normalizes against the largest
 * weight in the field — so one 395-item bucket drags every real goal's intensity toward
 * zero (Maintenance 1.00 → 0.16). They do not merely steal labels; they flatten the survey.
 *
 * On well-edged data containers absorb their members and this model returns `[]`, so the
 * tray disappears with no special case.
 */
import type { ResidualBucket } from '$lib/types/generated/graph_context';

export interface TrayCell {
	value: string;
	count: number;
	x: number;
	width: number;
}

/** Below this a cell cannot hold its label + count legibly. */
const MIN_CELL = 118;

export function trayModel(buckets: ResidualBucket[], width: number): TrayCell[] {
	if (buckets.length === 0) return [];
	const sorted = [...buckets].sort((a, b) => b.count - a.count);
	const total = sorted.reduce((a, b) => a + b.count, 0);
	let x = 0;
	return sorted.map((b) => {
		const w = Math.max(MIN_CELL, total > 0 ? (b.count / total) * width : MIN_CELL);
		const cell = { value: b.value, count: b.count, x, width: w };
		x += w;
		return cell;
	});
}
```

Create `ResidualTray.svelte` rendering the cells as buttons (keyboard-reachable, visible focus ring), each calling `goto(buildDrillBucketUrl($page.url, groupKey, value))`. Tint with `TERRITORY_TINTS.context` at low fill-opacity — cool, because a residual bucket is still the builder axis (spec D6). Label each cell `{value}` with `{count} resources` beneath, `font-variant-numeric: tabular-nums`.

Render `<ResidualTray>` from `TierPanorama.svelte`'s chrome slot, not inside the zoomed `<g>`.

- [ ] **Step 4: Run**

Run: `bun run test --run residualTray && bun run check`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-ui/src/lib/graph/atlas packages/temper-ui/src/lib/components/graph/atlas
git commit -m "feat(atlas): residual tray — a doorway, not a landmark"
```

---

## Task 9: Reads, loader, tier dispatch, crumbs

**Files:**
- Modify: `packages/temper-ui/src/lib/server/graph-reads.ts`, `src/routes/(app)/graph/[owner]/+page.server.ts`, `src/lib/components/graph/atlas/AtlasCanvas.svelte`, `src/lib/graph/atlas/crumbModel.ts`, `src/lib/graph/atlas/viewData.ts`
- Test: `packages/temper-ui/src/lib/server/graph-reads.paths.test.ts`, `crumbModel.test.ts`

**Interfaces:**
- Consumes: Task 5's endpoints, Task 6's nav.
- Produces: `contextPanoramaPath(ref, groupBy?)`, `contextCompositionPath(ref, target, depth?)`, `readContextPanorama(token, ref, groupBy?)`, `readContextComposition(token, ref, target, depth?)`. `AtlasViewData` gains `contextSlug: string | null` and `panorama: ContextPanorama | null`.

- [ ] **Step 1: Write the failing path tests**

Append to `graph-reads.paths.test.ts`:

```ts
it('builds the context panorama path', () => {
	expect(contextPanoramaPath('@me/temper', 'doc_type'))
		.toBe('/api/graph/contexts/panorama?context_ref=%40me%2Ftemper&group_by=doc_type');
});

it('builds a container composition path', () => {
	expect(contextCompositionPath('@me/temper', { kind: 'container', id: 'abc' }, 1))
		.toBe('/api/graph/contexts/composition?context_ref=%40me%2Ftemper&container=abc&depth=1');
});

it('builds a bucket composition path, encoding the group value', () => {
	expect(contextCompositionPath('@me/temper', { kind: 'bucket', groupKey: 'doc_type', value: 'session' }, 1))
		.toBe('/api/graph/contexts/composition?context_ref=%40me%2Ftemper&group=doc_type%3Asession&depth=1');
});
```

- [ ] **Step 2: Run and watch it fail**

Run: `bun run test --run graph-reads`
Expected: FAIL — not exported.

- [ ] **Step 3: Implement the wrappers**

In `graph-reads.ts` — `context_ref` **must be percent-encoded** (it contains `/` and `@`):

```ts
export type CompositionTarget =
	| { kind: 'container'; id: string }
	| { kind: 'bucket'; groupKey: string; value: string };

export const contextPanoramaPath = (ref: string, groupBy = 'doc_type'): string =>
	`/api/graph/contexts/panorama?context_ref=${encodeURIComponent(ref)}&group_by=${encodeURIComponent(groupBy)}`;

export const contextCompositionPath = (ref: string, t: CompositionTarget, depth = 1): string => {
	const target = t.kind === 'container'
		? `container=${encodeURIComponent(t.id)}`
		: `group=${encodeURIComponent(`${t.groupKey}:${t.value}`)}`;
	return `/api/graph/contexts/composition?context_ref=${encodeURIComponent(ref)}&${target}&depth=${depth}`;
};

export const readContextPanorama = (token: string, ref: string, groupBy = 'doc_type') =>
	apiGet<ContextPanorama>(contextPanoramaPath(ref, groupBy), token);

export const readContextComposition = (token: string, ref: string, t: CompositionTarget, depth = 1) =>
	apiGet<AtlasSubgraph>(contextCompositionPath(ref, t, depth), token);
```

- [ ] **Step 4: Wire the loader and the dispatch**

In `+page.server.ts`, add a `?context` branch **before** the cogmap branch. Build the ref as `` `${params.owner}/${contextSlug}` `` (the same shape `get_subgraph` accepts). On `tier === 0` read the panorama; on a `container`/`bucket` focus read the composition, degrading to the panorama on 404 exactly as `compositionOrPanorama` does for ephemeral region ids.

In `AtlasCanvas.svelte:65`, the first branch **must** learn about the context door or a context view falls into `TierHome`:

```svelte
	{#if !cogmapId && !contextSlug && home}
		<TierHome {home} width={W} height={H} />
	{:else if tier === 0 && contextSlug && panorama}
		<TierPanorama overview={contextOverview} width={W} height={H} docTypes={filters.docTypes} />
	{:else if tier === 0 && territories}
		...
```

where `contextOverview` adapts `ContextPanorama.containers` into the `TerritoryOverview` shape `TierPanorama` already consumes (`{ territories, orphan_nodes: [], bridges: [] }`).

Pass `coreHome="context"` to `TierNeighborhood` on the context branch.

Extend `crumbModel` with a context segment (label = the slug) and a leaf for the container (its title) or bucket (`Unfiled · <value>`), with tests.

- [ ] **Step 5: Run everything and commit**

```bash
bun run test --run && bun run check
git add packages/temper-ui/src
git commit -m "feat(atlas): context door loader, tier dispatch, crumbs"
```

---

## Task 10: `/dev/atlas` fixtures + the nav repoint (and the a11y bug)

**Files:**
- Modify: `packages/temper-ui/src/lib/vault-url.ts:17`, `TierHome.svelte:149`, `HomeA11yList.svelte:92`, `ContextNavGroup.svelte`, `static/dev/atlas-fixtures.json`, `src/lib/graph/atlas/fixtures.test.ts`, `src/routes/dev/atlas/README.md`
- Create: `packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/graph/+page.server.ts` (temporarily, as a redirect — deleted in Task 12)

**Interfaces:**
- Consumes: `buildContextUrl` (Task 6).
- Produces: `contextGraphHref(ownerRef, slug)` → `/graph/<ownerRef>?context=<slug>`.

- [ ] **Step 1: Write the failing tests**

In `vault-url.test.ts`:

```ts
it('contextGraphHref points at the Atlas context door', () => {
	expect(contextGraphHref('@me', 'temper')).toBe('/graph/@me?context=temper');
});
```

In `fixtures.test.ts`, add `'contextPanorama'` and `'contextDrill'` to `EXPECTED_SCENARIOS`.

- [ ] **Step 2: Run and watch them fail**

Run: `bun run test --run vault-url fixtures`
Expected: FAIL on both.

- [ ] **Step 3: Repoint the URLs and fix the a11y bug**

`vault-url.ts`:

```ts
/** The Atlas context door. Both the left-nav "Graph" link and the Home build circle
 *  resolve here, so there is exactly one context-graph URL in the app. */
export function contextGraphHref(ownerRef: string, slug: string): string {
	return `/graph/${ownerRef}?context=${encodeURIComponent(slug)}`;
}
```

`TierHome.svelte:149` — replace `goto(`/vault/${ownerRef}/${slug}`)` with `goto(contextGraphHref(ownerRef, slug))`.

`HomeA11yList.svelte:92` — **this is a bug, not a cleanup.** It currently emits `href={`/vault/${c.owner_ref}`}`, dropping the context slug entirely, so the accessible mirror of the build circle never reaches the context. North-star Decision 9 requires the list be the *equivalent* of the field. Replace with `href={contextGraphHref(c.owner_ref, c.slug)}`.

Add a temporary redirect at the legacy route so bookmarks survive until Task 12:

```ts
import { redirect } from '@sveltejs/kit';
import { contextGraphHref } from '$lib/vault-url';
export const load = ({ params }) => {
	throw redirect(308, contextGraphHref(params.owner, params.context));
};
```

- [ ] **Step 4: Commit sanitized fixtures**

Capture `contextPanorama` (tier 0) and `contextDrill` (tier 1, `coreHome: 'context'`) per `src/routes/dev/atlas/README.md`, sanitize with `scripts/sanitize-atlas-fixtures.mjs`, and commit them into `static/dev/atlas-fixtures.json` — the **committed** bundle, never the gitignored `.local.json`.

- [ ] **Step 5: Verify in the harness, then commit**

```bash
bun run dev   # → localhost:5173/dev/atlas → contextPanorama, contextDrill
bun run test --run && bun run check
git add packages/temper-ui
git commit -m "feat(atlas): repoint context-graph nav to the door; fix HomeA11yList dropping the slug"
```

---

## Task 11: e2e — the door round-trip

**Files:**
- Create: `tests/e2e/tests/context_view_door_test.rs`

**Interfaces:** Consumes Task 5's endpoints.

- [ ] **Step 1: Write the failing test**

Model it on an existing file in `tests/e2e/tests/` (read one first). Assert that a seeded context's panorama returns its containers, that a stranger gets `404`, and that a container drill returns the container plus its members.

- [ ] **Step 2: Rebuild the CLI binary first**

`cargo nextest` does **not** rebuild a spawned `temper` binary, so a stale bin will silently test old code:

```bash
cargo build -p temper-cli --bin temper
```

- [ ] **Step 3: Run and watch it fail**

```bash
cargo make test-e2e
```
On macOS a fresh e2e binary can hang at nextest's `--list` step; if so run the single target with plain cargo: `cargo test --test context_view_door_test`.

- [ ] **Step 4: Make it pass, regenerate the e2e cache, commit**

```bash
cargo make prepare-e2e
cargo make test-e2e
cargo fmt --all && cargo make check
git add tests/e2e && git commit -m "test(e2e): context view door round-trip"
```

---

## Task 12: Retire the legacy Cytoscape surface

**Files:** the delete list in **File Structure** above.

> Do this **only after Tasks 1–11 are green and the new door renders in `/dev/atlas`.** This task deletes the fallback.
>
> **Do not drop any SQL function here.** `graph_subgraph_nodes` is still called by shipped code; dropping it in the release that stops calling it 500s any instance mid-deploy (temperkb.io and self-hosted are independent Vercel projects on their own cadence). The drop is Task 13's follow-up PR.

- [ ] **Step 1: Delete the UI**

Remove the legacy route directory, the five components, and the nine `src/lib/graph/*.ts` modules with their tests. **First** confirm nothing in `src/lib/graph/atlas/` imports them:

```bash
cd packages/temper-ui
grep -rn "from '\$lib/graph/\(elements\|derive\|styling\|layout\|tiers\|adjacency\|peek\|trail\|navigation\)'" src/lib/graph/atlas src/lib/components/graph/atlas
```
Expected: no output. (`src/lib/graph/atlas/trail.ts` is a *different* module — do not delete it.)

- [ ] **Step 2: Drop the cytoscape deps and verify nothing imports them**

```bash
grep -rn "cytoscape" src/ || echo "clean"
bun remove cytoscape cytoscape-fcose @types/cytoscape
bun run check && bun run test --run
```

- [ ] **Step 3: Delete the API + service + types + tests**

Remove `get_subgraph`, `SubgraphQuery`, `routes.rs:95`, the OpenAPI path/schema (update the count assertion), `aggregator_subgraph`, `AggregatorSubgraphParams`, `fetch_subgraph_nodes`, `fetch_subgraph_edges`, and `crates/temper-api/tests/graph_subgraph_test.rs`.

**Keep** `compute_excerpt` and `MAX_DEPTH` — `cogmap_neighborhood_slice`, `region_composition_slice`, and `cogmap_panorama` still use them. **Keep** `GraphEdgeRow`, `GraphNeighborRow`, `GraphTraversalRow`, `EdgeKind`, `Polarity`, `EdgeType` in `temper-workflow/src/types/graph.rs` — the CLI uses `GraphEdgeRow`. Delete only `SubgraphResponse`, `GraphNode`, `GraphEdge`, `is_aggregator`.

- [ ] **Step 4: Regenerate and verify**

```bash
cargo make generate-ts-types
cargo sqlx prepare --workspace -- --all-features && cargo make prepare-services && cargo make prepare-api
# prune orphaned .sqlx entries left by the deleted queries
cargo fmt --all && cargo make check && cargo make test-all
```

- [ ] **Step 5: Commit**

```bash
git add -A
git status --short   # verify the deletions AND the regenerated types are staged
git commit -m "refactor(atlas): retire the legacy cytoscape context graph"
```

> A `git add` with a non-matching pathspec can stage **nothing**. Always confirm with `git diff --cached --stat` before committing after a `git rm`.

---

## Task 13: File the SQL-drop follow-up

- [ ] **Step 1: Create the task**

The drop must not ride this PR (spec D9). Write the body to a temp file and pipe it — a heredoc into `temper resource create` hangs:

```bash
cat > /tmp/drop.md <<'MDEOF'
# Drop graph_subgraph_nodes + the dead team-graph trio

Beat E (PR TBD) removed every caller of `graph_subgraph_nodes`. Once that code is deployed
to every target, drop it, along with the confirmed-dead trio `team_viewable_by`,
`team_child_zones`, `team_descendants` (zero Rust/TS/test callers; only SQL callers are each
other; born in 20260703000002 for a `TeamZoneMark` that was never built).

Dropping the trio also retires `team_descendants`' `is_active` soft-delete gap — the SQL audit
prohibits reviving it as-is.

## Preconditions
- [ ] Beat E merged AND deployed to temperkb.io and every self-hosted target.
- [ ] `grep -rn "graph_subgraph_nodes" crates/ packages/` returns nothing outside migrations.

## Acceptance
- [ ] One additive migration, numbered above Beat E's `20260709000011`, dropping all four.
- [ ] `crates/temper-substrate/tests/graph_functions.rs` graph_subgraph_nodes cases removed.
- [ ] Migration is additive-only-on-main safe; never edits a shipped migration.
MDEOF
cat /tmp/drop.md | temper resource create --type task --title "Drop graph_subgraph_nodes + dead team-graph trio (post-Beat-E deploy)" --context @me/temper --mode build --effort small
```

- [ ] **Step 2: Push and open the PR**

```bash
git merge origin/main            # surface sibling drift before CI does
cargo make check && cargo make test-all
git push -u origin jct/atlas-beat-e
gh pr create --title "Atlas Beat E — context view + retire the legacy cytoscape graph" --body "..."
```

Never merge to `main` locally. `main` is unprotected and auto-deploys.

---

## Self-Review

**Spec coverage:** D1 → Tasks 6, 9, 10. D2 → Tasks 2, 8. D3 → Tasks 1, 2, 4. D4 → Tasks 2, 5. D5 → Task 7. D6, D7 → already shipped (`4e0e83d6`, `ad324b09`). D8 → Tasks 1, 3. D9 → Tasks 12, 13. D10 → Task 13. §4.5 error handling → Tasks 5 (404 deny-as-absence), 9 (composition→panorama redirect). §5 testing → Tasks 2 (deny direction + backfill invariance), 6, 7, 8 (pure TS), 10 (fixtures), 11 (e2e).

**Gap found and closed:** §5 asks for a `territoryWeight` unit test, including the *region with null salience but `member_count` > 0* case — a behaviour change introduced by `ad324b09` that is currently uncovered. It belonged to no task. Worse, writing the test revealed it is **impossible as the code stands**: `territoryWeight` is a local `const` inside `TierPanorama.svelte` and is not importable. Closed as **Task 8, Step 0**, which extracts and exports it from `labels.ts` before testing it. (A plan that had merely said "add a test" would have handed the implementer an impossible instruction.)

**Second gap:** `Territory` in `temper-core` derives `PartialEq` but not `Eq` (it holds `Option<f64>`), so `ContextPanorama` cannot derive `Eq` either — Task 1's `ContextPanorama` correctly derives only `PartialEq`, while `ResidualBucket`/`ResidualGroups`/`GroupKeyMeta` (no floats) derive both. Verified against `crates/temper-core/src/types/graph_territory.rs:22`.

**Placeholder scan:** one intentional marker remains — `todo_hydrate` in Task 4, Step 3, with an explicit instruction not to leave the name in the tree. The `stranger_token` in Task 5 Step 1 says "see fixtures"; the JWKS fixtures live in `tests/e2e/tests/fixtures/`, and `crates/temper-api/tests/common/` has the in-crate equivalent.

**Type consistency:** `member_count` is the column name in both `graph_context_containers` and `graph_context_residual_counts` (the residual one returns `member_count`, not `count`, so the Rust tuple destructuring matches) — the TS `ResidualBucket` field is `count`, mapped explicitly in `context_graph_service`. `CompositionTarget` is named identically in `graph-reads.ts` (TS) and `handlers/graph.rs` (Rust enum) though they are separate types. `coreHome` is `NodeHome`, matching the generated TS union `'context' | 'cogmap'`.
