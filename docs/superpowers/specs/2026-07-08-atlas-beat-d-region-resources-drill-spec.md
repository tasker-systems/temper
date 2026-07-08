# Atlas Beat D — Region → resources drill (composition)

**Status:** design spec (approved visual direction; structural decisions recorded here).
**Branch:** `jct/atlas-reshape` (HELD — no PR until Beat D closes).
**Goal:** `graph-atlas-visualization` (`019f28a1`). North star:
`docs/superpowers/specs/2026-07-06-atlas-reshape-projection-class-north-star.md` (Beat **D**).
**Subsumes:** north-star decision 7 ("Region → resources is composition"), roadmap Beat D.

---

## 1. The act and its attention-contract

Beat D serves **Composition** (with a **wayfinding** seam): the projection-class signature is
`(substrate, perspective, scope) → workable assembly`, attention shape = **depth**. The scope is a
**region** (or a shift-selected union of regions); the assembly is the region's **ideas** together
with the **work they were distilled from**.

The rubric consequence: the view may go *deep* on a bounded scope (that is what composition is
for), but it must stay a *workable* assembly — a bounded node set, not a firehose. This forces the
two bounding decisions in §6.

## 2. The problem this beat fixes (verified against prod)

The cogmap knowledge graph has two axes:

- **Knowledge axis** — cogmap facets (theme/concept/fact/domain/commitment…) homed in the cogmap,
  linked to each other (`contains`/`near`/`leads_to`).
- **Builder axis** — the context-homed resources (sessions/tasks/research/notes) that facets were
  `derived_from`. This is where the actual work lives.

**The current cogmap-nav path shows only the knowledge axis.** `graph_traverse_cogmap_scoped`
(migration `20260706120200`) fences the entire walk to `resources_in_cogmap_scope` — every edge
endpoint must be homed in the cogmap — so it *structurally cannot* surface context-homed resources.
Measured on the live "Temper — self-cognition" cogmap (prod, `019f2391`):

| Edge class | Count | Visible on the cogmap-nav canvas today |
|---|---|---|
| facet ↔ facet (knowledge) | 121 | ✅ |
| facet → context-resource (builder) | **150** | ❌ (only listed in the neighbor panel) |

**~55% of the edges touching facets point at work-products the canvas never draws.** Beat C's
retirement of team-scope removed the one path (team neighborhood, whole-footprint) that used to
render context nodes. Beat D restores the builder axis deliberately, at **region grain** (bounded),
on the cogmap-centric nav — rather than via the old unbounded team footprint.

## 3. The design (validated live in `/dev/atlas` against real T7 data)

Clicking a region on the cogmap panorama drills to a **two-axis force-graph**:

- **Nodes** = the region's facets (**seeds**) ∪ their **1-hop neighbors** across all *visible*
  edges. Neighbors include both other facets (knowledge) and context-resources (builder).
- **Edges** = all visible edges among that node set.

### 3.1 Marks — shape encodes the axis

`NodeChip` renders by `home`, not just fill:

- **cogmap-homed (idea)** → **filled circle**, doc-type hue. (unchanged)
- **context-homed (work-product)** → **filled rounded square** (`rx = 0.32·r`), doc-type hue, thin
  `CANVAS_BG` stroke for separation. (**new** — replaces the hollow-circle context treatment)

Rationale: shape carries the *axis* pre-attentively (circle = idea, rounded-rect = document);
color still carries doc-type. A context-resource reads as "a document I built," independent of hue.
`home` — not `doc_type` — drives the shape, so a steward-distilled facet whose doc_type is
`session` still renders as a circle (it is an idea in the map), while its context twin renders as a
square. This is correct and was verified against real dual-homed T7 data.

### 3.2 Layout — geography reinforces the axis

`forceNeighborhood` gains a `forceRadial` keyed on `home`:

- facets (ideas) pulled to an inner radius; context-resources (work) pulled to an outer ring.
- cross-home links run **looser + weaker** so the radial can hold the ring against the pull of the
  `derived_from` tethers; same-home links keep their structure.

Tuned params (locked against the harness; `minDim = min(width, height)`):

