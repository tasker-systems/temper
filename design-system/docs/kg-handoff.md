# Knowledge graph — implementation handoff

This document is the bridge between the design exploration in this project and the production code in `temper/packages/temper-ui`. It captures what was decided, why, what's proven in the prototype, and how to sequence the work.

It is intentionally short on hand-wavy principle — that all lives in the README's *"Knowledge graph — visual language"* section and the canonical preview cards. This file is the *engineering* view: what to build, in what order, and where the non-obvious contracts live.

---

## Status

- **Design:** settled. See README §*Knowledge graph — visual language* for the principles. See `preview/kg-scene-v2.html` for the visual spec and `preview/kg-language.html` for the parts catalogue.
- **Prototype:** working. `ui_kits/app/graph.html` implements the unified peek, breadcrumb traversal, standalone legend, and structural-mode rendering against a 21-node seed vault modeled on the real research chain (R7 → R11, plus the `llm-wiki` / `temper-maintenance` goals and `throughline` / `knowledge-graph` concepts).
- **Renderer:** `cytoscape.js` + `cytoscape-fcose` (force-directed with constraints). Chosen over D3 because it handles 500-node targets with viewport culling out of the box and because its stylesheet API matches the typographic-node grammar better than D3's manual rendering.

---

## Scope of this work

Production surface: `/vault/<ctx>/graph`.

One route, one hero component (`KnowledgeGraph.svelte`), one right-docked panel (`ResourcePeek.svelte`), plus the data plumbing to feed them. This is a self-contained PR package — no sidebar changes, no palette changes, no markdown renderer changes.

The two modes (`structural` / `meta-doc`) toggle exists in the prototype but only `structural` is shipping behavior. `meta-doc` stays as a stub labeled *"Emergent view — not implemented in this prototype yet"* until the Jaccard-projection work lands as a follow-up.

---

## Suggested PR sequencing

Designed so each PR is independently reviewable, shippable behind a flag, and leaves `main` in a working state.

### PR 1 — Renderer swap & data contract

**Goal:** replace the current D3 `/vault/<ctx>/graph` implementation with a Cytoscape-based renderer that draws typeset nodes. No peek panel yet, no hover states, no breadcrumb. Just the picture.

**Files**

- `packages/temper-ui/src/lib/components/graph/KnowledgeGraph.svelte` (new) — mount point, imports cytoscape + fcose, owns the `cy` instance.
- `packages/temper-ui/src/lib/graph/styling.ts` (extend) — add typographic style rules for `node.participant` / `node.aggregator` / `node.type-*`. See the `cy.style()` block in `ui_kits/app/KnowledgeGraph.jsx:93-170` for the exact rules.
- `packages/temper-ui/src/lib/graph/layout.ts` (new) — fcose config with `quality: 'proof'`, `nodeSeparation: 120`, aggregator mass of 2, `idealEdgeLength` of 110. See `ui_kits/app/KnowledgeGraph.jsx:175-195`.
- `packages/temper-ui/src/routes/(app)/vault/[owner]/graph/+page.svelte` (replace D3 component with new one).
- `packages/temper-ui/src/routes/(app)/vault/[owner]/graph/+page.server.ts` (extend) — data contract below.

**Data contract**

The graph component consumes two arrays, shape-compatible with the prototype:

```ts
type GraphNode = {
  id: string;           // stable, doctype-qualified (e.g. "goal/llm-wiki")
  type: 'research' | 'task' | 'session' | 'goal' | 'concept' | 'decision';
  aggregator: boolean;  // goal/concept/decision = true; research/task/session = false
  label: string;        // display label, already truncated/date-stripped per naming rules
  fullTitle: string;    // untruncated, for the peek
  dateStrip?: string;   // "2026-04-17" extracted from slug, optional
  stage?: string;       // for tasks: "in-progress" | "deferred" | "done", etc.
  edges: number;        // degree, precomputed server-side for font-weight tiering
};

type GraphEdge = {
  source: string;       // node id
  target: string;       // node id
  type: 'relates_to' | 'preceded_by' | 'supersedes' | 'decided_by' | 'emergent';
};
```

**Naming rules** — implemented server-side, see `ui_kits/app/graph-data.jsx` header comment. Date prefixes stripped, kebab-case preserved, `fullTitle` retained separately. A future `display_name:` frontmatter field should override.

