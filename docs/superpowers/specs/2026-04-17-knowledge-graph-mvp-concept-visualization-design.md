# Design: Knowledge graph MVP — concept-centric visualization

**Date:** 2026-04-17
**Context:** temper
**Goal:** llm-wiki
**Related tasks:** `2026-04-12-knowledge-graph-ui-d3-js-visualization` (this is a narrowed MVP cut)
**Related research:** `2026-04-13-r11-knowledge-graph-visualization-design`, `2026-04-01-r9-sveltekit-ui-design`
**Status:** Design

---

## Overview

A minimum-viable knowledge-graph visualization that proves the data built by the LLM-wiki pipeline hangs together end-to-end — from Postgres through Axum through SvelteKit to D3 in the browser. Scoped to land on the current `jct/temper-index-llm-wiki` branch before merge, so the branch closes with a visible demonstration of the 26 concepts and 186 member edges seeded during D3.

This is **not** the full R11 design. R11's two-mode toggle, cluster-by-doctype hulls, Jaccard emergent edges, filter panel, expand-on-double-click, and search integration all land on a follow-up branch. What lands here is one route, one endpoint, one SQL query, one force-directed canvas, and the pure-logic modules that feed it.

---

## Scope

### In

- `GET /api/graph/subgraph` — context-scoped, depth-2 BFS from concept seeds
- `/vault/[owner]/[context]/graph` — new SvelteKit route with SSR data load
- `GraphCanvas.svelte` — D3 v7 force-directed SVG renderer
- `GraphTooltip.svelte` — floating hover card
- Pure-logic modules under `packages/temper-ui/src/lib/graph/` with vitest specs
- New `graph_service.rs` with `aggregator_subgraph()` function + `AggregatorSubgraphParams` struct
- New types: `GraphNode`, `GraphEdge`, `SubgraphResponse` in `temper-core` with `ts-rs` derives
- Sidebar nav link added to `ContextNavGroup.svelte`
- vitest added to `packages/temper-ui` as a baseline
- Extensions to `scripts/seed-dev-data.sql` + new `scripts/seed-graph-fixtures.sql` for wide-and-deep integration-test coverage
- Interactions: zoom, pan, drag, click-to-navigate, hover tooltip

### Out (follow-up branch)

- R11 two-mode toggle (structural vs meta-doc projection)
- Jaccard emergent edges between aggregators
- Cluster-by-doctype + convex hull rendering
- Filter panel (`GraphControls`)
- `GraphLegend` component
- `POST /api/graph/expand` — double-click neighbor expansion
- Search "View in graph" action
- Concept-focus spotlight mode
- Goals / decisions / sessions as aggregator types
- Cross-context graph (`/vault/[owner]/graph`)
- WebGL/Canvas renderer
- Resource-detail edges panel
- Minimap, right-click context menu

---

## Extension posture

Every layer is designed so the follow-up branch bolts on the full R11 design without rewrites:

- **Service function** accepts `aggregator_types: &[DocType]` — currently `&[DocType::Concept]`. Adding goals/decisions/sessions is a one-line call-site change.
- **Depth** is a service-struct field (`depth: u32`) clamped at `min(depth, 10)` inside the function. v1 passes `2`. The clamp's rationale: recursive-CTE cost grows superlinearly, 10 hops covers any imaginable UI traversal. `edge_count` on `GraphNode` is `i32` to match Postgres' `int4` (COUNT cast) — 2B edges per node is well beyond any realistic ceiling.
- **SubgraphResponse** already carries `doc_type` on every node and `edge_type` on every edge, so the client can start cluster-by-doctype rendering without server changes.
- **Handler** is `GET /api/graph/subgraph` with no body. When filters arrive, we migrate to `POST /api/graph/subgraph` with a typed filter body — a contained breaking change (one endpoint, one consumer).
- **Route path** is the R9/R11 spec's canonical form, so deep-linking (`?seed=<id>`) and search integration drop in cleanly.

