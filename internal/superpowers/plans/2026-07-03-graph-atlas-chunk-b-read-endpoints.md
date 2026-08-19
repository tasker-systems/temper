# Graph Atlas — Chunk B: Read Endpoints + Wire Types — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the four Atlas read endpoints (R2 territory overview, R3 territory slice, R4 neighborhood slice, R5 element event-trail) plus their `ts-rs` wire types, each scoping through the R1 team-scope substrate that shipped in PR #250.

**Architecture:** Every read is a *service-direct* Rust fn (runtime sqlx, no `query!` macro, no `.sqlx` cache churn) that calls a new `LANGUAGE sql STABLE` function composed from the existing visibility substrate (`resources_visible_to`, `cogmap_readable_by_profile`, `edges_visible_to`, `resources_in_team_scope`, `team_ancestors`, `graph_traverse`). Handlers are thin (`AuthUser` → `ProfileId` → one service call → `Json`), registered in the `gated` router. Access is **deny-as-absence** (404, never 403). Coverage is the **e2e access tier** (two-tier: SQL-level + HTTP), matching R1 exactly.

**Tech Stack:** Rust (axum, sqlx runtime API, ts-rs), PostgreSQL 17/18 (version-portable SQL), TypeScript codegen via `cargo make generate-ts-types`.

## Global Constraints

- **Mirror R1 verbatim.** R1 (`GET /api/teams/{id}/graph-scope`) is the template. Its files: migration `migrations/20260703000002_team_graph_scope_reads.sql`; wire types `crates/temper-core/src/types/graph_scope.rs`; service `crates/temper-services/src/services/team_service.rs::graph_scope` (L493-565); handler `crates/temper-api/src/handlers/teams.rs::graph_scope` (L224-244); route `crates/temper-api/src/routes.rs` (L108-111, `gated` router); tests `tests/e2e/tests/team_graph_scope_sql_test.rs` + `tests/e2e/tests/team_graph_scope_e2e.rs`.
- **Purpose-built Atlas types; legacy graph types are frozen.** The Atlas reads use *new* wire types (`AtlasNode`, `AtlasEdge`, `AtlasSubgraph`, …) in **temper-core**. The legacy `GraphNode`/`GraphEdge`/`SubgraphResponse` in `crates/temper-workflow/src/types/graph.rs` and the legacy `/api/graph/subgraph` endpoint are **left completely untouched** — they serve the old UI route until Chunk D deletes both. Do not extend or reuse the legacy types; do not carry their `aggregator`/`session_count` fields (dead R11 encoding, see Design decisions).
- **Runtime sqlx, not macros.** Use `sqlx::query_scalar(...)`, `sqlx::query_as::<_, (T, …)>(...)` with `.bind(...)` and `.map(...)`. No `sqlx::query!`/`query_as!` macros in the new service code → **no `.sqlx` cache regeneration needed**. (If any macro sneaks in, run `cargo make prepare-services` / `prepare-e2e`.)
- **SQL functions:** `LANGUAGE sql STABLE`, args prefixed `p_profile`/`p_team`/`p_cogmap`/etc., `RETURNS TABLE(...)`, no `SET search_path`, additive over existing substrate. Version-portable (no PG18-only SQL) — runs on both Neon PG17 and local PG18.
- **Prefer CTE joins over `IN (SELECT …)`.** In the *new* SQL we author, express set-membership as a `JOIN` against a named CTE, not a correlated `IN (SELECT …)` subquery, wherever it's a filter over a set (e.g. join the scope/visible CTE rather than `WHERE id IN (SELECT … )`). Shipped functions we merely *call* (`graph_traverse`, `resources_visible_to`) stay as-is; the preference governs the SQL bodies we add.
- **Migrations are immutable once shipped.** Never edit `migrations/2026070300000{1,2}...` or any prior file (including `graph_traverse` in `20260624000002`). Each new SQL function goes in a **new** migration file. At execution time run `ls migrations/ | tail -3` and confirm your `YYYYMMDDNNNNNN` prefix is strictly greater than the latest (bump if a same-day collision exists — this bit PR #249).
- **Wire type derive stack** (copy verbatim onto every new wire struct/enum):
  ```rust
  #[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
  #[cfg_attr(feature = "typescript", ts(export, export_to = "<file>.ts"))]
  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  #[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
  #[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
  ```
- **Deny-as-absence:** a team/cogmap/region the profile can't read returns `ApiError::NotFound`, never `Forbidden`.
- **Commit all regenerated TS** (`packages/temper-ui/src/lib/types/generated/*.ts`) in the same commit as the Rust type change — codegen rides along even if the diff looks noisy.
- **No openapi.rs edits.** R1 precedent: teams handlers carry `#[utoipa::path]` annotations but are not wired into `ApiDoc`. Follow that — annotate, don't register.
- **Reads are service-direct.** Never route reads through `DbBackend`; only writes go through the backend trait.
- **Test modules gate:** every `#[cfg(test)]` block containing `#[sqlx::test]` must be `#[cfg(all(test, feature = "test-db"))]` — a bare `#[cfg(test)]` panics in the no-DB Unit Tests CI job.
- **doc_type is one optional, free-form property — never a gate or a default.** doc_type lives in `kb_properties` (`property_key='doc_type'`), and a resource (especially a cogmap-authored node) may not have it at all. In every new read, `LEFT JOIN` it and carry it as **`Option<String>`** (raw property value) on the wire — never `INNER JOIN` (which silently erases doc-type-less nodes), never `COALESCE(..., 'concept')`, never force it through the closed `DocType` enum. The join pattern (adapt from `migrations/20260626000003_graph_subgraph_nodes_by_id.sql:16-19`, but **LEFT**):
  ```sql
  WITH doc AS (
    SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
    FROM kb_properties p
    WHERE p.owner_table = 'kb_resources' AND p.property_key = 'doc_type' AND NOT p.is_folded
  )
  -- … LEFT JOIN doc ON doc.rid = r.id  → doc.dt is NULL-able, mapped to Option<String>
  ```

---

## Design decisions resolved here (spec open-questions + R4 reshape)

These were open in the spec (§Open questions) or surfaced in plan review. Each is pinned; each is tunable later in Chunk C.

1. **Default Tier-0 lens.** When a cogmap carries regions under multiple lenses, R2 defaults to the global **`telos-default`** lens (seeded, `cogmap_id IS NULL`, `migrations/20260624000003_canonical_seed.sql:68-94`), resolved by name. R2/R3 accept an optional `lens_id` override. Rationale: `telos-default` is the canonical balanced weight vector; picking one lens keeps territory sizing single-valued instead of summing across overlapping lens partitions.
2. **Orphan-node salience proxy.** No per-resource salience column exists (salience is region-level; `affinity` is per-member). For the sparsity fallback (cogmaps with no materialized region), R2 ranks orphan nodes by **visible edge-degree** (count of `edges_visible_to` edges touching the resource). Cheapest signal; tunable to reference-standing later.
3. **Cross-territory bridge aggregation.** Tier-0 bridges are **aggregated counts** of visible edges whose endpoints fall in different cogmap territories, grouped by `(source_territory, target_territory)`. Cheapest bounded payload; never scales with edge volume.
4. **R4 does not delete `/api/graph/subgraph`.** Its sole runtime consumer is the old UI route `+page.server.ts` (`packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/graph/+page.server.ts:13-16`), which Chunk D deletes. R4 *adds* the generalized team-scoped endpoint; the old endpoint (and its `GraphNode`/`SubgraphResponse` types + `graph_service::aggregator_subgraph`) stays fully intact until Chunk D removes it with its consumer. **Deliberate deviation** from the spec's "retire as part of R4" — retiring in Chunk B would 404 the still-live page for two chunks. The integration test `crates/temper-api/tests/graph_subgraph_test.rs` also stays green.
6. **R5 event-trail keying (emitter investigation).** Grounded against the event writer (`_event_append`) and projectors, not guessed: (a) **`correlation_id` never groups a lifecycle** — no mutation passes it, so every event is its own root; the reliable edge key is the stable **`payload->>'edge_id'`** embedded in every `relationship_*` payload. (b) The **edge trail gates via `anchor_readable_by_profile(home_anchor_*)`, not `edges_visible_to`** — the latter filters `NOT is_folded`, which would erase a folded edge's own trail. (c) The **node trail has no single key** — it's a UNION of three grounded shapes (`payload->>'resource_id'`; `payload->'owner'->>'id'` guarded by `owner.table='kb_resources'`; block events carrying only `block_id`, joined through `kb_content_blocks`), gated once via `resources_visible_to`. (d) **Confidence band is `metadata->>'confidence'`** (bare lowercase string from `AgentAuthorship`), not `confidence_band`. (e) **Order by `e.id`** (UUIDv7) — `occurred_at` is transaction-start time and ties co-transaction events. R5 ships **both** node and edge trails: the node union is fully specified now, so it's a defined read, not a speculative one.
7. **R4 is a purpose-built Atlas read; the legacy node model is not reused (plan review).** The legacy subgraph node (`GraphNode`) hard-codes `doc_type`-mandatory seeding + node projection (INNER JOINs that silently drop doc-type-less nodes), an `aggregator: bool` classification whose own doc-comment ties it to the retired "R11 visual distinction" (larger/italic/radial-wash — the old dark-editorial aesthetic Atlas replaces), and a `session_count` that's hard-coded `0`. None of that belongs in Atlas (encoding grammar: home=fill/outline, doc-type=hue, edge-kind=line, polarity=arrowhead, weight=thickness). So R4 gets new types (`AtlasNode`/`AtlasEdge`/`AtlasSubgraph`) with only Atlas-grammar fields, **`doc_type: Option<String>`** (free-form), and a nullable `AtlasEdge.label`/explicit `weight` (the real `kb_edges.label` is nullable and `weight` exists — the legacy `GraphEdge` dropped weight and non-null'd label). R4 **requires explicit seeds** (it's the focus+depth "neighborhood" tier; the no-focus panorama is R2's job) — the concept/goal/decision default-seed set is dropped entirely. The **edge-kind filter constrains the traversal** (induced subgraph), so it lives inside a new `graph_traverse_scoped` (the shipped `graph_traverse` is directional, visibility-only-scoped, edge-kind-unaware, and drops `weight` — all reasons to author a team-scoped variant rather than reuse it).

