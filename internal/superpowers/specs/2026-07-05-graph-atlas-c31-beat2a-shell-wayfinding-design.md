# Graph Atlas C3.1 Beat 2a — shell, wayfinding & legibility design

**Goal:** Graph Atlas — team-scoped, cross-home, historical graph visualization (`019f28a1`).
**Task:** Graph Atlas C3.1 — Atlas wayfinding, legibility & node-content pass (`019f2fbe`, plan/large).
**Predecessors:** C3 chrome (PR #265) + C3.1 Beat 1 bugs (PR #267, merged + prod-verified 2026-07-04).
**Parent specs:** [`2026-07-04-graph-atlas-c3-chrome-design.md`](2026-07-04-graph-atlas-c3-chrome-design.md),
[`2026-07-03-temper-ui-graph-visualization-atlas-design.md`](2026-07-03-temper-ui-graph-visualization-atlas-design.md).
**Sibling spec (later session):** Beat 2b — node content (N1 excerpt/neighbors, N2 richer hover).

## Throughline

C3 shipped the chrome; browser-verifying it in prod surfaced a cluster of **wayfinding, layout,
and legibility** gaps. Beat 1 fixed the four behavioral bugs. Beat 2a is the **presentation pass**:
it reorganizes the Atlas shell so the map is navigable and legible, without touching the data.

The task's eight items split by cost. Beat 2a takes the six that are frontend-centric — W1
(wayfinding), L1/L2/L3 (chrome/layout), G1/G2 (legibility) — and defers the two that need a
substantive backend read (N1/N2 node content) to Beat 2b. Nearly everything lives in
`packages/temper-ui`. The **one** backend touch is a thin extension to W1's existing, already-
visibility-gated R3 region-slice read — it gains the region's `label` (folded into the read's
current readability query; **no migration, no new endpoint, no new visibility surface, no new e2e
tier** — the label is strictly less sensitive than the member titles R3 already returns), plus a
`ts-rs` regen. That is the whole backend delta. Merge auto-deploys temper-ui to prod (same posture
as Beat 1); no operator step.

The unifying move is a **shell reorganization**: the old IDE-style left dock (a C3 decision) is
retired for the legend and filters; the vault sidebar collapses to an icon rail; search, the
breadcrumb, and filters move to a **top bar**; the legend moves to a **collapsible bottom bar**.
The canvas goes near-full-bleed. Wayfinding and legibility then slot into that reframed shell.

## Design decisions (resolved in brainstorm)

1. **Atlas shell — collapse the vault rail, top bar + bottom bar, retire the Atlas dock (L1+L2).**
   The C3 left dock is replaced. The outer vault sidebar (`+layout.svelte` / `Sidebar.svelte`)
   becomes **collapsible to an icon rail** — a *general* app-shell affordance (a dual-win for
   mobile), defaulting collapsed on `/graph`. The Atlas page's 232px dock is removed; **search +
   breadcrumb move to a top bar**, **the legend moves to a collapsible bottom bar**. Net: a
   near-full-bleed canvas, near-immersive, with a small experience-delta. Chosen over keeping the
   dock (A) and over full-bleed focus-mode with no persistent nav (C).
2. **Wayfinding — depth-aware clickable crumb + explicit ascend, in the top bar (W1).** One shared
   breadcrumb component renders the full drill path (`⌂ Atlas › team|cogmap › territory › node`),
   every segment clickable-to-jump. To make the crumb depth-aware and give a true single-level
   ascend, **`focus` becomes a path** (`?focus=territory:X,node:Y`) instead of a single element:
   drilling a node *from a territory* appends to the path; `deriveTier`/loader read the **leaf**
   segment (unchanged seeding); `buildAscendUrl` **pops the last segment** (node → its territory →
   panorama) rather than clearing focus. The `↑` button wires the (currently **unused**)
   `buildAscendUrl`. The territory hop is **named** by extending the existing R3 region-slice read
   with the region `label` (the one backend touch — see Throughline); drilling a node directly from
   panorama carries no territory segment, so the crumb is simply `Atlas › team › node`. Path is
   shallow (≤4) → rendered in full, **no ellipsis-collapse**. This unifies `ScopeBar`'s
   team-ancestor crumb and `CogmapCrumb` into one component (the Beat-1-deferred crumb dedup); the
   team-DAG `ancestors` stay a de-emphasized set (their wire type documents them as a set, not a
   linear path), distinct from the drill path.
