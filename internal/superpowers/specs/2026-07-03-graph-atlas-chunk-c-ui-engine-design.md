# Graph Atlas — Chunk C — Atlas UI Engine — Design

**Date:** 2026-07-03
**Status:** Design — approved, pending spec review
**Goal:** `graph-atlas-visualization` (`019f28a1-03f2-7aa1-a367-f6f8db8b0e7f`) — roadmap Chunk C
**Parent spec:** [`2026-07-03-temper-ui-graph-visualization-atlas-design.md`](2026-07-03-temper-ui-graph-visualization-atlas-design.md) (object model, reads R1–R5, wire types, tiers, encoding grammar)
**Visual targets:** [`design-system/preview/graph-atlas/`](../../../design-system/preview/graph-atlas/) — `visual-direction.html` (Atlas ✓), `semantic-zoom.html` (three tiers)
**Mode / effort:** build / large

## What this spec covers

Chunk C is the temper-ui **Atlas UI engine** — the client that consumes the Chunk-A/B reads
(all shipped: R1 team-scope + descendant-zones, R2 territory overview, R3 territory slice, R4
neighborhood slice, R5 element trail) and renders the team-scoped, cross-home, semantic-zoom
graph the parent spec designed. The parent spec fixed *what* renders; this spec fixes *how* we
build it — the five implementation decisions taken in the 2026-07-03 Chunk-C brainstorm, the
palette, the component/module architecture, and the build phasing.

### Grounding — everything Atlas is greenfield

The current `lib/graph/` module and all five `lib/components/graph/` components are the **old
context-scoped stack**, wired to `GET /api/graph/subgraph` and consuming the old
`SubgraphResponse`/`GraphNode`/`GraphEdge` types. The Chunk-B wire types (`AtlasSubgraph`,
`TerritoryOverview`, `TeamScopeView`, `EventTrail`, …) are generated and present but referenced
by **no** route, component, or lib module. Chunk C therefore builds a **new Atlas stack
alongside** the old one; Chunk D deletes the old route wholesale (`/[owner]/[context]/graph`
plus `/api/graph/subgraph`). We salvage only pure Tier-2 helpers as *math*, never as rendering.

## Decisions

### D1 — Renderer: d3 toolkit (math) + Svelte-rendered SVG (one substrate, all tiers)

Cytoscape.js + fcose is **retired**. All three tiers render to a single Svelte-managed SVG
substrate; d3 supplies the math and interaction primitives, each fit-for-purpose:

| Atlas need | d3 primitive |
|---|---|
| Territory sizing/packing (Tiers 0–1) | `d3-hierarchy` (pack / treemap) |
| Region hull overlays | `d3-polygon` (convex hull) |
| Tier-2 neighborhood layout | `d3-force` — the **only** place force runs |
| Camera (pan/zoom of the active viewport) | `d3-zoom` |
| Palette scales / salience ramp | `d3-scale` |
| Curved / encoded edges | `d3-shape` |

**Rationale.** The primary maintainer is Claude across many sessions, so the decision axis is
legibility / testability / composability, not raw library power. The modern *d3-for-math,
Svelte-for-DOM* pattern (d3 computes positions/scales/hulls as pure functions; Svelte renders
SVG reactively with `{#each}`) sidesteps d3's one ergonomic wart (imperative
`selection.enter().append()`) and makes the data→pixel mapping explicit and unit-testable —
extending the repo's existing "test the lib, not the render" habit to *more* of the surface,
because layout is now ours rather than inside Cytoscape. The three tiers want different
layouts (cartographic aggregate vs. force-directed); one graph library would fight the
cartographic tiers. Import only the d3 submodules used; d3's API is stable and ubiquitous in
training data.

**Escape hatch (YAGNI now).** SVG is smooth into the low hundreds of nodes; the bounded-tier
design guarantees Tier 2 never exceeds that. If a future need pushes Tier 2 to thousands, swap
**only** the Tier-2 renderer to Canvas behind the same data interface.

### D2 — Navigation: explicit click-to-enter drill only; zoom decoupled from tier

The reads are **targeted** — Tier 1 needs a territory id, Tier 2 needs seed ids; only Tier 0 is
target-free. So a tier transition is never "zoom crossed a threshold" (which would have to
*guess* a target from the viewport centre); it is an explicit act with an unambiguous target.

- **Drill / enter is the only tier transition, and it is always an explicit click.** Click a
  territory → Tier 1 (target = that territory). Click a cluster / salient node → Tier 2 (seeds
  = that node). A breadcrumb ascends.