**Session halos** — sessions are *not* graph participants. Instead, each resource node carries a precomputed `sessionCount` surfaced via `window.SESSION_COUNTS[id]`. Rendered as a small green `⌊3⌋` glyph positioned top-right of the node. In production this should come from the same query that hydrates the node's edges.

**Style knobs that matter** (from the prototype, worth lifting verbatim):

```ts
// Participant nodes
{ selector: 'node.participant',
  style: {
    'shape': 'rectangle',
    'background-opacity': 0,
    'label': 'data(label)',
    'text-valign': 'center',
    'text-halign': 'center',
    'color': 'data(fill)',
    'font-family': '"Source Serif 4", Georgia, serif',
    'font-size': 'data(fontSize)',
    'width': 'data(widthPx)',
    'height': 'data(heightPx)',
  }}

// Aggregator nodes — larger, italic, gravity wash via oversized transparent ellipse
{ selector: 'node.aggregator',
  style: {
    'shape': 'ellipse',
    'background-color': 'data(fill)',
    'background-opacity': 0.05,     // the wash
    'border-width': 0,
    'label': 'data(label)',
    'color': 'data(fill)',
    'font-family': '"Source Serif 4", Georgia, serif',
    'font-style': 'italic',
    'font-weight': 600,
    'font-size': 'data(fontSize)',   // 19 vs 13 for participants
    'width': 'data(widthPx)',
    'height': 'data(heightPx)',      // 70 vs 22
    'text-valign': 'center',
    'text-halign': 'center',
  }}
```

**Acceptance criteria**

- Graph renders with typeset nodes — no dots.
- Aggregators are visibly heavier (larger italic) and sit at the center of their clusters.
- Edges encode type via stroke-dash per the table in README.
- Layout converges within 2s for the seed vault.
- No regression in `/vault/<ctx>` navigation.

---

### PR 2 — ResourcePeek (the right-docked panel)

