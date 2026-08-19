# Graph Atlas — Chunk C2 — Tier 1 & 2 renderers + drill + sparse-state — Design

**Date:** 2026-07-03
**Status:** Design — approved, pending spec review
**Goal:** `graph-atlas-visualization` (`019f28a1-03f2-7aa1-a367-f6f8db8b0e7f`) — roadmap Chunk C, phase C2
**Task:** `graph-atlas-chunk-c2-...` (`019f2a7a-e219-7913-ae22-f485ee509d69`) — mode plan / effort large
**Parent specs:**
- [`2026-07-03-temper-ui-graph-visualization-atlas-design.md`](2026-07-03-temper-ui-graph-visualization-atlas-design.md) — object model, reads R1–R5, encoding grammar, tiers
- [`2026-07-03-graph-atlas-chunk-c-ui-engine-design.md`](2026-07-03-graph-atlas-chunk-c-ui-engine-design.md) — Atlas UI engine decisions D1–D5 (renderer, nav, palette)
**Builds on:** [`2026-07-03-graph-atlas-chunk-c1-foundations.md`](../plans/2026-07-03-graph-atlas-chunk-c1-foundations.md) (shipped, PR #256) — route + `nav` + `palette` + `AtlasCanvas` + `camera` + Tier-0 `TierPanorama`.

## What this spec covers

C2 makes the Atlas **real and interactive**. C1 shipped a navigable Tier-0-only shell; C2 adds the
two remaining tier renderers (Tier 1 region interior, Tier 2 force-directed neighborhood), wires
click-to-drill across every element, turns the sparse / all-orphan state into a first-class
interactive view, resolves the four C1-deferred items, and relocates the route. **C2 is entirely
frontend** — it consumes reads that already shipped (Chunk B: R1–R5) and adds no backend. The
backend-heavy canonical membership home is deliberately split into its own follow-on chunk
(**Atlas Home**, recorded below).

This spec was designed *against the real canvas* — C1 was left unplanned past Tier 0 by design so
C2 could be grounded in what actually ships, and in the verified Chunk-B read contracts (below)
rather than the parent spec's intentions.

### Real-canvas grounding

C1 verified live in prod at `/vault/@me/graph`: route + auth + ScopeBar + Tier-0 render all work.
But the default team's data is a single **region-less L0 kernel cogmap with no materialize run**, so
R2 returns every facet as an `orphan_node`, `territories` is empty, and the panorama renders only
the sparsity fallback — an inert, overflowing single grey column with no drill affordance. Correct
behavior for the data shape, but it exposed the seam C2 must own: **the sparse state must be a
first-class, bounded, interactive view**, and drilling must reach the Tier-2 neighborhood where a
facet's `relationship_assert` edges actually live.

### Verified backend contract (the ground truth C2 is designed against)

| Read | Contract facts that constrain C2 |
|---|---|
| **R2** territories | Emits only `region` + `context` territories (never `cogmap`). `region.anchor_id = cogmap_id`, `salience = Some`; `context.anchor_id = context_id`, `salience = None`. `OrphanNode.anchor_id = the cogmap it is homed in`; orphans appear **only** when the home cogmap has no materialized region; capped 50, ranked by degree. |
| **R3** region slice | `{region_id}` accepts **only a materialized region id** → a context/cogmap id **404s**. `components` and `members` are **two independent flat lists** — no member→component mapping. **No edges.** `RegionMember` has **no `home` field**. Members cap 100, ordered `affinity` desc. |
| **R4** neighborhood | `seeds` = any in-scope resource id (orphan / member / node all valid). Walk is **directed / successors-only** — depth 1 = out-neighbors, not a full ring. Edges are the directed-reachable subset (a node's `degree` may exceed returned edges). `label` carries the human relation word (`derived_from`, …); `edge_kind` is only the 4 coarse kinds. **`salience` is always `None`** at this tier — `degree` is the only node-size signal. |
| membership | `list_teams` returns the caller's direct member teams (id/slug/name/description, **no counts**). `kb_team_cogmaps` is **many-to-many** (a cogmap belongs to 0..N teams). No read lists a profile's cogmaps or a team's cogmaps; no cogmap-scoped panorama read exists (only the SQL set-predicate `cogmap_visible_maps`). |

## Decisions

### C2-D1 — Route relocation: `/graph/[owner]`

Move the Atlas route out of the vault tree: `/vault/[owner]/graph` → `/graph/[owner]`. The vault tree
already carries `/vault/[owner]/[context]`, and a sibling static `graph` segment both overloads that
namespace and blocks any context literally named `graph`. A top-level `/graph/[owner]` section
removes the collision. Impact is mechanical: relocate the route dir, update the two internal links
and the `nav.test` base URL. The `nav.ts` builders are already pathname-relative (`${pathname}${search}`),
so their **logic is untouched** — only the pathname they run at changes.

### C2-D2 — Canonical `@me` home: null-team is the membership view, not a silent dive

Today a missing `?team` silently falls back to `listTeams()[0]` and dives into that one team's Tier-0
— an arbitrary, invisible choice (the M4 wart). C2 makes **the null-team state a real, renderable
view**: `/graph/@me` with no `?team` renders **`TierHome`**, and `/graph/@me?team=X` renders team X's
Tier-0.

The full home is the **membership graph** (`you → teams → cogmaps`, edges = `kb_team_cogmaps`) — chosen
because teams⋈cogmaps is many-to-many and a profile participates in cogmaps *through* teams: a
hierarchy (cogmaps nested under one team) would be a lie, and a flat wall would drop the "reached
through a team" relationship. The membership graph encodes both truths (a shared cogmap simply shows
two team edges).

**C2 ships the `you → teams` half only** — on the same `AtlasCanvas` substrate, so the Atlas Home
chunk later grows the cogmap column onto it with no throwaway. This reuses `list_teams` and adds **no
backend**. Team nodes are doors → `?team=X`. The cogmap column + counts + the enter-a-cogmap read are
the Atlas Home chunk (recorded under *Deferred*).

C2 team nodes are **count-free** (`list_teams` carries no counts; the size hints in the mockups are
the Atlas Home chunk). New nav builder `buildHomeUrl` clears both `team` and `focus`; ascend from a
team's Tier-0 returns to the home.

### C2-D3 — Drill semantics (backend-forced)

`tier` stays derived from `focus` (C1's model). Click-to-enter wiring per element:

- **Region territory → Tier 1** (R3, `focus=territory:<region_id>`). Only region-kind territories drill
  — R3 404s on a context/cogmap id.
- **Context territory → non-drillable.** No read takes a context id to show its interior; rendered but
  inert (no false affordance).
- **Orphan node → Tier 2** (`focus=node:<orphan_id>`), **region member → Tier 2**, **Tier-2 node → Tier 2**
  (re-seed / re-center on each click). All valid R4 seeds.
- **Sparse cogmap hull** is a frame, not a door; its **facet dots are the doors** (→ Tier 2).

Focus is a single value, not a stack: explicit **ascend** (`buildAscendUrl`) clears focus → Tier 0;
finer back-steps are the browser back button (the URL frame makes this free). No focus stack (YAGNI).

### C2-D4 — Sparse / all-orphan Tier-0: cogmap-as-territory

Group `orphan_nodes` by their `anchor_id` (the cogmap they're homed in) into **synthetic cogmap
territories**, and pack each cogmap's facet dots *inside* its dashed hull — the same cartographic
language as dense region territories. This simultaneously:
- **bounds the layout** (resolves M5 — no more off-canvas single column),
- makes every facet **clickable → its Tier-2 neighborhood** (the new C2 requirement; a facet's
  `relationship_assert` edges are a real little graph reachable via R4),
- **generalizes**: several region-less cogmaps render as several labeled circles, visually
  consistent with dense territories.

For the real prod L0 case this yields one `system-default` circle of clickable facet dots instead of
today's inert list.

### C2-D5 — Tier 1 renderer (region interior): members-focused, honest about the missing mapping

R3 gives components and members as two **unlinked** flat lists and no edges. Rendering component
bubbles *containing* members would imply a containment the data can't back. So:

- **Members are the payload**: packed inside the region hull, sized/centred by `affinity` (top 100).
- **Components are demoted to an honest badge** ("N sub-clusters") — the structural signal without a
  false spatial claim.
- Warm=filled / cool=outline **by doc-type family** (`isAuthored`) since `RegionMember` carries no
  `home`.
- **No edges** at this tier (edges are Tier 2, per D1 of the parent spec).
- Clicking a member → Tier 2.

### C2-D6 — Tier 2 renderer (neighborhood): d3-force + full edge grammar

The only tier where force runs (spec D1). R4 `AtlasSubgraph` → `forceNeighborhood` layout.

- **Nodes:** hue = doc-type, fill/outline = `home` (real field here), **size = `degree`**. Salience is
  `None` at Tier 2, so the parent spec's "salience → opacity" ramp is a **Tier-0-only signal** — C2
  documents this rather than faking a uniform-floor opacity. The seed node is enlarged + ringed.
- **Edges (spec-fixed grammar):** `edge_kind` → line style (contains=solid, leads_to=dashed,
  express=dotted, near=thin/light), `polarity` → arrowhead, `weight` → thickness,
  `label === "derived_from"` → dashed provenance bridge in its own colour, `label === "contradicts"`
  → warning red (rare — not in the canonical relation set; honored if asserted as a raw label), else
  structural gray.
- **Edge labels on hover/select only** — the grammar is dense; a quiet canvas reveals a relation word
  on the touched edge (the C3 TrailRail then shows that element's full history). Every-edge-labelled
  crowds at ~6 edges; real neighborhoods are bushier.
- **Drill depth default = 2** (the walk is directed/successors-only, so depth 1 often looks bare);
  the drill may offer 1–3.
- Clicking a neighbor re-seeds the neighborhood (re-center).

### C2-D7 — The four C1-deferred items

- **I2 (region sizing):** regions sized by `salience`, contexts by `member_count` (matches the wire
  contract: region carries salience, context carries member_count), normalized onto one pack scale.
  Changes the C1 Task-4 test + the Tier-0 visual.
- **M4 (owner param):** `[owner]` is `@me`-canonical — display-only, scope is team-driven; no
  `owner→team` resolution. Subsumed by C2-D2.
- **M5 (unbounded orphan layout):** resolved by the cogmap-as-territory packing (C2-D4).
- **M6 (camera persists across re-scope):** reset the d3-zoom transform whenever the rendered content
  changes fundamentally — `{#key}` the `AtlasCanvas` on a `teamId + focus` composite so it remounts
  (zoom is within-tier ephemeral, per spec D2).

## Component / module architecture

Extends C1's `lib/graph/atlas/` (pure math) + `lib/components/graph/atlas/` (thin reactive SVG).

```
lib/graph/atlas/
  nav.ts                         # + buildHomeUrl (clear team+focus); home = null-team state
  palette.ts                     # + edgeStyle(edge) → { dash, color, width, marker } (consume EDGE_COLORS)
  layout/
    cogmapTerritories.ts         # group orphan_nodes by anchor_id → synthetic cogmap territories;
                                 #   pack facets inside each hull                          (pure, tested)
    regionInterior.ts            # pack region members by affinity inside the hull (Tier 1)(pure, tested)
    forceNeighborhood.ts         # d3-force → node/edge positions (Tier 2)                 (pure-ish, tested on the deterministic parts)
    hull.ts                      # d3-polygon convex hull → region/neighborhood outline    (pure, tested)
    homeLayout.ts                # you → team node positions (deterministic columns)       (pure, tested)

lib/components/graph/atlas/
  TierHome.svelte                # null-team: you-node + team-doors (Atlas Home grows the cogmap column)
  TierTerritory.svelte           # Tier 1: region hull + affinity-packed member chips + sub-cluster badge
  TierNeighborhood.svelte        # Tier 2: force graph — node chips + typed edges + hull
  marks/
    NodeChip.svelte              # hue/home/degree node mark (Tier 2)
    Edge.svelte                  # edge grammar mark (line style / arrowhead / thickness / label-on-hover)
    RegionHull.svelte            # dashed hull outline + label
    MemberChip.svelte            # Tier-1 member dot + title
  AtlasCanvas.svelte             # + tier dispatch for home/1/2; {#key teamId+focus} remount (M6)

routes/(app)/graph/[owner]/      # relocated from vault/[owner]/graph
  +page.server.ts                # branch: null-team → home; tier 0/1/2 → R2 / R3 / R4
  +page.svelte                   # shell; ScopeBar only when scoped to a team (home has no TeamScopeView → minimal "Home" header)
```

**Load branching** (`+page.server.ts`): no `?team` → `listTeams` (home, no territories); tier 0 →
`readTerritories`; tier 1 → `readRegionSlice(focus.id)`; tier 2 →
`readNeighborhood(teamId, { seeds:[focus.id], depth:2, edge_kinds:[] })`.

**New d3 deps:** `d3-force`, `d3-polygon` (+ `d3-shape` if curved edges are wanted). Import named
submodules only (spec D1).

### Reads consumed

| Tier / feature | Read | Wire types |
|---|---|---|
| Home (you → teams) | `list_teams` | `TeamRow` |
| Tier 0 panorama (+ sparse) | R2 | `TerritoryOverview`, `Territory`, `OrphanNode` |
| Tier 1 region interior | R3 | `TerritorySlice`, `Component`, `RegionMember` |
| Tier 2 neighborhood | R4 | `AtlasSubgraph`, `AtlasNode`, `AtlasEdge`, `SliceRequest` |

## Constraints (carried from C1)

Svelte 5 runes only, **no `$effect`**; vitest node-env pure-fn tests only (components verified by
running the app); generated wire types read-only; server-only reads via `$lib/server/graph-reads.ts`;
URL-as-truth nav via `goto(url, { replaceState: true })`; single-source `palette.ts` (no color drift
outside it — D5 holds); d3 named submodules only; **force runs ONLY on Tier 2** (spec D1); tabs
indentation.

## Deferred / not in scope

### → New chunk: **Atlas Home** (the rich membership home)

Recorded here so it becomes a clean task. Grows the C2 `you → teams` home into the full membership
graph. **Decisions already made (this session):**
- **Model = membership graph A** (`you → teams → cogmaps`, edges = `kb_team_cogmaps`; a shared cogmap
  shows multiple team edges — no hierarchy).
- **"Enter a cogmap" = cogmap as its own place** — clicking a cogmap door shows *that cogmap's*
  interior directly, team-independent. Forced by the data: a cogmap belongs to 0..N teams and may
  have no region, so "shortcut into its team" is ill-defined.

**Net-new backend it requires** (none of which C2 touches):
- A **list-cogmaps-for-profile** read (service + route + ts-rs type) — the SQL set-predicate
  `cogmap_visible_maps(profile)` already exists and is reusable; nothing wraps it over HTTP.
- **Counts** for home cards: per-team resource count (the `resources_in_team_scope` count idiom
  exists but isn't wired onto `/api/teams`), per-team cogmap count, per-cogmap region/resource/facet
  counts (all net-new).
- A **cogmap-scoped panorama read** (enter-a-cogmap) — no read accepts a `cogmap_id` to render a map's
  interior today; a cogmap is only ever surfaced as region territories inside a team's R2.

### → Chunk C3 (unchanged from the C-chunk spec)

TrailRail (R5 element history), SearchAccelerator, AtlasLegend, ScopeBar filters (context/cogmap/
region/lens).

### → Chunk D (unchanged)

Redirect + delete the old `/vault/[owner]/[context]/graph` route and `/api/graph/subgraph`.

### Followups / notes

- **Directed-BFS neighborhoods:** R4 returns successors only, so a node whose edges are mostly
  incoming looks sparse at Tier 2. C2 mitigates with depth-2 default; a future "undirected
  neighborhood" option would be a backend change (out of scope).
- **`contradicts` / `supports` edges** are not in the canonical relation set; the palette honors them
  if asserted as raw labels, but they will rarely appear.

## Acceptance criteria

- Clicking a region territory drills to a legible Tier-1 interior (R3: affinity-packed members + hull
  + sub-cluster badge); clicking a node/orphan/member drills to a force-directed Tier-2 neighborhood
  (R4: node chips + typed edges, labels on touch); ascend + browser-back move up. URL frame carries
  focus; the load fetches the right read per tier.
- The sparse / all-orphan Tier-0 state is **interactive** (facet dots clickable → Tier 2) and
  **bounded/legible** (cogmap-as-territory packing) — verified against the *real* prod-shape L0 data,
  not only a dense seed.
- `/graph/@me` with no `?team` renders the `you → teams` home; team nodes enter their Tier-0; no
  arbitrary `teams[0]` dive.
- Palette hues + light-mode ring + the edge grammar defined in `palette.ts` are consumed by the Tier-2
  marks; no new color outside `palette.ts` (D5 holds).
- The four C1-deferred items (I2, M4, M5, M6) are each resolved as above.
- Route lives at `/graph/[owner]`; old `/vault/[owner]/graph` is gone (its Chunk-D-era siblings
  untouched).
- `bun run check` 0 errors; `vitest` green; production build OK; browser-verified in the authed env.