---

## File structure

**New wire-type files (temper-core):**
- `crates/temper-core/src/types/graph_atlas.rs` — R4 + shared: `NodeHome`, `AtlasNode`, `AtlasEdge`, `AtlasSubgraph`, `SliceRequest`.
- `crates/temper-core/src/types/graph_territory.rs` — R2/R3: `Territory`, `TerritoryKind`, `TerritoryOverview`, `Bridge`, `OrphanNode`, `RegionMember`, `Component`, `TerritorySlice`.
- `crates/temper-core/src/types/element_trail.rs` — R5: `ElementKind`, `ElementEvent`, `EventTrail`.

**Modified wire-type files:**
- `crates/temper-core/src/types/relationship_events.rs` — add the derive stack to the six `Relationship*` payload structs + `TargetEndpoint` (R5).
- `crates/temper-core/src/types/mod.rs` — register the three new modules + re-exports.

*(Untouched: `crates/temper-workflow/src/types/graph.rs`, `graph_service::aggregator_subgraph`, `/api/graph/subgraph`.)*

**New migrations (verify timestamps at execution — see Global Constraints):**
- `migrations/20260703000003_graph_atlas_slice.sql` (R4: `graph_traverse_scoped` + `graph_atlas_nodes`)
- `migrations/20260703000004_graph_territory_overview.sql` (R2)
- `migrations/20260703000005_graph_territory_slice.sql` (R3)
- `migrations/20260703000006_element_event_trail.sql` (R5)

**New/modified service code (temper-services):**
- `crates/temper-services/src/services/graph_service.rs` — add `neighborhood_slice` (R4), `territory_overview` (R2), `territory_slice` (R3). (File already hosts `aggregator_subgraph` — leave it.)
- `crates/temper-services/src/services/event_service.rs` — add `element_trail` (R5).

**New/modified handlers + routes (temper-api):**
- `crates/temper-api/src/handlers/graph.rs` — add `neighborhood_slice`, `territory_overview`, `territory_slice` handlers. (Leave `get_subgraph`.)
- `crates/temper-api/src/handlers/events.rs` — add `element_trail` handler.
- `crates/temper-api/src/routes.rs` — register four new routes in the `gated` router.

**New tests (tests/e2e/tests/):** one SQL-level file + one HTTP e2e file per read (8 files), mirroring R1's split.

**Generated TS (commit, never hand-edit):** `graph_atlas.ts` (new), `graph_territory.ts` (new), `element_trail.ts` (new), `relationship_events.ts` (new).

---

## Task ordering & independence

Task 1 (define the Atlas types) is foundational — R4 consumes them directly and R2's orphan nodes share `NodeHome`/the doc_type convention — so it lands first. Tasks 2–5 are logically independent (each = migration + wire types + service fn + handler + route + e2e), but all append to shared files (`routes.rs`, `graph_service.rs`, `mod.rs`), so execute them **sequentially** to keep merges clean and reviews scoped.

Recommended order: **1 → 2 (R4) → 3 (R2) → 4 (R3) → 5 (R5) → 6 (final gate)**.

---

### Task 1: Atlas graph wire types (temper-core)

**Files:**
- Create: `crates/temper-core/src/types/graph_atlas.rs`
- Modify: `crates/temper-core/src/types/mod.rs` (register module + re-exports)
- Test: inline `#[cfg(test)]` round-trip in `graph_atlas.rs` + regenerate TS

**Interfaces:**
- Consumes: `EdgeKind`, `Polarity` (already in temper-core — the neutral primitives).
- Produces: `NodeHome`, `AtlasNode`, `AtlasEdge`, `AtlasSubgraph`, `SliceRequest`. Consumed by R4 (Task 2); `NodeHome` + the `doc_type: Option<String>` convention referenced by R2/R3.

- [ ] **Step 1: Create `crates/temper-core/src/types/graph_atlas.rs`:**

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::graph::{EdgeKind, Polarity}; // confirm the actual module path of the neutral primitives

/// Which home a node is bound to — drives the Atlas fill-vs-outline encoding
/// (cogmap-homed = filled chip, context-homed = outlined chip). Dual-homed
/// resources resolve to `Cogmap` (the authored side wins).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_atlas.ts"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum NodeHome {
    Context,
    Cogmap,
}

/// A node on the Atlas canvas. `doc_type` is the raw, optional `kb_properties`
/// value (a node may carry none); the UI maps it to a hue with a fallback.
/// `degree` is the node's total visible edge count (sizing hint). `salience`
/// is region-derived and may be `None` in the neighborhood tier.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_atlas.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AtlasNode {
    pub id: Uuid,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    pub home: NodeHome,
    pub degree: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salience: Option<f64>,
}

/// A directed edge on the Atlas canvas. `label` is nullable (matches
/// `kb_edges.label`), `weight` drives stroke thickness in the Atlas grammar.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_atlas.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AtlasEdge {
    pub source: Uuid,
    pub target: Uuid,
    pub edge_kind: EdgeKind,
    pub polarity: Polarity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub weight: f64,
}

/// The response body for an R4 neighborhood slice.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_atlas.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AtlasSubgraph {
    pub nodes: Vec<AtlasNode>,
    pub edges: Vec<AtlasEdge>,
}

/// R4 request: focus seeds (required, non-empty), BFS depth, and an optional
/// edge-kind filter that constrains the *traversal* (induced subgraph).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_atlas.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct SliceRequest {
    /// Focus resource ids. Must be non-empty — R4 is always drilled in around a focus.
    pub seeds: Vec<Uuid>,
    /// BFS depth from the seed set. Clamped server-side to MAX_DEPTH (10).
    pub depth: u32,
    /// Edge-kind filter constraining the walk; empty = all kinds.
    #[serde(default)]
    pub edge_kinds: Vec<EdgeKind>,
}
```

> Verify the import path for `EdgeKind`/`Polarity` (Step: `grep -rn "pub enum EdgeKind" crates/temper-core/src`). Adjust the `use` to the real module. If those enums lack the `mcp`/`web-api` derives needed to compose here, that's fine — `AtlasEdge` only needs them to be `Serialize`/`ts_rs::TS`, which they already are (the legacy `GraphEdge` uses them).

- [ ] **Step 2: Register the module** in `crates/temper-core/src/types/mod.rs` — add `pub mod graph_atlas;` (alphabetized) and `pub use graph_atlas::{AtlasEdge, AtlasNode, AtlasSubgraph, NodeHome, SliceRequest};`.

- [ ] **Step 3: Write a round-trip test** in the `#[cfg(test)]` mod of `graph_atlas.rs`:

```rust
#[test]
fn atlas_node_doc_type_is_optional() {
    let n = AtlasNode {
        id: uuid::Uuid::nil(), title: "t".into(), doc_type: None,
        home: NodeHome::Cogmap, degree: 3, salience: Some(0.8),
    };
    let json = serde_json::to_string(&n).unwrap();
    let back: AtlasNode = serde_json::from_str(&json).unwrap();
    assert_eq!(n, back);
    assert!(json.contains("\"home\":\"cogmap\""));
    assert!(!json.contains("doc_type")); // None is skipped
}
```

- [ ] **Step 4: Run it, expect PASS:**

Run: `cargo nextest run -p temper-core atlas_node_doc_type_is_optional`
Expected: PASS.

- [ ] **Step 5: Regenerate TS + verify:**

Run: `cargo make generate-ts-types`
Expected: `packages/temper-ui/src/lib/types/generated/graph_atlas.ts` exists with `NodeHome`, `AtlasNode` (`doc_type?: string`, `salience?: number`), `AtlasEdge` (`label?: string`, `weight: number`), `AtlasSubgraph`, `SliceRequest`.

- [ ] **Step 6: Full gate:**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 7: Commit:**

```bash
git add crates/temper-core/src/types/graph_atlas.rs crates/temper-core/src/types/mod.rs \
        packages/temper-ui/src/lib/types/generated/graph_atlas.ts
git commit -m "feat(graph): Atlas wire types (AtlasNode/Edge/Subgraph, SliceRequest) — new, legacy graph types untouched"
```

---

### Task 2: R4 — team-scoped parameterized neighborhood slice

**Files:**
- Create: `migrations/20260703000003_graph_atlas_slice.sql`
- Modify: `crates/temper-services/src/services/graph_service.rs` (add `neighborhood_slice` + helpers)
- Modify: `crates/temper-api/src/handlers/graph.rs` (add handler)
- Modify: `crates/temper-api/src/routes.rs` (register route)
- Test: `tests/e2e/tests/graph_atlas_slice_sql_test.rs`, `tests/e2e/tests/graph_atlas_slice_e2e.rs`

**Interfaces:**
- Consumes: `AtlasNode`/`AtlasEdge`/`AtlasSubgraph`/`SliceRequest`/`NodeHome` (Task 1); `resources_in_team_scope`, `resources_visible_to`, `edges_visible_to` (substrate).
- Produces: `graph_service::neighborhood_slice(pool, profile_id, team_id, req) -> ApiResult<AtlasSubgraph>`; `POST /api/teams/{id}/graph/slice`.

- [ ] **Step 1: Read the reference SQL you're generalizing** — `graph_traverse` (`migrations/20260624000002_canonical_functions.sql`, the `CREATE OR REPLACE FUNCTION graph_traverse` block). Note its shape: `WITH RECURSIVE visible AS (...)`, a `walk` CTE seeded from `source_id = ANY(p_seed_ids)`, recursing `e.source_id = w.target_id`, returning walked edges. Your new function keeps this shape but (a) swaps `visible` for a team-scope CTE, (b) adds an edge-kind filter in both the seed and recursive arms, (c) returns `weight`, (d) uses CTE joins not `IN (SELECT …)`.

- [ ] **Step 2: Write the migration** `migrations/20260703000003_graph_atlas_slice.sql`. Two functions: the scoped traversal (returns edges) and the node projector.

```sql
-- R4 Atlas neighborhood slice: team-scoped, edge-kind-filtered traversal + node projection.
-- Composes resources_in_team_scope (team clamp) with a graph_traverse-shaped recursive walk.

-- Scoped, edge-kind-filtered directed walk. p_edge_kinds empty/NULL => all kinds.
CREATE FUNCTION graph_traverse_scoped(
    p_profile     uuid,
    p_team        uuid,
    p_seed_ids    uuid[],
    p_depth       int,
    p_edge_kinds  edge_kind[]
) RETURNS TABLE(
    source_id uuid, target_id uuid, edge_kind edge_kind,
    polarity edge_polarity, label text, weight double precision, depth int
) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE scope AS (
        SELECT resource_id AS id FROM resources_in_team_scope(p_profile, p_team)
    ),
    walk AS (
        SELECT e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, e.weight, 1 AS depth
        FROM kb_edges e
        JOIN scope ss ON ss.id = e.source_id
        JOIN scope st ON st.id = e.target_id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND NOT e.is_folded
          AND e.source_id = ANY(p_seed_ids)
          AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
               OR e.edge_kind = ANY(p_edge_kinds))
        UNION
        SELECT e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, e.weight, w.depth + 1
        FROM kb_edges e
        JOIN walk w ON e.source_id = w.target_id
        JOIN scope st ON st.id = e.target_id
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND NOT e.is_folded
          AND w.depth < LEAST(p_depth, 10)
          AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
               OR e.edge_kind = ANY(p_edge_kinds))
    )
    SELECT source_id, target_id, edge_kind, polarity, label, weight, depth FROM walk;
$$;

-- Project Atlas node attributes for a set of ids, clamped to team scope.
-- doc_type is LEFT-joined (nullable). home = cogmap if any cogmap home exists, else context.
CREATE FUNCTION graph_atlas_nodes(
    p_profile uuid, p_team uuid, p_ids uuid[]
) RETURNS TABLE(id uuid, title text, doc_type text, home text, degree int)
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
           COALESCE(deg.degree, 0) AS degree
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
        WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
          AND (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true;
$$;
```

