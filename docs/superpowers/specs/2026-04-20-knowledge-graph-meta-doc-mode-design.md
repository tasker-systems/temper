# Knowledge graph — meta-doc mode (Jaccard projection) scoping

**Date:** 2026-04-20
**Context:** temper
**Task:** PR 6 of the `kg-handoff.md` bundle — the `VIEW: meta-doc` toggle
currently renders an "Emergent view — not implemented" stub.
**Related:**
- `design-system/docs/kg-handoff.md` (§ *PR 6 — meta-doc mode*)
- `design-system/docs/kg-handoff-next.md` (§ *Deferred work — 4. PR 6*)
- `docs/superpowers/specs/2026-04-17-knowledge-graph-mvp-concept-visualization-design.md`
  (the structural-mode spec; extension posture is already set up for this)
- `design-system/preview/kg-scene-v2.html` (visual reference, v2 variant)
- R11 D2, D3 (original research — not checked in; referenced in
  `design-system/docs/kg-handoff.md` decision log)
**Status:** Scoping — **do not implement yet.** This doc lists the open
questions and proposes answers. The next session with the R11 research
on hand + two weeks of structural-mode usage data should validate or
redirect each answer before any code lands.

---

## Why this is its own doc

Everything else in `kg-handoff.md` was a mechanical port of a settled
prototype. Meta-doc mode has three pieces of genuine design judgment
that the prototype didn't resolve:

1. What counts as a "member" when you compute Jaccard similarity
   between two aggregators.
2. Where the emergent-edge cutoff lives (threshold, top-K, or both).
3. Whether the edges are precomputed or derived on the fly.

Each of those is cheap to change early and expensive to change late.
Get them right before cutting server code.

---

## End goal (restated)

The `VIEW: meta-doc` toggle swaps the graph from the structural
presentation — participants clustered around their aggregator via
explicit edges — to an *emergent* projection where aggregator ↔
aggregator edges are drawn from **shared-member overlap**. Two
concepts that share many of the same research / task / decision
children get a heavy emergent edge; two that share none get nothing.

The goal is to expose the vault's latent conceptual structure without
asking a human to author it. The structural view shows what the
author declared; the meta-doc view shows what the graph implies.

---

## Deferral preconditions

From `kg-handoff.md`:

> Deferred until:
> 1. The structural mode has shipped and been used in anger for
>    ~2 weeks.
> 2. We've seen which aggregators users actually click into (inform
>    which emergent-edge view is most valuable).
> 3. A decision on precompute vs on-the-fly Jaccard.

This doc addresses (3) with a recommendation. (1) and (2) are
calendar-time / telemetry waits the doc cannot satisfy. Before
implementation starts, confirm:

- [ ] Structural mode has been on `main` for at least two weeks.
- [ ] Click-through data on the peek is in hand, or the team
      consciously waives it.
- [ ] The R11 research doc (cited in the decision log but not
      currently in-tree) has been read against this scoping — in
      particular R11 D2's take on what emergent edges should mean
      in context, and R11 D3's stance on Jaccard vs other measures.

If any box is unchecked, surface that as a blocker rather than
proceeding on the proposals below.

---

## Open questions, with proposed answers

### Q1. What's a "member" of an aggregator?

The R11 decision log (D1) defines aggregators as `goal | concept |
decision` and participants as `research | task | session`. A "member"
needs to be the set that Jaccard operates over.

**Options:**

| Option | Set definition | Pro | Con |
|---|---|---|---|
| A | Direct neighbors (depth-1) with any edge type | Simple; matches the structural mental model | Ignores transitive relationships that are how vaults actually accrete |
| B | Depth-≤-2 neighbors, excluding other aggregators | Captures "this concept touches that research via a task" | Double-counting if the same participant is reachable through multiple paths; needs a set-membership dedupe |
| C | Participants only, at any reachable depth (≤ 3) | Matches the "this aggregator's participant footprint" intuition | Recursive-CTE cost grows; may need precompute |

**Proposed answer: Option B** — depth-≤-2 neighbors, participants
only, deduplicated by resource id. Depth-2 is the same budget the
structural subgraph already uses, so the query cost is familiar, and
excluding aggregators keeps A∩A from being inflated by a shared
concept neighbor (that's a different emergent-edge idea, and if we
want it we should name it explicitly later).

**Decision to validate:** does R11 D2/D3 already take a position on
this?

### Q2. Where's the emergent-edge cutoff?

Jaccard(A, B) = |A ∩ B| / |A ∪ B| is in [0, 1]. Drawing all non-zero
pairs produces an unreadable hairball on any realistic vault.

**Options:**

| Option | Rule | Pro | Con |
|---|---|---|---|
| A | Static threshold (e.g. ≥ 0.2) | Simple; same cutoff everywhere | Dense clusters get too many edges, sparse clusters get zero |
| B | Top-K per aggregator (e.g. K = 3) | Every aggregator has *some* emergent context | Can draw very weak edges in a sparse corner; asymmetric (A's top-3 may not include B even when B's top-3 includes A) |
| C | Both — threshold floor *and* top-K ceiling | Guarantees minimum signal + maximum noise | Two knobs to tune |

