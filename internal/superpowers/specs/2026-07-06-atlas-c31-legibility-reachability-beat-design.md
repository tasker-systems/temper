# Graph Atlas C3.1 — Legibility & Reachability beat (design)

**Date:** 2026-07-06
**Parent task:** `019f2fbe` — Graph Atlas C3.1 (Atlas wayfinding, legibility & node-content pass)
**Goal:** `019f28a1` — Graph Atlas (team-scoped, cross-home, historical graph visualization)
**Predecessor:** Beat 2b node-content (PR #282, merged + prod-verified 2026-07-06) shipped N1 excerpt / N2 hover / N3 history.

## Motivation

Prod browser-verifying Beat 2b confirmed the node-content layer works, but walking the real
graph above the node exposed a cluster of defects that make the Atlas **unusable as a map**: you
cannot tell what a territory is, you cannot reach node content inside a cogmap, a region header
lies about its contents, and a node's neighborhood renders as an island. This beat closes the
"can't read the map / can't reach the content" gap.

All four blockers were traced to exact code (see each item). The shape of the map's *data* is
good — Tier-2 neighborhoods, edge grammar, and the TrailRail panel all read well once you reach
them. The gaps are **reads and presentation above the node**, not the substrate.

## Scope

**In scope** — the four prod blockers plus the coupled legibility siblings from the C3.1 backlog:

- **#1 / B1** — territory circles render with no labels (headline).
- **#2 / A2** — cogmap-view node click dead-ends ("not available in cogmap view yet").
- **#3 / A3** — region header claims "N sub-clusters" but renders far fewer chips.
- **#4 / A1** — R4 neighborhood renders only the focus node despite a nonzero edge count.
- **G1** — aggregate-tier layout communicates nothing (keep-pack decision, below).
- **G2** — Tier-2 node labels overlap / illegible.
- **L3** — empty territories render as large empty circles.
- **payload nit** — expanded event-payload key column collides into its values.

**Rejected** (load-bearing decisions — resist scope creep):

- **Force-weighting the aggregate tiers (G1 "add bridges layout").** The circle-pack is a fine
  sparsity-aware layout; the panorama was illegible because it had *no labels*, not because the
  arrangement was wrong. Labels (B1) do the heavy lifting. A relational/force layout at Tier-0 is
  higher-risk and lower-ROI than labelling the pack. Not this beat.
- **Steward-authored region names.** `kb_cogmap_regions.label` is an optional agent-authored
  field; populating it at materialization is the durable "real names" fix but pulls in the
  clustering/steward pipeline. This beat derives a representative label in the read (B1) and
  *prefers* the stored label when present, so a later steward-naming arc composes cleanly without
  rework.
- **True member sub-clustering at Tier-1.** The schema has no grain finer than a region's members
  (see A3); inventing one is a materialization effort, not a legibility fix.

**Deferred** (in scope for C3.1 elsewhere, not this beat): the wayfinding cluster — B1-back-button
(history push), B2 cogmap breadcrumb, W1 depth-aware breadcrumb — and the layout cluster L1
(double left-nav) / L2 (legend relocation). These are a separate *wayfinding* beat; this beat is
*legibility + reachability*.

## Structure — one spec, two phases, shippable as separate PRs

Phase A (backend reads) and Phase B (frontend + one read tweak) are independent workstreams that
touch different layers and can land as separate PRs. The one cross-phase dependency: Phase B's
`TierTerritory` edit consumes A3's shrunk `TerritorySlice` shape (components removed), so A3 ships
before B. (B1's R2 label change lives inside Phase B — it is not a Phase A dependency.)

---

## Phase A — Reachability (backend reads)

### A1 (#4) — Bidirectional neighborhood walk + honest degree

**Root cause.** `graph_traverse_scoped` (`migrations/20260703130000_graph_atlas_chunk_b_reads.sql:44-84`)
seeds its first arm on `e.source_id = ANY(p_seed_ids)` **only** (line 65) — outgoing edges only —
and both arms require *both* endpoints ∈ `resources_in_team_scope` (lines 60-61, 72). The hover
"N edges" comes from `graph_atlas_nodes.degree` (`:88-118`), computed as an **undirected**
count (`e.source_id = r.id OR e.target_id = r.id`, line 116) scoped only by
`edges_visible_to(p_profile)` — **no team clamp**. So a focus node whose visible edges are
incoming, or reach resources outside team scope, yields zero walk rows while `degree` still counts
them → "4 edges, 0 shown."

**Fix.**
1. Rewrite `graph_traverse_scoped` as a **bidirectional** walk: the frontier is a *node* (seed ∪
   reached); each step includes edges incident to the frontier in **either** direction and adds the
   opposite endpoint to the next frontier. Preserve the full edge-visibility predicate on every
   edge — `NOT is_folded` AND `anchor_readable_by_profile(home)` AND both endpoints ∈ team scope —
   and keep the terminal `DISTINCT` dedup. Depth clamp stays (`LEAST(p_depth,10)`).
2. Make `degree` **honest to what is reachable**: clamp the `graph_atlas_nodes` degree LATERAL to
   team scope (both endpoints ∈ `resources_in_team_scope`), matching the walk's endpoint boundary.
   Degree becomes **scope-relative** (a node's number differs between team views) — this is correct
   for a scoped map and is what makes the hover count equal the rendered neighbor count.

**Why not widen the walk to match the broad degree instead?** Team scope is a **security
boundary** (`resources_in_team_scope ⊆ resources_visible_to`). Reaching neighbors outside team
scope would leak cross-team resources. The walk cannot exceed team scope, so the honest number is
the team-scoped degree, not the broad one. "Across both homes" is already satisfied: team scope
spans context-homed *and* cogmap-homed resources.

**Migration discipline.** New migration; `CREATE OR REPLACE FUNCTION` (signatures unchanged). Never
edit the shipped `20260703130000` file.

### A2 (#2) — Cogmap-scoped neighborhood read (cogmap nodes become drillable peers)

**Root cause.** The gate is **scope-based, not node-based**: the cogmap loader branch hard-codes
`neighborhood: null` (`packages/temper-ui/src/routes/(app)/graph/[owner]/+page.server.ts:117`) and
handles only Tier 0/1. There is **no cogmap equivalent of `neighborhood_slice()`** — the R4 service
and `graph_traverse_scoped` are team-parameterized (require `team_id` + `team_viewable_by`). The
"not available in cogmap view yet" string (`AtlasCanvas.svelte:41-48`) is the honest as-shipped gap.

**Fix — new cogmap-scoped read stack, mirroring the team stack:**

1. **SQL.** `resources_in_cogmap_scope(p_profile, p_cogmap)` = resources homed in the cogmap
   (`kb_resource_homes` anchor `kb_cogmaps`, `anchor_id = p_cogmap`) that are
   `resources_visible_to(p_profile)`, gated by `cogmap_readable_by_profile(p_profile, p_cogmap)`.
   Then `graph_traverse_cogmap_scoped(...)` and a cogmap variant of the node projection, using this
   scope CTE in place of the team one — otherwise structurally identical to A1's bidirectional walk.
2. **Scope semantics.** A cogmap door is "you are inside this map": the neighborhood shows the
   cogmap's own nodes + edges among them. Edges from a cogmap node to a *context-homed* resource
   outside the cogmap are **out of scope for the door** (they surface in the team view, where both
   homes are peers). Cogmap-view degree is clamped to cogmap scope for the same honesty as A1.
3. **Service + transport.** `cogmap_neighborhood_slice()` in `graph_service.rs`; a
   `/api/cogmaps/{cogmapId}/graph/slice` route (mirrors `/api/teams/{teamId}/graph/slice`); a
   `readCogmapNeighborhood()` client read.
4. **Loader wiring.** In the cogmap branch, replace `neighborhood: null` with the Tier-2 read, **and
   add TrailRail parity**: the cogmap branch currently omits `selection` / `trail` / `resourceRow`
   (lines 118-120), so even a rendered neighborhood would show an empty rail. R5 `readTrail` and
   `readResourceRow` are profile-scoped (not team-scoped), so the same block as the team branch
   (`:167-174`) works verbatim.

**Security invariant (hard requirement).** The cogmap walk MUST reproduce the FULL
`edges_visible_to` predicate conjunct-for-conjunct — `NOT is_folded` AND `anchor_readable_by_profile`
AND `endpoint_readable_by_profile(source)` AND `endpoint_readable_by_profile(target)` — exactly as
the migration's SECURITY INVARIANT header (`20260703130000:10-18`) demands. This is net-new
access-navigation semantics; it gets **e2e access-tier** coverage with **deny-direction** tests (a
cogmap-member resource you can't read is not reachable; an edge to a private endpoint does not
leak; a sibling cogmap does not bleed in). test-db green is a false signal here.

**Migration discipline.** New additive migration (`CREATE FUNCTION`s only). `cargo make prepare-services`
+ `prepare-api` after, since these are macro queries in service/test targets.

### A3 (#3) — Drop the inverted "sub-clusters" badge; show honest member count

**Root cause — two bugs, one inverted premise.** `TierTerritory.svelte:39` renders
`{slice.components.length} sub-clusters` while the body renders `slice.members`
(`:18,31-33`) — two unrelated arrays. Worse, `graph_region_components`
(`20260703130000:218-227`) joins components on `cogmap_id + lens_id` with **no `reg.id` tie**, so it
returns *every component in the whole cogmap/lens* (hence "25"). And most fundamentally, the schema
has **components as the region's parent grain**, not its children:
`kb_cogmap_regions.component_id` is "the input-grain group this region was **clustered within**"
(`20260624000001_canonical_schema.sql:729`). There is no grain finer than a region's members, so
"sub-clusters of a region" is semantically inverted — the badge cannot be made correct, only removed.

**Fix.** Remove the components concept from the R3 slice:
- Drop `graph_region_components` (new migration, `DROP FUNCTION`); stop selecting it in
  `territory_slice()`.
- Remove `components` from the `TerritorySlice` wire type / Rust struct; regenerate ts-rs.
- `TierTerritory.svelte`: replace the badge with an honest `{slice.members.length} members` (or
  drop the badge and rely on the visible chips + count already implied).

This is additive-safe (function drop + wire shrink, no data loss) and simplifies the surface.

---

## Phase B — Legibility (frontend + one R2 tweak)

### B1 (#1) — Derive region labels in R2; truncate in the mark

**Root cause.** `TerritoryCircle.svelte:45` gates the label `{#if displayLabel}` with **no
fallback**; region territories inherit `kb_cogmap_regions.label`, which is **usually NULL** (regions
are machine-materialized, rarely agent-named). Context territories always label (from
`kb_contexts.name`, `graph_context_territories:149`). So every blank circle is an unlabelled region.
A Tier-0 `Territory` carries no member titles (`graph_territory.ts:32`), so a name must come from
the read.

**Fix.**
1. **R2 derives a representative label.** In `graph_region_territories`
   (`20260703130000:126-137`), emit `label = COALESCE(reg.label, <representative>)`. The
   representative is the region's **top member by affinity** (`kb_cogmap_region_members.affinity DESC
   NULLS LAST`, tie-break the same visibility-scoped ordering R3 already uses) — its title, so an
   unnamed region reads as "what its most-central member is about." This **prefers the stored
   agent-authored label** when present, so a future steward-naming arc needs no read change.
   - Value validation is a judgment call; verify on the harness against real region shapes before
     committing to affinity-vs-degree for the representative.
2. **Truncate in the mark.** The single centered `<text>` (`TerritoryCircle.svelte:45-58`) has no
   wrapping/truncation; a derived member-title label can be long. Truncate to fit the circle radius
   (ellipsis), full label on hover/`aria-label` (already wired at `:29`).

**Migration discipline.** New migration; `graph_region_territories` `RETURNS TABLE` shape is
unchanged (label was already a column), so `CREATE OR REPLACE`. Re-run `prepare-services` if the
macro query text changes.

### B2 (G1) — Keep the pack; make sizing consistent

Decision: **keep the circle-pack.** The panorama's illegibility was missing labels (B1), not
arrangement. Scope here is verification-plus-polish: confirm regions size by `salience` and
contexts/cogmaps by `member_count` consistently (`packTerritories.ts`), and that B1's labels make
the packed panorama legible on the harness. No layout-engine change.

### B3 (G2) — Tier-2 label collision

Node titles centered on nodes overlap at Tier-2. Apply the standard treatment: truncate + offset
the label below the node + reveal the full title on hover/selection (the hover card already carries
the full title). Prefer collision-avoidance via offset/gating over a force re-layout.

### B4 (L3) — Empty territories

`TerritoryCircle` already has a `ghost` prop (de-emphasized wash + "· empty" suffix, `:12-21`), so
L3 is **mostly done**. Scope: verify the empty-context case renders as a ghost (not a bare circle),
and confirm the panorama blanks were #1 (unlabelled non-empty regions), not empties. Decide
suppress-vs-ghost for zero-member territories — default keep the ghost (still drillable), which is
the current behavior.

### B5 (payload nit) — Event-payload key/value column

In the expanded TrailRail event payload, long keys (`originator_pro…`, `owner_profile_…`) collide
into their values (observed on prod). Widen or stack the key column so key and value never overlap.

---

## Testing strategy

- **Phase A / access semantics** — A2 (new cogmap walk) and A1 (widened reachability) change what is
  reachable, so they get **e2e access-tier** tests with deny-direction cases, per the goal's Chunk A
  gate and the read-gate-visibility rule. Do **not** trust test-db green alone. Run
  `cargo make test-e2e` (+ `test-e2e-embed` if fixtures touch the embed path).
- **Value assertions** — A3 (member count matches rendered chips), A1 (a node with only incoming
  edges now renders its neighbors; hover count == rendered count), B1 (an unnamed region shows its
  top member's title).
- **Phase B / render** — iterate and verify on the `/dev/atlas` harness against captured fixtures
  (regen from a logged-in prod tab per `dev/atlas/README.md` if the synthetic fixtures don't exercise
  a case). Scenarios: `teamPanorama` (B1/B4), `regionSlice` (A3), `nodeNeighborhood`/`leafBare` (A1),
  `cogmapPanorama` (A2). Final authed verification is **prod post-merge** (Vercel previews can't carry
  Auth0).
- **sqlx / codegen discipline** — new migrations only (never edit shipped ones); `CREATE OR REPLACE`
  for unchanged signatures, `DROP FUNCTION` for A3's removal; after SQL changes run
  `cargo sqlx prepare --workspace -- --all-features` → `cargo make prepare-services` → `prepare-api`;
  `cargo make generate-ts-types` for the `TerritorySlice` shrink.

## PR sequencing

1. **PR A1 + A3** (walk direction/honesty + component-badge removal) — self-contained backend reads.
2. **PR A2** (cogmap neighborhood stack) — the largest, net-new access surface; own PR for a focused
   access-tier review.
3. **PR B** (legibility: B1 label derivation + truncation, B2 verify, B3 label collision, B4 verify,
   B5 nit) — B1's R2 label change is self-contained here; the only upstream dependency is A3's
   shrunk `TerritorySlice` shape.

(A1+A3 and A2 may split further if review prefers; B is one PR.)