> Confirm the pg enum type names are `edge_kind` / `edge_polarity` (they are — see `kb_edges` column types). Validate the recursive CTE compiles against the DB in Step 6. The `bool_or` home LATERAL yields `context` when a resource has no home row at all (shouldn't happen for an in-scope resource, but is a safe default).

- [ ] **Step 3: Add the service fn** `neighborhood_slice` in `crates/temper-services/src/services/graph_service.rs` (runtime sqlx, mirror `graph_scope`'s style):

```rust
use temper_core::types::graph_atlas::{AtlasEdge, AtlasNode, AtlasSubgraph, NodeHome, SliceRequest};

/// R4 — team-scoped parameterized neighborhood slice. Service-direct read.
pub async fn neighborhood_slice(
    pool: &sqlx::PgPool,
    profile_id: ProfileId,
    team_id: uuid::Uuid,
    req: SliceRequest,
) -> ApiResult<AtlasSubgraph> {
    if req.seeds.is_empty() {
        return Err(ApiError::BadRequest("seeds must be non-empty".into()));
    }
    // Deny-as-absence: profile must read the team (member of it or a descendant).
    let viewable: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM team_descendants($1) d \
         JOIN kb_team_members tm ON tm.team_id = d.team_id AND tm.profile_id = $2)",
    )
    .bind(team_id)
    .bind(profile_id.as_uuid())
    .fetch_one(pool)
    .await?;
    if !viewable {
        return Err(ApiError::NotFound);
    }

    let depth = req.depth.min(10) as i32;
    // EdgeKind → its pg enum text. Confirm the Display/serde repr matches the pg enum labels.
    let edge_kinds: Vec<String> = req.edge_kinds.iter().map(|k| k.to_string()).collect();

    // Walk: returns the edges of the induced subgraph.
    let walked = sqlx::query_as::<_, (
        uuid::Uuid, uuid::Uuid, String, String, Option<String>, f64, i32,
    )>(
        "SELECT source_id, target_id, edge_kind::text, polarity::text, label, weight, depth \
         FROM graph_traverse_scoped($1, $2, $3, $4, $5::edge_kind[])",
    )
    .bind(profile_id.as_uuid())
    .bind(team_id)
    .bind(&req.seeds)
    .bind(depth)
    .bind(&edge_kinds)
    .fetch_all(pool)
    .await?;

    let edges: Vec<AtlasEdge> = walked
        .iter()
        .map(|(source, target, ek, pol, label, weight, _depth)| AtlasEdge {
            source: *source,
            target: *target,
            edge_kind: parse_edge_kind(ek), // reuse the existing str→EdgeKind path in this module
            polarity: parse_polarity(pol),
            label: label.clone(),
            weight: *weight,
        })
        .collect();

    // Node id set = seeds ∪ all walked endpoints.
    let mut node_ids: Vec<uuid::Uuid> = req.seeds.clone();
    for (s, t, ..) in &walked {
        node_ids.push(*s);
        node_ids.push(*t);
    }

    let nodes: Vec<AtlasNode> = sqlx::query_as::<_, (uuid::Uuid, String, Option<String>, String, i32)>(
        "SELECT id, title, doc_type, home, degree FROM graph_atlas_nodes($1, $2, $3)",
    )
    .bind(profile_id.as_uuid())
    .bind(team_id)
    .bind(&node_ids)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(id, title, doc_type, home, degree)| AtlasNode {
        id,
        title,
        doc_type,
        home: if home == "cogmap" { NodeHome::Cogmap } else { NodeHome::Context },
        degree,
        salience: None, // neighborhood-tier salience deferred (no per-node source yet)
    })
    .collect();

    Ok(AtlasSubgraph { nodes, edges })
}
```

> `parse_edge_kind`/`parse_polarity`: use whatever str→enum conversion already exists (the legacy `fetch_subgraph_edges` maps DB enum text → `EdgeKind`/`Polarity` — read it and reuse the same mechanism, whether that's `FromStr`, a `From<String>`, or sqlx's native enum decode). Prefer decoding the pg enum **natively** via sqlx `#[derive(sqlx::Type)]` if `EdgeKind` already has it — then drop the `::text` casts and bind `req.edge_kinds` directly. Check `crates/temper-services/src/services/graph_service.rs::fetch_subgraph_edges` (L207-235) for the established pattern and match it exactly.

- [ ] **Step 4: Add the handler** in `crates/temper-api/src/handlers/graph.rs`:

```rust
/// POST /api/teams/{id}/graph/slice — R4 team-scoped parameterized neighborhood slice.
#[utoipa::path(
    post,
    path = "/api/teams/{id}/graph/slice",
    tag = "Graph",
    params(("id" = Uuid, Path, description = "Team id to scope the slice to")),
    request_body = SliceRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Neighborhood slice", body = AtlasSubgraph),
        (status = 400, description = "Empty seed set"),
        (status = 404, description = "Team not viewable by this profile")
    )
)]
pub async fn neighborhood_slice(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Json(req): Json<SliceRequest>,
) -> ApiResult<Json<AtlasSubgraph>> {
    graph_service::neighborhood_slice(&state.pool, ProfileId::from(auth.0.profile.id), team_id, req)
        .await
        .map(Json)
}
```

Imports: `use temper_core::types::graph_atlas::{AtlasSubgraph, SliceRequest};`.

- [ ] **Step 5: Register the route** in `crates/temper-api/src/routes.rs` in the `gated` router (near the existing `/api/graph/subgraph` at L75), adding `post` to the axum routing import:

```rust
.route("/api/teams/{id}/graph/slice", post(handlers::graph::neighborhood_slice))
```

- [ ] **Step 6: SQL-level test** `tests/e2e/tests/graph_atlas_slice_sql_test.rs`. Gate `#![cfg(feature = "test-db")]`, `#[sqlx::test(migrator = "temper_api::MIGRATOR")]`. Insert a team + a resource-with-no-doc_type + edges of two kinds; assert: `graph_traverse_scoped` respects depth and the `p_edge_kinds` filter (an edge of an excluded kind is not walked); `graph_atlas_nodes` returns the doc-type-less node with `doc_type IS NULL` (proving no INNER-JOIN erasure) and correct `home`/`degree`; a resource outside `resources_in_team_scope` is excluded.

- [ ] **Step 7: HTTP e2e test** `tests/e2e/tests/graph_atlas_slice_e2e.rs`: member `POST`s a `SliceRequest` → 200 `AtlasSubgraph`; empty `seeds` → 400; outsider → 404. Assert an `AtlasNode` with no doc_type serializes without the `doc_type` key.

- [ ] **Step 8: Run tests:**

Run: `cargo make test-e2e`
Expected: PASS, suite green.

- [ ] **Step 9: Regenerate TS + gate:**

Run: `cargo make generate-ts-types && cargo make check`
Expected: `graph_atlas.ts` unchanged from Task 1 (no new types here); check clean.

- [ ] **Step 10: Confirm the legacy endpoint is untouched:**

Run: `cargo nextest run -p temper-api --features test-db --test graph_subgraph_test`
Expected: PASS — `/api/graph/subgraph` still serving.

- [ ] **Step 11: Commit:**

```bash
git add migrations/20260703000003_graph_atlas_slice.sql \
        crates/temper-services/src/services/graph_service.rs \
        crates/temper-api/src/handlers/graph.rs crates/temper-api/src/routes.rs \
        tests/e2e/tests/graph_atlas_slice_sql_test.rs tests/e2e/tests/graph_atlas_slice_e2e.rs
git commit -m "feat(graph): R4 team-scoped neighborhood slice (scoped edge-kind-filtered traversal, optional doc_type)"
```

---

### Task 3: R2 — territory overview (Tier 0)

**Files:**
- Create: `migrations/20260703000004_graph_territory_overview.sql`
- Create: `crates/temper-core/src/types/graph_territory.rs`
- Modify: `crates/temper-core/src/types/mod.rs` (register module + re-exports)
- Modify: `crates/temper-services/src/services/graph_service.rs` (add `territory_overview`)
- Modify: `crates/temper-api/src/handlers/graph.rs` (add handler)
- Modify: `crates/temper-api/src/routes.rs` (register route)
- Test: `tests/e2e/tests/graph_territory_overview_sql_test.rs`, `tests/e2e/tests/graph_territory_overview_e2e.rs`

**Interfaces:**
- Consumes: `NodeHome` convention, `resources_in_team_scope`, `cogmap_readable_by_profile`, `edges_visible_to`, `kb_cogmap_regions`, `kb_team_cogmaps`, `kb_contexts`.
- Produces: `Territory`, `TerritoryKind`, `TerritoryOverview`, `Bridge`, `OrphanNode`; `graph_service::territory_overview(pool, profile_id, team_id, lens_id: Option<Uuid>) -> ApiResult<TerritoryOverview>`; `GET /api/teams/{id}/graph/territories?lens_id=`.

- [ ] **Step 1: Create `crates/temper-core/src/types/graph_territory.rs`:**

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A territory on the Atlas panorama: a region, a context, or a cogmap.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_territory.ts"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerritoryKind {
    Region,
    Context,
    Cogmap,
}

/// A tinted, sized territory (Tier-0 aggregate). `salience` sizes regions;
/// `member_count` sizes contexts/cogmaps.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_territory.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct Territory {
    pub id: Uuid,
    pub kind: TerritoryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub member_count: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salience: Option<f64>,
    /// Cogmap/context this territory belongs to (for drill-in addressing).
    pub anchor_id: Uuid,
}

/// An aggregated cross-territory bridge (Tier-0).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_territory.ts"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct Bridge {
    pub source_territory: Uuid,
    pub target_territory: Uuid,
    pub edge_count: i32,
}

/// A high-degree standalone node surfaced where its cogmap home has no region
/// (sparsity rule). `doc_type` is optional/free-form.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_territory.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct OrphanNode {
    pub id: Uuid,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    pub degree: i32,
    pub anchor_id: Uuid,
}

/// The whole Tier-0 panorama for a team scope.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_territory.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct TerritoryOverview {
    pub territories: Vec<Territory>,
    pub orphan_nodes: Vec<OrphanNode>,
    pub bridges: Vec<Bridge>,
}
```

- [ ] **Step 2: Register the module** in `mod.rs` — `pub mod graph_territory;` and `pub use graph_territory::{Bridge, OrphanNode, Territory, TerritoryKind, TerritoryOverview};`.

- [ ] **Step 3: Write the migration** `migrations/20260703000004_graph_territory_overview.sql` (join-based; `doc_type` LEFT-joined and nullable; default lens resolved in the service):

```sql
-- R2 territory overview: region + context territories, orphan salient nodes
-- (sparsity fallback = edge-degree), and aggregated cross-territory bridges.

CREATE FUNCTION graph_region_territories(
    p_profile uuid, p_team uuid, p_lens uuid
) RETURNS TABLE(region_id uuid, cogmap_id uuid, label text, member_count int, salience double precision)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.cogmap_id, reg.label, reg.member_count, reg.salience
    FROM kb_cogmap_regions reg
    JOIN kb_team_cogmaps tc ON tc.cogmap_id = reg.cogmap_id
    JOIN team_ancestors(p_team) a ON a.team_id = tc.team_id
    WHERE NOT reg.is_folded
      AND reg.lens_id = p_lens
      AND cogmap_readable_by_profile(p_profile, reg.cogmap_id);
$$;