**Proposed answer: Option C** — Jaccard ≥ 0.15 floor *and* top-5 per
aggregator. Render only edges satisfying both. Asymmetric top-K is
resolved by treating the edge set as the *union* of each endpoint's
top-K, then re-filtering by threshold. The 0.15 / 5 numbers are
first-draft guesses to tune against a real vault during review.

**Additional knob:** in R11's visual language emergent edges are
rendered with a different stroke treatment than declared edges (D3).
`kg-handoff.md` doesn't pin the treatment; propose dotted line at the
`sourceFill` hue, 0.9 px, with line width scaled linearly by Jaccard
weight over [threshold, 1.0] mapping to [0.9 px, 1.6 px].

### Q3. Precompute vs on-the-fly?

This is the question `kg-handoff.md` explicitly flags as open.

**Option A — on-the-fly.** A new SQL query inside
`aggregator_subgraph` when `mode == 'meta-doc'`: for each pair of
returned aggregator nodes, compute the member sets and Jaccard score.
Discard the structural edge rows and emit the emergent ones instead.

  - *Pro:* always fresh; no invalidation story; zero new schema; one
    query swap.
  - *Con:* cost is O(aggregators²) in member-set intersections per
    request. Fine for today's ~30-aggregator vaults; questionable at
    200+. Acceptable until we hit a user vault that feels sluggish.

**Option B — precomputed.** New table `kb_emergent_edges (source_id,
target_id, weight)` populated by a background job that runs after
ingest. Query becomes a simple `SELECT ... WHERE weight >= 0.15`.

  - *Pro:* query cost is constant regardless of aggregator count;
    trivially cacheable.
  - *Con:* invalidation is non-trivial — any edge add/remove,
    resource soft-delete, or doctype change can shift member sets.
    Need either a periodic rebuild (stale view) or reactive
    invalidation (complex).

**Proposed answer: Option A first, Option B if/when it hurts.**
Ship on-the-fly with a documented performance envelope (≤ 200
aggregators / 2 k edges comfortable). Add a crate-level benchmark on
the happy path. Revisit with real numbers before adding schema.

This is *reversible*: the query can swap out for a table scan later
without changing the wire shape.

### Q4. Replace or overlay the structural edges?

When the user flips to meta-doc mode, do the explicit edges
disappear, or do we draw both?

**Proposed answer: replace.** The whole point of the toggle is a
different lens; mixing the two produces a mud-coloured diagram that
tells neither story clearly. Participants should also hide or
gray-out — meta-doc is an aggregator-first view.

Concretely: in `mode == 'meta-doc'` the server returns aggregator
nodes only (no participants, no sessions), plus emergent edges.
Session counts carry over as `⌊N⌋` annotations on the aggregators
themselves (union of the counts across members, deduped). The peek's
neighbors list for an aggregator in meta-doc mode should show other
*aggregators* sorted by Jaccard weight, not participants.

This needs a concrete R11 signoff — if R11 D2 explicitly wants both
overlaid, this proposal is wrong and we should instead dim declared
edges to 20 % alpha and draw emergent edges on top.

### Q5. Edge type semantics

Declared edges have a typed `edge_type` enum. Emergent edges don't
map onto any of them.

**Proposed answer:** extend the `EdgeType` enum with a new variant
`EdgeType::Emergent` (or `EmergentSimilarity`). The variant never
appears in `kb_resource_edges` — it only exists as a return-shape
tag on `GraphEdge`. Client-side `styling.ts` gets a new dash pattern
entry for the new type and renders it as proposed in Q2.

Alternative: add a separate `emergent_weight: Option<f64>` on
`GraphEdge` and keep the enum structural. That's cleaner (emergent
edges carry a numeric, not a nominal, signal) but means the wire
type has to carry a dead field in structural mode. The enum-variant
approach is more ergonomic; propose that.

### Q6. API surface

Two shapes to choose between:

**Option A — `mode` query param on `/api/graph/subgraph`.**

```
GET /api/graph/subgraph?owner=…&context=…&mode=meta-doc
```

Handler branches on `mode`; same response type, different contents.

**Option B — new endpoint `/api/graph/emergent`.**

Different shape, no branching, cleaner test surface.

**Proposed answer: Option A.** Same consumer
(`+page.server.ts` → `KnowledgeGraph.svelte`), same response type,
one client-side swap. The handler is a five-line match. The existing
`AggregatorSubgraphParams` struct grows a `mode: GraphMode` field,
no new handler file needed.

---

## Implementation sketch (assuming the proposals above)

### Server

1. `temper-core`
   - Add `#[derive(…)] pub enum GraphMode { Structural, MetaDoc }`.
   - Add `EdgeType::Emergent` to the enum *on the API return type
     only* — the DB enum stays unchanged; we serialize it but never
     select it.