3. **Filters — a `⚑ Filters` popover, top-right (chrome relocation).** With the dock gone, the C3
   filters (lens / edge-kind / doc-type) collapse into a popover button carrying an
   **active-count badge**, beside Search in the top bar. Keeps the bar calm at any depth and any
   filter count. The filter *behavior* and URL params are unchanged from C3 — this is relocation,
   not re-design.
4. **Legend — collapsible bottom bar, collapsed by default (L2).** The six-section legend
   (currently `open` by default in a 232px column) becomes a horizontal bottom bar that is
   **collapsed on load** (a `▦ Legend` toggle), expandable. The edge-grammar key is reference
   material, not always-needed; the default maximizes canvas.
5. **Empty territories — ghost state (L3).** An empty context territory (e.g. the empty "TEMPER"
   context that renders as a big empty circle) is drawn **small, dimmed, and labeled "empty"** —
   still visible and drillable, but obviously de-emphasized. Chosen over full suppression (loses
   discoverability of an empty-but-real context) and over an edge chip (more machinery than the
   payoff warrants).
6. **Aggregate-tier bridges — draw them, keep packing; force is deferred (G1).** Panorama/cogmap
   territories keep their deterministic circle-packing positions, but the **`bridges`** already in
   `TerritoryOverview` are **drawn as ribbons** whose thickness = shared-edge count. Connection
   becomes visible now; position stays packing-driven. The bridge-render layer is built to be
   **reusable by the future force layout**. The heavier **bridge-weighted force layout (option C)
   is explicitly deferred** to its own chunk on the goal — B is its honest first half (same bridge
   data, reuses the `forceNeighborhood` d3-force pattern), and drawing bridges first *reveals*
   whether the field is dense enough to make force worthwhile.
7. **Tier-2 labels — anchor + hover (G2).** Instead of every node drawing its title (today's
   illegible overlap), label the **seed + a few highest-degree nodes** always; all other labels
   **reveal on hover**; titles **truncate** with ellipsis. Uses `degree`, already on `AtlasNode`.
   The hover reveal is a peek card that dovetails with Beat 2b's N2. **Zoom-gated LOD is deferred**
   (add only if graphs grow large).

## Grounding — what already exists (do not rebuild)

**App shell / layout:**
- Outer shell `src/routes/(app)/+layout.svelte` — `flex h-screen` with `Sidebar.svelte` (vault
  contexts / user / admin) on the far left and a `<main>` column (top search header → scroll
  wrapper). The Atlas page renders *inside* this. `+layout.server.ts` supplies `contexts`,
  `profile`, `entitlements`, `instanceName`.
- Atlas page `src/routes/(app)/graph/[owner]/+page.svelte` — a 3-column grid `232px 1fr auto`
  (`.atlas-page`): `<aside class="dock">` (SearchAccelerator [team only] → ScopeBar/CogmapCrumb/
  plain "Atlas · your teams" → AtlasLegend) · `.canvas-wrap` (`AtlasCanvas`, keyed on `viewKey`
  so re-scope remounts + resets camera; `$navigating` loading veil from Beat 1) · `TrailRail`
  (right, mounted only when `selection.kind !== 'none' && hasPanelData`).

**nav.ts (`src/lib/graph/atlas/nav.ts`) — the URL frame (state model):**
- `deriveTier` (territory→1, node→2, else 0) — tier is derived from `?focus`, never stored.
- Builders: `buildScopeUrl`, `buildCogmapUrl`, `buildDrillTerritoryUrl`, `buildDrillNodeUrl`,
  `buildHomeUrl`, `buildFiltersUrl`, `buildEdgeSelectUrl`, `clearSelectionUrl` — **all wired**.
- **`buildAscendUrl` (pop one level) is defined + unit-tested but never called** — the seam W1's
  `↑` wires. No "ascend one level" affordance exists today.
- Beat 1 established the **history-mode policy**: scope/drill transitions PUSH; ephemeral state
  (filters, `?sel` edge-select, panel close) REPLACE — chosen per call site.