CREATE FUNCTION graph_context_territories(
    p_profile uuid, p_team uuid
) RETURNS TABLE(context_id uuid, label text, member_count int) LANGUAGE sql STABLE AS $$
    WITH scope AS (SELECT resource_id FROM resources_in_team_scope(p_profile, p_team)),
    homed AS (
        SELECT h.anchor_id AS context_id, h.resource_id
        FROM kb_resource_homes h
        JOIN scope s ON s.resource_id = h.resource_id
        WHERE h.anchor_table = 'kb_contexts'
    )
    SELECT c.id, c.name, count(homed.resource_id)::int
    FROM homed
    JOIN kb_contexts c ON c.id = homed.context_id
    GROUP BY c.id, c.name;
$$;

-- Orphan salient nodes: in-scope resources whose cogmap home has NO live region,
-- ranked by visible edge-degree. doc_type LEFT-joined (nullable). Bounded in Rust.
CREATE FUNCTION graph_orphan_salient_nodes(
    p_profile uuid, p_team uuid
) RETURNS TABLE(id uuid, title text, doc_type text, degree int, anchor_id uuid)
LANGUAGE sql STABLE AS $$
    WITH scope AS (SELECT resource_id FROM resources_in_team_scope(p_profile, p_team)),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table='kb_resources' AND p.property_key='doc_type' AND NOT p.is_folded
    ),
    cogmap_homed AS (
        SELECT h.resource_id, h.anchor_id AS cogmap_id
        FROM kb_resource_homes h
        JOIN scope s ON s.resource_id = h.resource_id
        WHERE h.anchor_table = 'kb_cogmaps'
    ),
    region_maps AS (
        SELECT DISTINCT cogmap_id FROM kb_cogmap_regions WHERE NOT is_folded
    )
    SELECT r.id, r.title, d.dt AS doc_type,
           deg.degree, ch.cogmap_id
    FROM cogmap_homed ch
    LEFT JOIN region_maps rm ON rm.cogmap_id = ch.cogmap_id
    JOIN kb_resources r ON r.id = ch.resource_id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN LATERAL (
        SELECT count(*)::int AS degree
        FROM kb_edges e
        JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
        WHERE (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true
    WHERE rm.cogmap_id IS NULL  -- home cogmap has no materialized region
    ORDER BY deg.degree DESC;
$$;

-- Aggregated cross-territory bridges: visible edges whose endpoints' cogmap homes differ.
CREATE FUNCTION graph_territory_bridges(
    p_profile uuid, p_team uuid
) RETURNS TABLE(source_territory uuid, target_territory uuid, edge_count int)
LANGUAGE sql STABLE AS $$
    WITH scope AS (SELECT resource_id FROM resources_in_team_scope(p_profile, p_team)),
    homed AS (
        SELECT h.resource_id, h.anchor_id AS territory
        FROM kb_resource_homes h
        JOIN scope s ON s.resource_id = h.resource_id
        WHERE h.anchor_table = 'kb_cogmaps'
    )
    SELECT LEAST(sh.territory, th.territory), GREATEST(sh.territory, th.territory), count(*)::int
    FROM kb_edges e
    JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
    JOIN homed sh ON sh.resource_id = e.source_id
    JOIN homed th ON th.resource_id = e.target_id
    WHERE NOT e.is_folded AND sh.territory <> th.territory
    GROUP BY LEAST(sh.territory, th.territory), GREATEST(sh.territory, th.territory);
$$;
```

> Confirm `kb_contexts` has a human-label column named `name` (`grep -n "CREATE TABLE kb_contexts" -A 15 migrations/20260624000001_canonical_schema.sql`); if it's `slug`/`title`, adjust. Cogmap territories (a whole cogmap as a territory) are folded into region+orphan coverage for v1; a distinct `TerritoryKind::Cogmap` row is a Chunk-C refinement.

- [ ] **Step 4: Add the service fn** `territory_overview` in `graph_service.rs`:

```rust
use temper_core::types::graph_territory::{Bridge, OrphanNode, Territory, TerritoryKind, TerritoryOverview};

/// R2 — Tier-0 territory overview for a team scope. Service-direct read.
pub async fn territory_overview(
    pool: &sqlx::PgPool,
    profile_id: ProfileId,
    team_id: uuid::Uuid,
    lens_id: Option<uuid::Uuid>,
) -> ApiResult<TerritoryOverview> {
    let viewable: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM team_descendants($1) d \
         JOIN kb_team_members tm ON tm.team_id = d.team_id AND tm.profile_id = $2)",
    ).bind(team_id).bind(profile_id.as_uuid()).fetch_one(pool).await?;
    if !viewable { return Err(ApiError::NotFound); }

    // Default lens = the global 'telos-default' (cogmap_id IS NULL).
    let lens: uuid::Uuid = match lens_id {
        Some(l) => l,
        None => sqlx::query_scalar(
            "SELECT id FROM kb_cogmap_lenses WHERE name = 'telos-default' AND cogmap_id IS NULL LIMIT 1",
        ).fetch_one(pool).await?,
    };

    let mut territories: Vec<Territory> = Vec::new();

    let regions = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, Option<String>, i32, f64)>(
        "SELECT region_id, cogmap_id, label, member_count, salience FROM graph_region_territories($1, $2, $3)",
    ).bind(profile_id.as_uuid()).bind(team_id).bind(lens).fetch_all(pool).await?;
    for (region_id, cogmap_id, label, member_count, salience) in regions {
        territories.push(Territory {
            id: region_id, kind: TerritoryKind::Region, label,
            member_count, salience: Some(salience), anchor_id: cogmap_id,
        });
    }

    let contexts = sqlx::query_as::<_, (uuid::Uuid, String, i32)>(
        "SELECT context_id, label, member_count FROM graph_context_territories($1, $2)",
    ).bind(profile_id.as_uuid()).bind(team_id).fetch_all(pool).await?;
    for (context_id, label, member_count) in contexts {
        territories.push(Territory {
            id: context_id, kind: TerritoryKind::Context, label: Some(label),
            member_count, salience: None, anchor_id: context_id,
        });
    }

    const ORPHAN_LIMIT: usize = 50;
    let orphan_nodes: Vec<OrphanNode> = sqlx::query_as::<_, (uuid::Uuid, String, Option<String>, i32, uuid::Uuid)>(
        "SELECT id, title, doc_type, degree, anchor_id FROM graph_orphan_salient_nodes($1, $2)",
    ).bind(profile_id.as_uuid()).bind(team_id).fetch_all(pool).await?
        .into_iter().take(ORPHAN_LIMIT)
        .map(|(id, title, doc_type, degree, anchor_id)| OrphanNode { id, title, doc_type, degree, anchor_id })
        .collect();

    let bridges: Vec<Bridge> = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, i32)>(
        "SELECT source_territory, target_territory, edge_count FROM graph_territory_bridges($1, $2)",
    ).bind(profile_id.as_uuid()).bind(team_id).fetch_all(pool).await?
        .into_iter()
        .map(|(source_territory, target_territory, edge_count)| Bridge { source_territory, target_territory, edge_count })
        .collect();

    Ok(TerritoryOverview { territories, orphan_nodes, bridges })
}
```

- [ ] **Step 5: Add the handler** in `crates/temper-api/src/handlers/graph.rs`:

```rust
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct TerritoryQuery {
    /// Optional lens override; defaults to the global `telos-default` lens.
    pub lens_id: Option<Uuid>,
}

/// GET /api/teams/{id}/graph/territories — R2 Tier-0 panorama.
#[utoipa::path(
    get,
    path = "/api/teams/{id}/graph/territories",
    tag = "Graph",
    params(("id" = Uuid, Path, description = "Team id"), TerritoryQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Territory overview", body = TerritoryOverview),
        (status = 404, description = "Team not viewable by this profile")
    )
)]
pub async fn territory_overview(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
    Query(q): Query<TerritoryQuery>,
) -> ApiResult<Json<TerritoryOverview>> {
    graph_service::territory_overview(&state.pool, ProfileId::from(auth.0.profile.id), team_id, q.lens_id)
        .await
        .map(Json)
}
```

Confirm `axum::extract::Query` is imported.

- [ ] **Step 6: Register the route** in `routes.rs` (`gated`):

```rust
.route("/api/teams/{id}/graph/territories", get(handlers::graph::territory_overview))
```

- [ ] **Step 7: SQL-level test** `tests/e2e/tests/graph_territory_overview_sql_test.rs`: build a team + cogmap with one materialized region (insert a `kb_cogmap_regions` row under `telos-default`) and one region-less cogmap with a high-degree, **doc-type-less** resource; assert `graph_region_territories` returns the region, `graph_orphan_salient_nodes` returns the region-less resource with `doc_type IS NULL` ordered by degree, and `graph_territory_bridges` counts a cross-cogmap edge once.

- [ ] **Step 8: HTTP e2e test** `tests/e2e/tests/graph_territory_overview_e2e.rs`: member GET → 200 with ≥1 territory; outsider → 404; default-lens path works with no `lens_id`.

- [ ] **Step 9: Run tests:** `cargo make test-e2e` → PASS.

- [ ] **Step 10: Regenerate TS + gate:** `cargo make generate-ts-types && cargo make check` → `graph_territory.ts` generated; clean.

- [ ] **Step 11: Commit:**

```bash
git add migrations/20260703000004_graph_territory_overview.sql \
        crates/temper-core/src/types/graph_territory.rs crates/temper-core/src/types/mod.rs \
        crates/temper-services/src/services/graph_service.rs \
        crates/temper-api/src/handlers/graph.rs crates/temper-api/src/routes.rs \
        tests/e2e/tests/graph_territory_overview_sql_test.rs tests/e2e/tests/graph_territory_overview_e2e.rs \
        packages/temper-ui/src/lib/types/generated/graph_territory.ts
