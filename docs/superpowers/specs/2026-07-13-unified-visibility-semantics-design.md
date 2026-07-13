# Unified visibility semantics for contexts and cognitive maps

**Goal:** `019f5c66-755e-7fc1-bd87-ee2de8e4cd3f` · **Task:** UV1 · **Date:** 2026-07-13
**Children:** Graph Atlas (`019f28a1`) · The ledger as a readable surface (`019f51e3`)

> **Amended 2026-07-13 by UV2**, which reconciled this spec against the Graph Atlas goal's live tasks
> and found three things it had missed. **(a)** There are **three** region reads, not two — this spec
> named `anchor_shape` and `graph_cogmap_territories` and never mentioned `graph_region_territories`,
> the team panorama read, which is a third copy of the same query. D1 is corrected below, and the
> drift evidence for it turns out to be considerably stronger than the original draft claimed.
> **(b)** Retiring that third read onto `anchor_shape` is **not** a drop-in — it is team-scoped and
> `anchor_shape` takes one anchor — so D1 now states the three ways to close that and rejects the one
> that hides an N+1. **(c)** D5's empty-suppression rule was stated for the region tier and is silent
> about the home tier, where it is currently violated. D5 is extended below. All three corrections
> are grounded in prod reads taken 2026-07-13, cited in place.

---

## Problem

The substrate learned that a context and a cognitive map have the same kind of shape: both
materialize into `kb_cogmap_regions`, through the same lens machinery, behind the same anchor-generic
read (`anchor_shape`). The surfaces did not learn it. The Atlas renders the two as different kinds of
thing; the CLI, API and MCP each see a different subset of what a region knows; and the join key from
a territory to the event that formed it is written on every row and returned by nobody.

The tempting fix is *one read to rule them all*. **The data refuses it**, and the shape of this spec
is the shape of that refusal.

## What is actually true (measured on prod, 2026-07-13)

Coverage is not the problem. **1099 of 1100** context-homed resources sit in a live region (99.9%).
Pointing the context door at the region read would hide nothing.

**Altitude is the problem.**

| | context (`temper`) | cogmap |
|---|---|---|
| live regions | 278 | 250 |
| members per region | ~4 — **208 hold ≤5** | — |
| **connected components** | **2** (one holds 277 regions / 1098 members) | **43** (5.8 regions each) |
| lenses in use | 1 (`workflow-default`) | several |

A cogmap has a hierarchy — 43 components over 250 regions — so semantic zoom has something to zoom
*through*. A context is **one giant component** whose 278 content-coherent micro-clusters average
four members, and the Atlas labels only its top ten. That is not a survey. It is confetti.

**The cause is structural, not a tuning slip.** The region producer was built for cogmaps, where
nodes are *distilled* and sparsely, deliberately connected — so graph structure carries cluster
signal. A context's resources are densely and *uniformly* linked by workflow edges
(goal→task→session), so connectivity separates nothing and the whole context collapses into one
component. **Contexts and cogmaps share region storage. They do not share region meaning.**

### Two of the five metrics are degenerate on contexts

| metric | context | cogmap | verdict |
|---|---|---|---|
| `internal_tension` | **0.0000 on 278/278** (sd 0) | real | **cogmap-only** |
| `reference_standing` | **0 on 277/278** (avg 0.0036) | avg 2.44, sd 6.39 | **cogmap-only** |
| `telos_alignment` | avg 0.90, sd 0.05 | avg 0.74, sd 0.06 | both — **different scales** |
| `centrality` | avg 4.69, **sd 19.73** | real | both — **unbounded, heavy-tailed** |
| `content_cohesion` | real | real | both |

`internal_tension` is defined over oppositional declared edges; a context has none. Rendering it
would report a measurement that was never taken.

### The row is already inconsistent about visibility

The label fallback (`20260713000020`) carefully refuses to name a region after a member the caller
cannot see. The same row then returns a stored `member_count` computed over **all** members, visible
or not. **We decline to name the invisible, then count it out loud.**

---

## Chosen approach

**Unify what is genuinely shared. Keep honestly separate what is genuinely different.**

The two anchors have different *natural surveys*, and that is a fact about them rather than a defect
to paper over. A context is organized by workflow; a cogmap by concept. The unification is therefore
**not** one territory read — it is one **region** read, one **metric contract**, one **visibility
rule**, and one **provenance surface**, beneath two honestly-named surveys.

```
CONTEXT DOOR (survey)              COGMAP DOOR (survey)
  goal-rooted containers             emergent region field
  ~34 territories                    43 components
  + residual tray (completeness)     + orphan nodes (sparsity)
            │                                  │
            └──────────── SHARED ──────────────┘
        region drill  ·  metric contract (per-kind normalized)
        region provenance (asserted_by / last_event)
        one visibility rule  ·  CLI + MCP + API + UI parity
```