**Breadcrumb surfaces (to unify):**
- `atlas/ScopeBar.svelte` — team scope: crumb `⌂ Atlas / …scope.ancestors / team.name`, **plus**
  the C3 filter controls (edge-kind chips, doc-type chips, lens input). Renders **team-DAG
  ancestors, not the drill path** — region/node never appear.
- `atlas/CogmapCrumb.svelte` — cogmap scope: `⌂ Atlas / {cogmap name}`. No filters. (Added in
  Beat 1.)
- Home scope: a plain `<nav>Atlas · your teams</nav>` in `+page.svelte`.
- **The shared crumb + `↑` replace all three; the filters relocate to the popover.**

**Rendering + layout engines (all SVG, pure layout in `atlas/layout/`):**
- `AtlasCanvas.svelte` dispatches tiers into one `<svg viewBox="0 0 1040 620">` viewport `<g>`;
  camera = d3-zoom on that `<g>` (`atlas/camera.ts`, scaleExtent 0.3–4).
- **Aggregate tiers = circle-packing, no physics:** `packTerritories.ts` (Tier 0 territory
  circles, sized by salience/member_count), `cogmapTerritories.ts` (cogmap facet dots),
  `regionInterior.ts` (Tier 1 members), `homeLayout.ts`, `hull.ts`.
- **Force only at Tier 2:** `forceNeighborhood.ts` is the ONLY place d3-force runs (deterministic
  ring init, 300 synchronous ticks) — **the reuse target for the deferred G1 force layout.**
- Marks (`atlas/marks/`): `NodeChip` (label drawn *below* node at `y + r + 13`, `text-anchor
  middle`; fill=cogmap-homed / outline=context-homed; seed gets an outer ring), `MemberChip`,
  `TerritoryCircle`, `TeamZoneMark`, `OrphanNodeMark` (the one mark that *already hover-gates* its
  label — the G2 pattern to generalize), `Edge` (`edgeStyle` from palette; arrow markers).

**Bridges + territories (the G1/L3 data, already present):**
- `TerritoryOverview` (Tier 0, R2/cogmap panorama) carries `bridges` (aggregate territory→
  territory shared-edge counts, `Bridge` in `graph_territory.ts`) **and** `orphan_nodes` — the
  bridges are in the wire type but **not currently drawn**. `Territory` carries kind + member
  counts (the L3 empty signal). Neither Tier 0 `TerritoryOverview` nor Tier 1 `TerritorySlice`
  carries node-level edges — only Tier 2 `AtlasSubgraph` does.

**Legend:**
- `atlas/AtlasLegend.svelte` — `open = $state(true)` (open by default), six stacked sections
  (DOC TYPE / HOME / EDGE KIND / EDGE COLOR / POLARITY / WEIGHT), collapsible via a `▦ Legend`
  header. Model from `legend.ts` / `palette.ts`. Beat 2a re-homes it to a bottom bar and flips the
  default to collapsed — content unchanged.

**Test pattern:** pure-logic modules only — `nav.test.ts`, `palette.test.ts`, `layout/*.test.ts`,
`legend`/`neighbors`/`trail` model tests. No `.svelte` component tests. New pure units get a
colocated `.test.ts`.

## Item designs

### L1 — collapsible vault sidebar + retire the Atlas dock

- **Vault sidebar (`Sidebar.svelte` + `+layout.svelte`):** add a collapsed **icon-rail** mode with
  a toggle. State persists (a small localStorage bit) so the choice sticks across navigation and is
  a general affordance (mobile + power users), **not** an Atlas-only hack. On `/graph` it **defaults
  collapsed** (Atlas wants the width); elsewhere it defaults to today's expanded state. The rail
  shows context/nav icons with tooltips; expand returns the full sidebar.
- **Atlas page (`graph/[owner]/+page.svelte`):** drop the `232px` dock column. The grid becomes
  top-bar (row) → `canvas 1fr` + `TrailRail auto` → bottom-bar (row). Search, breadcrumb, and the
  legend move out of the dock (see W1 / filters / L2). `viewKey` remount + camera reset unchanged.
