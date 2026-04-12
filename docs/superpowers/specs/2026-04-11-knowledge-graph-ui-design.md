# Knowledge Graph UI — Design Spec

**Date:** 2026-04-11
**Task:** `2026-04-11-knowledge-graph-ui-and-seeding-the-vault-for-relationships`
**Based on:** R9 §8.4, §12 (SvelteKit UI Design — Knowledge Graph Explorer)
**Related specs:**
- `2026-04-11-sync-metadata-only-patch-design.md`
- `2026-04-11-open-meta-intentionality-and-graph-build-design.md`
  (provides the seed data this UI visualizes)
**Mode:** build
**Effort:** large (multi-session)

---

## Overview

D3.js force-directed graph visualization of the knowledge base, integrated
into the existing SvelteKit UI at `packages/temper-ui`. Context-scoped,
owner-bounded, progressively loaded. This spec adapts the R9 research
design to the current codebase — the R7 knowledge graph infrastructure
(schema, edge extraction, graph traversal SQL functions, combined search)
is already built.

## First Principle: Owner Boundary

The graph visualization must only show resources within the owner boundary.
No cross-owner edges or nodes. The API scopes through
`resources_visible_to()`, and the UI passes owner context for correct
filtering. Same constraint as `temper graph build` (Spec 2).

## Routing

### Primary Route

```
(app)/vault/[owner]/[context]/graph
```

Context-scoped graph explorer. Gets `owner` and `context` from URL path
params, consistent with the existing vault URL hierarchy:

```
(app)/vault/[owner]/[context]/          — VaultGrid (existing)
(app)/vault/[owner]/[context]/graph     — GraphCanvas (new)
(app)/vault/[owner]/[context]/[doc_type] — resource list (existing)
```

A "View graph" link on the context page (`+page.svelte`) navigates to the
graph route.

### Future Consideration

A cross-context graph at `(app)/vault/[owner]/graph` (all contexts for one
owner) is a v2 consideration — not in this spec.

## API Endpoints

### Existing (already built)

| Method | Path | SQL Function | Notes |
|--------|------|-------------|-------|
| `GET` | `/api/resources/{id}/edges` | `graph_resource_edges()` | Lists edges with peer metadata |

### New Endpoints Required

**`POST /api/graph/subgraph`**

Returns a D3-consumable subgraph rooted at seed resources. This is the
primary data source for the graph canvas.

Request:
```json
{
  "seed_ids": ["uuid1", "uuid2"],
  "context_name": "temper",
  "max_depth": 2,
  "max_nodes": 100,
  "edge_types": []
}
```

If `seed_ids` is empty, the server selects the top-N most-connected
resources in the context as seeds (progressive loading entry point).

Response:
```json
{
  "nodes": [
    {
      "id": "uuid",
      "title": "Resource Title",
      "slug": "resource-slug",
      "context": "temper",
      "doc_type": "task",
      "edge_count": 5
    }
  ],
  "edges": [
    {
      "id": "edge-uuid",
      "source": "source-uuid",
      "target": "target-uuid",
      "edge_type": "depends_on",
      "weight": 1.0,
      "metadata": {}
    }
  ]
}
```

The `source` and `target` fields use `id` references (not nested objects)
so D3's `forceLink().id(d => d.id)` resolves them directly.

**Server implementation:**
1. Resolve seeds (explicit or top-N by edge count)
2. Call `graph_traverse()` with `p_profile_id`, `p_seed_ids`, `p_max_depth`,
   `p_edge_types`
3. Collect all resource IDs (seeds + traversal results)
4. Fetch resource metadata (title, slug, doc_type, context) for all nodes
5. Fetch all edges between the collected nodes
6. Cap at `max_nodes` — if traversal exceeds the limit, truncate by
   depth (prefer closer nodes)
7. Return `{ nodes, edges }`

All queries scoped through `resources_visible_to()`.

**`POST /api/graph/expand`**

Expand a single node — fetch its neighbors not already in the client's
node set. Used for double-click-to-expand interaction.

Request:
```json
{
  "resource_id": "uuid",
  "exclude_ids": ["uuid1", "uuid2"],
  "max_nodes": 20
}
```

Response: same `{ nodes, edges }` shape. The `exclude_ids` prevents
re-fetching nodes already displayed.

## Component Architecture

### Directory Structure

```
packages/temper-ui/src/lib/components/graph/
├── GraphCanvas.svelte      — D3 force simulation + SVG rendering
├── GraphControls.svelte    — Filter panel (edge types, depth, doc types)
├── GraphTooltip.svelte     — Hover tooltip with resource metadata
└── GraphLegend.svelte      — Color/shape legend for context + doc type
```

### GraphCanvas.svelte

The core visualization component. Uses D3.js v7 force simulation rendered
to SVG (not Canvas — SVG is sufficient for the ~500 node cap and gives us
CSS styling + DOM event handling for free).

