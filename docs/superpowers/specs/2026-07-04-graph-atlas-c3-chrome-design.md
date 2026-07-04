# Graph Atlas C3 — Atlas chrome design

**Goal:** Graph Atlas — team-scoped, cross-home, historical graph visualization (`019f28a1`).
**Task:** Graph Atlas C3 — Atlas chrome (`019f2def`).
**Predecessors:** Chunk A (R1), Chunk B (R2–R5 reads + wire types), Chunk C2 (tier renderers +
drill + sparse state), Atlas Home (PR #262). All merged; tree clean at `a8858fe6`.
**Parent spec:** [`2026-07-03-temper-ui-graph-visualization-atlas-design.md`](2026-07-03-temper-ui-graph-visualization-atlas-design.md).

## Throughline

The Atlas map engine ships: a team-scoped route, three semantic-zoom tiers, cogmap doors,
sparse-state territories, and a consolidated dark palette. What it lacks is **chrome** — the
surrounding controls that make the map navigable and legible. C3 adds four largely-independent
siblings layered over the shipped canvas, plus the one thin backend read that search needs:

- **TrailRail** — surfaces the per-element event-trail (R5, shipped but unconsumed) for a
  selected node *or edge*.
- **SearchAccelerator** — locate a node by name across the team scope and camera-jump to it,
  reusing the existing search substrate's ranking verbatim.
- **AtlasLegend** — the visual key for the encoding grammar, sourced entirely from `palette.ts`.
- **ScopeBar filters** — lens / edge-kind / doc-type controls layered onto the C1
  breadcrumb-only shell.

C3 is **additive over shipped reads, wire types, and the palette**. It is chrome-layer plus one
thin composed read — no new foundational data modeling, no new ranking model.

## Design decisions (resolved in brainstorm)

1. **Layout — left dock (IDE).** A narrow persistent left sidebar holds Search, Filters, and a
   collapsible Legend; the map takes the remaining width; TrailRail is a right panel that
   appears **only on selection** (not permanent). Chosen over a cartographic floating overlay
   and a framed cockpit for its always-visible, discoverable affordances. Keep the dock narrow
   and the legend collapsible so an unselected map keeps near-full width.
2. **State stays in the URL frame.** No new Svelte stores/context. Selection, scope, tier, and
   filters all live in `$page.url` and route through `nav.ts` builders — consistent with the
   shipped engine (the URL *is* the state model).
3. **Theme — dark-only, palette theme-ready.** The app shell is hardcoded dark (`bg-zinc-950`)
   and has no light mode. C3 builds all chrome dark-only. `palette.ts` remains the single source
   of truth; the legend consumes its tokens (`--dt-*` CSS vars) rather than re-declaring hex, so
   the palette stays theme-ready — but **no light-mode path is built** (a whole-app theming
   concern, deferred). The `LIGHT_MODE_RING` stub stays unused.
4. **Palette shipped as-is.** The "Vivid Cartographer" palette is treated as done; C3 does **not**
   tune hues (the goal's "vibrancy pass"). The legend merely exposes it. Revisit vibrancy only if
   the legend surfaces a real legibility gap.
5. **TrailRail covers nodes and edges.** R5 serves trails for both; the spec's headline trail
   example (`asserted → reweighted → folded`) is an edge lifecycle. C3 adds the missing
   edge-selection surface so R5 is fully consumed. **Two plan-grounding refinements** (see below):
   (a) edge selection uses an **orthogonal `?sel=edge:<id>` param**, not `?focus=edge:` — because
   `?focus` also *seeds the Tier-2 neighborhood* (a node), which an edge cannot; `?sel` drives only
   the panel and leaves the loaded neighborhood intact. (b) The rendered `AtlasEdge` carries **no
   `id`** today, so edge trails need a small backend prerequisite: add `id` to `AtlasEdge` +
   `graph_traverse_scoped` + the `neighborhood_slice` mapping (a new additive migration that
   DROP/CREATEs the traversal fn with the extra column — the shipped migration stays immutable).
6. **TrailRail is the Atlas evolution of `ResourcePeek`, not a bare event list.** The old
   Cytoscape stack shipped a well-considered resource-level detail panel
   (`ResourcePeek.svelte`); C3 borrows its content vocabulary — doctype-hue hairline aside,
   mono-cap doctype header, serif title, neighbors list, metadata rows, excerpt, ESC/scrim close,
   slide-in — and adds the **R5 event trail as a first-class new "History" section**. The
   selected-element panel *is* TrailRail; the history is its defining new capability.
7. **Search reuses the substrate, does not reinvent it.** `unified_search` is scope-agnostic and
   already takes a `p_scope_ids` bound; SearchAccelerator is a third scope-resolution front-end
   over the same blend — no new ranking SQL, no new weights.
8. **Filter dimensions: lens + edge-kind + doc-type.** Context-as-filter is **deferred to Chunk
   D**, which owns the "redirect old `/[owner]/[context]/graph` with context pre-filtered"
   migration. If all three strain the effort, edge-kind (the heaviest — it threads through the
   Tier-2 slice) breaks out first.

## Grounding — what already exists (do not rebuild)

**R5 element trail (shipped, unconsumed):**
- Wire types `ElementEvent`, `EventTrail`, `ElementKind` (`node`|`edge`) —
  `crates/temper-core/src/types/element_trail.rs`; generated TS at
  `packages/temper-ui/src/lib/types/generated/element_trail.ts`.
- Endpoint `GET /api/graph/elements/{kind}/{id}/trail` — `crates/temper-api/src/routes.rs:222`,
  handler `crates/temper-api/src/handlers/events.rs::element_trail`.
- Service `temper_services::services::event_service::element_trail` → SQL `element_trail_node` /
  `element_trail_edge` (`migrations/20260703130000_graph_atlas_chunk_b_reads.sql`), each fully
  visibility-gated, ordered by event id.
- Frontend wrapper `readTrail(token, kind, id)` +
  `trailPath` — `packages/temper-ui/src/lib/server/graph-reads.ts:32,64`. **No UI consumes it
  yet — TrailRail is the first.**
- **Gotchas:** (a) the trail is unbounded server-side (no limit/cursor); (b) `ElementEvent.confidence`
  is Rust-optional but ts-rs emits `confidence: string | null` (required key) — handle at the
  UI boundary.

**Atlas UI surface:**
- One route `src/routes/(app)/graph/[owner]/` (`+page.server.ts`, `+page.svelte`); Atlas Home is
  its zero-param state.
- `AtlasCanvas.svelte` (tier dispatch), `TierHome/Panorama/Territory/Neighborhood.svelte`,
  `marks/{NodeChip,Edge,MemberChip,OrphanNodeMark,TeamZoneMark,TerritoryCircle}.svelte`.
- `ScopeBar.svelte` — **breadcrumb only** (`⌂ Atlas` + ancestors + team name); no filters yet.
- `nav.ts` — the URL frame: `Focus = {kind:'none'|'territory'|'node', id}`, `parseFocus`,
  `deriveTier`, `parseTeam/parseCogmap/parseFilters` (`?lens_id`), builders
  `buildScopeUrl/buildCogmapUrl/buildDrillTerritoryUrl/buildDrillNodeUrl/buildAscendUrl/buildHomeUrl`.
  **No edge focus today.**
- `palette.ts` — single source of truth ("Vivid Cartographer", dark-only): `DOC_TYPE_HUES` (14),
  `docTypeHue`, `isAuthored`, `nodeMark`, `EDGE_COLORS`, `edgeStyle`, `TERRITORY_TINTS`,
  `TEAM_DOOR`, `COGMAP_DOOR`, `salienceOpacity`, `paletteStyleVars()` (emits `--dt-*`).
- Tests: pure-logic modules only (`nav.test.ts`, `palette.test.ts`, `layout/*.test.ts`); no
  `.svelte` component tests. Established pattern: unit-test the pure functions.
- `?lens_id` is parsed server-side and threaded to `readTerritories` but has **no UI control** —
  the anchor seam for the filter bar.

**Old resource-level detail panel (borrow, don't rebuild):**
- `packages/temper-ui/src/lib/components/graph/ResourcePeek.svelte` — the Cytoscape stack's
  selected-resource aside: right slide-in (`peekSlide` 240ms), `backdrop-blur`, doctype-hue
  hairline left border (`{hue}55`), scrim click-to-close + ESC. Structure: mono-cap doctype header
  (`PARTICIPANT · <doc_type>` / `AGGREGATOR · …`) → serif hue title (italic for aggregators) →
  session strip → **neighbors/members list** (dir glyph `→`/`←` · mono-cap edge type · serif
  neighbor title, click-to-focus) → metadata rows (DOCTYPE/SLUG/EDGES/DATE) → excerpt → footer
  (`ESC · CLOSE` + `OPEN RESOURCE →`).
- `packages/temper-ui/src/lib/graph/peek.ts` — `buildNeighborEntries(focusId, nodes, edges)`, the
  pure, deterministically-sorted neighbors builder (participants before aggregators, then edge
  type, then title). Reusable directly against the loaded Tier-2 slice.
- **Field gap to bridge:** the old `GraphNode` carried `slug`, `excerpt`, `session_count`,
  `edge_count`, dates, `aggregator`; the Atlas `AtlasNode` is leaner (`id, title, doc_type, home,
  degree, salience`). A ResourcePeek-grade node panel needs the richer identity/excerpt/metadata —
  see TrailRail's data plan.

**Search substrate (reuse target):**
- `unified_search` SQL fn — `migrations/20260626000002_search_beat2_surface_a.sql` (+ scope-ids
  extension `20260629000004_search_scope_ids.sql`); called via runtime `query_as` at
  `crates/temper-substrate/src/readback/mod.rs:1083`. Blend = weighted sum
  `w_fts 1.0 · fts_norm + w_vec 1.0 · vec_norm + w_graph 0.5 · graph_score`, `γ 0.5`,
  `ORDER BY combined_score DESC, id LIMIT`. Weights are SQL-resident constants in one `k` CTE —
  never API params. Visibility enforced inside each candidate fn via `resources_visible_to`.
  Accepts `p_scope_ids uuid[]` — the generalized scope bound.
- `resources_in_team_scope(profile, team)` (`migrations/20260703000002`) — the visibility-gated,
  team-scoped resource-id set the Atlas canvas already uses. `team_viewable_by(profile, team)` —
  the deny→zero-rows team gate used by `graph_service.rs` reads.
- `AtlasNode.id` **is** `kb_resources.id` (`crates/temper-core/src/types/graph_atlas.rs`), so a
  search hit needs no id mapping. Drill-target = `region_id` / territory `anchor_id`
  (`graph_territory.rs`), resolvable from `kb_cogmap_region_members`.
- **Do not reuse** `UnifiedSearchResultRow` (`crates/temper-core/src/types/api.rs`) — it flattens
  home and omits `cogmap_id`/`region_id`. **Do not reuse** `--wayfind` (`wayfind_scope_ids`) — its
  top-N salience funnel would drop a named node whose region isn't salient-enough.

## Component designs

### ① TrailRail — the selected-element panel (ResourcePeek lineage)

**Purpose:** the right-side detail panel for the currently-selected node or edge — Atlas's
evolution of the old `ResourcePeek`, whose defining new capability is the **R5 event-trail
History section**. It borrows ResourcePeek's design vocabulary wholesale; it does not reinvent a
panel.

**Design vocabulary (borrowed from `ResourcePeek.svelte`):** right slide-in aside (`peekSlide`
240ms, `backdrop-blur`), doctype-hue hairline left border, scrim click-to-close + ESC, mono-cap
header, serif hue title, metadata rows, footer with `OPEN RESOURCE →`. Ported from the old
Tailwind/`styling.ts` hues to `palette.ts` (`docTypeHue`) so it shares the Atlas palette. Width
~420px as a right panel in the left-dock layout (appears on selection).

**Selection surface (net-new):**
- **Node selection** stays on `?focus=node:<id>` (unchanged — it re-seeds the neighborhood and
  drives the panel). `NodeChip` already navigates via `buildDrillNodeUrl`.
- **Edge selection is orthogonal:** add a `?sel=edge:<id>` param (new `nav.ts`
  `parseSelection` + `buildEdgeSelectUrl`). Make `marks/Edge.svelte` clickable at Tier 2 (it
  carries only ephemeral `hoveredEdge` today) — on click the *parent* (`TierNeighborhood`) calls
  `goto(buildEdgeSelectUrl($page.url, edgeId), {replaceState:true})` (mirroring how `NodeChip`'s
  `onEnter` delegates its `goto` to the parent). `?focus` and the loaded neighborhood are untouched.
- **Panel target** (new pure `selectedElement(focus, url)`): `?sel=edge:<id>` → edge panel; else
  `focus.kind==='node'` → node panel; else no panel.
- **Backend prerequisite (edge id):** `AtlasEdge` has no `id` today; add `id` to the wire type +
  `graph_traverse_scoped` SQL + `neighborhood_slice` mapping so an edge can be addressed for
  `readTrail('edge', id)`.

**Node panel — content (ResourcePeek lineage) + History:** a *new Atlas-native component* that
borrows ResourcePeek's markup/design (ResourcePeek itself is old-`GraphNode`-typed — not editable
in place).
- **Identity:** title / doc_type / home come from the **already-loaded neighborhood seed node**
  (`AtlasNode`) — zero extra fetch for the core panel.
- **Metadata:** richer rows (context, cogmap, stage) from the existing `GET /api/resources/{id}`
  → `ResourceRow` (already used by the vault route). Note `ResourceRow` has **no `excerpt`**.
- **Excerpt (optional):** derived from the existing `GET /api/resources/{id}/content` markdown
  (first paragraph). One extra read, only on selection — acceptable for a detail panel; may defer.
- **Neighbors:** a **new Atlas-native builder** `atlasNeighbors(focusId, nodes, edges)` over the
  loaded slice — `buildNeighborEntries` is *not* reusable (it is typed against the old
  `GraphNode`/`GraphEdge`, sorts on `aggregator`, and assumes non-null `label`). The Atlas version
  coalesces `label ?? edge_kind` and sorts by home/degree/title. Click a neighbor → refocus
  (`buildDrillNodeUrl`).
- **History (net-new, R5):** a section rendering `readTrail(token, 'node', id)`.

**Edge panel:** lighter — edge kind (mono-cap), source/target titles (from the loaded slice),
polarity + weight (via `edgeStyle()`), and the **edge History** from `readTrail(token, 'edge', id)`
(the `asserted → reweighted → folded` lifecycle — the spec's headline case). No excerpt/resource
identity (edges aren't resources).

**Shared render rules:**
- Normalize `ElementEvent.confidence` at the boundary (Rust-optional vs. ts-rs `string | null` —
  treat `null`/absent uniformly).
- **History display bound:** cap to most-recent N (e.g. 50) with a "show all" affordance rather
  than touching the unbounded trail SQL. Trails are short in practice; avoids a migration.
- Empty trail (`[]` for unreadable/nonexistent element) → a quiet "no recorded history" state, not
  an error.

**Pure-testable units:** new `atlasNeighbors(focusId, nodes, edges)` (Atlas-typed neighbors,
label coalesce, deterministic sort); `selectedElement(focus, url)`; `nav.ts` `?sel` parse/build;
`trailModel(events)` (humanize kind, sort, confidence normalization). Each with a colocated test,
mirroring the `nav`/`palette` test pattern.

### ② SearchAccelerator

**Backend (the primary new server work in C3 — see the lens-enumeration caveat in ④):**
- New service read `atlas_search(profile, team, query, limit)` in the Atlas service layer
  (alongside `neighborhood_slice`/`territory_overview` in `graph_service.rs`):
  1. **Front gate:** `team_viewable_by(profile, team)` → deny yields zero rows (matches sibling
     reads).
  2. **Bound + rank:** `unified_search(p_scope_ids := resources_in_team_scope(profile, team), …)`
     with `graph_expand = false` (name-locate precision; structural self-seed off). Ranking,
     weights, and the visibility gate are inherited unchanged.
  3. **Hit projection (net-new SQL, folded into the same function):** project each hit `id →
     {title, doc_type, home}` via the `graph_atlas_nodes` LATERAL-join pattern (home from
     `kb_resource_homes`, doc_type from `kb_properties`), plus an optional best-affinity
     `region_id` from `kb_cogmap_region_members` — one function, no N+1. `atlas_search` can be a
     single SQL fn that calls `unified_search(...)` internally with `p_scope_ids := array_agg from
     resources_in_team_scope`, so Rust makes one `query_as` call.
- **New wire type** `AtlasSearchHit { node_id, title, doc_type: Option<String>, home: NodeHome,
  region_id: Option<Uuid>, combined_score, fts_score, vector_score, graph_score }` in
  `crates/temper-core/src/types/graph_atlas.rs`, mirroring `AtlasNode`'s exact derive/ts-rs stack
  (`export_to = "graph_atlas.ts"`), re-exported from `types/mod.rs`, flowing through `cargo make
  generate-ts-types`. **Never hand-model in the UI.**
- **Embedding:** v1 passes `query` text with **NULL embedding** (`unified_search` zeroes the vector
  term) — a pure FTS + team-scope name-locate. Adding query embedding later is a drop-in (the
  param already exists). `graph_expand = false`.
- **Endpoint:** `GET /api/graph/search?team=<id>&q=<str>&limit=<n>` → `Vec<AtlasSearchHit>`, gated
  in the handler.
- **e2e (access tier):** member sees only in-scope hits; visibility gate holds (a resource the
  profile can't read never appears); team deny returns empty. This is the risk surface — it gets
  the e2e access tier like R1.

**UI:**
- Search input in the left dock. On query, fetch hits; render a compact hit list (title + home
  glyph + doc-type hue dot).
- Pick a hit → `goto(buildDrillNodeUrl($page.url, hit.node_id))` — sets `?focus=node:<id>` within
  the current `?team`, which seeds the Tier-2 neighborhood around the hit; the existing
  `{#key viewKey}` remount lands the camera on it. (`region_id` is informational / future
  territory-first landing; the node-focus jump alone suffices.)

### ③ AtlasLegend

**Purpose:** the visual key for the encoding grammar, so color/shape read as information.

- **Three collapsible sections, sourced entirely from `palette.ts`** (no re-declared hex):
  1. **Doc-type hues** — the 14 `DOC_TYPE_HUES`, grouped warm (authored) / cool (workflow), each
     swatch via `docTypeHue()`.
  2. **Home encoding** — fill (cogmap-homed) vs. outline (context-homed), via `nodeMark()`.
  3. **Edge grammar** — kind = line style, polarity = arrowhead, weight = thickness,
     `derived_from` = dashed cross-home bridge, via `edgeStyle()` / `EDGE_COLORS`.
- Reads the same `--dt-*` CSS vars the canvas emits (`paletteStyleVars()`); collapsible to keep
  the dock narrow; dark-only but token-sourced (theme-ready).
- **Pure-testable unit:** a `legendModel()` that derives the sections from `palette.ts` exports,
  with a colocated test asserting it stays in sync with the palette (guards drift).
- **Optional synergy:** a doc-type swatch can double as the doc-type filter toggle (see ④).

### ④ ScopeBar filters

**Purpose:** extend the breadcrumb-only C1 `ScopeBar` with filter controls; state in the URL.

- **Lens picker** — the anchor. Surfaces the existing `?lens_id` seam (already parsed + threaded
  to `readTerritories`). A dropdown of available lenses → sets `?lens_id=` → re-load re-sizes
  territories / re-ranks salient nodes. New: a small read (or reuse) to enumerate available
  lenses for the picker.
- **Edge-kind filter** — toggle set of edge kinds. R4's `SliceRequest` already accepts an
  edge-kind filter; C3 threads a `?edge_kinds=` param to `readNeighborhood` and adds the UI. Scoped
  to Tier 2 (where edges render). **Breakout candidate** if effort runs long.
- **Doc-type / element-kind** — dim or hide nodes by doc-type hue. Implemented as **client-side
  visual dimming** over already-loaded nodes (no read change) — pairs with the legend (click a hue
  swatch to filter). Cheapest of the three.
- All filter state lives in the URL (`?lens_id`, `?edge_kinds`, `?doc_types`) via new `nav.ts`
  parse/build helpers, keeping the "URL is state" invariant. Pure-testable in `nav.test.ts`.

## Build sequence (data before UI)

One PR (Cole's "C3 as one chunk"), SDD build, one consolidated end-of-plan opus review.

1. **Backend — two additive migrations + reads (data-before-UI, the only risk surface):**
   - **`atlas_search`** — SQL fn (calls `unified_search` with `p_scope_ids` from
     `resources_in_team_scope`, projects home/doc_type/region) + `atlas_search` service read
     (`team_viewable_by` gate) + `AtlasSearchHit` wire type + `ts-rs` regen + endpoint + **e2e
     access-tier test**.
   - **Edge-id in the neighborhood projection** — a migration that DROP/CREATEs
     `graph_traverse_scoped` to also return `id`, add `id` to the `AtlasEdge` wire type, and thread
     it through `neighborhood_slice`'s mapping. Unblocks edge trails. (`ts-rs` regen.)
   - (The lens picker in ④ may need a tiny lens-enumeration read if none is reusable — trivial, not
     a risk surface. TrailRail's node metadata/excerpt reuse existing resource endpoints — no new
     backend.)
2. **Frontend siblings (parallelizable):**
   - **TrailRail** — `nav.ts` `?sel` edge selection + clickable `Edge` marks + the new
     Atlas-native ResourcePeek-lineage panel (`palette.ts`-styled, `atlasNeighbors` + resource
     metadata/excerpt reads) + the R5 History section (`readTrail`) + `atlasNeighbors`/`trailModel`
     unit tests.
   - **SearchAccelerator UI** — dock search input + hit list + jump-to-drill.
   - **AtlasLegend** — dock legend component + `legendModel` unit test.
   - **ScopeBar filters** — lens picker + edge-kind toggles + doc-type dimming + `nav.ts` filter
     helpers + tests.

If effort runs long, **edge-kind filter** breaks out to a follow-on task first (it is the only
filter that threads through a tier read); doc-type (client-side) and lens (existing seam) stay.

## Testing & gates

- **Pure-logic units** (established pattern): `nav.ts` edge focus + filter params, `trailModel`,
  `legendModel`. Colocated `.test.ts`, run under vitest.
- **e2e access tier** for `atlas_search` (visibility scoping is the risk — mirrors R1's gate).
  Run with the embed feature where the substrate needs it; verify with the access-scenario harness,
  not just `test-db` green (per the goal's standing risk).
- **Gates:** `cargo make check` green; all graph e2e targets pass; vitest green; `ts-rs` types
  regenerated and committed. Push + PR.

## Out of scope

**Rejected (load-bearing):**
- New ranking model / new search weights — the substrate is scope-agnostic; C3 adds only a scope
  front-end and an output projection.
- Light-mode path — no app-level light mode to host it; deferred to a whole-app theming arc.
- Palette hue changes ("vibrancy pass") — palette is shipped as-is.

**Deferred (in-scope elsewhere / later):**
- **Context-as-filter** — belongs to Chunk D (context-preset redirect migration).
- **Global timeline scrub / steward-tick replay** — R5 gives per-element history now (parent spec
  defers global replay).
- **Trail server-side pagination** — display-capped client-side for now; add a SQL limit/cursor
  only if real trails grow long.
- **Edge-kind filter** may break out to its own task if C3 effort runs long.

## Acceptance

- Each sibling is independently useful and does not regress the shipped canvas.
- TrailRail renders the ResourcePeek-lineage detail panel for a selected **node or edge** —
  doctype-hue header, serif title, neighbors (node), metadata/excerpt (node) — plus the R5
  **History** section, visibility-scoped; edge selection works via `?sel=edge:` (with `AtlasEdge.id`
  threaded through the neighborhood projection).
- Search locates + camera-jumps to a node in the visible team-scoped set; hits are visibility-
  scoped; ranking is `unified_search`'s, unchanged.
- Legend accurately reflects doc-type hues, home encoding, and edge grammar, all sourced from
  `palette.ts`.
- ScopeBar filters (lens, edge-kind, doc-type) drive the canvas via URL state.
- Gates green (above); one PR; one consolidated opus review.
