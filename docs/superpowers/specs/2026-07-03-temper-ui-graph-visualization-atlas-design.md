# Temper-UI Graph Visualization — Atlas Rethink — Design

**Date:** 2026-07-03
**Status:** Design — approved, pending spec review
**Task:** Rethink temper-ui graph visualization (`019f25fc-b786-7771-82ff-ee8df0e005a8`)
**Goal:** _pending — created after spec review_
**Mode / effort:** plan / large (this deliverable is the design + roadmap, not code)

## Context

The substrate has grown a rich, real graph — cogmap-homed authored nodes
(concepts / facts / memories / questions with `derived_from` provenance + inter-node
edges), context-homed resources, and an event history behind every node and edge — and
on 2026-07-03 the deployed steward proved it in production: 7 connected nodes + 28 edges
authored over a team's real corpus. The current graph view cannot express any of it.

This is a **greenfield rethink**, not a patch. It targets a large deployment: hundreds of
users across an org, hundreds of thousands of resources + cogmap nodes, and dozens to
hundreds of teams arranged as a DAG. Scale and progressive disclosure are first-class
requirements, not polish.

### What exists today (grounding, file:line)

**The current view — context-bound, workflow-coupled.**
- One route only: `packages/temper-ui/src/routes/(app)/vault/[owner]/[context]/graph/`
  (`+page.server.ts`, `+page.svelte`). The URL *is* the scope — no cross-context or
  cross-cogmap graph is expressible.
- One endpoint: `GET /api/graph/subgraph?context_ref=` → `SubgraphResponse { nodes, edges }`,
  hard-wired to **Concept aggregator seeds, depth-2 BFS, one context**
  (`crates/temper-api/src/handlers/graph.rs:50-59`). Loaded once server-side; no
  client refetch, pagination, or streaming.
- Renders with **Cytoscape.js + fcose** (`packages/temper-ui/package.json`), dark
  editorial serif ("the word IS the node"). The palette covers only the 7 original
  doctypes; the 7 newer cogmap doctypes (`fact/question/theme/concern/principle/
  commitment/domain`) fall to a gray fallback. `aggregator`/`participant` binary,
  sessions-as-glyph, and task-stage special-casing are all baked in.
- Edges are styled by the free-text `label`, **ignoring** the structural `edge_kind`
  and `polarity`. No arrowheads, no weight encoding.
- The doctype color palette is **duplicated in three places and has drifted**
  (`src/lib/graph/styling.ts` `NODE_COLORS`, `src/app.css` `@theme` `--color-graph-*`,
  `src/app.css` `:root` `--graph-*`).
- No time axis of any kind.

**The substrate underneath (much richer).**
- Nodes = `kb_resources` (identity-slim), homed polymorphically via `kb_resource_homes`
  (`anchor_table ∈ {kb_contexts, kb_cogmaps}`); doc_type is a **property**
  (`kb_properties key='doc_type'`), not a column
  (`migrations/20260624000001_canonical_schema.sql:216-285`).
- Edges = `kb_edges`: polymorphic endpoints (`kb_resources | kb_cogmaps`), `edge_kind`
  enum (`express/contains/leads_to/near`), `edge_polarity` (`forward/inverse`),
  free-text `label`, `weight`, `home_anchor`, `is_folded`, and event provenance
  (`asserted_by_event_id`, `last_event_id`) (`…schema.sql:628-644`). Semantic labels
  (`derived_from`/`relates_to`/`part_of`/`answers`/`supports`/`contradicts`) are strings;
  the Rust `EdgeType::legacy_mapping()` maps human words → `(EdgeKind, Polarity, label)`
  (`crates/temper-workflow/src/types/graph.rs:30-70`).
- Everything is **event-sourced** — `kb_events` append-only (`…schema.sql:465-506`),
  act-grain (`correlation_id`) and run-grain (`invocation_id`), emitted by agent
  **entities** with authorship + `ConfidenceBand` in `metadata`. Replay is real
  (`crates/temper-substrate/src/replay.rs`).