**Goal:** click any node, open the unified peek. Includes members/neighbors list and click-to-drill (but *not* the breadcrumb — that's PR 3). Excerpt block renders from `GRAPH_CONTENT[id].excerpt` if present.

**Files**

- `packages/temper-ui/src/lib/components/graph/ResourcePeek.svelte` (new) — the panel. Prototype lives at `ui_kits/app/ResourcePeek.jsx`; Svelte port is a near-mechanical translation.
- `packages/temper-ui/src/lib/components/graph/KnowledgeGraph.svelte` (extend) — add `peekNodeId` state, wire `cy.on('tap', 'node', ...)` and `cy.on('tap', ...)` (background click) handlers.
- `packages/temper-ui/src/lib/graph/content.ts` (new) — `GRAPH_CONTENT` map: `{ [nodeId]: { meta, excerpt } }`. Start with `meta` only (doctype, slug, edges, updated_at). Excerpts come from the resource's first 2–3 sentences, computed server-side.

**Key behaviors**

- Clicking a member/neighbor row rebinds the peek (calls the component's internal `setPeekNodeId`).
- Camera recenters on the new node with 380ms ease-in-out animation.
- Escape closes.
- Background click (outside any node) closes.
- Peek width `420px`, top offset `168px` (to clear the legend).

**Style details that are easy to miss**

- Panel has a `border-left` in `${typeColor}55` — hairline accent in the node's type color.
- Title uses `font-style: italic` when `node.aggregator`, `normal` otherwise.
- Neighbors list sorts participants first, aggregators last, then by type.

**Acceptance criteria**

- Click any node → peek opens with doctype marker, title, members/neighbors.
- Click a row in the list → peek rebinds, camera animates.
- Close button, background click, and Escape all dismiss.
- Legend at top-right is never eclipsed.

---

### PR 3 — Breadcrumb traversal

**Goal:** make the drill-path explicit. State changes from `peekNodeId: string | null` to `peekTrail: string[]`.

**Files**

- `packages/temper-ui/src/lib/components/graph/ResourcePeek.svelte` (extend) — accept `trail: string[]` prop and `onCrumbClick(i: number)` callback.
- `packages/temper-ui/src/lib/components/graph/KnowledgeGraph.svelte` (extend) — convert `peekNodeId` to `peekTrail`. Fresh click sets `[id]`; drilling appends; crumb-click slices to that depth.

**Key behaviors**

- Depth 1 (fresh click): no breadcrumb rendered.
- Depth ≥ 2: breadcrumb bar at top of peek, below doctype marker, above title.
- Aggregators in trail render in italic serif at 11px; participants in mono caps at 8.5px. The crumb announces its type.
- Trails of 5+ collapse to `first › … › penult › current`.
- Current node (last crumb) is not clickable. Earlier crumbs are buttons.
- Clicking a crumb slices `peekTrail.slice(0, i + 1)` AND animates camera to that node.
- Closing the peek resets the trail.

See `ui_kits/app/ResourcePeek.jsx:77-146` for the render logic and `ui_kits/app/KnowledgeGraph.jsx:235-260,360-390` for trail state management.

**Acceptance criteria**

- Three-level drill works: click goal → click member task → click neighbor research → breadcrumb shows all three.
- Click the middle crumb → trail and camera both return to that node.
- Close and reopen → no persistence (trail empties).

---

### PR 4 — Hover gradient & emphasis

**Goal:** on node hover, all incident edges lift to 1.1px with a source-color-to-target-color gradient. Non-incident edges dim to `0.03` alpha. Non-neighbor nodes fade to `0.35` opacity.

**Files**

- `packages/temper-ui/src/lib/components/graph/KnowledgeGraph.svelte` (extend) — `cy.on('mouseover', 'node', ...)` handler that toggles classes.
- `packages/temper-ui/src/lib/graph/styling.ts` (extend) — `.emphasized`, `.dimmed`, `.dimmed-edge` classes.

Cytoscape doesn't support per-edge linear gradients natively. The prototype approximates with solid source-color at full saturation on hover. For a proper gradient we'd need a custom renderer layer — out of scope for this PR. Document the deferral in the code comment.

**Acceptance criteria**

- Hover a node → its edges brighten, neighbors stay at full opacity, everything else dims.
- Mouseleave returns to steady state within 180ms.

---

### PR 5 — Zoom tiers & label culling

**Goal:** three-tier threshold-fade zoom behavior, as described in README §*Zoom tiers*.

**Files**

- `packages/temper-ui/src/lib/components/graph/KnowledgeGraph.svelte` (extend) — `cy.on('zoom', ...)` handler that reads `cy.zoom()` and applies `.tier-overview` / `.tier-mid` / `.tier-detail` classes to the graph root.
- `packages/temper-ui/src/lib/graph/styling.ts` (extend) — tier-gated label visibility per-class.

**Thresholds** (from prototype comments):

- `< 0.5` → overview: only aggregators labeled; participants render as short colored tick marks.
- `0.5 – 1.2` → mid: all labels on, full typography.
- `> 1.2` → detail: labels + date strips + stage tags under tasks.

Transitions are 220ms opacity fades.

**Acceptance criteria**

- Zoom out → participant labels disappear, aggregators remain.
- Zoom in past 1.2 → dates and stages appear under relevant nodes.
- No jitter at threshold boundaries.

---

### PR 6 (optional, follow-up) — meta-doc mode

**Goal:** the mode toggle at top-left actually does something. Ship the Jaccard-projection edges.

This is a bigger piece and deserves its own scoping doc. Deferred until:

1. The structural mode has shipped and been used in anger for ~2 weeks.
2. We've seen which aggregators users actually click into (inform which emergent-edge view is most valuable).
3. A decision on precompute vs on-the-fly Jaccard.

Leave the UI stub in place with the *"Emergent view — not implemented"* copy.

---

## Non-obvious contracts

### `aggregator: boolean` is the load-bearing distinction

R11 D1's entire argument — and everything downstream in the visual language — hinges on this single field. Goals, concepts, and decisions are aggregators; research, task, session are participants. Everything else (font size, italics, background wash, peek header copy) derives from this boolean. **Don't let a new doctype slip in without deciding which side it's on.**

### Sessions are annotations, not nodes

Repeating because it's easy to lose: sessions never appear as graph nodes. They only annotate other nodes via `sessionCount`. This is per R11 D4 and is the reason the graph stays legible at vault scale. If a future PR adds session participation to the graph, it should open this section of the design doc for re-review.

### Colors live in `lib/graph/styling.ts`

The existing `temper-ui/src/lib/graph/styling.ts` file owns the doctype palette. Both the graph renderer AND the peek panel read from it. Any color change is one file, one source of truth.

### The prototype is a reference, not a port

The JSX in `ui_kits/app/` uses plain React + Babel-in-browser because that's what this design system project supports. Production is Svelte 5 (`$state`, `$derived`, snippets). The component *shapes* and *state names* should match; the render syntax will differ. Don't hand-port JSX — use it as the spec, write idiomatic Svelte.

### Typography must be real Source Serif 4

The node labels read as typeset words only if the real serif font renders. If you're hosting offline or behind a firewall, self-host Source Serif 4 (`fonts/README.md` has the SIL OFL note). Falling back to Georgia works at a pinch but loses the kerning and italic detail that makes aggregators feel heavier than participants.

---

## Open questions for production

1. **Where do excerpts come from?** Prototype uses seeded `GRAPH_CONTENT`. Production needs to decide: first N sentences of the markdown body, dedicated frontmatter field, LLM summary? Defaulting to "first paragraph, up to 280 chars" is probably fine for v1.

2. **How do we handle very dense clusters?** At 500 nodes, if any single aggregator has 50+ members, the cluster overflows its gravity well and the wash stops reading. Two options: cap visible members at ~30 with a "see all" affordance in the peek, or add a cluster-collapse affordance. Recommend punting this until we have real density.

3. **Keyboard navigation.** Prototype has Escape-to-close but no tab-through of nodes. Production should think about this for accessibility — a graph visualization with no keyboard affordance is hostile. Consider: `cmd+k` opens the command palette filtered to graph nodes; arrow keys within an open peek traverse neighbors; enter drills.

4. **Right-click / context menu.** Not in scope, but likely wanted eventually: "open this resource in a new vault tab," "copy link," "expand neighbors inline." Leave a hook, don't implement.

5. **Session halo when session count is huge.** `⌊3⌋` is fine; `⌊247⌋` overflows its rendering space. Either cap displayed count (`⌊99+⌋`) or scale the glyph. Prototype doesn't handle this.

---

## What's in this project, for reference

| Artifact | Purpose |
|---|---|
| `ui_kits/app/graph.html` | Working prototype — clickable, all behaviors live. |
| `ui_kits/app/KnowledgeGraph.jsx` | Main component — Cytoscape init, styling, event handlers, trail state. |
| `ui_kits/app/ResourcePeek.jsx` | The right-docked panel — breadcrumb, members/neighbors, excerpt. |
| `ui_kits/app/graph-data.jsx` | Seed nodes + edges — models the real R7→R11 chain and the `llm-wiki` / `throughline` aggregators. |
| `ui_kits/app/graph-content.jsx` | Seed `GRAPH_CONTENT`: per-node meta + excerpt for the peek. |
| `preview/kg-language.html` | Parts catalogue: every node type, edge type, annotation glyph shown once with labels. |
| `preview/kg-scene.html` | v1 static visual spec — frame-based aggregators (rejected). Kept for the record. |
| `preview/kg-scene-v2.html` | v2 static visual spec — gravity-well aggregators (chosen). Canonical visual reference. |
| `README.md` §*Knowledge graph — visual language* | The design principles in prose. |

---

## Decision log (condensed)

| # | Decision | Rationale | Source |
|---|---|---|---|
| D1 | Participant / aggregator split | R11 research | `uploads/2026-04-13-r11-knowledge-graph-visualization-design.md` |
| D2 | Two modes: structural / meta-doc | R11 D2 | same |
| D3 | Emergent edges use Jaccard similarity | R11 D3 | same |
| D4 | Sessions annotate, not participate | R11 D4 | same |
| D5 | Renderer: cytoscape.js + fcose (not D3) | Scale target + typographic-node fit | this project, v2 exploration |
| D6 | The word is the node (no dots or glyphs) | Temper's content-first principle | README §*Visual foundations*, §*Iconography* |
| D7 | Gravity well, not frame, for aggregators | Frames introduce tabular language into a force-directed context | `preview/kg-scene-v2.html` verdict block |
| D8 | One docked panel for every node type (no expand overlay) | Consistent grammar; keeps graph visible behind panel | this project, late-iteration decision |
| D9 | Breadcrumb exposes drill path explicitly | Drilling is traversal; the UI should name it | same |
| D10 | Close resets breadcrumb (no persistence) | Traversal is current-interaction state | same |
| D11 | Legend is standalone chrome, peek docks below it | Chrome and content panels are different affordances and shouldn't collide | same |