### Why the container walk survives, and why T9 still stands

T9 was cancelled on the grounds that **a context is never authored**. That rejected authoring a
context's *birth spec* — a seed, a declared shape. Reading the goal-containment structure that
workflow has *already produced* is not authoring a shape; it is reading an emergent one. The
container walk's sin is not that it exists. Its sin is that it is the **silent, unnamed default**,
and that the region field is absent beside it.

So it is **promoted, not retired**: it becomes a *named* lens (the context's workflow shape), it
declares itself, and it offers the region field alongside as a second named lens.

---

## The six decisions

### D1 — The unified region read

> **Amended by UV2.** The original draft named two region reads and one prior resync. Both counts
> were low.

**There are three region reads, and all three retire onto `anchor_shape`:**

| read | scope | shape |
|---|---|---|
| `anchor_shape` | anchor-generic | the survivor |
| `graph_cogmap_territories` | one cogmap (`reg.cogmap_id = p_cogmap`) | + `coherence` |
| **`graph_region_territories`** | a team (`kb_team_cogmaps` ⋈ `team_ancestors`) | salience only |

`graph_region_territories` — the team panorama read — went unmentioned in the first draft. Prod
(2026-07-13) shows it carrying a **byte-identical copy** of the same visibility-gated label LATERAL,
the same `visible_members` count, the same `seen.visible_members > 0` filter and the same
`cogmap_readable_by_profile` gate as `graph_cogmap_territories`. It differs only in how it selects
which regions to consider. It is a third copy of one query, and retiring the second while leaving it
standing would have preserved exactly the drift this decision exists to end.

**The drift is not a prediction. It has already recurred, at four times the width this spec assumed.**
The draft said the reads "have already been hand-resynced once." In fact
`20260713000050_region_visible_member_count.sql` (PR #416, merged 2026-07-13 — the same day this spec
was written) had to hand-apply **one** visibility fix to **four** functions in a single migration:
`anchor_shape`, `graph_cogmap_territories`, `graph_region_territories`, and `anchor_region_metrics`.
The label derivation, likewise, has been written three times across three functions
(`20260706130000`, `20260713000020`, and the team read). One rule, four hand-edits, one migration.
That is the argument for D1, and it is an empirical one.

**Retiring the team read is not a drop-in, and this decision must say how.** *(UV2.)*
`graph_cogmap_territories` is one-cogmap-scoped, so it collapses onto `anchor_shape` trivially.
`graph_region_territories` does **not**: it is **team**-scoped, fanning out over every cogmap in team
reach (`kb_team_cogmaps ⋈ team_ancestors(p_team)`), whereas `anchor_shape(p_anchor_table, p_anchor_id,
…)` takes **exactly one anchor**. Three ways to close that, and the choice is load-bearing:

1. **Call `anchor_shape` once per cogmap in team scope.** An N+1. Today team scope spans **4 cogmaps
   across 3 teams**, so this would be free — *and invisible*. The Atlas goal explicitly targets
   "hundreds of teams, 100k+ nodes." This is the pattern that hides an N+1 behind a natural-looking
   composition; reject it.
2. **A team-scoped SQL wrapper** that resolves the cogmap set once and unions `anchor_shape` over it
   in one round trip. Keeps the region semantics in exactly one place — the recommended shape.
3. **Give `anchor_shape` a set-of-anchors form** (`p_anchor_ids uuid[]`). More general; more surface.

Whichever is taken, note that `anchor_shape` returns `region_id` and `lens_id` but **not the anchor
id** — so a team panorama unioning it across cogmaps cannot say which cogmap a region came from
(today `graph_region_territories` returns `cogmap_id` precisely for this). The unified read must
carry the anchor identity, or the wrapper must tag it. Do not discover this at integration time.

- `anchor_shape` + `anchor_region_metrics` become the one region read, at every surface.
- `Territory` does **not** collapse into the region row. It stays the *survey* wire type, and gains a
  `kind` that says which lens produced it. A region-kind Territory is a projection of the unified row.
- **Lens defaulting is a service-layer policy, not SQL.** `anchor_shape(p_lens => NULL)` returns
  regions from *every* lens at once, while `cogmap_panorama` picks the modal lens and renders exactly
  one. Today contexts have a single lens (`workflow-default`) so the mixing is invisible — **it
  becomes a live bug the moment a second context lens exists.** Extract the modal-lens default into
  one shared helper both doors call; no rendering caller may pass a null lens.

### D2 — The fate of the container walk

**Promoted to a named lens.** The context door ships two lenses, and says which it is showing:

- **`workflow`** (default survey) — the goal-rooted container walk + residual tray. Legible, and
  *complete*: the tray's `COALESCE(…, '(none)')` guarantees every context-homed resource lands in a
  bucket. Nothing is invisible.
- **`affinity`** (drill) — the region field. 99.9% coverage, high cohesion, 278 micro-clusters. A
  real instrument at the wrong altitude for a survey, and the right one for a drill.

The residual tray **stays outside the force field** (page chrome, not camera-transformed). This is
already correct and its rationale is load-bearing: in-field, one 395-item bucket drags every real
goal's intensity toward zero via `intensityOf`'s max-normalization — it does not merely steal labels,
it flattens the survey.

**Cheap win, already half-built:** the server computes `group_keys` (`GroupKeyMeta`) so the UI can
offer alternative groupings for the residual tray. **No UI code reads it** — the picker the server has
been offering was never built. The `?group_by` param and the `bucket:<key>:<value>` drill token both
already work. Build the picker.

### D3 — What a region is allowed to say

**A metric reaches a rendering surface only with a stated normalization and a stated anchor-kind
applicability. No metric renders raw.**

| metric | applicability | normalization | channel |
|---|---|---|---|
| `salience` | both | **per-anchor-kind `percent_rank`** | radius, opacity, glow *(taken)* |
| `content_cohesion` | both | native 0–1 | hover card *(today: text-only)* |
| `telos_alignment` | both | **per-anchor-kind `percent_rank`** | free second glow layer, or tint mix |
| `centrality` | both | **`percent_rank`** (unbounded, sd 19.7) | inner core dot |
| `internal_tension` | **cogmap only** | native 0–1 | stroke-width / dash period |
| `reference_standing` | **cogmap only** | `percent_rank` | hover card |

Two hard rules:

1. **Return `null`, never `0`, where no measurement was taken.** `internal_tension` and
   `reference_standing` are stored as `0` on contexts. A zero asserts "measured, and found none";
   a null says "not measured here." The read must map context → `null`. *(Producer follow-up: it
   should write `NULL` rather than `0`; shipped migrations are checksum-locked, so this is an
   additive migration, and the read-side mapping lands first.)*
2. **Never pool across anchor kinds.** `telos_alignment` runs 0.90±0.05 on contexts and 0.74±0.06 on
   cogmaps; `salience` scales differ likewise. A pooled min-max crushes one kind against the other —
   the exact failure salience already suffered. Percent-rank within anchor kind, always.

**Channels are available.** `TerritoryCircle` has a free stroke-width, a free inner glyph, a
composable second glow layer (already built, used only by Home for recency), and a hover card that
takes rows trivially. Nothing needs to be displaced to say what a region means.

### D4 — Region provenance (NOT territory history)

`asserted_by_event_id` and `last_event_id` are populated on **524 of 524** live regions and returned
by nothing. They ride the unified region read as two nullable fields. That is the whole change.

Call it **region provenance**, not history — the name matters, because it is what keeps this from
colliding with the ledger goal:

- It is **not as-of fold (L4).** It says *when this region formed and when it last changed*. It does
  not reconstruct state at event N.
- It is **not the supersession cascade (L3).** A moving `last_event_id` is not a dependent-needs-
  review signal.

It neither waits on those nor pre-empts them. It composes with the existing `element_trail` reader
rather than introducing a second traversal.

### D5 — The visibility rule, stated once

**Anchor gate for the row. Member gate for anything derived *from* members.**

> **Invariant: no returned value is computed over members the caller cannot see.**

Today the label honors this and `member_count` violates it. Every value the unified surface returns —
label, count, metric, event key — is held to it identically at every door.

This has a consequence worth stating plainly rather than discovering later: the stored metrics
(`centrality`, `content_cohesion`, `telos_alignment`, `salience`) are computed **at materialize time
over all members**, with no reader in scope. A caller who can see three of a region's twelve members
is shown a cohesion score computed over all twelve.

- `member_count` → return a **visible** count. Straightforward, and it closes a real cardinality leak.
- The stored metrics → **an aggregate over invisible members is still a disclosure about them.** The
  position this spec takes: a region whose *visible* member set is empty is not returned at all; a
  region with any visible member returns its stored metrics as-is, and this is recorded as an
  accepted, bounded disclosure (an aggregate, never an identity). **Flagged as the spec's principal
  risk** — see Open Questions. Per-reader metric recomputation is the alternative and it is
  expensive; it should not be adopted without a threat model that demands it.

**The rule does not stop at the region tier.** *(Amended by UV2.)* As drafted, D5 said "a region whose
visible member set is empty is not returned at all" — and all three region reads already enforce
exactly that (`seen.visible_members > 0`). But the **home tier** does not. `graph_home_contexts`
returns a context whose visible resource count is zero, which is why the Atlas draws the empty
"TEMPER" context as a large empty circle (Graph Atlas C3.1, item L3). That is the same rule one tier
up, and it was never stated.

> **Extended invariant: an anchor with no visible members is not returned at all — at *any* tier.**
> A circle drawn for a container the caller can see nothing inside of is a cardinality leak with a
> radius. Fix it in the read, not in the renderer.

*(Checked and clean: `graph_home_contexts.resource_count` **does** join `resources_visible_to`, so it
is not a second `member_count`-style leak. Only the empty-suppression is missing.)*

Where a home-gated edge walk is needed, **lift `element_trail_edge`'s pattern** (anchor-readable plus
both endpoints readable). Do not write a second one.