2. `temper-api::services::graph_service`
   - `AggregatorSubgraphParams` gains `pub mode: GraphMode`.
   - `aggregator_subgraph` branches at the top:
     - `Structural` → unchanged today's path.
     - `MetaDoc` → new helper `emergent_subgraph(…)` that:
       - Resolves the aggregator node set via the same seed CTE,
         ignoring the depth-N participant expansion.
       - Runs `member_sets(aggregator_id)` returning `HashMap<Uuid,
         HashSet<Uuid>>` — a second query that returns
         `(aggregator_id, participant_id)` rows over the depth-≤-2
         neighborhood, filtered to `participant` doctypes.
       - Computes Jaccard pairwise in Rust (or SQL with
         `array_intersect`; prefer Rust for testability).
       - Emits `GraphEdge { source, target, edge_type: Emergent,
         weight }` for pairs passing the threshold and top-K filter.
   - New integration tests:
     - `emergent_subgraph_uses_shared_members_only`
     - `emergent_subgraph_respects_jaccard_threshold`
     - `emergent_subgraph_returns_only_aggregator_nodes`
     - `emergent_subgraph_hides_sessions_and_participants`
     - `emergent_mode_session_count_rolls_up_to_aggregator`
3. `temper-api::handlers::graph`
   - Parse `mode` query param with serde; default to `Structural`.

### Client

1. `lib/types/generated/graph.ts` regenerates with the new enum
   variant and `GraphMode` type.
2. `lib/components/graph/KnowledgeGraph.svelte`
   - `mode` state already exists (shipped in item #3). Route change
     from local-only to pushing through:
     - `+page.server.ts` reads `mode` from URL search params; passes
       into the API call.
     - When `mode` changes client-side, invalidate the load function
       via `invalidate()` or reload with `?mode=`.
     - Cytoscape elements rebuild from the new payload; fcose
       re-layouts.
3. `lib/graph/styling.ts`
   - New selector `edge.etype-emergent` with dotted stroke, computed
     line width from a data attribute, and a `data(weight)` pass-
     through from the payload.
4. `lib/graph/peek.ts`
   - `buildNeighborEntries` branches: in meta-doc mode, the emergent
     edges feed directly (weight determines sort order, not
     edge_type). Unit tests split by mode.
5. `ModeToggle.svelte`
   - Drop the "not implemented" stub. The toggle is live.

### Scope that stays out

- Weight-based animation on mode transition (fade structural out,
  fade emergent in). Nice to have, not required.
- Minimum member-set size floor (e.g. skip aggregators with fewer
  than 2 members). Can add when we see real-vault behavior.
- Tweakable threshold/K via UI controls. Server-side constants only
  for v1.

---

## Test-surface deltas

- `packages/temper-ui`: expect **~6 new vitest** (peek re-derivation
  in meta-doc mode, stylesheet rule presence, elements edge count by
  mode).
- `crates/temper-api`: expect **5 new integration tests** above, on
  top of today's 18.
- `crates/temper-core`: unit tests for a new
  `compute_jaccard_edges(aggregator_members: &HashMap<…>,
  threshold: f64, top_k: usize) -> Vec<EmergentEdge>` helper (pure,
  easy to test).

Expected pass counts at completion: **UI 88+/88+**, **graph_subgraph
23+/23+**, clippy clean, fmt clean, svelte-check clean.

---

## Open questions for the implementer

1. **Is Jaccard actually right?** R11 D3 said yes; worth a fresh
   look. Overlap coefficient `|A ∩ B| / min(|A|, |B|)` biases toward
   "this smaller aggregator is almost a subset of the bigger one,"
   which is arguably what a knowledge worker wants to see. If R11
   D3's rationale is fragile, consider offering both and letting
   `mode` carry a sub-parameter.

2. **What about goal↔goal or decision↔decision specifically?**
   Emergent edges between same-doctype aggregators may be the only
   view that matters (cross-doctype overlap is often accidental).
   Propose: ship cross-doctype initially, add a `same_doctype_only`
   filter later if users tell us cross-doctype noise dominates.

3. **Session count roll-up math:** when an aggregator "inherits" the
   session counts of its members in meta-doc mode, do we sum
   (counting the same session multiple times if it touches multiple
   members) or dedupe? Dedupe is almost certainly right — surface
   this as an explicit choice in code, not a silent SUM.

4. **Cold-start behavior:** what does the meta-doc view look like
   when the vault has only three concepts and no shared members?
   An empty canvas with a "No emergent links yet — meta-doc becomes
   useful as the vault accretes" helper would be kinder than a
   silent empty state.

5. **Deep-link shape:** `?mode=meta-doc&seed=<aggregator_id>`. Once
   meta-doc ships, the peek probably wants an "open in structural
   view" link back, which means the seed needs to survive a mode
   swap. Out of scope here but worth keeping in mind.

---

## What lands in `main` when this ships

- `GET /api/graph/subgraph?mode=meta-doc` works.
- `ModeToggle` drops the deferral stub; `meta-doc` actually switches
  the view.
- No existing tests regress.
- This scoping doc gets a short "Status: Implemented" amendment at
  the top pointing at the PR, but otherwise stays as the record of
  why the shape is what it is.

Nothing in this doc is load-bearing for the shipped structural view.
If the answers above turn out to be wrong mid-flight, the
revert-and-rethink path is a single server branch + a handful of UI
tests — small blast radius.
