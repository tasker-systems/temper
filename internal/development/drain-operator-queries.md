# Drain operator queries (TraceQL)

The queries an operator runs to answer *is the drain keeping up?* for the two `api/internal` drains —
region materialization and embedding.

Companion to the drain instrumentation design
(`internal/superpowers/specs/2026-08-03-drain-instrumentation-design.md`)
and [OpenTelemetry setup](../../docs/playbooks/send-traces-to-an-otlp-backend.md). Datasource: the Tempo datasource
(`grafanacloud-traces`).

---

## Read this before trusting a query

**Verification status is marked per query, and it is not uniform.**

| Mark | Means |
|---|---|
| **`[live]`** | Executed against production Tempo and returned real data. The output is quoted, with its date. |
| **`[shape]`** | The *aggregation form* was executed live and works, and the attribute is confirmed present — but this exact query has not itself been run. |
| **`[blind]`** | Written against the design, never executed. Names spans and attributes the instrumentation has not shipped. |

> **Field presence was verified locally on 2026-08-03** — kept for the record, and superseded by the
> production pass below. `crates/temper-services/tests/drain_span_test.rs` confirms every name in
> `DRAIN_DISPATCH_FIELDS` and `DRAIN_JOB_FIELDS` is carried by a real exported span, and
> `queue_wait_ms` is mutation-tested to fail on a wrong value rather than only on a missing one. That
> gate still runs in CI and is what keeps the fields from drifting; what it could never do is tell you
> whether a query returns something useful, because local spans never reach Tempo.