```svelte
<script lang="ts">
    import * as d3 from 'd3';
    import type { GraphNode, GraphEdge } from '$lib/types';

    let { nodes, edges, onNodeClick, onNodeExpand }: {
        nodes: GraphNode[];
        edges: GraphEdge[];
        onNodeClick: (id: string) => void;
        onNodeExpand: (id: string) => void;
    } = $props();

    let svgElement: SVGSVGElement;
    let width = $state(800);
    let height = $state(600);

    $effect(() => {
        if (!svgElement || nodes.length === 0) return;
        renderGraph(svgElement, nodes, edges, width, height);
    });
</script>

<div class="graph-container" bind:clientWidth={width} bind:clientHeight={height}>
    <svg bind:this={svgElement} {width} {height}></svg>
</div>
```

**Force simulation parameters** (from R9):

```typescript
const simulation = d3.forceSimulation(nodes)
    .force('link', d3.forceLink(edges).id(d => d.id).distance(d => 100 / d.weight))
    .force('charge', d3.forceManyBody().strength(-200))
    .force('center', d3.forceCenter(width / 2, height / 2))
    .force('collision', d3.forceCollide().radius(30));
```

### Node Styling (from R9)

| Property | Mapping |
|----------|---------|
| **Color** | By context — consistent palette across the app |
| **Shape** | By doc type — circle (task), square (research), diamond (session), hexagon (goal), triangle (decision), pentagon (concept) |
| **Size** | By edge count — more connected = larger radius |
| **Label** | Truncated title, shown on hover via tooltip |

### Edge Styling (from R9)

| Property | Mapping |
|----------|---------|
| **Stroke style** | By edge type — solid (`depends_on`), dashed (`relates_to`), dotted (`references`) |
| **Thickness** | By weight (1.0 = normal, 0.5 = thin) |
| **Arrow** | Directional marker on target end |
| **Color** | Subtle gray by default, highlighted on hover |

### GraphControls.svelte

Filter panel providing:
- Edge type toggles (checkbox per type: depends_on, relates_to, extends, etc.)
- Doc type filter (show/hide by type)
- Depth slider (1-5 hops from seeds)
- Layout controls (reset zoom, re-center, pause/resume simulation)

Changing a filter triggers a re-fetch from the subgraph API with updated
parameters. The simulation re-initializes with the new data.

### GraphTooltip.svelte

Appears on node hover. Shows:
- Resource title (full, not truncated)
- Doc type badge
- Context name
- Edge count
- Click to navigate, double-click to expand

### GraphLegend.svelte

Static legend showing:
- Context → color mapping
- Doc type → shape mapping
- Edge type → stroke style mapping

## Interactions (from R9 §12.3)

| Interaction | Behavior |
|-------------|----------|
| Page load | Fetch subgraph with no seeds — server returns top-20 most-connected resources |
| Click node | Navigate to resource detail page (`/vault/[owner]/[context]/[doc_type]?resource=[id]`) |
| Double-click node | Expand: call `/api/graph/expand`, add new nodes/edges to existing graph |
| Hover node | Show tooltip with resource metadata |
| Hover edge | Highlight edge, show edge type label |
| Drag node | Reposition — simulation fixes the node position |
| Scroll/pinch | Zoom in/out via D3 zoom behavior |
| Pan | Click-drag on empty canvas space |
| Filter panel | Toggle edge types, doc types; adjust depth; re-fetch subgraph |

## Performance (from R9 §12.4)

- **Max nodes**: Cap at ~500 in the viewport. Server-side `max_nodes`
  parameter limits response size.
- **SVG rendering**: Sufficient for 500 nodes. If we later need 1000+,
  switch to Canvas renderer or WebGL (sigma.js). Not in this spec.
- **Progressive loading**: Start with most-connected resources, expand on
  demand. Never load the entire graph at once.
- **Incremental updates**: When expanding a node, append to the existing
  simulation rather than rebuilding from scratch. D3's force simulation
  handles dynamic node/edge addition.

## Search Integration

The existing search page gains a "View in graph" action. When triggered:

1. Take the search result resource IDs as seed IDs
2. Navigate to the graph route with seeds as a query parameter:
   `(app)/vault/[owner]/[context]/graph?seeds=uuid1,uuid2,...`
3. The graph page fetches the subgraph seeded with those resources
4. Search results appear as the initial focal nodes, with their graph
   neighbors loaded around them

This connects the existing search UI to the graph naturally — search finds
the starting points, the graph shows how they relate.

## SvelteKit Page Structure

### `(app)/vault/[owner]/[context]/graph/+page.server.ts`

```typescript
export async function load({ params, url, locals }) {
    const { owner, context } = params;
    const seeds = url.searchParams.get('seeds')?.split(',') ?? [];

    // Fetch initial subgraph via API
    const subgraph = await apiPost(locals.token, '/api/graph/subgraph', {
        seed_ids: seeds,
        context_name: context,
        max_depth: 2,
        max_nodes: 100,
        edge_types: [],
    });

    return { owner, context, subgraph };
}
```

