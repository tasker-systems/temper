# Drain instrumentation — the operator can see how far behind the queues are

**Date:** 2026-08-03
**Goal:** `019f9404-2a4e-7530-8744-92ae4ab6d83e` — *OpenTelemetry across the deployable surface — one
trace per user action*. Specifically its done-criterion *"the `maxDuration: 300` internal function
instrumented — it is the longest-running compute and the current blind spot."*
**Related goal:** `019fc46c-5731-75d2-b00f-891069ce6f31` — *A write completes without waiting on
projection*. This spec is **not** filed under it, and §"Which goal owns this" says why.

---

## The problem, in one sentence

An operator can see **how long a drain pass took** and cannot see **how far behind the drain is** —
and after PR #636 those became different questions for the first time.

## Which goal owns this, and what stays uncovered

Goal `019fc46c` carries a clause `projection-lag-is-readable`:

> A reader of an anchor's shape can determine, at read time, how far that shape trails the ledger.

That clause names a **per-anchor reader, inline with a shape read**. The reader this spec serves is
the **operator watching the drain**, who wants an aggregate across anchors over time. Those are
different readers wanting different artifacts, so the work is filed under `019f9404`, whose stated
scope already covers *"a metrics taxonomy owned in one module, covering temper's own domains"* and
names the `api/internal` function as the blind spot.

**`projection-lag-is-readable` therefore remains declared uncovered, deliberately.** This spec does
not move it, and a reader must not infer that instrumenting the drain covered it. It names a reader
we are choosing not to serve yet. Coverage is never inferred from absence — see §"What this does not
do".

**A correction the register needs.** `019fc46c` records `projection-lag-is-readable` as
*"mechanism unbuilt"*. That is inaccurate and should not be inherited: `cogmap_staleness`
(`migrations/20260624000002_canonical_functions.sql:527`) has existed since the canonical schema, its
own comment states the intent almost verbatim — *"ON-READ aggregate, not a denormalized watermark…
Stale reads are allowed and LEGIBLE — this reports staleness, never blocks on it"* — and it reaches
real surfaces through `cogmap_analytics` (`handlers/cognitive_maps.rs:319`, and the MCP tool of the
same name). Whoever eventually takes that clause is **extending an incumbent**, not building
greenfield, and inherits four known gaps:

1. **Contexts have no staleness read at all.** `anchor_shape` and `anchor_region_metrics` were
   generalized to both anchor kinds; `cogmap_staleness` never was, and there is no `anchor_staleness`
   or `anchor_analytics`. The largest anchor in production is a **context** (`temper`, 429 regions).
2. **The queue is invisible to it.** It predates #636, when settling was inline. It cannot express
   *enqueued, not yet run* — measured at 0.84–60s.
3. **`materialized_at` is systematically early.** It reads the materialize event's `occurred_at`,
   which defaults to `now()` — transaction-start. The event fires *first* (`write.rs:279`) and the
   expensive work happens after it in the same transaction, so on a 33s formation pass the timestamp
   precedes the shape it names by ~33s.
4. **It is not on the shape read.** `anchor_shape` returns regions and no staleness column.

---

## Grounding

Every claim below was executed or read on disk, not recalled. Findings that came from a live system
say so.

### G1 — The dispatch span is flat and transport-only

`GET /api/region/dispatch` is exported and queryable. A live TraceQL search returned the 22:15:20Z
formation pass as `durationMs: 33337`, `spanCount: 1` — a single root span carrying route and
duration and nothing about the work. *[verified against live Tempo, 2026-08-03]*

### G2 — Neither drain has a single span

`rg -n "instrument|info_span" crates/temper-services/src/services/{embed,region}_service.rs` returns
nothing. Both are entirely uninstrumented; the dispatch span in G1 is the HTTP middleware's root
span, not theirs.

### G3 — There is no metrics pipeline, by design

`crates/temper-telemetry/src/` is `export.rs`, `init.rs`, `lib.rs`, `link.rs`, `propagate.rs`,
`redact.rs`, `request_span.rs`. No meter, no counters, no histograms. RED comes from Tempo's
span-metrics processor, and `lib.rs:139-142` records the measured constraint:

> Tempo's span-metrics and service-graph processors derive RED metrics and graph edges only from
> `server`/`client` spans — so `http_request` and `mcp_request` were received and stored fine yet
> excluded from `traces_spanmetrics_*` and the service graph.