**Updated 2026-08-04, after the spans went live** (PR #642, merged; production deploy Ready 13:01Z).
The instrumentation was verified end to end — the full three-level tree arrives with correct
parenting, `SPAN_KIND_INTERNAL` on both children, and no job fields on the request root. A1, B1 and
D1 have now been **run against production** and re-marked, with their real output quoted.

**The remaining `[blind]` queries are still `[blind]` on purpose.** They need traffic this drain has
not produced yet — a deferred job, a failed one, a second anchor under load. Re-run and re-mark each
as its case actually occurs. A query that stays `[blind]` indefinitely is one nobody has ever run,
and *"is the drain keeping up?"* should not rest on one of those.

**Two traps came out of running them, both in this file where you will hit them:** B1's quantiles are
exponentially bucketed and read high (see B1), and A2's series has legitimate gaps rather than missing
data (see A2). Both were invisible until the queries met real spans.

**These spans are `internal` kind, so none of this is in `traces_spanmetrics_*`.** That is by design
— see the instrumentation spec, §G3. TraceQL metrics is the aggregation route and does not need
span metrics. Do not "fix" it by marking the spans `server`.

**Everything is sampled at 100%** (`ParentBased(AlwaysOn)`, and there is never a remote parent), so
counts here are complete rather than estimates. If a sampler is ever introduced, every count-based
query below silently becomes a lower bound.

---

## A. Is the drain keeping up?

The headline question. Backlog depth is the one number that distinguishes *slow* from *falling
behind* — a drain can be slow forever and still keep up, and a fast drain can fall behind if arrivals
outpace it.

**A1 — Backlog depth over time** `[live]`

```traceql
{ name = "region_dispatch" } | max_over_time(span.backlog_depth)
```

> Run against production 2026-08-04, ~10 min after the spans went live: returned **1**, matching the
> single queued job a write had just produced. `max_over_time` is exact — unlike the quantiles, see B1.

Read as: how many jobs were waiting each time a tick arrived. Flat-and-low is healthy. A rising
floor is the falling-behind signal — not a spike, which is just a burst the next tick absorbs.

**A2 — Age of the oldest waiting job** `[shape]`

```traceql
{ name = "region_dispatch" } | quantile_over_time(span.oldest_pending_age_ms, .95)
```

> The attribute is confirmed present in production (`oldest_pending_age_ms: 9210` on a tick that found
> one job waiting). The quantile form itself is B1's, verified there.

The companion to A1 and the more honest of the two. Depth 1 with an age of 40 minutes is a *stuck*
job, which A1 alone reports as a healthy queue.

> **This series has gaps, and they are not missing data.** `oldest_pending_age_ms` is recorded ONLY
> when the queue was non-empty — "nothing waiting" is not an age of zero, and writing one would put a
> false floor under every aggregate. Confirmed in production: ticks with `backlog_depth: 0` carry no
> such attribute at all. So this query aggregates over *ticks that found work*, which is the right
> denominator, and a panel reading "no data" on a quiet drain is correct rather than broken.

**A3 — Both drains side by side** `[blind]`

```traceql
{ name =~ "(region|embed)_dispatch" } | max_over_time(span.backlog_depth) by (span:name)
```

The twins share a function and a wall-clock budget (`maxDuration: 300`). One starving the other is
visible here and nowhere else.

---

## B. How far behind is a settled shape?

**B1 — Queue wait distribution** `[live]`

```traceql
{ name = "region_job" } | quantile_over_time(span.queue_wait_ms, .5, .95, .99)
```

> Run against production 2026-08-04. Returned p50 = p95 = **16384** over a window whose only sample
> was a real `queue_wait_ms` of **9216**.
>
> **That is not a bug, and it is the most important caveat on this page.** TraceQL's
> `quantile_over_time` buckets exponentially, so a 9.2s sample reports in the 16.384s bucket — a 78%
> overstatement at this magnitude, converging only as sample count grows. **Never set an alert
> threshold on this query's absolute value**, and never quote it as *the* queue wait. For a real
> number, read a `region_job` span directly (C3 gets you to one). Use this for *shape and trend*,
> which is what it is good for.

Enqueue-to-lease. Measured directly from `kb_workflow_jobs` on 2026-08-03 at **0.84–60s**, bounded
by the 1-minute cron plus a consistent ~24s-past-the-minute landing — so a p95 near 60s is the
*expected* shape, not an incident. Anything materially above it means ticks are being skipped or
the claim is starving.

**B2 — Queue wait by anchor** `[blind]`

```traceql
{ name = "region_job" } | quantile_over_time(span.queue_wait_ms, .95) by (span.anchor_id)
```

Anchors are not interchangeable: live-region counts span a 39× range (429 down to 11), and cost
tracks that count on both clocks. An aggregate across anchors hides the one anchor that is slow.

---

## C. Where is the time going?

**C1 — Tick duration** `[live]`

```traceql
{ name = "GET /api/region/dispatch" } | quantile_over_time(duration, .5, .95) by (span:name)
```

> Returned a real series over a 6h window; p50 ≈ **0.0168s**. That p50 is the *no clock fired* case,
> which is most ticks — the distribution is strongly bimodal and the p50 is not a useful summary on
> its own. Read it beside C2.

**C2 — Job duration split by which clock fired** `[blind]`

```traceql
{ name = "region_job" && span.materialized = true } | quantile_over_time(duration, .5, .95)
{ name = "region_job" && span.salience_refreshed = true && span.materialized = false } | quantile_over_time(duration, .5, .95)
```

Two queries, deliberately not one grouped by a clock dimension — a pass can fire **both** clocks, so
a single `by (...)` would either double-count it or need a synthetic combined value. Reference points
from production on 2026-08-03: formation-only **33.3–33.8s**; salience-only **0.19–0.20s** after
PR #639 (it was 21.6–23.4s before).

**C3 — The slowest passes, as traces rather than a metric** `[shape]`

```traceql
{ name = "GET /api/region/dispatch" && duration > 30s }
```

> The form is `[live]` — run with `duration > 1s` it returned the 22:15:20Z pass at
> `durationMs: 33337`. The `30s` threshold is a judgment, not a measured one.

Use this to get from "the p95 moved" to an actual trace with its job spans underneath.

---

## D. Is it failing?

**D1 — Outcome mix** `[live]`

```traceql
{ name =~ "region_job|embed_job" } | count_over_time() by (span.outcome)
```

> Run against production 2026-08-04: returned `completed` **1**. Counts are exact (unlike B1's
> quantiles). Widened to both drains — the outcome vocabulary is shared, so one panel covers the pair.

`outcome` is `completed` | `deferred` | `partial` | `failed` (`JobOutcome` in
`crates/temper-services/src/services/drain_span.rs`, asserted against these exact strings).

**Only `failed` is a failure.** `deferred` is the deadline path handing a job back cleanly having
attempted no work; a healthy drain under load produces them, and a rising share means the wall-clock
budget is the binding constraint. `partial` is embed-only: work *was* done but did not finish the
claim's budget, so the job was re-enqueued to resume — the normal path for a large resource
(production's biggest holds 939 chunks against a budget of 64).

**The two are worth reading separately, because `EmbedDispatchSummary` cannot.** Its `partial` field
counts both states, which is why the span distinguishes them and the summary was left alone. A drain
whose `deferred` is climbing while `partial` is flat is out of wall-clock; the reverse is just large
resources making progress.

**D2 — Tick cadence** `[live]`

```traceql
{ resource.service.name = "temper-internal" } | count_over_time() by (span.http.route)
```

> Returned `/api/region/dispatch` **356**, `/api/embed/dispatch` **1431**, `/api/embed/warm` **177**,
> `/api/slack/intents/reap` **6** over 6h.

356 over 6h is ≈1/min, which is the cron. **A drop here is the failure mode no other query on this
page can see** — if the cron stops firing, backlog and queue wait stop being *reported*, and every
panel above goes quiet rather than red. Check this first when the dashboard looks suspiciously calm.

**D3 — Server-side errors on the dispatch endpoint** `[blind]`

```traceql
{ name = "GET /api/region/dispatch" && status = error }
```

Depends on the `otel.status_code = ERROR` work from PR #638, which sets it on 5xx only — a 4xx is a
correct judgment about the request and deliberately not counted.

---

## Notes for whoever builds the dashboard

- **Establish the normal range before setting any threshold.** Nothing on this page has a
  characterized baseline except the reference points quoted inline, and those are single days on one
  anchor. Alerting on an uncharacterized metric produces pages nobody can act on.
- **A1/A2 belong together in one row**, and D2 belongs somewhere unmissable. Depth without age, or
  either without cadence, each has a blind spot the other covers.
- **Anchor id is high-cardinality** and only six anchors carry live regions today. It is a fine
  dimension now and would not be if that changed; B2 is the query to revisit first.

## Post-deploy follow-up

Once these spans are flowing in production, re-run every query on this page against Tempo and
re-mark it. A query still marked `[blind]` after the spans exist is a query nobody has run — and the
answer to "is the drain keeping up?" should not rest on one of those.

Two things to check on that pass, both of which local testing structurally cannot answer:

- **Does `oldest_pending_age_ms` stay bounded in practice?** It is recorded as a raw unbounded value
  because bucketing before anyone has seen its range would be guessing. A never-claimed job grows it
  without limit, which is a cardinality question for whatever aggregates it.
- **Does `queue_wait_ms` read the way the p95 in B1 predicts** (a ceiling near 60s from the 1-minute
  cron)? If it runs materially higher, ticks are being skipped or the claim is starving, and that is
  a finding rather than a tuning exercise.