### `(app)/vault/[owner]/[context]/graph/+page.svelte`

```svelte
<script lang="ts">
    import type { PageData } from './$types';
    import GraphCanvas from '$lib/components/graph/GraphCanvas.svelte';
    import GraphControls from '$lib/components/graph/GraphControls.svelte';
    import GraphLegend from '$lib/components/graph/GraphLegend.svelte';
    import RuleHeading from '$lib/components/RuleHeading.svelte';

    let { data }: { data: PageData } = $props();

    let nodes = $state(data.subgraph.nodes);
    let edges = $state(data.subgraph.edges);

    async function handleExpand(resourceId: string) {
        const expansion = await fetch('/api/graph/expand', {
            method: 'POST',
            body: JSON.stringify({
                resource_id: resourceId,
                exclude_ids: nodes.map(n => n.id),
                max_nodes: 20,
            }),
        });
        const { nodes: newNodes, edges: newEdges } = await expansion.json();
        nodes = [...nodes, ...newNodes];
        edges = [...edges, ...newEdges];
    }
</script>

<svelte:head>
    <title>Graph — {data.context} — temper</title>
</svelte:head>

<div class="flex h-full">
    <div class="flex-1 relative">
        <GraphCanvas {nodes} {edges}
            onNodeClick={(id) => goto(`/vault/${data.owner}/${data.context}?resource=${id}`)}
            onNodeExpand={handleExpand}
        />
        <GraphLegend />
    </div>
    <GraphControls />
</div>
```

## Dependencies

### New Dependencies for `packages/temper-ui`

```json
{
  "dependencies": {
    "d3": "^7"
  },
  "devDependencies": {
    "@types/d3": "^7"
  }
}
```

D3 v7 is the only new dependency. It's tree-shakeable — import only the
modules needed (`d3-force`, `d3-selection`, `d3-zoom`, `d3-scale`,
`d3-shape`).

### Codebase Dependencies

- R7 SQL functions (done): `graph_traverse()`, `graph_neighbors()`,
  `graph_resource_edges()`
- Edge service (done): `list_resource_edges()`
- `GET /api/resources/{id}/edges` handler (done)
- Known open fields registry (Spec 2, Part A)
- Seeded vault data (Spec 2, Part B) — the graph needs edges to display

## TypeScript Types

These types should be code-generated from Rust via `ts-rs` (consistent
with the existing type generation pipeline):

```rust
// In temper-core/src/types/graph.rs
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct GraphNode {
    pub id: Uuid,
    pub title: String,
    pub slug: Option<String>,
    pub context: String,
    pub doc_type: String,
    pub edge_count: i32,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct GraphEdge {
    pub id: Uuid,
    pub source: Uuid,
    pub target: Uuid,
    pub edge_type: String,
    pub weight: f64,
    pub metadata: serde_json::Value,
}

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct SubgraphResponse {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}
```

Run `cargo make generate-ts-types` to produce the TypeScript equivalents
consumed by the SvelteKit app.

## Scope Boundary

**In scope:**
- Graph route at `[owner]/[context]/graph`
- Subgraph and expand API endpoints
- GraphCanvas, GraphControls, GraphTooltip, GraphLegend components
- D3 force simulation with node/edge styling
- Click-to-navigate, double-click-to-expand, drag, zoom, pan
- Filter panel (edge types, doc types, depth)
- Search integration ("View in graph" action)
- Owner boundary enforcement

**Out of scope:**
- Cross-context graph view (`[owner]/graph`) — v2
- Resource detail page with edges panel — separate task
- Manual edge creation/deletion UI — separate task
- WebGL/Canvas renderer for large graphs — performance optimization if needed
- Minimap — nice-to-have, not required for v1
- Right-click context menu — click + double-click is sufficient for v1

## Testing Strategy

- **Component tests**: GraphCanvas renders nodes/edges correctly with mock
  data; filter changes re-render; expand adds nodes without duplicates
- **API integration tests**: subgraph endpoint returns correct nodes/edges
  for seed IDs; respects max_nodes; owner boundary enforced; expand
  excludes existing IDs
- **E2E test**: seed vault with `temper graph build`, navigate to graph
  route, verify nodes appear, click to expand, verify new nodes load
- **Visual testing**: manual verification of layout quality, node/edge
  styling, interaction responsiveness

## Estimated Effort

Large — multi-session:
1. API endpoints (subgraph + expand) + Rust types (~1 session)
2. GraphCanvas component + D3 force simulation (~1-2 sessions)
3. GraphControls + GraphTooltip + GraphLegend (~1 session)
4. Search integration + routing + page wiring (~1 session)
5. Testing + polish (~1 session)