| force | value |
|---|---|
| radial radius | facet `0.06·minDim` · context `0.44·minDim` |
| radial strength | `0.6` |
| link distance | cross-home `150` · same-home `80` |
| link strength | cross-home `0.15` · same-home `0.6` |
| charge / collide / center | unchanged from C2 |

Reads as: *"a cluster of ideas at the center, and radiating out from them the sessions and tasks
they were distilled from,"* each document still tethered to the idea it produced.

### 3.3 Multi-region union (shift-click)

Shift-clicking additional regions unions their seed sets into **one** force-graph (seeds =
⋃ members; nodes/edges computed over the union). v1 renders the union as a single assembly with **no
per-region chrome** — the force layout co-locates related facets regardless of region, and the
ideas-core / sources-ring invariant survives (verified at 30 nodes / 2 regions). Region provenance
is intentionally *not* visually distinguished in v1 (see §8 Deferred).

## 4. Backend — the new cross-home read

Two SQL functions (new migration; DROP+CREATE not needed — these are new objects) plus a service
function. Mirrors the `cogmap_neighborhood_slice` template but **removes the cogmap fence** and
seeds from region membership.

### 4.1 `graph_region_composition_edges(p_profile, p_region_ids uuid[], p_depth int)`
Returns the induced-subgraph edges. Seeds = the union of the given regions' members
(`kb_cogmap_region_members`). Walk outward to `p_depth` (default **1**) over `kb_edges` where **both
endpoints are visible** (`resources_visible_to(p_profile)`), `NOT is_folded`, and the edge's home
anchor is `anchor_readable_by_profile`. **No `resources_in_cogmap_scope` restriction** — this is
what lets the walk cross to context-homed resources. Depth is capped at a small constant (composition
is depth-1 by default; the walk never becomes an unbounded traversal).

### 4.2 `graph_atlas_nodes_visible(p_profile, p_ids uuid[])`
Node rows `(id, title, doc_type, home, degree, first_chunk)` for an arbitrary id set, each gated
through `resources_visible_to` (a non-cogmap-scoped analog of `graph_atlas_nodes_cogmap`). `home` =
`'cogmap'` if the resource has any `kb_cogmaps` home, else `'context'`. `degree` counted over
`edges_visible_to`.

### 4.3 `region_composition_slice(pool, profile, region_ids, depth) -> AtlasSubgraph`
Service function (temper-services, service-direct read):
1. **Entry gate (deny-as-absence):** every region must exist, be `NOT is_folded`, and have a
   `cogmap_readable_by_profile` cogmap — else `NotFound`. (Same gate shape as `territory_slice`.)
2. seeds = union of `graph_region_members` across the regions (bounded, §6).
3. edges = `graph_region_composition_edges(profile, region_ids, depth)`.
4. node ids = seeds ∪ edge endpoints; nodes = `graph_atlas_nodes_visible(profile, ids)`.
5. return `AtlasSubgraph { nodes, edges }` (the existing wire type — no new type needed).

### 4.4 Visibility — reproduce the full predicate (leak-class lesson)
Both node and edge gates reproduce the canonical visibility predicate **conjunct-for-conjunct**
(both endpoints ⊆ `resources_visible_to`, `NOT is_folded`, home anchor readable) — the same class of
gate that PR #254 and the Beat-C array_agg fix turned on. e2e MUST include the **deny direction**:
a caller who can see the region's facets but *not* a linked context-resource (it is homed in a
context they lack) must get the facet without the invisible neighbor, and no edge to it.
See `[[feedback_read_gate_must_match_full_canonical_visibility]]`,
`[[reference_array_agg_scope_null_fall_open_leak]]`.

## 5. Entry & routing

- **Endpoint:** `GET /api/graph/regions/composition?ids=<r1>,<r2>&depth=1` (or a POST with a body if
  the id list grows) → `AtlasSubgraph`. Deny-as-absence per §4.3.
- **Nav:** on the cogmap panorama (Beat A field), clicking a region navigates to
  `?cogmap=<id>&focus=territory:<r1>`; shift-click appends → `focus=territory:<r1>,<r2>`.
  `parseFocus` learns to parse a **comma list** of territory ids (today it is single). The union
  read is driven by that id list.
