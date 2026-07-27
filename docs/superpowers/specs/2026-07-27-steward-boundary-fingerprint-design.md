# The steward's change-detection frame moves without emitting an event

Design note for migration `20260727000040_steward_boundary_fingerprint.sql` and the Rust surfaces
that carry its two cursors. The migration itself is kept terse and points here; this is where the
reasoning lives.

Task `019f9bb3-e2cf-7710-9b90-db4ebefb8f64`. PR #557.

## The defect

`steward_ingest_delta` answers *"did anything change?"* by counting `kb_events` whose
`producing_anchor_id` falls inside `steward_team_contexts(p_cogmap)` — team-OWNED contexts ∪
contexts SHARED to a joined team or its ancestors (`20260716000010:58-84`). That set is the window
frame, and **the frame moves with no event of its own**:

- **SHARED — the sharp case.** `context_service::share` is a bare
  `INSERT INTO kb_team_contexts … ON CONFLICT DO NOTHING` (`context_service.rs:421`). No event is
  emitted, by an explicit product decision the module states in its own header: *"Context creation is
  a plain INSERT (no event emission — product decision 5: contexts are infrastructure)"*
  (`context_service.rs:10-11`). The resources already living in the newly-shared context carry
  `resource_created` events **below** the cogmap's watermark, so the count over the widened frame is
  `new_resources = 0` and the steward never ticks. An entire corpus becomes distillable and the map
  never learns it exists.
- **UNSHARED — the same hole in reverse.** `unshare` is a bare `DELETE` (`context_service.rs:456`).
  The frame *shrinks* with no event either, so material the map has distilled is now out of scope.
- **OWNED.** Ownership moves by `context_reassigned` — one event for N resources, and not even of the
  type the gate counts. The gate filters `type_name = 'resource_created'` (`20260716000010:110`)
  against a default threshold of 5, so one event of the wrong type clears nothing.
- **ANCESTRY.** `steward_team_contexts` reaches shares through `team_ancestors`, so re-parenting a
  team moves the frame without touching `kb_team_contexts` or `kb_contexts` at all.

Counting events inside a boundary cannot detect the boundary moving. This is a category error in the
measurement, not a tuning parameter — no threshold value and no per-event weight fixes it. It
strictly **under**-triggers, the direction `steward_team_contexts`'s own comment names as the
dangerous one (*"would UNDER-trigger … and silently stale the map"*, against over-approximation being
merely *"a wasted tick"*).

## The fix is a state comparison, not a new event

Product decision 5 is **not** overturned. The migration stops *depending* on an event that decision
guarantees will never exist. Each completed run stores a digest of its boundary; the next tick
recomputes and compares. Events remain the *"what landed inside the frame"* signal; the fingerprint
is the *"did the frame itself move"* signal. Different questions, different mechanisms.

**Stored, not derived**, and that asymmetry is the design content. The watermark could be a cursor
into the trail because the trail *has* history. `kb_team_contexts` is **current-state only**, so the
boundary cannot be reconstructed as-of a past watermark. Do not "improve" this into a derived value;
there is nothing to derive it from.

**Rejected, do not revive without new argument:** adding `context_shared` / `context_unshared`
events. It contradicts product decision 5, needs a projector + replay arm + backfill, and *still*
leaves the one-event-for-N-resources problem on the OWNS half. Worth doing later for ledger
legibility — never as the trigger mechanism.

## Why there is no backfill

`kb_cogmaps.steward_boundary_fingerprint` is nullable and NULL means *"never snapshotted"*, which
`IS DISTINCT FROM` renders as **moved**. Every pre-existing cogmap fires exactly once, then settles.

Backfilling a digest at migration time would be strictly worse: it would silently swallow every
boundary move that has **already** happened and never been distilled — which is the entire population
this fix exists for.

The operational consequence is real and intended: **the first tick after deploy sweeps every
team-joined cogmap once, including the L0 kernel**, then settles. Asserted by
`a_never_snapshotted_boundary_reads_as_moved`.

## Two hot loops, and the reasoning error behind both

Both were found during implementation, and in both cases the *first* design was worse than the bug
it replaced.

**1. The store-back was unreachable in exactly the canonical case.** The digest is written by
`DbBackend::advance_steward_watermark`, which originally took a **non-optional** `event_id` gated on
`steward_event_in_ingest_window`. But a boundary-move-only tick has `new_events = 0` and therefore
`max_event_id = NULL` — no id to pass, and any substitute is 404'd by the hygiene gate. That cogmap
would distill, fail to record its digest, and re-fire on the next tick and every tick after.

**Closed:** `event_id` is now `Option<Uuid>`. Absent leaves the watermark where it is; a supplied id
is validated exactly as strictly as before.

**2. An absent fingerprint storing NULL would do the same thing.** NULL is *"never snapshotted"* is
*"moved"*, so a caller that never sends one re-fires on every tick, forever.

**Closed:** absence recomputes the boundary at write time —
`COALESCE($3, steward_boundary_fingerprint($1))`. Degraded (it absorbs a boundary move that happened
*during* the run) but it **settles**. Callers holding the delta's value must pass it; that is the only
way a mid-run change survives to the next tick.

**The shared error:** *"over-trigger is the safe direction"* is true of a **bounded** excursion.
Applied to a state that never clears, the same sentence licenses an unbounded one. "Fires once more"
and "fires forever" are both *over-triggering*, and the safe-direction argument does not distinguish
them. Any such argument needs a named termination condition, and a check that the clearing act is
**reachable** in the case being argued about.

