# Ledger Projection Spiral — diagram design

**Date:** 2026-06-11
**Branch:** `jct/ledger-projection-spiral-diagram`
**Status:** ported — live as the HERO on `/cognitive-maps/the-substrate-beneath-it`
**Scope (decided):** port the **v2 "whole-map-materializing"** build only. Do **not**
build the single-node-lifecycle helix alternative in this pass.

## What this is

A new cognitive-maps diagram that shows how the append-only event ledger
(`kb_events`) materializes the point-in-time projections — `kb_resources`
(vertices), `kb_edges` (relationships), and `kb_cogmap_regions` (the convergence
glow) — by drawing the ledger as a tilted plane at the base and the graph rising
out of it over time. It is the moving, time-aware successor to the existing
static `LedgerSpineDiagram.svelte` (which makes the same events-are-primary
claim as a flat fan-out).

The diagram is **interactive**: a scrubber (and a Replay button) walks the ledger
event-by-event, so you watch nodes, edges, and regions appear in `occurred_at`
order. This is the honest demonstration of "the ledger is primary; every higher
surface is a projection materialized at read time."

## Port target

`packages/temper-ui/src/lib/components/cognitive-maps/diagrams/LedgerProjectionSpiral.svelte`,
sitting beside `LedgerSpineDiagram.svelte` and the other twelve diagrams.

Then wire it into the set:
- add it to the `the-set/+page.svelte` `SHOW` array (it belongs with
  "The substrate beneath it" thematically, but it is its own picture — decide
  whether it joins that page or earns a placement of its own).
- if it gets a home page, follow the `id`-namespacing-by-prop convention every
  diagram uses (`let { id = 'ledger-projection-spiral' }: { id?: string } = $props()`).

## Reference artifact

`docs/superpowers/specs/mockups/2026-06-11-ledger-projection-spiral.html` is the
complete, framework-agnostic build — open it in a browser to see it render with
interaction. It is deliberately zero-dependency (inline hex, script-driven DOM
mutation). **The geometry math in it is correct and should be carried over
verbatim**; only the framework wrapper changes.

## The port (mechanical, not a reinvention)

The existing diagrams are pure declarative Svelte — they template SVG directly,
namespace `<defs>` by the `id` prop, and never mutate the DOM from script. The
reference artifact uses `createElementNS` + `setAttribute` because it had to be
dependency-free. Translate as follows:

| Reference (vanilla) | Svelte port |
|---|---|
| `const EVENTS = [...]` | `$state` (or a module-const if never mutated) |
| `<input id=scrub oninput=setStep>` | `<input type=range bind:value={step}>` |
| `setStep(n)` imperative show/hide | `$derived` per-element opacity, bound in the template |
| `createElementNS` rows loop | `{#each EVENTS as e, i}` rendering `<g>` rows |
| `createElementNS` risers loop | `{#each visibleRisers as r}` |
| `togglePlay()` + `setInterval` | a `$state` `playing` + `setInterval` in an effect; clear on destroy |
| inline `#7eb8da` etc. | `var(--…)` tokens (table below) |
| `id="up"`, `id="glowA"` defs | `id="{id}-up"`, `id="{id}-glowA"` (namespaced) |

The per-step logic is just: an element is visible iff its event index `< step`;
opacity scales with recency `(i+1)/EVENTS.length`; the two `region_materialized`
events flip the glow groups on; the final `relationship_reweighted` flips the
ghost + the STALE readout. All of that becomes `$derived` rather than imperative
mutation.

### Palette tokens (from `packages/temper-ui/src/app.css` `:root`)

| Reference hex | Token | Used for |
|---|---|---|
| `#7eb8da` | `--temper-blue` | telos + α nodes, risers, spine, FTS glow A |
| `#82c99a` | `--graph-session` | β / deployment region + nodes |
| `#d48ac7` | `--graph-concept` | the `express` edge (e5) |
| `#0a0a0f` | `--obsidian` | node fills |
| `#e8e4df` | `--parchment` | primary labels |
| muted whites | `--chalk` / `--graphite` | secondary / tertiary text |
| `rgba(255,255,255,0.06)` | `--rule` | hairlines |

Serif labels → `var(--font-serif)`; mono schema names → `var(--font-mono)`.

## Honest basis — every visual claim traces to schema/functions

This is the docstring the component must open with (the house pattern: each
diagram cites the exact columns/functions its picture rests on). Verbatim
mapping:

| Visual element | Schema / function ground truth |
|---|---|
| Base plane, append-only, left→right | `kb_events`; `occurred_at`; append-only trigger `kb_events_append_only()` |
| Row labels | the `kb_event_types` registry seeded in `03_seed.sql` |
| Riser to a resource/cogmap node | `genesis_event_id` (stamped by `_project_blocks` / `_project_cogmap_seeded`) |
| Riser to an edge | `asserted_by_event_id` (stamped by `_project_relationship_asserted`) |
| Region watermark riser | `shape_materialized_event_id` (set by `_project_region_materialized`) |
| Dash pattern = edge_kind | the `edge_kind` enum `{express, contains, leads_to, near}` (`01_schema.sql`) |
| Opacity = recency | `last_event_id` age — newer events drawn harder |
| Region glow with fogged members | `kb_cogmap_regions` (centroid/salience/member_count); `cogmap_shape()` returns salience/label/count and **never** member identities |
| α "first-week confidence" converged & tight | bound by genuine `near` + `express` content affinity (the α cast in `03_seed.sql`) |
| β "deployment" looser | bound by a shared `facet` property at weight 1.5, **not** by content (the β cast) |
| `solo-retro` isolate that never joins | content reads like α but has **no facet, no edge**; cosine WOULD merge, declared does NOT (the S6d falsification) |
| Ghost behind current + STALE flip | final `relationship_reweighted` advances `last_event_id` past `shape_materialized_event_id`; `cogmap_staleness()` then reports `is_stale = true` |

The cast (telos charter, α/β concepts, regulation, solo isolate, the late
reweight touch) is lifted directly from the worked scenario in
`schema-artifact/03_seed.sql` — keep the names aligned so the picture and the
seed stay mutually checkable.

## Known faithfulness tensions (carry forward, do not silently "fix")

1. **Convergence-as-motion vs point-in-time.** The animation interpolates between
   two materializations; the schema only stores snapshots. `cogmap_shape()`
   returns a materialized state, not a trajectory. The motion *between* states is
   a projection artifact, not a stored thing — which is, satisfyingly, exactly
   the model's own claim about itself (cf. confidence-inventory OQ #15: reproduce
   vs record). Acceptable for a demo; worth a one-line caption acknowledging it
   rather than implying the system stores the becoming.

2. **β drawn as "forming" (lower opacity).** In the seed, β actually
   materializes as a real region via facet-overlap (the falsification suite
   confirms it forms). The staggered opacity reads "convergence as motion"; if
   the intent shifts to showing the *finished* projection, both regions should
   read converged. Current choice: staggered, to keep the motion legible. Flag,
   don't assume.

3. **Ghost sits on the regulation node for narrative clarity.** Strictly, the
   late `relationship_reweighted` touches the regulation `express` **edge**, so
   it is the edge's `last_event_id` that advances. Moving the ghost onto the edge
   is more literally correct but visually subtler. Decide together when looking
   at it rendered.

## Open call for the working session — resolved 2026-06-11

Decided with the diagram rendered locally:

- **Placement.** Two homes:
  1. *Replaces* `LedgerSpineDiagram` as the HERO on
     `/cognitive-maps/the-substrate-beneath-it` — the far more powerful
     communication of the events-are-primary claim.
  2. *Leads* the `the-set` page as a featured synthesis figure above the grid,
     with framing text ("Start here · the whole argument in motion") — it is the
     single clearest statement of the value prop the other twelve diagrams
     elaborate in precise parts. Placed as a standalone lead (not a plate), so
     its interactive controls aren't trapped inside a linked `<a>`.

  The static `LedgerSpineDiagram` is **kept** as a plate in the-set grid (the
  at-a-glance reading), unchanged. So: spiral leads both pages; spine survives
  as a grid elaboration.
- **Tension #2 (β staggered vs both-converged): kept staggered.** The staggered
  opacity *is* the thesis — convergence-as-motion is the whole reason this beats
  the static spine. Drawing both regions converged would flatten the one thing
  the scrub exists to show. The `shows` prose carries the honesty (β bound by
  facet, not content), so nothing false is implied.
- **Tension #3 (ghost on node vs edge): kept on the node.** Ghost-on-node reads
  instantly as "superseded"; ghost-on-edge is too subtle to parse and trades a
  clear beat for a literal correctness the honest-basis text already states
  (`last_event_id` advancing past `shape_materialized_event_id`). The glyph
  simplifies; the caption stays truthful.
- **Reference off-by-one fixed.** The mockup's scrubber was `max="12"` against 13
  events, leaving the final `relationship_reweighted` → STALE flip reachable only
  via Replay. The port sets `max={EVENTS.length}` so the payoff is directly
  scrubbable; resting frame stays at the healthy-converged state (step 12).

## Why this earns its place (a note, not a requirement)

The visualization coiled *cleanly* without fighting the schema — the ledger-as-
ground, lineage-columns-as-risers, fogged-members-as-glow all fell out of the
DDL rather than being imposed. That "fewer primitives describing more phenomena"
ease is the same soundness tell already flagged for the semantic model. Mild
evidence the primitives are well-chosen; not proof. Worth noticing.