git commit -m "feat(graph): R2 Tier-0 territory overview (regions/contexts/orphans/bridges, telos-default lens, optional doc_type)"
```

---

### Task 4: R3 — territory slice (Tier 1)

**Files:**
- Create: `migrations/20260703000005_graph_territory_slice.sql`
- Modify: `crates/temper-core/src/types/graph_territory.rs` (add `RegionMember`, `Component`, `TerritorySlice`)
- Modify: `crates/temper-core/src/types/mod.rs` (extend re-exports)
- Modify: `crates/temper-services/src/services/graph_service.rs` (add `territory_slice`)
- Modify: `crates/temper-api/src/handlers/graph.rs` (add handler)
- Modify: `crates/temper-api/src/routes.rs` (register route)
- Test: `tests/e2e/tests/graph_territory_slice_sql_test.rs`, `tests/e2e/tests/graph_territory_slice_e2e.rs`

**Interfaces:**
- Consumes: `kb_cogmap_components`, `kb_cogmap_region_members`, `resources_visible_to`, `cogmap_readable_by_profile`.
- Produces: `RegionMember`, `Component`, `TerritorySlice`; `graph_service::territory_slice(pool, profile_id, region_id) -> ApiResult<TerritorySlice>`; `GET /api/graph/regions/{region_id}/slice`.

- [ ] **Step 1: Add the R3 wire types** to `crates/temper-core/src/types/graph_territory.rs`:

```rust
/// A member of a region's interior (resolved per-member through resources_visible_to).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_territory.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct RegionMember {
    pub id: Uuid,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affinity: Option<f64>,
}

/// A sub-cluster (component) within a territory.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_territory.ts"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct Component {
    pub id: Uuid,
    pub member_count: i32,
}

/// R3 territory drill-in: components + top-N members (visibility-scoped).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_territory.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct TerritorySlice {
    pub region_id: Uuid,
    pub components: Vec<Component>,
    pub members: Vec<RegionMember>,
}
```

- [ ] **Step 2: Extend the re-export** in `mod.rs`:
`pub use graph_territory::{Bridge, Component, OrphanNode, RegionMember, Territory, TerritoryKind, TerritoryOverview, TerritorySlice};`

- [ ] **Step 3: Write the migration** `migrations/20260703000005_graph_territory_slice.sql` (join-based member deref, doc_type LEFT-joined):

```sql
-- R3 territory slice: components + visibility-scoped region members.

CREATE FUNCTION graph_region_components(
    p_profile uuid, p_region uuid
) RETURNS TABLE(component_id uuid, member_count int) LANGUAGE sql STABLE AS $$
    SELECT comp.id, cardinality(comp.member_ids)::int
    FROM kb_cogmap_regions reg
    JOIN kb_cogmap_components comp
      ON comp.cogmap_id = reg.cogmap_id AND comp.lens_id = reg.lens_id AND NOT comp.is_folded
    WHERE reg.id = p_region AND NOT reg.is_folded
      AND cogmap_readable_by_profile(p_profile, reg.cogmap_id);
$$;

CREATE FUNCTION graph_region_members(
    p_profile uuid, p_region uuid
) RETURNS TABLE(id uuid, title text, doc_type text, affinity double precision)
LANGUAGE sql STABLE AS $$
    WITH doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table='kb_resources' AND p.property_key='doc_type' AND NOT p.is_folded
    ),
    visible AS (SELECT resource_id FROM resources_visible_to(p_profile))
    SELECT r.id, r.title, d.dt AS doc_type, m.affinity
    FROM kb_cogmap_regions reg
    JOIN kb_cogmap_region_members m ON m.region_id = reg.id AND m.member_table = 'kb_resources'
    JOIN visible v ON v.resource_id = m.member_id
    JOIN kb_resources r ON r.id = m.member_id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    WHERE reg.id = p_region AND NOT reg.is_folded
      AND cogmap_readable_by_profile(p_profile, reg.cogmap_id)
    ORDER BY m.affinity DESC NULLS LAST;
$$;
```

- [ ] **Step 4: Add the service fn** `territory_slice` in `graph_service.rs` (bound members to top-N; deny-as-absence via existence+readability check):

```rust
use temper_core::types::graph_territory::{Component, RegionMember, TerritorySlice};

pub async fn territory_slice(
    pool: &sqlx::PgPool,
    profile_id: ProfileId,
    region_id: uuid::Uuid,
) -> ApiResult<TerritorySlice> {
    let readable: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM kb_cogmap_regions reg \
         WHERE reg.id = $1 AND NOT reg.is_folded \
           AND cogmap_readable_by_profile($2, reg.cogmap_id))",
    ).bind(region_id).bind(profile_id.as_uuid()).fetch_one(pool).await?;
    if !readable { return Err(ApiError::NotFound); }

    let components: Vec<Component> = sqlx::query_as::<_, (uuid::Uuid, i32)>(
        "SELECT component_id, member_count FROM graph_region_components($1, $2)",
    ).bind(profile_id.as_uuid()).bind(region_id).fetch_all(pool).await?
        .into_iter().map(|(id, member_count)| Component { id, member_count }).collect();

    const MEMBER_LIMIT: usize = 100;
    let members: Vec<RegionMember> = sqlx::query_as::<_, (uuid::Uuid, String, Option<String>, Option<f64>)>(
        "SELECT id, title, doc_type, affinity FROM graph_region_members($1, $2)",
    ).bind(profile_id.as_uuid()).bind(region_id).fetch_all(pool).await?
        .into_iter().take(MEMBER_LIMIT)
        .map(|(id, title, doc_type, affinity)| RegionMember { id, title, doc_type, affinity }).collect();

    Ok(TerritorySlice { region_id, components, members })
}
```

- [ ] **Step 5: Add the handler** in `graph.rs`:

```rust
/// GET /api/graph/regions/{region_id}/slice — R3 Tier-1 territory drill-in.
#[utoipa::path(
    get,
    path = "/api/graph/regions/{region_id}/slice",
    tag = "Graph",
    params(("region_id" = Uuid, Path, description = "Region id to slice")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Territory slice", body = TerritorySlice),
        (status = 404, description = "Region not readable by this profile")
    )
)]
pub async fn territory_slice(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(region_id): Path<Uuid>,
) -> ApiResult<Json<TerritorySlice>> {
    graph_service::territory_slice(&state.pool, ProfileId::from(auth.0.profile.id), region_id)
        .await
        .map(Json)
}
```

- [ ] **Step 6: Register the route** in `routes.rs` (`gated`):

```rust
.route("/api/graph/regions/{region_id}/slice", get(handlers::graph::territory_slice))
```

- [ ] **Step 7: SQL-level test** `tests/e2e/tests/graph_territory_slice_sql_test.rs`: insert a region + component + two members (one visible, one not, and one with no doc_type); assert `graph_region_components` returns the component with correct `member_count`, and `graph_region_members` returns only the visible member with `doc_type` NULL-able.

- [ ] **Step 8: HTTP e2e test** `tests/e2e/tests/graph_territory_slice_e2e.rs`: member GETs a readable region slice → 200 with members; unreadable/nonexistent region → 404.

- [ ] **Step 9: Run tests:** `cargo make test-e2e` → PASS.

- [ ] **Step 10: Regenerate TS + gate:** `cargo make generate-ts-types && cargo make check` → `graph_territory.ts` gains the three types; clean.

- [ ] **Step 11: Commit:**

```bash
git add migrations/20260703000005_graph_territory_slice.sql \
        crates/temper-core/src/types/graph_territory.rs crates/temper-core/src/types/mod.rs \
        crates/temper-services/src/services/graph_service.rs \
        crates/temper-api/src/handlers/graph.rs crates/temper-api/src/routes.rs \
        tests/e2e/tests/graph_territory_slice_sql_test.rs tests/e2e/tests/graph_territory_slice_e2e.rs \
        packages/temper-ui/src/lib/types/generated/graph_territory.ts
