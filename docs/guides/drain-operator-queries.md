# Drain operator queries (TraceQL)

The queries an operator runs to answer *is the drain keeping up?* for the two `api/internal` drains —
region materialization and embedding.

Companion to [the drain instrumentation design](../superpowers/specs/2026-08-03-drain-instrumentation-design.md)
and [OpenTelemetry setup](open-telemetry-setup.md). Datasource: the Tempo datasource
(`grafanacloud-traces`).

---

## Read this before trusting a query

**Verification status is marked per query, and it is not uniform.**

| Mark | Means |
|---|---|
| **`[live]`** | Executed against production Tempo on 2026-08-03 and returned real data. The output is quoted. |
| **`[shape]`** | The *aggregation form* was executed live against an existing attribute and works; the specific attribute it names does not exist yet. |
| **`[blind]`** | Written against the design, never executed. Names spans and attributes the instrumentation has not shipped. |

Most queries here are `[blind]`, and **that is the expected state of this file until the
instrumentation merges** — it was written alongside the design so the field set could be checked
against real operator questions rather than invented and then rationalized. A `[blind]` query is a
statement of intent, not a working artifact. Re-run and re-mark them when the spans land; a query
that stays `[blind]` after the spans are flowing is a query nobody has ever run.

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

**A1 — Backlog depth over time** `[blind]`

```traceql
{ name = "region_dispatch" } | max_over_time(span.backlog_depth)
```

Read as: how many jobs were waiting each time a tick arrived. Flat-and-low is healthy. A rising
floor is the falling-behind signal — not a spike, which is just a burst the next tick absorbs.

**A2 — Age of the oldest waiting job** `[blind]`

```traceql
{ name = "region_dispatch" } | quantile_over_time(span.oldest_pending_age_ms, .95)
```

The companion to A1 and the more honest of the two. Depth 1 with an age of 40 minutes is a *stuck*
job, which A1 alone reports as a healthy queue.

**A3 — Both drains side by side** `[blind]`

```traceql
{ name =~ "(region|embed)_dispatch" } | max_over_time(span.backlog_depth) by (span:name)
```

The twins share a function and a wall-clock budget (`maxDuration: 300`). One starving the other is
visible here and nowhere else.

---

## B. How far behind is a settled shape?

**B1 — Queue wait distribution** `[blind]`

```traceql
{ name = "region_job" } | quantile_over_time(span.queue_wait_ms, .5, .95, .99)
```

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

**D1 — Outcome mix** `[shape]`

```traceql
{ name = "region_job" } | count_over_time() by (span.outcome)
```

> The aggregation form is `[live]`: the same shape keyed on an existing attribute
> (`count_over_time() by (span.http.route)`) returned `/api/region/dispatch` 356 and
> `/api/embed/dispatch` 1431 over 6h.

`outcome` is `completed` | `deferred` | `failed`. **`deferred` is not a failure** — it is the
deadline path handing a job back cleanly, and a healthy drain under load produces them. A rising
`deferred` share means the wall-clock budget is the binding constraint; a non-zero `failed` means
something else.

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