- **Scope note:** the sidebar collapse is the one change that touches the shared app shell beyond
  `/graph`. In-scope by decision (Cole wants it generally); keep it self-contained and regression-
  guard the non-Atlas routes visually.

### W1 — shared depth-aware breadcrumb + ascend (top bar)

**Focus-as-path (`nav.ts`).** Today `focus` holds a single element (`territory:X` **or**
`node:Y`), so the URL forgets the territory once you drill into a node, and `buildAscendUrl` just
deletes `focus` (jumps straight to panorama). Change `focus` to a comma-joined **path**:
- `parseFocusPath(url)` → `Focus[]`; `parseFocus` stays but returns the **leaf** (last segment) so
  `deriveTier` + the loader's seeding are unchanged.
- `buildDrillTerritoryUrl` sets `focus=territory:X`. `buildDrillNodeUrl` **appends** `node:Y` to the
  existing path when a territory leaf is present (`territory:X` → `territory:X,node:Y`), else sets
  `focus=node:Y` (direct-from-panorama drill — no territory hop).
- `buildAscendUrl` **pops the last segment** (`territory:X,node:Y` → `territory:X` → *(empty)*),
  PUSH history (matches Beat 1's drill-transition policy).
- Pure units in `nav.test.ts`: path parse/round-trip; append-vs-set drill; ascend pop; leaf
  extraction. (These change existing `nav.test.ts` expectations — update them in the same task.)

**Backend — name the territory hop (the one backend touch).** Extend R3 `territory_slice`
(`crates/temper-services/src/services/graph_service.rs:610`): fold the region `label` into the
existing readability query (`SELECT reg.label FROM kb_cogmap_regions reg WHERE reg.id=$1 AND NOT
reg.is_folded AND cogmap_readable_by_profile($2, reg.cogmap_id)` → `fetch_optional`; `None` →
`NotFound`, `Some(label)` → readable). Add `label: Option<String>` to `TerritorySlice`
(`crates/temper-core/src/types/graph_territory.rs:115`); `cargo make generate-ts-types`. **No
migration** (`kb_cogmap_regions.label` exists), **no `.sqlx` regen** (this read uses runtime
`query_scalar`/`query_as`, not the `query!` macro), **no new e2e** (gate unchanged; label ≤ the
member titles already returned). Bonus: `TierTerritory` shows the real region name instead of the
generic "REGION · interior".

**New `AtlasCrumb.svelte`** (replaces ScopeBar's crumb + `CogmapCrumb.svelte` + the home `<nav>` in
`+page.svelte`): renders the path from URL state — `⌂ Atlas › {team|cogmap} › {territory} ›
{node}` — each segment a button. `⌂ Atlas` → `buildHomeUrl`; team/cogmap → scope URL; territory →
its `buildDrillTerritoryUrl`; node = current leaf. Segment **labels**: Atlas (static); team
(`scope.team.name`); cogmap (`cogmapName`, resolved as in Beat 1); territory (`slice.label` at
Tier 1; at Tier 2 the loader fetches the label via the now-labeled region-slice read for the path's
territory id); node (the loaded neighborhood seed `AtlasNode.title`). Team-DAG `ancestors` render as
a de-emphasized set between Atlas and the team (unchanged from ScopeBar).
- **`↑` ascend button** wires `buildAscendUrl($page.url)` — pops exactly one level; hidden at Atlas
  root (no focus, home scope).
- **Loader:** thread the territory label for the crumb — at Tier 1 it already loads the slice; at
  Tier 2, when the focus path has a territory segment, fetch that region's slice for its `label`
  (reuses the gated read; over-fetches components/members, acceptable for one label). Expose a
  `crumbTerritory: { id, label } | null` on the page data.
- **Pure unit:** `crumbModel({ scope, cogmapName, focusPath, crumbTerritory, seedTitle })` deriving
  the ordered, labeled, href-bearing segments — colocated `crumbModel.test.ts`. Plus a test that the
  `↑` target equals `crumbModel`'s parent segment.

### Filters — `⚑ Filters` popover (top bar, relocation)

- Move the C3 filter controls (lens picker, edge-kind toggles, doc-type dimming) out of `ScopeBar`
  into a **`FilterPopover.svelte`** anchored top-right beside Search. Button shows a `⚑ Filters`
  label + an **active-count badge** (count of non-default filters, derived from URL params).
- **No behavior change:** the same `?lens_id` / `?edge_kinds` / `?doc_types` params and
  `buildFiltersUrl` REPLACE-history semantics from C3. Only the container moves. `ScopeBar.svelte`
  is retired (crumb → `AtlasCrumb`, filters → `FilterPopover`).
- **Pure unit:** `activeFilterCount(url)` for the badge — colocated test.

### L2 — legend to a collapsible bottom bar, collapsed by default

- Re-home `AtlasLegend` into a bottom bar spanning the canvas width. Flip `open` default to
  **false**. Sections render horizontally (wrap as needed) when expanded. Content + `legendModel`
  otherwise unchanged, with one addition: since G1/B draws bridge ribbons in this same beat, add a
  one-line legend entry — "bridge = shared-edge count between territories; thickness = strength."

### L3 — empty-territory ghost state

- In `TierPanorama` (and the cogmap panorama), territories with zero members render as a **ghost**:
  reduced radius floor, dimmed tint (low `salienceOpacity`), dashed stroke, and an "empty" label.
  Still drillable (drill lands on the empty region's Tier-1, which shows its own empty state). A
  pure `isEmptyTerritory(territory)` predicate (member/anchor count) drives the branch — colocated
  test. No read change (member counts already on `Territory`).

### G1 — draw aggregate bridges (packing kept, force deferred)

- **New `BridgeRibbon.svelte` mark** (aggregate sibling of `Edge`): draws a `bridges` entry between
  two territory circle centers, thickness ∝ shared-edge count, low-opacity, beneath the circles.
  `TierPanorama` maps `overview.bridges` → territory positions (from `packTerritories`) → ribbons.
- **Force-ready structure:** keep the bridge geometry derivation pure and position-agnostic (takes
  a `Map<territoryId, {x,y,r}>`), so the deferred force layout can feed force-computed positions
  into the same mark. A pure `bridgeGeometry(bridges, positions)` unit — colocated test.
- **Deferred to a goal chunk (option C):** bridge-weighted force positioning at aggregate tiers,
  reusing `forceNeighborhood`'s d3-force pattern with bridges as links; packing survives as the
  deterministic init + sparse-field fallback. Logged on the goal, not built here.

### G2 — Tier-2 label anchor + hover

- **Anchor set:** in `TierNeighborhood`, always render labels for the **seed** and the top-K nodes
  by `degree` (K small, e.g. 4–6); a pure `labelAnchors(nodes, seedId, k)` selects them — colocated
  test. Other nodes render **no persistent label**.
- **Hover reveal:** generalize `OrphanNodeMark`'s hover-label pattern to `NodeChip` — on
  `mouseenter`, show a peek label (title, truncated) near the node; on leave, hide. (Beat 2b's N2
  enriches this hover into a fuller peek card; 2a ships the title-on-hover minimum so dense graphs
  are legible now.)
- **Truncation:** a pure `truncateLabel(title, max)` used by both anchor and hover labels —
  colocated test.
- **Zoom-LOD deferred:** no camera-coupled label thresholds in 2a.

## Build sequence (frontend-only, one PR)

One PR, SDD build, one consolidated end-of-plan opus review. One thin backend field (R3 `label` +
`ts-rs` regen); no migration. Suggested order (shell first, so the other items land in the reframed
frame; the W1 backend field before the crumb that consumes it):

1. **Shell (L1):** collapsible vault sidebar (icon rail + persisted toggle + `/graph` default) →
   retire the Atlas dock → top-bar + bottom-bar scaffolding in `+page.svelte`.
2. **W1 backend — region label:** extend `territory_slice` + `TerritorySlice` with `label`;
   `generate-ts-types`; commit regenerated `graph_territory.ts`.
3. **Wayfinding (W1) + Filters relocation:** `nav.ts` focus-as-path (+ `nav.test.ts` updates);
   loader `crumbTerritory` threading; `AtlasCrumb.svelte` (+ `crumbModel`) wiring `buildAscendUrl`;
   `FilterPopover.svelte` (+ `activeFilterCount`); retire `ScopeBar.svelte` / `CogmapCrumb.svelte` /
   home `<nav>`.
4. **Legend (L2):** re-home `AtlasLegend` to the bottom bar, default collapsed.
5. **Legibility (L3, G2):** empty-territory ghost (`isEmptyTerritory`); Tier-2 anchor+hover labels
   (`labelAnchors`, `truncateLabel`, `NodeChip` hover).
6. **Bridges (G1/B):** `BridgeRibbon.svelte` + `bridgeGeometry`; wire `overview.bridges` in
   `TierPanorama`; legend note.

Items 3–6 are largely independent siblings over the item-1 shell (item 3's crumb depends on item
2's field) and can otherwise parallelize across subagents.

## Testing & gates

- **Pure-logic units** (established pattern), each colocated `.test.ts` under vitest: `crumbModel`,
  `activeFilterCount`, `isEmptyTerritory`, `bridgeGeometry`, `labelAnchors`, `truncateLabel`, plus
  the `buildAscendUrl` ↔ `crumbModel` parent-target assertion.
- **Backend delta is one field on an existing gated read** (`territory_slice` gains `label`) — the
  visibility gate is unchanged and the field is strictly less sensitive than the members R3 already
  returns, so **no new e2e access tier is required** (contrast C3's net-new `atlas_search`, which
  did). Rust unit coverage of the existing read is unaffected; verify the label surfaces via the
  crumb in prod browser-verify.
- **Gates:** `packages/temper-ui` `bun run check` (svelte-check) + vitest green; workspace
  `cargo make check` green (the Rust field + `ts-rs` regen touch temper-core/temper-services);
  `cargo make generate-ts-types` run and the regenerated `graph_territory.ts` committed. Push + PR.
- **Verification is prod-only:** Vercel PR previews don't carry Auth0 auth
  (`reference_vercel_preview_no_auth0_verify_in_prod`). Browser-verify the shell / crumb / ascend /
  ghost territories / bridges / label legibility on temperkb.io/graph/@me **after** merge + rollout.

## Out of scope

**Rejected (load-bearing):**
- **Full-bleed focus-mode with no persistent nav (shell option C)** — rejected for too large an
  experience-delta and losing the always-available vault escape. The collapsible rail gets ~90% of
  the immersion at a fraction of the risk.
- **Full territory suppression for empties (L3 option)** — rejected: an empty-but-real context must
  stay discoverable and drillable.
- **New filter behavior / new params** — the popover is pure relocation of C3's filters; no new
  filter dimensions (context-as-filter remains Chunk D).

**Deferred (in-scope elsewhere / later):**
- **Beat 2b — node content (N1 excerpt/neighbors, N2 richer hover peek card).** Needs a backend
  read (excerpt has no Atlas read today); its own session/spec. 2a ships only the title-on-hover
  minimum for G2 legibility.
- **G1 option C — bridge-weighted force layout at aggregate tiers.** Its own chunk on the goal;
  2a draws the bridges and leaves the layer force-ready.
- **Zoom-gated LOD labels (G2).** Add only if neighborhoods grow large.
- **Chunk D** — retire legacy `/[owner]/[context]/graph` + delete `/api/graph/subgraph`; the goal's
  final chunk, after Beat 2.

## Acceptance

- Vault sidebar collapses to an icon rail (persisted; default-collapsed on `/graph`); no double
  stacked left-nav; non-Atlas routes unregressed.
- Search + a **depth-aware, clickable breadcrumb** (Atlas › team|cogmap › territory › node) sit in
  the top bar; the **`↑` ascend** button steps up exactly one level (`buildAscendUrl` wired); one
  shared crumb component serves team, cogmap, and home scopes.
- Filters live in a `⚑ Filters` popover with an active-count badge; behavior identical to C3.
- Legend is a collapsible bottom bar, collapsed by default.
- Empty territories render as a labeled, dimmed, drillable ghost — no big empty circles.
- Aggregate tiers **draw bridges** as strength-weighted ribbons; the bridge layer is force-ready.
- Tier-2 labels are legible: seed + top-degree anchored, others on hover, truncated.
- Pure-logic units + gates green; one PR; one consolidated opus review; prod browser-verify.
