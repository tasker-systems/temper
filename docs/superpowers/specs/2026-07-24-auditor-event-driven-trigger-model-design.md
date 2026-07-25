# The auditor's trigger model — event-driven staleness, not coverage-chasing

- **Date:** 2026-07-24
- **Status:** Design — **accepted**; all eight decisions settled 2026-07-24. Ready to plan.
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

## D4 — Facet events are metadata, not content, and are excluded

**Decision.** `property_asserted` / `_set` / `_folded` / `_retracted` / `_reweighted` are **not**
material.

`kb_properties` is `(owner_table, owner_id, property_key, property_value, weight)` — annotation
*about* a thing, never the content *of* it. Facets are intentionally metadata, and **we are not
auditing the content of properties, so they do not count as mutation.**

This follows from the auditor's own boundary, which is the tightest available justification:
*"assess only whether the source carries the connection claimed; never whether the claim is true,
and never what the source says."* A facet changes neither what the block claims nor what the source
contains. If it cannot enter the judgement, it cannot invalidate the verdict.

The apparent counter-example — a facet on a *relationship*, which would bear on corroboration —
costs nothing, because relationship changes already emit their own events (`relationship_reweighted`,
`relationship_retyped`) and those **are** in the material set.

## D5 — The auditor gets its own delta, over a shared observable space

**Decision.** A new `auditor_context_delta(p_cogmap, p_watermark)` counting the **material set**;
**not** a reuse of `steward_ingest_delta`.

Reuse fails twice, and both failures are by design rather than oversight — the steward's delta
serves a different purpose:

- it counts `resource_created` only, so it is blind to the relationship churn the auditor most
  cares about (undercount); and
- its `new_events` counts *every* event in scope, including `citation_audited` — so wiring the
  auditor to it would re-arm the auditor through the back door, defeating D3's exclusion.

**But the observable-space definition genuinely is shared and must not drift.** Extract the
steward's inline "team contexts, owned ∪ shared" CTE as `cogmap_observable_contexts(p_cogmap)` — a
new function, additive, touching no shipped one — and have both deltas call it. One spelling of
"what this cogmap can see."

Threshold stays a caller-supplied parameter with a clamped default, as `clamp_auditor_cap` already
does for the finding budget.

## D6 — Dispatch a citation list, not a finding list

**Decision.** The job payload becomes `[{finding_id, citations: [{block_id, source_id}]}]` — the
stale citations themselves, not the findings that contain them.

This is **context management, and it is the same discipline you would apply to a person: do not
hand an agent something you do not want reasoned about.** Being economical with what you pass is
not merely an efficiency measure; the information you supply is the information that gets acted on.

It is also the difference between an instruction and an invariant. With a finding list, "do not
re-audit an already-covered citation" can only ever live in a markdown file — and Set 5's Critical
already taught this lesson: the sweep was principal-scoped and the *claim* was not, because **a gate
is only as strong as the narrowest path around it**. Prose in an instructions file is a wider path
than a queue payload. With a citation list the agent cannot re-audit a non-stale citation, because
it was never handed one, and the whole defect class disappears at the queue.

Cost, accepted: a larger and more coupled payload. It is bounded by the same finding budget that
bounds the sweep today.

## D7 — Selection is `uncovered OR stale`, with `uncovered` scoped to what this principal may audit

**Decision.** Coverage governs the **first** audit; change governs **re-audit**. The sweep selects
the union.

Change-triggering cannot select a never-audited citation — it has no watermark to compare against —
so coverage remains the right first-pass signal, and the union is what makes both work.

**The subtlety that makes it correct:** "uncovered" is *permanent* for a citation this principal
cannot audit. The sharpest case is a **self-authored** citation: `AuditAuthority` 404s it forever,
so it never gains coverage, so it re-heads the queue every tick — the loop surviving its own fix.

The sweep already states the governing rule, and applied it to readability only:

> *"Filtering through the SAME predicate the gate uses makes the queue a SUBSET of what the gate
> admits by construction, so the auditor is never handed work that will 404."*

So `uncovered` must be scoped to citations **this principal could actually audit** — readability
*and* non-self-authorship. Same rule, one more conjunct. Costs a join the sweep does not do today;
worth it, because without it the queue offers work the gate refuses, which is the exact condition
that rule exists to forbid.

## D8 — The 30-day half-life stands

**Decision.** Unchanged.

The concern was that a half-life chosen for a clock-driven world would misbehave under change-driven
refresh — a quiet map's good verdicts rotting while nothing was wrong. **That concern is unfounded,
and the reason is worth stating because it is easy to get backwards.**

`citation_quality` is a weighted **mean**, `sum(w·v)/sum(w)`. For a single audit that is
`(w·v)/w = v` — full value, regardless of age. A ten-year-old lone `+1.0` still reads `+1.0`. Decay
is therefore **purely relative**: it never fades a verdict in absolute terms, it only arbitrates
between *competing* verdicts — exactly what `20260724000120:177` always claimed. At total underflow
the source drops out of the mean entirely rather than decaying toward zero, which the
`a_source_whose_every_auditor_decayed_drops_out_of_the_mean` test pins.

So the half-life is cadence-independent in the way that matters. The one place cadence bit was
within-bucket recency at hourly repeat; change-driven triggering spaces an auditor's successive
verdicts by real change, which is precisely the interval 30 days is a sensible arbitration window
for. It remains tunable (Set 5 §4.1) if evidence later says otherwise.

## Non-goals

- Changing the aggregation. The three-stage collapse is correct under any trigger model.
- Supersession or any retraction-as-a-verb. The trail stays append-only.
- Building the reaper. This reduces its urgency; it does not replace it.
- Any new authorization surface. Reach stays `steward_candidate_cogmaps` ∩ `resources_visible_to`.