git commit -m "feat(graph): R3 Tier-1 territory slice (components + visibility-scoped members, optional doc_type)"
```

---

### Task 5: R5 — element event-trail

**Files:**
- Create: `migrations/20260703000006_element_event_trail.sql`
- Create: `crates/temper-core/src/types/element_trail.rs`
- Modify: `crates/temper-core/src/types/relationship_events.rs` (add derive stack to the six payload structs + `TargetEndpoint`)
- Modify: `crates/temper-core/src/types/mod.rs` (register `element_trail`, re-exports)
- Modify: `crates/temper-services/src/services/event_service.rs` (add `element_trail`)
- Modify: `crates/temper-api/src/handlers/events.rs` (add handler)
- Modify: `crates/temper-api/src/routes.rs` (register route)
- Test: `tests/e2e/tests/element_trail_sql_test.rs`, `tests/e2e/tests/element_trail_e2e.rs`

**Interfaces:**
- Consumes: `kb_events` (ordered by `id` = UUIDv7 emission order), `kb_event_types`, `kb_edges`, `resources_visible_to`, `edges_visible_to`.
- Produces: `ElementKind`, `ElementEvent`, `EventTrail`; ts-rs derives on `relationship_events` payloads; `event_service::element_trail(pool, profile_id, kind, id) -> ApiResult<EventTrail>`; `GET /api/graph/elements/{kind}/{id}/trail` where `kind ∈ {node, edge}`.

- [ ] **Step 1: Add the derive stack to `relationship_events.rs`.** For each of `TargetEndpoint`, `RelationshipAsserted`, `RelationshipRetyped`, `RelationshipReweighted`, `RelationshipFolded`, `RelationshipDecayed`, `RelationshipCorrected`, prepend the four `cfg_attr` lines to the existing `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]`:

```rust
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "relationship_events.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipAsserted { /* unchanged */ }
```

(Fields are `Uuid`/`EdgeKind`/`Polarity`/`f64`/`String`/`Option<String>` — all ts-rs-friendly. `TargetEndpoint` is a plain enum, fine.)

- [ ] **Step 2: Create `crates/temper-core/src/types/element_trail.rs`** — carry the canonical event-type **string** on the wire (not the substrate `EventKind`, which is Copy-only/no-serde and lives in the wrong crate):

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Which element a trail belongs to.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "element_trail.ts"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Node,
    Edge,
}

/// A single event on an element's timeline.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "element_trail.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ElementEvent {
    pub event_id: Uuid,
    /// Canonical event-type name (kb_event_types.name), e.g. "relationship.asserted".
    pub kind: String,
    /// The authoring agent entity (kb_events.emitter_entity_id).
    pub actor_entity_id: Uuid,
    /// ISO-8601 emission time (kb_events.occurred_at).
    pub occurred_at: String,
    /// ConfidenceBand from event metadata, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
}

/// A time-ordered event trail for one node or edge.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "element_trail.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct EventTrail {
    pub element_kind: ElementKind,
    pub element_id: Uuid,
    pub events: Vec<ElementEvent>,
}
```

- [ ] **Step 3: Register modules/re-exports** in `mod.rs`: `pub mod element_trail;` and `pub use element_trail::{ElementEvent, ElementKind, EventTrail};`. Confirm `pub mod relationship_events;` already exists (it does).

- [ ] **Step 4: Write the migration** `migrations/20260703000006_element_event_trail.sql`. **Keying is grounded in the emitter investigation** (see the R5 grounding note in Design decisions) — do NOT use `correlation_id` (it's never populated by mutations; every event is its own root) and do NOT gate the edge trail with `edges_visible_to` (it filters `NOT is_folded`, which would hide a folded edge's own trail — precisely when history matters). Order by `e.id` (UUIDv7 emission order; `occurred_at` ties within a transaction).

```sql
-- R5 element event-trail: time-ordered events for a node or edge.

-- Edge trail: every relationship_* payload embeds a stable edge_id. Gate via the
-- edge's HOME ANCHOR (anchor_readable_by_profile), NOT edges_visible_to — a folded
-- edge must still show its trail (the fold event is part of the story).
CREATE FUNCTION element_trail_edge(
    p_profile uuid, p_edge uuid
) RETURNS TABLE(event_id uuid, kind text, actor_entity_id uuid, occurred_at timestamptz, metadata jsonb)
LANGUAGE sql STABLE AS $$
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata
    FROM kb_edges edg
    JOIN kb_events ev ON (ev.payload ->> 'edge_id')::uuid = edg.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    WHERE edg.id = p_edge
      AND anchor_readable_by_profile(p_profile, edg.home_anchor_table, edg.home_anchor_id)
    ORDER BY ev.id;
$$;

-- Node trail: NO single key exists — union three grounded key-shapes, then gate
-- once via resources_visible_to. (1) resource-keyed events (created/updated/deleted/
-- rehomed/block_created); (2) property events whose owner IS this resource
-- (guard owner.table='kb_resources'); (3) block events that carry only block_id →
-- join kb_content_blocks to attribute them.
CREATE FUNCTION element_trail_node(
    p_profile uuid, p_resource uuid
) RETURNS TABLE(event_id uuid, kind text, actor_entity_id uuid, occurred_at timestamptz, metadata jsonb)
LANGUAGE sql STABLE AS $$
    WITH ev_ids AS (
        SELECT ev.id FROM kb_events ev
         WHERE (ev.payload ->> 'resource_id')::uuid = p_resource
        UNION
        SELECT ev.id FROM kb_events ev
         WHERE ev.payload -> 'owner' ->> 'table' = 'kb_resources'
           AND (ev.payload -> 'owner' ->> 'id')::uuid = p_resource
        UNION
        SELECT ev.id FROM kb_events ev
         JOIN kb_content_blocks b ON b.id = (ev.payload ->> 'block_id')::uuid
        WHERE b.resource_id = p_resource
    )
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata
    FROM ev_ids
    JOIN kb_events ev ON ev.id = ev_ids.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    WHERE EXISTS (
        SELECT 1 FROM resources_visible_to(p_profile) v WHERE v.resource_id = p_resource
    )
    ORDER BY ev.id;
$$;
```

> The three node-trail key-shapes and the edge-id key are grounded in the emitter code (`_event_append` + the `_project_relationship_*` / `_project_resource_*` / property / block projectors in `migrations/20260624000002_canonical_functions.sql`, and the payload structs in `crates/temper-substrate/src/payloads.rs`). The `(payload ->> 'resource_id')::uuid` casts are safe because payloads are typed structs (the key, when present, is always a UUID string). Validate against the DB in Step 10; if any event type stores a non-UUID under one of these keys, wrap the cast in a `jsonb_typeof`/regex guard.

- [ ] **Step 5: Add the service fn** `element_trail` in `event_service.rs`:

```rust
use temper_core::types::element_trail::{ElementEvent, ElementKind, EventTrail};

pub async fn element_trail(
    pool: &sqlx::PgPool,
    profile_id: ProfileId,
    kind: ElementKind,
    element_id: uuid::Uuid,
) -> ApiResult<EventTrail> {
    let fn_name = match kind {
        ElementKind::Edge => "element_trail_edge",
        ElementKind::Node => "element_trail_node",
    };
    let rows = sqlx::query_as::<_, (uuid::Uuid, String, uuid::Uuid, chrono::DateTime<chrono::Utc>, serde_json::Value)>(
        &format!("SELECT event_id, kind, actor_entity_id, occurred_at, metadata FROM {fn_name}($1, $2)"),
    ).bind(profile_id.as_uuid()).bind(element_id).fetch_all(pool).await?;

    let events = rows.into_iter().map(|(event_id, kind, actor_entity_id, occurred_at, metadata)| {
        // metadata is AgentAuthorship-shaped for agent acts, {} for system acts.
        // The band is the bare lowercase string under `confidence` (NOT `confidence_band`).
        let confidence = metadata.get("confidence").and_then(|v| v.as_str()).map(str::to_string);
        ElementEvent { event_id, kind, actor_entity_id, occurred_at: occurred_at.to_rfc3339(), confidence }
    }).collect();

    Ok(EventTrail { element_kind: kind, element_id, events })
}
```

