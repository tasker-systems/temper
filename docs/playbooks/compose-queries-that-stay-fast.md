# Compose Queries That Stay Fast

**For integrators** composing against the query door, and **operators** who want to know which
knobs actually move a composed read.

The companion concept page, [Query cost and the bounds that shape
it](../concepts/query-cost-and-bounds.md), explains which layer bounds what and why. This page is
the practical half: what makes a composition expensive, what to do about it, and what to watch in
your own deployment.

The short version: most compositions are cheap and stay cheap. The ones that are not are almost
always expensive for one of the four reasons below, and three of them are things the caller
controls.

## Bind the producer, not the answer

The single largest factor is how well each producing stage is narrowed *before* it produces.

A filter-driven act with no filters returns everything visible to you. Adding a `limit` to that
stage does not make it cheaper — the rows were already produced, and the limit truncates the
output. What makes it cheaper is a predicate the act can push into its own scan: a doc type, an
owner, an anchor, a stage or status.

```
slow:  find-resources-with { }                    → limit 50
fast:  find-resources-with { doc_type, anchor }   → limit 50
```

Both return at most 50 rows. Only the second one *looks at* a bounded set. When you are handing
one stage's output into another as a bound, this matters twice over — the consumer's limit prunes
what the consumer emits, never what the producer had to walk.

## Every stage you declare executes

Naming a stage in `returns` selects what comes **back**. It does not select what **runs**. A plan
that declares a stage it never reads still pays for it in full.

So a composition should declare the stages it needs and no more. This is the cheapest optimization
available and it is entirely under the caller's control — if you built a plan by generating stages
from a list and then reading two of them, you are paying for the whole list.

## Fan-out multiplies, and it multiplies serially

A plan that invokes the same producing act many times — once per anchor, once per candidate,
once per element of an array — costs roughly what one invocation costs, times the number of arms.
There is no shared work between arms and no parallelism across them.

That is fine when each arm is well-bound and cheap. It is the thing to look at first when a plan
is slower than you expect and you have already checked that each arm is narrow. Two questions:

- Could this be one stage with a broader predicate instead of N stages with narrow ones?
- Do I need all N arms, or am I fanning out over a list I could filter first?

## The cliff is memory, not rows

Cost does not rise smoothly with plan size. A composition materializes an intermediate result per
stage, and while those fit in the database's working memory the cost is roughly linear. When they
stop fitting, the plan spills to temporary files and the curve bends sharply — a plan can take
several times longer than one doing *more* raw work but staying in memory.

This is the usual explanation for "two similar-looking queries, wildly different times." It is
also mostly an operator-side concern: see below.

## What to measure in your own deployment

Four numbers, in the order worth checking. All come from `pg_stat_statements`.

1. **`shared_blks_hit` vs `shared_blks_read`.** A statement that is nearly all hits is spending
   CPU, not waiting on storage — and the fix for CPU is not the fix for I/O. Check
   `track_io_timing` before trusting the `blk_read_time` columns; when it is off they read zero,
   which looks exactly like "no time in I/O."
2. **`temp_blks_written`.** Non-zero means the plan spilled. This is the most common source of a
   cost that grows faster than the work does, and raising `work_mem` for the read path is usually
   the highest-leverage single change available.
3. **`min_exec_time` and `stddev_exec_time` next to the mean.** A high floor and a long tail are
   different problems. A statement whose *minimum* is already slow is not a cache or contention
   story.
4. **Available CPU, and whether the functions in your plan are `PARALLEL SAFE`.** A single
   parallel-unsafe function anywhere in a plan forces the whole statement serial no matter how
   many workers are configured. On a small or autoscaling compute, a serial CPU-bound statement is
   bounded by one core.

## Operator knobs, in order of leverage

- **`work_mem`** — the spill cliff above. Worth setting for the read path specifically rather than
  globally, so a long-running maintenance write does not inherit it. `ALTER FUNCTION … SET` gives
  that grain without splitting pools or connections.
- **Compute size** — a CPU-bound serial statement scales with single-core speed, not with
  connection count. If the numbers say CPU and the compute is small or fractional, that is the
  constraint.
- **`statement_timeout`** — the backstop. See the concept page for how to choose it; the short
  version is that it must be measured against your slowest *legitimate* query, and attached per
  path rather than globally.

## See also

- **[Query cost and the bounds that shape it](../concepts/query-cost-and-bounds.md)** — the four
  bound layers, why a bound on waiting is not a bound on work, and the measurement discipline.
- **[Telemetry](../concepts/telemetry.md)** — getting traces out, so a slow call can be attributed
  to a route before it is attributed to a query.