What this explicitly does **not** buy: R11's "participant vs aggregator" visual distinction. Concepts render larger than members in the MVP, but they aren't cluster-anchored with hulls and we don't project emergent edges between them. Those are the main visual fruits deferred.

---

## Server architecture

### New service: `crates/temper-api/src/services/graph_service.rs`

One function, one responsibility:

```rust
pub struct AggregatorSubgraphParams<'a> {
    pub caller_profile_id: Uuid,
    pub context: &'a str,
    pub aggregator_types: &'a [DocType],  // v1: &[DocType::Concept]
    pub depth: u32,                        // v1: 2; hard-clamped to min(depth, 10)
}

pub async fn aggregator_subgraph(
    pool: &PgPool,
    params: AggregatorSubgraphParams<'_>,
) -> Result<SubgraphResponse, ServiceError>
```

The params struct is nominally optional at 4 fields but claims the surface for the follow-up branch's filter additions (`include_doc_types`, `edge_type_filter`, etc.) without refactoring call sites.

### The SQL — recursive CTE with depth guard

A single query, depth-limited recursive CTE, owner-scoped via `resources_visible_to` (matching the pattern in `resource_service.rs`, `edge_service.rs`, `event_service.rs`):

```sql
WITH RECURSIVE frontier AS (
  -- Seed: aggregator nodes in the context, visible to caller
  SELECT r.id, 0 AS depth
  FROM kb_resources r
  JOIN resources_visible_to($1) rv ON rv.resource_id = r.id
  WHERE r.context = $2
    AND r.doc_type = ANY($3::doc_type[])
    AND r.deleted_at IS NULL
  UNION
  -- Expand: one hop via any edge, filtered through visibility
  SELECT r.id, f.depth + 1
  FROM frontier f
  JOIN kb_resource_edges e
    ON (e.source_resource_id = f.id OR e.target_resource_id = f.id)
  JOIN kb_resources r
    ON r.id = CASE WHEN e.source_resource_id = f.id
                   THEN e.target_resource_id
                   ELSE e.source_resource_id
              END
  JOIN resources_visible_to($1) rv ON rv.resource_id = r.id
  WHERE f.depth < $4
    AND r.deleted_at IS NULL
),
node_ids AS (SELECT DISTINCT id FROM frontier),
resolved_nodes AS (
  SELECT
    r.id,
    r.slug,
    r.title,
    r.doc_type,
    (SELECT COUNT(*)::int
     FROM kb_resource_edges e
     WHERE e.source_resource_id = r.id OR e.target_resource_id = r.id) AS edge_count
  FROM kb_resources r
  JOIN node_ids n ON n.id = r.id
),
subgraph_edges AS (
  SELECT e.source_resource_id, e.target_resource_id, e.edge_type
  FROM kb_resource_edges e
  WHERE e.source_resource_id IN (SELECT id FROM node_ids)
    AND e.target_resource_id IN (SELECT id FROM node_ids)
)
SELECT ...;
```

Uses `sqlx::query!` / `query_as!` macros for compile-time verification. `.sqlx/` cache regenerated via `cargo sqlx prepare --workspace -- --all-features` after writing.

**Owner-boundary enforcement:** the `resources_visible_to($1)` join appears on both the seed side and the expansion side. A cross-owner resource is physically impossible to include, because the expansion join can't resolve the target row. This matches R7's hard constraint and the fundamentals' profile-scoping rule without needing a separate post-filter pass.

**Depth clamp:** at the top of the service function:

```rust
let depth = params.depth.min(10);
debug_assert!(params.depth == depth, "depth > 10 clamped to 10");
```

v1 call site passes `2`, so the clamp is inert today — it's guardrail against a future caller fat-fingering the value and DoS-ing Postgres with a runaway CTE.

### New handler: `crates/temper-api/src/handlers/graph.rs`

