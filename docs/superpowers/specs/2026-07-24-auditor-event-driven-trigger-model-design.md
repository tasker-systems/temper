# The auditor's trigger model — event-driven staleness, not coverage-chasing

- **Date:** 2026-07-24
- **Status:** Design — draft for review
- **Goal:** Evidential standing as substrate (`019f81e9-25f3-7fe1-b563-4acca2e391eb`)
- **Relates to:** Set 4 (steward's three jobs — grow → tend → reap), Set 5 (the adversary as
  citation auditor, shipped PR #531)
- **Supersedes in part:** Set 5 §6.3's deferred "reaper pass", which this reframes rather than
  implements

## Why this exists

Set 5 shipped the auditor's *reach* predicate but not its *change* predicate. The steward has
both; the auditor inherited only the first:

| | steward | citation auditor |
|---|---|---|
| reach — which maps may I touch | `steward_candidate_cogmaps(principal)` | **same function**, reused |
| change — has anything happened since I last looked | `steward_ingest_delta(cogmap, watermark)` against `kb_cogmaps.steward_watermark_event_id` | **none** |

With no change predicate, `audit_drift_sweep` fell back to chasing *coverage*: it selects any
finding with at least one cited source carrying zero audits, and re-selects it every tick until
that source is covered. The migration states the consequence in its own header — a citation the
auditor declines to verdict *"will re-head this queue every tick with the SAME `uncovered` count
forever."*

That produces a system that is **neither one-shot nor refresh-based, coherently**. It
re-dispatches *until covered*, which is one-shot in intent; but the agent's instructions say
*"emit one verdict per auditable citation"* with no skip-if-already-audited step, so each
re-dispatch appends another verdict to every already-covered citation in that finding. The
result is refresh-shaped in effect, on a cadence nobody chose.

**Nothing here is deployed and there are zero audit rows in any environment.** The cadence, the
selection predicate, the dispatch granularity, and the agent instructions are all still open.
This spec settles them together rather than defending any one of them.

## The bar this design must clear

Not correctness in isolation. The mechanism must not be able to **cycle**, to **over- or
under-count**, or to be **manipulated by volume**. A scoring rule that is provably right against
a workflow that can loop is not right. Every decision below is justified against that bar.

## D1 — Trigger on change, not on a clock

**Decision.** The auditor re-audits a citation when something it weighed has **materially
changed**, established by comparing event watermarks. The cron stays, but only as the *wake*;
the sweep's job becomes "what changed since I last looked," exactly as the steward's does.

The auditor's own instructions name four judgement inputs, and each has an event signature:

| What the auditor weighs | Goes stale on |
|---|---|
| the connection itself | `block_mutated`, `block_folded`, `resource_updated` (finding or source), `block_provenance_corrected` |
| the citing act's recorded confidence | never — immutable once written |
| corroboration in the surrounding map | `resource_created`, `resource_rehomed`, `relationship_asserted` / `_retyped` / `_reweighted` / `_retracted` / `_corrected` / `_folded` |
| the size of the citation set | `block_provenance_annotated`, `block_provenance_corrected` |

**Rationale.** This is the only formulation under which the rest of the standing math means
anything. Under a clock, an unrefreshed verdict is merely old, so decay penalizes it — while the
auditor refreshes hourly regardless, making within-bucket recency inert and retraction dead
letter. Under change-triggering, an unrefreshed verdict is a verdict about **something that has
not changed**, which is a genuine stability signal; successive verdicts from one principal become
successive opinions about *changed* evidence, so "the newest dominates" starts to mean something
and a 30-day half-life is defensible rather than arbitrary.

## D2 — Two tiers of staleness, because corroboration is map-wide

A single watermark cannot express this. Direct change is per-citation; corroboration change is
inherently map-wide — "does this connection stand alone or is it echoed elsewhere" depends on the
whole observable space, so *strictly*, any new resource invalidates every verdict that rested on
"stands alone." True, and operationally explosive: it re-audits everything, which is the current
defect wearing better manners.

**Decision.** Split it, and borrow the steward's threshold for the explosive half.

### Tier 1 — direct staleness (per citation, no new state)

The citation, its citing block, or its cited source changed since the last audit **of that
citation**. This needs **no new column**: `kb_citation_audits` already records
`audited_by_event_id`, and `kb_events.id` is UUIDv7, so `e.id > <last audit's event id>` is a
time-ordered cursor. The watermark *is* the trail.

That is also the right idiom for this subsystem: standing is recomputed live from the trail on
every read and never trusted from the memo (`resource_standing_shape`). A derived watermark
conforms; a stored one would be a second source of truth to drift.

### Tier 2 — contextual staleness (per cogmap, at a threshold)

The map's observable space moved materially, measured as the steward measures it — against
`kb_cogmaps.auditor_watermark_event_id`, a new column mirroring `steward_watermark_event_id`, and
a delta function mirroring `steward_ingest_delta` (whose "observable space" is already correctly
defined as the cogmap's team's contexts, **owned ∪ shared**).

A threshold, not a boolean — `steward_drift_sweep` already does exactly this with
`WHERE d.new_resources >= p_threshold`. One new resource must not invalidate a map's every
verdict.

## D3 — The material-event set, and the exclusion that prevents the cycle

**Decision.** Staleness is computed over a **named allow-list** of event types, never "any
event."

**`citation_audited` is excluded, and this is the load-bearing exclusion.** If audits count as
material change, the auditor's own writes re-arm its own trigger — a tighter and faster loop than
the coverage one this replaces, and one that would also let any principal keep a map permanently
"stale" by auditing into it.

Also excluded: `salience_refreshed`, `region_materialized` (derived projections, not new
information); `invocation_closed`, `delegated_launch` (envelope bookkeeping); everything in the
`admin` and `system` categories.

> **Tripwire for phase 4.** `relationship_decayed` is **schema-only today — its mechanics are
> deferred to phase 4** (`crates/temper-core/src/types/relationship_events.rs:96`). Nothing emits
> it, so it cannot self-arm now. If phase 4 makes it timer-driven, admitting it to the material
> set would re-arm contextual staleness on every tick — the cycle this spec exists to prevent,
> arriving later through a different door. Decide it explicitly when phase 4 lands.

## What this makes coherent

- **Decay** arbitrates between competing verdicts, which is what its own comment always claimed
  (`20260724000120:177`) and what a clock-driven refresh made false.
- **The per-auditor collapse** (shipped on `jct/set5-per-auditor-collapse`) becomes the piece
  that makes an event-driven auditor *safe*: one principal re-weighing across many change events
  must remain one voice. Its correctness does not depend on this spec, but its purpose is clearer
  under it.
- **Retraction** becomes real: an auditor's successive verdicts are now spaced by actual change
  rather than by cron ticks, so the newest genuinely dominates.
- **The reaper** (Set 5 §6.3, Set 4's growth) shrinks. A declined citation no longer re-heads a
  queue forever, because "not yet audited" stops being a selection signal on its own. A terminal
  "cannot assess" verdict is still wanted, but it is no longer load-bearing against an infinite
  loop.

## Open questions — genuinely undecided

1. **Do `property_*` (facet) events count as material?** A facet set on the finding or the source
   arguably changes what the connection asserts; equally, facets are often bookkeeping. Argue it
   either way — needs a decision, not a default.
2. **Threshold value and shape for Tier 2.** The steward counts `resource_created` only. The
   auditor cares about relationships too, which the steward's delta does not count. Does Tier 2
   need its own delta function rather than a reuse?
3. **Dispatch granularity.** The queue is per-cogmap and the job payload is a finding list. Tier 1
   is per-*citation*. Does the payload become a citation list, or does the agent re-derive which
   citations are stale on arrival?
4. **First-audit selection.** Change-triggering answers "re-audit"; a never-audited citation has
   no watermark. Coverage is still the right signal for the *first* pass — so the sweep is
   plausibly `uncovered OR stale`, and that union needs stating.
5. **Half-life against the new cadence.** 30 days was chosen for a clock-driven world. Under
   change-driven refresh it should be revisited, and it is explicitly tunable (Set 5 §4.1).

## Non-goals

- Changing the aggregation. The three-stage collapse is correct under any trigger model.
- Supersession or any retraction-as-a-verb. The trail stays append-only.
- Building the reaper. This reduces its urgency; it does not replace it.
- Any new authorization surface. Reach stays `steward_candidate_cogmaps` ∩ `resources_visible_to`.