- **Rendering:** territory-focus now renders the **composition force-graph** (reusing
  `forceNeighborhood` + `NodeChip` + the neighborhood marks), NOT the old R3 members hull. Tier for
  territory focus is rendered force-graph-style; `TierTerritory`'s affinity-hull view is **retired**
  (superseded — see §7). Node/edge selection reuses the shipped `TrailRail` + `?sel=edge:`.

## 6. Bounding (composition must stay "workable")

- **Per-region members:** existing `MEMBER_LIMIT` (100) on `graph_region_members`.
- **1-hop neighbor cap:** cap the neighbor set (soft cap, e.g. 60) so a hub facet with dozens of
  `derived_from` docs does not flood the assembly. When the cap truncates, **`log`/surface it** — no
  silent truncation.
- **Regions-per-union:** soft cap (~6) on the number of unioned regions; beyond it, ignore extras
  and surface the fact. Keeps the central idea-cluster legible.

## 7. What this beat retires

- **`TierTerritory` (R3 affinity-hull members view)** is superseded by the composition drill. The R3
  `territory_slice` service fn + `GET /api/graph/regions/{id}/slice` endpoint + `TerritorySlice`
  wire type + the hull component become dead once territory-focus renders the composition graph.
  Retire them in this beat (they have no other consumer after the re-point). Follow the Beat-C
  teardown discipline: grep both the type symbols and the fn names before deleting
  (`[[feedback_plan_gate_audit_both_ends]]`).

## 8. Out of scope

### Rejected (load-bearing — deliberately not built)
- **Per-region chrome in a union** (hulls / tints distinguishing region A from B). The composition
  act wants one workable assembly; the shape+home encoding already carries two dimensions, and
  region-tinting would overload it. The force layout's natural co-location is the intended behavior.
- **Deep N-hop composition.** Depth-1 (facets + their direct builder ring + inter-facet edges) is
  the "workable assembly." A deep walk is a different act (traversal) and breaks the attention
  contract.

### Deferred (later, if warranted)
- **Region-provenance in union** (subtle convex hulls per region) — revisit only if merged-union
  provenance proves confusing in real use.
- **Excerpt / hover enrichment** for context-resource nodes (the builder axis may want a different
  hover card than facets).
- **Builder-axis palette differentiation** — session/task doc-types sit near each other in the
  palette, so document-squares are well-distinguished *from circles* but not much *from each other*.
  A palette question, tracked separately from Beat D.

## 9. Testing

- **Backend e2e (temper-e2e, `test-db`):** composition slice returns facets + linked
  context-resources + edges for a seeded region; **deny-direction** test (invisible neighbor
  omitted, no dangling edge); union across two regions; bounding/truncation surfaced; entry-gate
  deny (unreadable region → NotFound). Regenerate the per-crate sqlx caches after new SQL
  (`prepare-services`, `prepare-e2e`).
- **Frontend (vitest, node env, pure-fn):** `NodeChip` shape-by-home; `forceNeighborhood` radial
  separation (facets inner / context outer, deterministic); `parseFocus` comma-list of territory
  ids; union seed assembly; a11y list groups nodes by axis.
- **a11y:** the field is `svg role=img`; provide a list fallback grouping nodes into **Ideas** and
  **Sources**, each a link with metadata (north-star decision 9). Small nodes stay hoverable/clickable.
- **Harness:** synthetic (personal-data-free) `regionDrill` + `regionDrillUnion` scenarios added to
  the committed `atlas-fixtures.json` (the real-data captures live only in the gitignored
  `.local.json`). Verify layout/legibility in `/dev/atlas` per `[[feedback_local_proddata_render_harness_for_ui]]`.

## 10. Visual targets

Locked against `/dev/atlas` with real T7 data (harness scenarios `regionDrillFenced` = today,
`regionDrillFull` = Beat D single region, `regionDrillUnion` = two-region union, in `.local.json`):
- Single region: idea-circles clustered center, 5 document-squares on the outer ring, each tethered
  by its `derived_from` line.
- Union (2 regions, 30 nodes): ideas-core / sources-ring invariant holds; provenance merges (v1).