- **Regions** are self-materialized clusters under **lenses** (weight vectors):
  `kb_cogmap_regions` (centroid, salience, content_cohesion, internal_tension,
  centrality, label, member_count), `kb_cogmap_region_members` (affinity),
  `kb_cogmap_components`, `kb_cogmap_lenses` (`…schema.sql:675-755`). Produced by the
  pure `MaterializeCogmapShape` on a drift threshold.

**The access model — already a DAG, already transitive (this is the good news).**
- `kb_teams_parents(child_id, parent_id)` is a real team↔team DAG (multi-parent),
  every team rooted under `temper-system` (`…schema.sql:203-208`).
- `team_ancestors(team)` is a `WITH RECURSIVE` walk **up** the DAG
  (`migrations/20260624000002_canonical_functions.sql:29-39`), `CROSS JOIN LATERAL`-ed
  into `resources_visible_to` (`:125-151`), `can_modify_resource` (`:164-188`),
  `vis_team` (`:200-212`), `anchor_readable_by_profile` (`:274-287`), and
  `cogmap_readable_by_profile` (flipped up-expanded by
  `migrations/20260701000002_cogmap_read_up_flip.sql:35-46`), and `edges_visible_to`
  (`:305-313`).
- Semantics: **flat membership × ancestor closure** = DOWN-only grant inheritance. A
  member of a child team reads all of an ancestor team's granted/shared resources; an
  ancestor gains nothing from a descendant's privates; siblings don't see each other.
  Cole's EPD scenario is literally a test fixture
  (`tests/fixtures/access-scenarios/epd-bridge-access.yaml`).
- Team nesting exists: `temper team create <slug> --parent <+slug>` writes a
  `kb_teams_parents` edge (owner/maintainer on parent required)
  (`crates/temper-services/src/services/team_service.rs:82-167`). There is no
  re-parent verb — DAG topology is write-once at creation.

### What does NOT exist (the gap this design fills)