- **Zoom is within-tier observability only** and never changes tier. Mixing continuous zoom
  (observability) with discrete drill (bounded scale) is a metaphor collision; keeping them
  separate also *removes* all threshold-detection, debounce, viewport-centre inference, and
  zoom-triggered refetch from the controller.
- **Re-scope vs. drill are distinct.** Entering a **team zone** re-scopes to that child team
  (new scope T → full Tier-0 refetch, URL `team` changes; asymmetric per the parent spec — a
  child's interior is only visible once entered). Entering a **territory/node** is a drill
  within the current scope.

**State model — the URI frame.** The URL holds `{ team, focus, filters }`; **tier is *derived*
from focus** (none → 0, territory → 1, node/seeds → 2) and is not stored separately. The
d3-zoom transform (pan/zoom within a tier) is ephemeral client state, not in the URL. Result:
back-button works, links are shareable, and a Claude session or e2e test can drive the whole
graph **by URL**. A tier transition collapses to one legible chain: **click sets focus → focus
determines tier → tier determines which read fires.**

### D3 — Palette direction: "Vivid Cartographer" (warm/cool by home)

The wheel is split in half so temperature alone tells you which half of the substrate you are
looking at, then hue tells you the exact doc-type and fill/outline tells you home:

- **Warm semicircle → authored / knowledge doc-types** (cogmap-homed, rendered **filled**)
- **Cool semicircle → workflow doc-types** (context-homed, rendered **outline**)

High chroma throughout (the brainstorm settled the level between the "warm/cool" and "vibrant
spectrum" candidates). The 14-hue table (final dark-canvas values):

| Doc-type | Hue | Hex | | Doc-type | Hue | Hex |
|---|---|---|---|---|---|---|
| concept | deep amber | `#e8942e` | | research | cyan | `#33b0e2` |
| fact | bright gold | `#f7c62b` | | task | emerald | `#34cf7e` |
| domain | yellow-green | `#d3d84e` | | session | green | `#7ed24a` |
| principle | orange | `#f2743a` | | goal | blue | `#3a8ae8` |
| commitment | vermilion | `#f0533f` | | decision | indigo | `#6a6ee8` |
| concern | rose | `#ef5090` | | memory | teal | `#2ec9b0` |
| theme | magenta | `#e24fc0` | | | | |
| question | violet | `#a95cf0` | | | | |

Notes captured during the exploration:
- **concept / fact / domain** are the hue-crowded warm-yellow trio (the ~10-hue distinguishability
  ceiling biting). Separated by **lightness as well as hue** — concept deepened toward
  amber-orange, fact made the peak-bright gold, domain pushed toward yellow-green. Validated by
  eye as distinct without new collision against principle/commitment.

### D4 — `goal` is cool (structural purity over legacy gold)

`goal` has historically been gold. Gold now belongs to the authored family (`fact`), and the
value of Vivid Cartographer is that **temperature is a rule you can trust at a glance** — the
first exception is where that trust starts leaking. `goal` therefore becomes cool blue
(`#3a8ae8`) in the workflow family; the legacy gold is retired.

### D5 — Theme handling: one hue set + light-mode contrast ring

**One** hue set is the single source of truth — the 14 hexes never fork per theme. On a light
canvas the saturated mid-tones hold, but the pale hues (fact-gold, domain-yellow-green,
session-green) lose contrast; light mode adds a **hairline dark contrast ring** to every dot (a
single theme-keyed CSS rule) so pale dots read. Rejected alternative: theme-tuned deepened
variants for the pale hues — better contrast but reintroduces two-values-per-type, the exact
drift the palette-consolidation paragraph below removes. Final WCAG contrast (against both canvases) is validated
through the **dataviz palette validator** at plan time.

Derived from this set, finalized in the plan: region/team **hull tints** (low-opacity washes of
the dominant member hue, or a neutral territory tint), the **salience ramp** (a `d3-scale`
sequential lightness/opacity ramp), and **edge colors** (`contradicts` → warning red; the
neutral structural edge gray). Edge *kind* = line style, *polarity* = arrowhead, *weight* =
thickness, `derived_from` = dashed cross-home bridge — carried unchanged from the parent
spec's encoding grammar.

**Palette consolidation.** The 3-place hex drift (`lib/graph/styling.ts` `NODE_COLORS`,
`app.css` `@theme --color-graph-*`, `app.css` `:root --graph-*` — today only `styling.ts` is out
of sync on research/session/concept) collapses into **one** source: `lib/graph/atlas/palette.ts`
emits the tokens and the CSS custom properties both, so SVG and CSS can never drift again.

## Component / module architecture

Pure math + pure data + pure tokens + thin reactive SVG. Everything testable-in-isolation is a
pure function; the Svelte components are thin reactive-SVG over that math.

```
lib/graph/atlas/
  nav.ts            # {team, focus, filters} state ⇄ URL; derived `tier`;
                    #   actions: enterZone / drill / ascend / setFilter        (pure, tested)
  reads.ts          # one typed fetch wrapper per R1–R5 (consume generated types); mockable
  palette.ts        # SINGLE source of truth: 14 hues + home treatment + edge styles +
                    #   salience ramp + hull tints; emits CSS custom properties too
  layout/
    packTerritories.ts   # d3-hierarchy → territory positions (Tier 0/1)         (pure, tested)
    forceNeighborhood.ts # d3-force → node positions (Tier 2)                     (pure, tested)
    hull.ts              # d3-polygon → region hull path                          (pure, tested)
  camera.ts         # d3-zoom wrapper — pan/zoom of the active viewport ONLY (never tier)

lib/components/graph/atlas/
  AtlasCanvas.svelte      # SVG root; owns the camera <g>; renders the active tier layer by
                          #   derived tier; crossfade on tier change
  TierPanorama.svelte     # Tier 0: zones + territories + orphan salient nodes + bridges (R1+R2)
  TierTerritory.svelte    # Tier 1: components + salient members + thin edges (R3)
  TierNeighborhood.svelte # Tier 2: force graph — chips + full edge grammar + hulls (R4)
  marks/                  # NodeChip · Edge · TerritoryHull · TeamZone  (tiny reusable SVG)
  ScopeBar.svelte         # team selector + ancestor breadcrumb + context/cogmap/region filters
  TrailRail.svelte        # R5 element event trail (the side rail from the mockups)
  SearchAccelerator.svelte# jump-to-element; pans the camera to the hit
  AtlasLegend.svelte      # encoding legend, rebuilt for the new grammar

routes/(app)/vault/[owner]/graph/
  +page.server.ts   # resolve profile + team scope from URL; SSR the initial tier read
  +page.svelte      # mount canvas + chrome; own the nav store
```

**Initial load (D-sub decision): SSR the first tier read** from the URL params, so a shared
`/graph?team=…&focus=…` link paints the correct view on first load (honoring the URI frame). All
subsequent drill / re-scope fetches are client-side. Rejected: client-only fetch (simpler
server, but a shared link flashes empty then loads).

### Reads consumed

| Tier / feature | Read | Wire types |
|---|---|---|
| Scope + zones (breadcrumb, enterable child zones) | R1 | `TeamScopeView`, `TeamRef`, `TeamZone` |
| Tier 0 panorama | R2 | `TerritoryOverview`, `Territory`, `OrphanNode`, `Bridge` |
| Tier 1 territory slice | R3 | `TerritorySlice`, `Component`, `RegionMember` |
| Tier 2 neighborhood | R4 | `AtlasSubgraph`, `AtlasNode`, `AtlasEdge`, `NodeHome`, `SliceRequest` |
| Trail rail | R5 | `EventTrail`, `ElementEvent`, `ElementKind`, `relationship_events` payloads |

## Build phasing

- **C1 — Foundations:** new route + `nav` (URL frame) + `palette.ts` (with the light-mode ring
  + CSS-var emission, retiring the 3-place drift) + `AtlasCanvas` shell + `camera`. Delivers a
  navigable, shareable-by-URL shell.
- **C2 — The three tiers:** `TierPanorama` (R1+R2) · `TierTerritory` (R3) · `TierNeighborhood`
  (R4 + `d3-force`) + the `marks/`. The visual payload; the three tier renderers are largely
  parallelizable (each = read wrapper already exists + layout fn + component).
- **C3 — Chrome:** `TrailRail` (R5) · `SearchAccelerator` · `AtlasLegend` · `ScopeBar` filters.

Chunk D (out of scope here) then redirects and deletes `/[owner]/[context]/graph` and
`/api/graph/subgraph`.

## Deferred / not in scope

- Global timeline scrub / accretion replay (R5 gives per-element history now) — parent-spec deferral.
- Cross-team (multi-scope union) panorama beyond DAG-enclosure navigation — parent-spec deferral.
- Chunk D migration (redirect + delete of the old route/endpoint).

## Acceptance (design phase)

- ✅ Renderer, navigation, palette, theme handling, and file decomposition are decided with rationale.
- ✅ Concrete enough that C1/C2/C3 tasks fall out with named files, reads, and wire types.
- ✅ Palette explored visually to a locked 14-hue set + theme strategy (Vivid Cartographer).
- ✅ Single-source-of-truth palette module specified, retiring the 3-place drift.