```rust
#[derive(Deserialize)]
pub struct SubgraphQuery {
    pub owner: String,    // "@me" or a profile handle
    pub context: String,
}

pub async fn get_subgraph(
    State(state): State<AppState>,
    Query(query): Query<SubgraphQuery>,
    Extension(auth): Extension<AuthContext>,
) -> Result<Json<SubgraphResponse>, ApiError>
```

Resolves `owner` via the existing handle-resolution pattern, extracts `caller_profile_id` from `AuthContext`, calls `graph_service::aggregator_subgraph(...)` with `aggregator_types: &[DocType::Concept]` hardcoded for v1. Wired into the router at `GET /api/graph/subgraph`.

### Types in `crates/temper-core/src/types/graph.rs` (extending existing file)

```rust
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: Uuid,
    pub slug: String,
    pub title: String,
    pub doc_type: DocType,
    pub edge_count: i32,
}

pub struct GraphEdge {
    pub source: Uuid,
    pub target: Uuid,
    pub edge_type: EdgeType,
}

pub struct SubgraphResponse {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}
```

Regenerate TS bindings with `cargo make generate-ts-types` — lands in `packages/temper-ui/src/lib/types/graph.ts`.

### Auth, scoping, visibility

Same JWT flow as every other read endpoint. The owner-boundary enforcement is in the SQL itself via `resources_visible_to`, not a separate pass. If an edge somehow pointed across owners, the visibility join drops the endpoint and the edge is then filtered out because both endpoints must be in `node_ids`.

No schema migration needed. Only the `.sqlx/` cache requires regenerating.

---

## UI architecture

### Route

```
packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/graph/
├── +page.server.ts    # SSR load: fetches /api/graph/subgraph
└── +page.svelte       # thin shell + <GraphCanvas />
```

`[owner]` and `[context]` already resolve in the parent layout, so no layout changes needed.

### Components

```
packages/temper-ui/src/lib/components/graph/
├── GraphCanvas.svelte     # D3 runtime layer (SVG + force sim + interactions)
└── GraphTooltip.svelte    # floating hover card
```

Scoping under `graph/` keeps the flat `components/` directory clean and lets follow-up components (`GraphControls`, `GraphLegend`, `GraphFilter`) land alongside these without clutter.

### Three-layer logic split

The architectural principle: domain logic lives in pure modules, D3-shape transformations live in a middle layer, and actual D3 runtime calls live only in the Svelte component. If D3 ever gets swapped for a different renderer, layer 1 is untouched, layer 2 gets one new translation module, and only layer 3 rewrites.

**Layer 1 — pure domain logic (no D3, no DOM)**

```
packages/temper-ui/src/lib/graph/
├── styling.ts          # nodeRadius(node), nodeColor(docType), edgeStrokeDasharray(edgeType)
├── labels.ts           # truncateLabel(title, max), shouldShowLabel(node, zoomLevel)
├── navigation.ts       # resourceHref(owner, context, node)
├── adjacency.ts        # buildAdjacencyIndex(edges), neighborsOf(id)
└── positions.ts        # seedPositions(nodes, viewport) — concepts on a spiral,
                        # members jittered near their nearest concept
```

Every function takes plain data and returns plain data. No `d3` import in any of these files. Vitest specs colocated: `styling.test.ts`, `labels.test.ts`, etc.

**Layer 2 — D3 shape transforms (imports D3 types, produces D3-shaped data, no DOM)**

```
packages/temper-ui/src/lib/graph/
├── simulation-input.ts  # toSimulationNodes(pureNodes, positions) → d3.SimulationNodeDatum[]
│                        # toSimulationLinks(pureEdges) → d3.SimulationLinkDatum[]
└── force-config.ts      # buildForceConfig(viewport) → plain object of tuning values
```

These translate our domain types into shapes D3's `forceSimulation` consumes. They *describe* a simulation; they don't run one. Still testable: "given these nodes + these positions, output array has x/y set and index assigned."

**Layer 3 — D3 runtime, inside `GraphCanvas.svelte`**