### G4 — TraceQL metrics works over these spans, so no pipeline is needed

`{ name = "GET /api/region/dispatch" } | quantile_over_time(duration, .5, .95) by (span:name)` over a
6h window returned a real series (p50 ≈ 0.0168s). *[verified against live Tempo, 2026-08-03]*

This is the load-bearing finding: **the operator's dashboard can be built from spans alone.** No new
telemetry infrastructure, and no contorting span kinds to satisfy the span-metrics processor.

### G5 — Everything is sampled

`export.rs:37-42`: the SDK default is `ParentBased(AlwaysOn)`, and *"there is never a remote
parent… the parent branch is unreachable and the root sampler always decides."* So metrics derived
from these spans are **complete, not an estimate** — which is what makes counting spans a legitimate
way to answer an operator's question here.

### G6 — Queue wait is reachable without a migration

`workflow_job_claim_anchor` returns `(id, cogmap_id, context_id, attempts, payload)`
(`migrations/20260802000020_workflow_jobs_anchor_scope.sql:84-85`) — **no `enqueued_at`**.

Postgres cannot `CREATE OR REPLACE` a function with a changed `RETURNS TABLE`; widening it requires
`DROP FUNCTION`, which is **non-additive** and cannot ride the additive-only-on-`main` deploy. It
would force an operator cutover for a telemetry field, which is the wrong trade.

**A plain `SELECT` on the claimed ids avoids the question entirely.** This spec requires zero
migrations.

### G7 — The interesting values are already computed and then discarded

`region_clocks::tick` returns `materialized` and `salience_refreshed`. `RegionDispatchSummary`
already tallies `claimed`/`completed`/`deferred`/`materialized`/`salience_refreshed`
(`region_service.rs`). All of it is serialized into a JSON response body for the cron and thrown
away. The work is largely **stop discarding what the drain already knows**.

### G8 — The twins are structurally identical

`embed_service::dispatch_tick_inner` and `region_service::dispatch_tick_inner` share their shape:
persona/dispatch constants → reap → cap → summary → `start = Instant::now()` → do-while claim loop
with a deadline check. `region_service.rs`'s module doc states the intent:

> It deliberately mirrors `embed_service::dispatch_tick`… That symmetry is the point — the two
> workers have the same failure modes and should not drift in how they handle them.

Instrumenting one and not the other would create exactly that drift.

---

## Design

### Span shape — three levels, two drains, one contract

```
GET /api/region/dispatch          server · unchanged, transport only
└─ region_dispatch                the tick: backlog + tallies
   ├─ region_job                  one per claimed job
   └─ region_job
```

The HTTP root stays purely transport. **This is not stylistic** — CLAUDE.md's span-field convention
states that acts get their own span and never record onto the root, and
`tests/e2e/tests/logging_test.rs` asserts the carrying span is not the root. The trap it exists to
catch: with no child spans, `Span::current().record(...)` resolves to the root and *works*, right up
until the first nested span makes it silently wrong.

The `*_dispatch` span is 1:1 with the root and is still worth its cost for two reasons: it keeps
work-shaped data off a transport span, and it gives TraceQL a stable name to key on that is
independent of the route (a route rename must not break every operator query).

### Field set

Split into a shared core and a per-drain tail, so the twins cannot drift on the part that matters.

| Span | Shared core | Region tail | Embed tail |
|---|---|---|---|
| `*_dispatch` | `backlog_depth`, `oldest_pending_age_ms`, `claimed`, `completed`, `deferred`, `failed` | `materialized`, `salience_refreshed` | `redriven`, `partial`, `chunks_embedded` |
| `*_job` | `queue_wait_ms`, `attempts`, `outcome` | `anchor_id`, `anchor_kind`, `materialized`, `salience_refreshed` | `resource_id`, `chunks_embedded` |

`outcome` is a closed vocabulary — `completed` | `deferred` | `failed` — not a boolean, because
`deferred` (the deadline path) is neither success nor failure and collapsing it into either would
misreport a healthy drain under load as a failing one.

**The shared core has one definition, and the tie must be asserted.** Follow the precedent of
`ACT_SPAN_FIELDS` / `act_span_declares_every_act_field`: a constant that nothing asserts its
consumers against prevents no drift at all.