### D6 — Surface parity

| gap | decision |
|---|---|
| MCP has **no trail tool** (CLI + HTTP both do) | **Close.** Full surface parity is always intended. |
| No `context_analytics` peer to `cogmap_analytics` | **Close the staleness half.** `kb_contexts` already carries `shape_materialized_event_id` — a straight port. Do **not** invent context *regulation* or a context *telos resource*: contexts have a `telos_centroid` but no telos resource and no regulation edges. Return what exists; do not fabricate a peer field. |
| `materialize_delta` cogmap-only on every surface, typed on `cogmap_id` | **Generalize to the anchor pair.** Add the context route + MCP tool. Add a **CLI** subcommand — today neither anchor has one. |
| `bridges` is `[]` on every shipped path | **Drop it.** Both panorama producers hardcode `Vec::new()`; `graph_territory_bridges` has zero Rust callers; and `bridgeGeometry` keys bridges by *cogmap* id against a position map keyed by *region* id, so even a non-empty list would drop every line. It is unexercised dead code that cannot work as written. Per no-premature-backward-compat: remove `BridgeRibbon`, the SQL function, and the wire field together. |

---

## Components affected

| component | change |
|---|---|
| `migrations/` | additive: retire `graph_cogmap_territories` **and `graph_region_territories`**; extend `anchor_shape` (provenance keys, visible member_count, anchor-kind-aware metric nulling); **empty-anchor suppression in `graph_home_contexts`**; context staleness read; drop `graph_territory_bridges` |
| `temper-substrate` (`readback`) | `CogmapShapeRow` gains provenance + visible count |
| `temper-core/types` | `CogmapRegionRow` extended; `Territory` gains lens `kind`; `TerritoryOverview.bridges` removed |
| `temper-services` | `graph_service::cogmap_panorama` composes `anchor_shape`; shared modal-lens helper; `context_graph_service` declares its lens |
| `temper-api` / `temper-mcp` / `temper-cli` | parity: trail tool (MCP), `materialize_delta` (context + CLI), context staleness |
| `temper-ui` | lens declaration + switcher on both doors; residual group-key picker; metric channels (glow / stroke / core dot); `BridgeRibbon` deleted |