The component imports `d3.forceSimulation`, `d3.zoom`, `d3.drag`, calls the pure modules for its inputs, applies the force config, owns the tick loop, and mutates SVG attributes directly. No business logic — it's a D3+DOM harness wrapping the pure modules.

### GraphCanvas responsibilities

- Takes `nodes: GraphNode[]` and `edges: GraphEdge[]` as props
- On mount: calls `seedPositions`, `toSimulationNodes`, `toSimulationLinks`, `buildForceConfig`; instantiates `d3.forceSimulation` with `forceLink` (by `id`), `forceManyBody`, `forceCenter`, `forceCollide`
- Renders SVG inside a `$effect`: `<circle>` nodes (radius from `nodeRadius`, fill from `nodeColor`), `<line>` edges (dash pattern from `edgeStrokeDasharray`)
- Updates positions per tick via direct SVG attribute writes (not Svelte reactivity — D3's tick rate would thrash the reconciler)
- `d3.zoom()` on the root `<svg>` — wheel-zoom, drag-to-pan on empty canvas, extent `[0.3, 3]`
- `d3.drag()` on nodes — standard force-drag handler (fix position during drag, release on end)
- Click handler on nodes → `goto(resourceHref(owner, context, node))` via SvelteKit navigation
- Hover → emits `node` to parent, which positions `<GraphTooltip>`
- Labels: concepts always show truncated title; members show title only on hover or above zoom level 1.5

### Color palette (lifted from R11 settled encoding, plus concept)

| Doc type | Color | Source |
|----------|-------|--------|
| `research` | `#7eb8da` | R11 |
| `task` | `#f0a870` | R11 |
| `session` | `#82c99a` | R11 |
| `concept` | `#d48ac7` | new — warm pink to differentiate from R11's tentative purple for goals |

### GraphTooltip

Thin positioned card. Props: `node: GraphNode | null`, `x: number`, `y: number`. Renders title, doc-type badge, edge count, and for concepts a "members: N" line. Styled with Tailwind consistent with `ResourceMetaHeader`.

### Sidebar navigation

Add a "Graph" entry to `ContextNavGroup.svelte` alongside existing doc-type groupings, pointing at `/vault/{owner}/{context}/graph`. One-line addition.

### Bundle impact

Imports the D3 sub-packages individually (not the `d3` umbrella):

- `d3-selection`
- `d3-force`
- `d3-drag`
- `d3-zoom`

Roughly ~40KB gzipped across the four. `d3-shape` and other umbrella pieces aren't needed.

---

## Data flow end-to-end

```
Browser → SvelteKit server → Axum API → Postgres
              │                   │          │
              │  GET /api/graph/  │          │
              │  subgraph?owner=  │          │
              │  @me&context=     │          │
              │  temper (JWT) ───►│          │
              │                   │ resolve  │
              │                   │ caller   │
              │                   │ profile  │
              │                   │ from JWT │
              │                   │          │
              │                   │ graph_   │
              │                   │ service::│
              │                   │ aggregator_
              │                   │ subgraph ►  recursive CTE,
              │                   │          │  depth-2, joined
              │                   │          │  on resources_
              │                   │          │  visible_to($1)
              │                   │◄─────────┤  SubgraphResponse
              │◄──────────────────┤          │
              │                   │          │
              ◄── HTML + data ────┤          │
   │                                         │
   GraphCanvas mounts                        │
   → createForceSimulation                   │
   → render SVG, tick loop                   │
   → user drags/zooms/hovers/clicks          │
   → click → goto(/vault/.../concept/slug) ─►
```

The page is SSR-rendered with subgraph data baked into the load function's return — no client-side fetch after mount. Bookmarking `/graph` deep-links to the same data (subject to auth). No client-side refetch, no live updates — reload to refresh.

---

## Error handling and edge cases

| Condition | Where handled | UX |
|-----------|---------------|-----|
| Empty graph (no concepts in context) | `+page.svelte` checks `data.nodes.length === 0` | `EmptyState.svelte` with copy: *"No concepts in this context yet. Run `temper graph index` to generate them."* |
| API 401 (token expired) | existing `+layout.server.ts` auth middleware | Redirects to `/auth/login`, existing pattern |
| API 500 / timeout | SvelteKit `+error.svelte` | Standard error page with retry link |
| Malformed query params | Axum's `Query<T>` extractor | Auto-400 (defensive — UI never sends malformed params) |
| Context not visible to caller | service returns empty via visibility join | Same empty-state as "no concepts" — does not differentiate between "no concepts" and "hidden context" because knowing a context exists but is hidden is itself data leakage |
| Deleted resource in DB | `deleted_at IS NULL` in CTE | Silently excluded |
| Cross-owner edge (defense-in-depth) | `resources_visible_to` join drops orphan endpoints | Silently excluded; the edge is then filtered out because both endpoints must be in `node_ids` |
| `depth > 10` (future misuse) | service clamps at top of function | Silently clamped + debug-assert |

**Explicitly not handled:**

- Cross-owner shared contexts (visibility function handles transparently)
- Pagination / streaming (not needed at this scale)
- Stale data / real-time updates (reload to refresh)
- Incremental / partial failure (query is atomic — full subgraph or error)

---

## Performance budget

For the temper context as it stands today:

- ~26 concepts × ~7 members = ~180 tier-1→tier-2 edges
- Tier-2→tier-3 expansion adds ~30–50 nodes and ~80 more edges
- Payload: ~27KB uncompressed, ~5KB gzipped — inside one TCP segment
- SQL: depth-2 recursive CTE at this size returns in milliseconds
- D3 force sim: converges visually in under 2s at default alpha
- Initial SSR render: under the existing `[doc_type]/+page.server.ts` budget

No rate limiting, caching, or pagination in v1 — endpoint is cheap enough that adding those is premature for a single-user deployment.

---

## Testing strategy

### Rust

- **`graph_service.rs` integration test** under `#[cfg(feature = "test-db")]`:
  - Loads the graph test fixture (see "Test fixtures" below) before each test via `BEGIN; ... ROLLBACK;` transaction wrapping for isolation
  - Calls `aggregator_subgraph` with `aggregator_types: &[DocType::Concept], depth: 2`
  - Separate test cases cover each assertion target (see fixture scenarios below): every-concept-returned, all-direct-members-present, tier-3-reachable-included, tier-3-unreachable-excluded, cross-owner-excluded, singleton-concept-returned-as-isolated-node, diamond-member-appears-once, `edge_count` correct, empty-context-returns-empty
- **Handler test** in new `crates/temper-api/tests/graph_test.rs`:
  - 200 + valid `SubgraphResponse` shape for authenticated request
  - 401 for unauthenticated request
- **Depth-clamp unit test** — calling with `depth: 100` clamps to 10

### Test fixtures

Sql-function service logic is easy to under-test without wide-and-deep fixture data. To fix that, this work extends `scripts/seed-dev-data.sql` and introduces a companion `scripts/seed-graph-fixtures.sql`:

**Extensions to `seed-dev-data.sql`** (benefit both UI dev and test confidence):
- Add edges between the existing 3 concepts and their related tasks/research/sessions — currently the seed creates resources with zero relationships, so the graph route would render three floating concepts. Adding 2–3 member edges per concept gives the dev UI a realistic render and proves the end-to-end path locally.
- Add a handful of inter-member edges (`research depends_on research`, `task preceded_by task`) so tier-3 expansion has something to expand to.

**New `scripts/seed-graph-fixtures.sql`** — comprehensive test fixtures, invoked by Rust integration tests via `sqlx::query_file!()` or similar. Parameterized by a profile-ID GUC (same pattern as `seed-dev-data.sql`'s `seed.email`) so tests can seed into a scratch profile without colliding with dev data. Scenarios covered:

| Scenario | Setup | What it proves |
|----------|-------|----------------|
| **Happy path** | 3 concepts × 4 members each | Base depth-2 BFS, `edge_count` correctness |
| **Tier-3 reachable** | Member → member edge (`depends_on`) | Depth-2 traversal actually expands beyond direct members |
| **Tier-3 unreachable** | Node 4 hops away | Depth cutoff works |
| **Singleton concept** | Concept with no member edges | Isolated node still returned |
| **Diamond overlap** | Two concepts sharing a member | Member appears exactly once in nodes list |
| **Cross-owner leak check** | Second profile owns a concept in the same context | Caller's query does not return the other owner's concept |
| **Cross-owner edge attempt** | Edge between caller's concept and other owner's resource | Edge is filtered because target fails `resources_visible_to` join |
| **Deleted resource** | Concept with `deleted_at` set | Excluded from results |
| **Empty context** | Context with zero concepts | Returns `{ nodes: [], edges: [] }` |
| **Multi-context isolation** | Concepts in two contexts | Query for context A returns only A's concepts |

Test harness pattern (illustrative):

```rust
#[sqlx::test(fixtures(path = "../../scripts", scripts("seed-graph-fixtures.sql")))]
async fn happy_path_returns_all_concepts_and_members(pool: PgPool) { ... }
```

If `sqlx::test`'s fixture loader doesn't fit (e.g., needs GUC setup), fall back to explicit `sqlx::query_file_unchecked!` with transaction wrapping. The plan will pick the mechanism that matches temper-api's existing test patterns.

**Fixture hygiene:** the graph-fixtures SQL uses the same `ON CONFLICT` idempotency pattern as `seed-dev-data.sql`, uses well-known UUIDs for the scratch profile(s)/context/resources so assertions can reference them by constant, and drops its helper functions at the end.

### TypeScript (vitest — new to temper-ui)

Add to `packages/temper-ui/package.json`:

```json
"scripts": {
  "test": "vitest run",
  "test:watch": "vitest"
},
"devDependencies": {
  "vitest": "^2"
}
```

Vitest config via `vite.config.ts` extension (no new config file). Specs colocated with modules:

- `styling.test.ts` — radius/color/dash for every `DocType` × `EdgeType` combination
- `labels.test.ts` — truncation boundary, zoom-level thresholds
- `navigation.test.ts` — `resourceHref` for every `DocType`
- `adjacency.test.ts` — symmetric index, correct neighbor sets
- `positions.test.ts` — deterministic output (same input → same positions), viewport bounds respected
- `simulation-input.test.ts` — every input node has exactly one output, positions preserved, edges reference existing nodes by id
- `force-config.test.ts` — viewport math

No component-level tests for v1 — components are thin, pure modules carry the logic weight. If layer-3 grows, we revisit.

### CI wiring

Add `cd packages/temper-ui && bun run test` to the appropriate `test-typescript.yml` job (mirrors the existing `temper-cloud` pattern).

---

## Success criteria

This work is done when all the following are true:

1. `GET /api/graph/subgraph?owner=@me&context=temper` returns 200 with a valid `SubgraphResponse` containing at least the 26 concepts currently in the context
2. `/vault/@me/temper/graph` renders the force-directed graph in the browser
3. Zoom, pan, drag, hover, click-to-navigate all work in the browser
4. `cargo make check` passes (fmt + clippy + docs + machete)
5. `cargo make test-all` passes (including the new `graph_service` integration test)
6. `cd packages/temper-ui && bun run test` passes (new vitest specs, all green)
7. `.sqlx/` cache regenerated and committed
8. Manual smoke: click a concept node lands on `/vault/@me/temper/concept/<slug>`; click a research node lands on research detail

---

## Follow-up task seeds

Once this lands, create tasks for:

- Filter panel + `POST /api/graph/subgraph` migration (breaking change to the endpoint, contained to one consumer)
- Goals / decisions / sessions as aggregator types
- Cluster-by-doctype + convex hulls + R11 two-mode toggle
- Expand-on-double-click + focus-mode interactions
- Search "View in graph" action