> `fn_name` is a fixed internal literal (not user input) — the `format!` is injection-safe. The confidence key is `metadata->>'confidence'` (from `AgentAuthorship`, `crates/temper-core/src/types/authorship.rs`); it is NULL for unauthored/system acts. An unreadable/nonexistent element yields an empty trail (200) rather than 404 — acceptable because the UI only requests trails for elements already surfaced in a slice, and the visibility gate lives inside the SQL so no data leaks.

- [ ] **Step 6: Add the handler** in `crates/temper-api/src/handlers/events.rs`:

```rust
/// GET /api/graph/elements/{kind}/{id}/trail — R5 element event-trail. kind ∈ {node, edge}.
#[utoipa::path(
    get,
    path = "/api/graph/elements/{kind}/{id}/trail",
    tag = "Events",
    params(
        ("kind" = String, Path, description = "node | edge"),
        ("id" = Uuid, Path, description = "resource id (node) or edge id")
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Event trail", body = EventTrail),
        (status = 400, description = "Unknown element kind")
    )
)]
pub async fn element_trail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((kind, id)): Path<(String, Uuid)>,
) -> ApiResult<Json<EventTrail>> {
    let element_kind = match kind.as_str() {
        "node" => ElementKind::Node,
        "edge" => ElementKind::Edge,
        _ => return Err(ApiError::BadRequest("element kind must be 'node' or 'edge'".into())),
    };
    event_service::element_trail(&state.pool, ProfileId::from(auth.0.profile.id), element_kind, id)
        .await
        .map(Json)
}
```

Imports: `use temper_core::types::element_trail::{ElementKind, EventTrail};`.

- [ ] **Step 7: Register the route** in `routes.rs` (`gated`):

```rust
.route("/api/graph/elements/{kind}/{id}/trail", get(handlers::events::element_trail))
```

- [ ] **Step 8: SQL-level test** `tests/e2e/tests/element_trail_sql_test.rs`. Edge trail: insert an edge + an assert event and a later reweight event **both carrying the same `payload->>'edge_id'`** (NOT a shared correlation_id — that's the bug this test guards against); assert `element_trail_edge` returns both, ordered by `id`. Fold the edge (`is_folded=true`) and assert its trail is STILL returned (proves the `anchor_readable_by_profile` gate, not `edges_visible_to`, is used). Node trail: insert a `resource_created` event (key `resource_id`), a `property_set` event (key `owner.id` + `owner.table='kb_resources'`), and a `block_mutated` event (key `block_id`, with a `kb_content_blocks` row whose `resource_id` is the node); assert `element_trail_node` returns all three via the union, and excludes an event for a non-visible resource.

- [ ] **Step 9: HTTP e2e test** `tests/e2e/tests/element_trail_e2e.rs`: member GETs `/api/graph/elements/edge/{id}/trail` → 200 time-ordered; `kind=bogus` → 400.

- [ ] **Step 10: Run tests:** `cargo make test-e2e` → PASS.

- [ ] **Step 11: Regenerate TS + gate:** `cargo make generate-ts-types && cargo make check` → `element_trail.ts` + `relationship_events.ts` generated; clean.

- [ ] **Step 12: Commit:**

```bash
git add migrations/20260703000006_element_event_trail.sql \
        crates/temper-core/src/types/element_trail.rs \
        crates/temper-core/src/types/relationship_events.rs crates/temper-core/src/types/mod.rs \
        crates/temper-services/src/services/event_service.rs \
        crates/temper-api/src/handlers/events.rs crates/temper-api/src/routes.rs \
        tests/e2e/tests/element_trail_sql_test.rs tests/e2e/tests/element_trail_e2e.rs \
        packages/temper-ui/src/lib/types/generated/element_trail.ts \
        packages/temper-ui/src/lib/types/generated/relationship_events.ts
git commit -m "feat(graph): R5 element event-trail (per-node/edge history + ts-rs derives on relationship payloads)"
```

---

### Task 6: Final gate + consolidated review

**Files:** none (verification only).

- [ ] **Step 1: Full workspace gate:** `cargo make check` → clean fmt + clippy + machete + TS typecheck.

- [ ] **Step 2: Full e2e tier** (acceptance gate — access scoping proven end-to-end): `cargo make test-e2e` → all green.

- [ ] **Step 3: Confirm no `.sqlx` drift:** `git status --porcelain crates/*/.sqlx tests/e2e/.sqlx` → empty. If any changed, a `query!` macro slipped in — revert to runtime API or run the matching `cargo make prepare-*` and commit the cache.

- [ ] **Step 4: Confirm the legacy subgraph endpoint still works** (Chunk-D deferral held): `cargo nextest run -p temper-api --features test-db --test graph_subgraph_test` → PASS.

- [ ] **Step 5: Final TS regen + clean tree:** `cargo make generate-ts-types && git status --porcelain packages/temper-ui/src/lib/types/generated` → empty.

- [ ] **Step 6: Consolidated review** (deferred-review cadence — spec + code-quality across the whole Chunk B diff): `/code-review high`, then address findings, re-run `cargo make check && cargo make test-e2e`, commit fixes.

---

## Self-Review

**Spec coverage** (spec §"Read model" + §"Wire types"):
- R2 → Task 3 ✓ (`Territory`/`TerritoryOverview`/`Bridge`/`OrphanNode`). R3 → Task 4 ✓ (`RegionMember`/`Component`/`TerritorySlice`). R4 → Task 2 ✓ (Atlas types Task 1 + `SliceRequest`). R5 → Task 5 ✓ (`ElementEvent`/`EventTrail` + ts-rs derives on `relationship_events`).
- Extended-`GraphNode` spec item → **intentionally not done as an extension.** The reshape (Design decision 7) replaces it with purpose-built `AtlasNode`, leaving legacy `GraphNode` frozen. The spec's `home`/`salience`/`is_folded` intent is honored: `home`/`salience` land on `AtlasNode`; `is_folded` is **dropped from the node** (it's an edge/region property — resources gate on `is_active`, not `is_folded` — so it was never meaningful on a node; folded *edges* are already excluded by the traversal). Documented, not silently dropped.
- Legacy `/api/graph/subgraph` retirement → deferred to Chunk D (Design decision 4), flagged.
- `EventKind` wire gap → surfaced as the canonical event-type **string** on `ElementEvent.kind` rather than dragging the substrate-internal `EventKind` into temper-core (Design decision, Task 5). Documented.
- Open questions (lens, orphan salience, bridges) → resolved in Design decisions ✓. e2e access tier → every read has SQL-level + HTTP tests ✓.

**Placeholder scan:** no "TBD"/"add error handling"/"similar to Task N". Full struct defs, full SQL bodies, full service/handler code, real assertions. The "confirm against real code/schema" notes (EdgeKind import path, EdgeKind↔pg-enum decode via `fetch_subgraph_edges`, `kb_contexts` label column, node-trail payload keys, confidence metadata key) are honest DB-truth verification steps, not deferred design.

**Type consistency:** `NodeHome`/`AtlasNode`/`AtlasEdge`/`AtlasSubgraph`/`SliceRequest` defined Task 1 (temper-core), consumed Task 2. `doc_type: Option<String>` uniformly across `AtlasNode`/`OrphanNode`/`RegionMember` (the cargo-cult `COALESCE(...,'concept')` is gone; every doc_type join is LEFT). `Territory`/`TerritoryKind`/`TerritoryOverview`/`Bridge`/`OrphanNode` (Task 3) extended with `RegionMember`/`Component`/`TerritorySlice` (Task 4) — one re-export line grows each time. `ElementKind`/`ElementEvent`/`EventTrail` (Task 5). Service signatures match handler call sites; `SliceRequest` fields match the service bind order; all four routes land in the `gated` router. `AtlasEdge.label` is `Option<String>` and carries `weight` — matching the real nullable `kb_edges.label` and its `weight` column (the legacy `GraphEdge` did neither).

**Style:** new SQL uses CTE joins (`JOIN scope`, `JOIN visible`, `JOIN edges_visible_to(...) ev`) rather than `IN (SELECT …)` per the reviewer's preference; shipped functions we merely call are left as-is. R4 composes `graph_traverse`'s proven recursive *shape* in a new team-scoped, edge-kind-aware, weight-returning `graph_traverse_scoped` rather than reinventing a BFS or mutating the shipped one.