## Trade-offs accepted

- **Two surveys, not one.** We give up the elegance of a single territory read in exchange for each
  door telling the truth about the thing behind it. The unification lands one layer down, where the
  anchors genuinely agree.
- **Stored metrics stay reader-independent.** We accept a bounded aggregate disclosure rather than pay
  for per-reader recomputation. Revisited if a threat model demands it.
- **The container walk keeps a doc-type dependency** (`container_types = ['goal']`). A context with no
  goals has a thin survey and leans on the residual tray. Acceptable; the tray is complete by
  construction.

## Open questions / risks

1. **The aggregate-disclosure position (D5) is the principal risk.** It is a deliberate, stated
   acceptance, not an oversight — but it is the one decision here that a security review could
   reasonably overturn, and it should be reviewed adversarially against a real team DAG rather than
   argued from the schema.
2. **Is the context's 278-region granularity a producer knob at all?** This spec routes *around* the
   question by keeping regions as a drill. If the granularity turns out to be cheaply tunable, a
   coarser context region field could later become a genuine third lens. Not blocking; do not
   speculatively build for it.
3. **The single-component collapse may be intrinsic.** Dense uniform workflow edges may mean a context
   can *never* have meaningful graph components. If so, the affinity lens should say so rather than
   render a component structure that carries no signal.
4. **`content_cohesion` is currently text-only in the hover card** ("sizes nothing"). D3 leaves it
   there. If the metric pass finds a free channel worth spending, revisit.