## Traps encoded in the SQL

Each of these is a place where the obvious form is wrong. The migration carries a one-line marker; the
reasoning is here.

**The empty-set trap — the zero-row digest must not be NULL.** An aggregate over zero rows returns
NULL, so a naive `sha256(string_agg(...))` over a cogmap with no contexts in scope yields NULL —
indistinguishable from *"never snapshotted"*, so that cogmap would report `boundary_moved` forever.
*"No contexts in scope"* is a real, stable state and gets a real, stable digest. `coalesce(…, '')`
goes **inside** the hash, which is this repo's existing answer to exactly this trap
(`20260715000030:57-58`, `20260726000030:58`, `canonical_functions.sql:601` all hash the
coalesced-to-empty aggregate rather than coalescing the hash afterwards). Empty scope digests to
`sha256('')` = `e3b0c442…`, a genuine member of the digest space that no non-empty scope can collide
with — the smallest non-empty aggregate is a 36-character uuid.

**`IS DISTINCT FROM`, not `<>`.** `p_fingerprint` is NULL for a never-snapshotted cogmap, and
`NULL <> digest` is NULL, which a `WHERE` clause reads as *"not moved"* — exactly inverting the
intended default and rendering the whole fix inert.

**The cardinality trap.** The fingerprint is read through a scalar subquery over a CTE, **not**
`FROM win CROSS JOIN fp`. The join shape looks tidier and is wrong: projecting `fp.digest` alongside
the aggregates would demand a `GROUP BY`, and a grouped query over an empty `win` returns **zero**
rows instead of one. Callers (`steward_service::ingest_delta`, via `fetch_one`) require exactly one
row for a cogmap with no new events.

**`fp AS MATERIALIZED`.** Two references already defeat inlining, but declaring it makes
one-call-per-invocation the floor rather than a planner behaviour — the lesson `20260727000010:60-61`
paid for.

**DROP + CREATE, not `CREATE OR REPLACE`.** `20260727000010:41-44` argues for replacing in place,
correct there because its return columns were unchanged. Both functions here **gain** return columns,
and `CREATE OR REPLACE FUNCTION` cannot change a return type — the same constraint
`20260716000010:48-52` hit, with the same remedy. `steward_ingest_delta` is dropped explicitly rather
than left as a 2-argument overload; an overload would let a stale caller keep resolving to the blind
version.

## One definition of "moved"

`boundary_moved` is computed **once**, inside `steward_ingest_delta`, and returned. Neither
`steward_drift_sweep` nor the Rust read surface restates the comparison — the same *"REUSE, not
re-implement … one source of truth for 'what counts as drift'"* rule `20260705000002:3-4` states for
the sweep's use of the delta.

This matters because the threshold gate **already** has two implementations (SQL
`20260705000002:29`, and Rust's `exceeds_threshold` in `steward_service::ingest_delta`), and that
pair is precisely the drift this rule exists to prevent. A second copy is not added.

**The sweep is the load-bearing half.** `DbBackend::steward_dispatch_tick` →
`steward_service::drift_sweep` → `steward_drift_sweep` is the path the cron actually runs and the
only one that enqueues work. The Rust `exceeds_threshold` on the single-cogmap read is an
informational surface; a fix landing only there would fix nothing that runs.

## `ORDER BY boundary_moved DESC, new_resources DESC, cogmap_id`

A boundary-moved row typically has `new_resources = 0` — the **minimum** of the incumbent sort key —
so under the old ordering it would sort behind every map with a single new resource.

The classes are not symmetric, and that is what decides it: a boundary move is **self-clearing** (the
run that consumes it stores the digest and the row does not come back), while count-drift **recurs**
every tick until distilled. Putting the self-clearing class first bounds its latency to one tick and
costs the recurring class nothing.

This is **presentation, not admission**: the sweep has no `LIMIT` and `steward_dispatch_tick`
enqueues *every* returned row before the cap is applied downstream by `workflow_job_service::claim` —
unlike `audit_drift_sweep`, where the cap sits on the `ORDER BY` and starvation is structural
(`20260727000010:68-72`).

The trailing `cogmap_id` makes the sort total. `20260705000002` calls itself a *"Deterministic drift
sweep"* while sorting on a single non-unique key; `20260727000010:89` ends its ordering with
`k.finding_id` for the same reason.

## Provenance

Retargeted from register correction **C-7**
(`2026-07-25-auditor-trigger-model-outcome-register.md:452`), which attributed this defect to the
auditor's *"tier 2"*. **Tier 2 does not exist** — `auditor_watermark_event_id` and
`auditor_context_delta` return zero hits across `migrations/` and `crates/`, and the only auditor
selection surface that ships is `audit_drift_sweep`, which has no threshold and no boundary count.
The register's own disposition banner says as much. The reasoning was sound; the subject was
misidentified. C-7 remains design input for tier 2 if it is ever built, and tier 2 would inherit this
defect by construction.

## Not addressed here

Whether the steward should ever tick the L0 kernel at all. L0's charter declares ambient steward wake
= never, but `steward_drift_sweep` does not encode that — and did not before this change either,
since the counted arm would have surfaced L0 just as readily. Pre-existing, now more visible.