The visibility functions answer "*what* can profile P see" (already DAG-expanded
upward). They do not answer the view's questions, and there is no historical or
aggregate read surface:
1. No **whole-scope graph** read (nodes + edges across a team's homes).
2. `/api/graph/subgraph` is hard-wired (Concept seeds, depth-2, one context); no
   parameterized or cross-scope slice.
3. No **descendant-team enumeration** (downward, membership-gated) for zone navigation.
4. No **team-scope filter** on the visible set (entering `engineering` must not flatten
   `squad A`'s interior).
5. No **region membership projection** — region reads return scalars + `member_count`,
   never the members.
6. No **per-node / per-edge event trail** — only an event *cursor*
   (`GET /api/events/{context}/cursor`) and formation *counts* exist.
7. **Wire-type gaps:** `relationship_events` payloads, `EventKind`, and the region
   interior (`kb_cogmap_region_members`/`_components`/`_lenses`) have no `ts-rs` derives.

## Design

### Object model — a team-scoped whole graph, lensed through the team DAG

The **primary object** is a team's whole graph, where "team" is a *position in the team
DAG*, and context-homed resources + cogmap-homed nodes are **peers** on one canvas
(distinguished by treatment, not separated into views).

You open the graph at a team you can access (`/vault/[owner]/graph`, team-selectable).
At scope **T** you see:
- **In-scope substrate:** resources + cogmap nodes bound at T's level (T's own
  contexts/cogmaps) plus what T inherits upward via the existing `team_ancestors` gate —
  nothing from descendants' privates.
- **Descendant zones:** child teams of T the profile is a member of, drawn as
  **enterable nested territories** (doors, not windows). Entering a zone **re-scopes** to
  that child. Asymmetric by design: `engineering` shows the `squad A` door; you only see
  squad A's interior once you enter squad A, where upward-access then unrolls squad A + its
  ancestors.

**Three nested "territory" kinds**, all cartographic, all drillable:
1. **Team zones** — the DAG enclosure (outermost). Membership-gated.
2. **Home territories** — within a scope, each context is a territory and each cogmap is a
   territory. Both homes, peers.
3. **Regions** — within a cogmap, materialized clusters. **Sparsity-aware:** where no
   region has emerged, the territory shows its high-salience nodes directly.

Access stays the existing gate, unchanged. The design adds *navigation* (descendant-zone
enumeration, team-scope filter) and *projection* (aggregates, slices, trails) on top.

### Canvas — Atlas visual language + semantic zoom

The chosen aesthetic is **Atlas**: cartographic, dense, and legible at org scale (over the
alternative "Constellation" — an evolution of today's moody dark-editorial serif — which is
lovely but not information-dense enough for hundreds of teams and 100k+ nodes). Regions and
teams render as tinted **territories** with hull outlines and map-style labels; nodes are
compact chips.

**Semantic zoom — three tiers, each a different bounded read** (payload never scales with
org size; force layout only ever runs on a Tier-2 slice):
- **Tier 0 — Team panorama** (`zoom < ~0.4`): aggregates only. Team-DAG zones (enterable,
  with counts), region/context territories (sized by salience / member-count), and — the
  sparsity rule — orphan **salient nodes rendered directly** where no region has
  materialized. Only the heaviest aggregated cross-territory bridges. No individual edges.
- **Tier 1 — Territory** (`~0.4–1.2`): a territory resolves into sub-clusters (components) +
  named high-salience member nodes; edges thin/implied.
- **Tier 2 — Neighborhood** (`> ~1.2`): the real force-directed graph — full chips, full
  edge encoding, region hull overlays, and per-element history on click.

(Zoom thresholds above are indicative; final values are a Chunk-C tuning detail.)

### Encoding grammar (color/shape do semantic work)

One consistent grammar across tiers, encoding substrate truth so the eye reads structure
without extra words:
- **home** = fill (cogmap-homed) vs. outline (context-homed)
- **doc-type** = hue
- **edge kind** = line style; **polarity** = arrowhead; **weight** = thickness
- **`derived_from`** = the dashed cross-home provenance bridge
- **region / team** = hull / territory tint + label
- **history** = the selected element's event trail in a side rail

See the palette guidepost below — the graph likely warrants a brighter, more expressive
palette than the site chrome, tuned so color carries information density.

### Time — per-element event trail

History is expressed **per node and per edge**: select any element and a side rail shows its
time-ordered trail (authored → re-asserted → reweighted → folded, each with actor,
timestamp, `ConfidenceBand`). The graph itself stays "now." This is the pragmatic,
read-model-cheap choice; global timeline scrub and steward-tick replay are deferred (below).

### Surface & migration

A **new team-scoped surface** (`/vault/[owner]/graph`, or `/graph`), anchored on a team
you can access; context and cogmap become **filters** within it. The existing
`/[owner]/[context]/graph` route redirects here with the context pre-filtered, then is
removed. One graph surface.

## Read model — five new reads

Every read is gated by an existing visibility function (`resources_visible_to` /
`edges_visible_to` / `cogmap_readable_by_profile`, all already DAG-recursive). New work is
navigation + projection + wire types — see the **data-model guidepost**: reusing the data
model does not preclude new SQL read functions (composed or standalone) where they buy
clarity or efficiency.

| # | Read | Serves | Reuses / net-new |
|---|------|--------|------------------|
| **R1** | **Team-scope + descendant-zones** — given profile + team T: T's ancestor breadcrumb, and child teams of T the profile is a *member* of, as enterable zones with counts | Zone navigation (Tier 0) | Reuse `team_ancestors` for the read gate; **net-new**: a *downward* `kb_teams_parents ∩ membership` walk (opposite direction from the access closure) + a **team-scope filter** so a scope shows only its own bindings. New access-navigation surface → **e2e access tier**. |
| **R2** | **Territory overview** (Tier 0) — scoped to T's own bindings: region territories + context territories + orphan salient nodes where no region exists + aggregated cross-territory bridges | Panorama | Reuse region reads (`cogmap_shape_select`) + `resources_visible_to` filtered to T; **net-new**: sparsity-fallback salient-node ranking + bridge aggregation |
| **R3** | **Territory slice** (Tier 1) — a territory → sub-clusters (components) + top-N member nodes + intra-territory edges | Drill-in | Reuse `kb_cogmap_components` / `kb_cogmap_region_members` (interior — untyped on the wire today) |
| **R4** | **Neighborhood slice** (Tier 2) — focus / viewport + filters + depth → nodes + edges across both homes, salience-bounded | The real graph | **Generalizes** today's hard-wired `/api/graph/subgraph` into parameterized seeds / seed-types / depth / edge-kind filter / team-scope |
| **R5** | **Element trail** — a node or edge → time-ordered `kb_events` with act kind, actor entity, confidence / rationale | History rail | Reuse `kb_events` + replay/readback ordering; **net-new** read (today only an event *cursor* exists) |

## Wire types (temper-core, ts-rs → temper-ui)

All flow through the existing `cargo make generate-ts-types` pipeline. Never hand-model in
the UI.
- `TeamZone`, `Territory` (region | context | cogmap variants), `TerritoryOverview` (R1/R2)
- `RegionMember`, `Component` — region interior, currently untyped on the wire (R3)
- Extended `GraphNode` (add `home`, `salience`, `is_folded`) + a parameterized
  `SliceRequest` (R4)
- `ElementEvent` / `EventTrail` — the `relationship_events` payloads + `EventKind` that lack
  TS derives today (R5)

## Build sequence (data before UI, per the `feedback_ui_last` convention)

- **Chunk A — Access-navigation foundation:** R1 (descendant-zones + team-scope filter).
  Riskiest because it is net-new access-navigation semantics — lands first, with the **e2e
  access tier**, so everything scopes through a proven gate.
- **Chunk B — Read endpoints + wire types:** R2–R5 and their `ts-rs` types. Largely
  independent → parallelizable; each endpoint = service fn + wire type + e2e test. Retire the
  hard-wired subgraph as part of R4.
- **Chunk C — Atlas UI engine (temper-ui):** the new team-scoped route, semantic-zoom LOD
  controller, territory / chip / edge renderer, trail rail, search accelerator, and palette
  consolidation (resolve today's 3-place hex drift into one source of truth). This is where
  `bun run dev` iteration lives, including the palette-vibrancy exploration.
- **Chunk D — Migration:** redirect, then delete, `/[owner]/[context]/graph`.

## Guideposts (carried, not resolved here)

1. **Data-model reuse ≠ query-layer freeze.** The foundational data modeling (teams DAG,
   visibility functions, homes, edges, events, regions) is correct and stays. This does *not*
   forbid new SQL read functions — composing existing behaviors for more efficient queries,
   or net-new standalone functions for a particular slice, is expected and welcome. The
   guidepost is: don't re-model foundations; do build the reads you need.
2. **Graph palette wants more vibrancy than site chrome.** Keep the understated editorial
   blues-and-golds for the *site*; the *graph* likely needs a brighter, more expressive
   palette where color-and-tone carry information density (home, doc-type, edge-kind, region,
   salience) rather than adding words. Explore in Chunk C using an accessible, theme-aware,
   categorical method (e.g. the dataviz palette approach) — validated for legibility in both
   light and dark, and for the many doc-type hues the current 7-color palette can't cover.

## Deferred (not rejected)

- **Global timeline scrub / accretion play** and **steward-tick / invocation replay.** R5
  gives per-element history now; a global replay read + engine (leaning on the existing
  `replay.rs`) is a later beat once the Atlas surface is stable.
- **Cross-team (multi-scope union) panorama** beyond the DAG-enclosure navigation — the
  design scopes to one team position at a time (with descendant zones as doors); a flattened
  cross-team union is out of scope for this arc.

## Open questions (for build)

- Default Tier-0 salience **lens** (which stored lens drives territory sizing / salient-node
  ranking when several exist).
- Cheapest **cross-territory bridge aggregation** for Tier 0 (aggregated `derived_from`
  counts vs. sampled representative edges).
- Whether R1's descendant walk needs its own SQL function or composes from existing ones
  (guidepost 1 says either is fine — decide on efficiency).
- Final semantic-zoom thresholds and the chip/label density budget per tier.

## Acceptance (design phase)

- ✅ A written design spec (this document): object model + scales, how both homes compose,
  how relationships and histories render, the interaction model, and the read-model / API
  needs it implies.
- ✅ Concrete enough that build tasks fall out (Chunks A–D, five reads, wire-type list).
- ✅ At least one visual direction explored — two, in fact (Atlas chosen over Constellation),
  plus a semantic-zoom triptych, captured during the brainstorm.