### Where each number comes from

| Field | Source | Cost |
|---|---|---|
| `backlog_depth`, `oldest_pending_age_ms` | one `SELECT count(*), min(enqueued_at)` over `kb_workflow_jobs` by persona + dispatch_type + pending status | one query per tick |
| `queue_wait_ms` | one `SELECT id, enqueued_at, leased_at WHERE id = ANY($1)` on the claimed ids | one query per claim batch |
| `attempts` | already returned by `claim_anchor` | free |
| clocks fired, all tallies | already computed (G7) | free |

**Backlog is read BEFORE the claim loop.** The operator's question is *how deep was the queue when
this tick arrived*; read afterwards, a tick reports the queue it has just drained and a
falling-behind drain would look healthy at exactly the moment it is not.

### Two things that must be written down or they will be "fixed" later

**These child spans are `internal` kind and will not appear in `traces_spanmetrics_*`.** That is
expected (G3), and the aggregation route is TraceQL metrics (G4). Marking them `server` to get them
into span metrics would be wrong — they are not request boundaries, and the span-kind convention
exists to make the service graph mean something.

**`queue_wait_ms` carries a known potential bias, and the implementation must resolve it rather than
inherit this paragraph.** `enqueued_at` defaults to `now()`, which in Postgres is **transaction
start**, not statement time — the same trap that made a 23s salience refresh look instant when read
through `occurred_at`. If the enqueue runs inside a long write transaction, the measured wait is
overstated by that transaction's duration.

Post-#636 the enqueue appears to happen after the create commits — `create_resource_with_mode_
idempotent` commits at `db_backend.rs:1708-1711` and the clock call follows at `db_backend.rs:1767`
— which would make it statement-accurate. **That is a reading, not a verification.** The
implementation must confirm it against the current `queue_region_clocks` call site and either state
the field is unbiased or name the bias in the field's doc comment.

---

## What this does not do

Named so that no reader infers coverage from absence.

- **It does not cover `projection-lag-is-readable`.** Different reader, different artifact. That
  clause stays declared uncovered under `019fc46c`, with the four gaps in §"Which goal owns this" as
  its starting point.
- **It does not instrument the steward drain.** The steward's interesting number is watermark lag,
  not queue wait; folding it in would dilute one clean span shape into a three-way compromise.
  Deliberately excluded, not overlooked.
- **It says nothing about whether settling is correct.** `019fc46c` hole 3 is untouched. A drain
  pass that completes cleanly and produces a wrong shape is invisible to every field here.
- **It does not alert.** Thresholds — what backlog depth is too deep, what wait is too long — are an
  operator judgment about a system whose normal range nobody has characterized yet. The queries in
  `docs/guides/drain-operator-queries.md` are written to *establish* that range first.
- **Rate-shaped axes stay open.** Nothing here bounds arrival cadence or examines the drain under
  concurrency; observed convoy depth is 1 at low traffic, which is an observation and not a load
  test.

---

## Testing

The witness lives in the build, not here. Two things about its shape are decided now because they
are easy to get wrong:

**The bite is a value assertion, not an existence one.** "A `region_job` span exists" fails today
against the *absence of the feature* — which is a bite against nothing, satisfiable by any code that
emits a span with that name. The witness must assert `queue_wait_ms` matches a controlled
enqueue-to-claim gap within tolerance, so it fails on a wrong computation and not merely on a missing
span.

**Locate spans by kind and name among exported spans, following precedent.**
`crates/temper-api/tests/telemetry_flush_test.rs` and `tests/e2e/tests/mcp_span_link_test.rs` both do
this, and both were once broken by keying on a hard-coded exported name that later changed. The root
span is the only `server`-kind span in-process; `region_dispatch` and `region_job` are located by
name beneath it.

**One test must assert the negative the convention exists for:** that the job fields are *not* on the
root span. That is the assertion `logging_test.rs` already makes for act fields, for the same reason.

---

## Open questions

- Whether `oldest_pending_age_ms` should be capped or bucketed before becoming a span attribute. A
  never-claimed job would grow it without bound, and an unbounded numeric attribute is a cardinality
  question for whatever eventually aggregates it. Left open because the answer depends on the range
  the queries in the operator reference are meant to establish.
