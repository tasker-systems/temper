# Query Cost and the Bounds That Shape It

**For integrators** — anyone composing queries against Temper, whether from the CLI, the HTTP
API, or an agent runtime driving MCP. Also relevant to **operators**, who own the layer the
database enforces.

Temper's read surface is compositional: you declare a plan of stages, each stage invokes an act,
and the server compiles the whole plan into one statement. That is the intended use — composing
programmatically is what the query door is for. It also means a caller can describe a great deal
of work in a small request, so it is worth understanding which layer bounds what.

## The decision this reflects

Temper bounds a composed read in three places, and the split is deliberate:

- **The contract bounds a plan's shape.** Anything knowable from the request alone — how many
  stages it declares, whether it is well-formed — is refused by the validator before the request
  costs anything. These bounds are **published in the wire contract**, not merely enforced, so a
  client can refuse the same plan the server would.
- **Each act declares its own ceilings.** A row ceiling belongs to the act that produces the rows,
  because only the act knows whether truncating its answer would make that answer wrong. There is
  no global row cap, and that is a choice rather than an omission.
- **The deployment owns the execution bound.** How long a statement may run is a property of the
  hardware, the workload mix and the duration budget of the surface it serves — none of which the
  software knows. Temper does not bake in a number that would be wrong for most deployments; it
  gives the operator the grain to set one. The last section is how to choose it.

The rest of this page is how to reason within that split. It is not a tuning guide, and the
numbers your deployment can afford are yours to measure.

## Bounds answer different questions

There are four distinct layers, and they are not interchangeable. Confusing one for another is
the most common way to believe a query is bounded when it is not.

| Layer | Refuses | When | Cost of the refusal |
|---|---|---|---|
| **Shape** | A plan that is malformed or oversized | Before any embedding or database contact | Free |
| **Admission** | A caller without standing | Before the plan runs | One database round-trip |
| **Delivery** | Rows beyond an act's declared ceiling | As the statement produces them | The work was already done |
| **Execution** | A statement that runs too long | While it runs | Whatever it consumed |

The shape layer is the cheapest and the only one that can refuse *before* the request costs
anything. `MAX_STAGES` lives there: a composition may declare at most **64** stages, and one that
declares more is refused as `TooManyStages`. Because the check sits in `validate`, it covers the
HTTP API, MCP `run_query`, and `temper query --check` from a single site — all three route
through the same validator, so a plan the CLI refuses is a plan the server would have refused.

That cap is published as `max_items` on `Composition.stages` in the wire contract, and that
matters more than it looks. A bound a client enforces but the contract does not declare is a
client refusing plans a newer server would happily run. Publishing it is what makes local
`--check` trustworthy rather than a source of false refusals.

## A bound on waiting is not a bound on work

This is the distinction worth carrying past this page, because it generalizes to every guard you
will ever read.

A connection pool's acquire timeout bounds how long a caller waits **for a connection**. It does
nothing whatsoever to a statement that already holds one. Under load its effect is to make the
*next* caller fail faster while the expensive statement runs to completion — which is the
opposite of protective, and it reads like protection.

So for any guard, ask two questions and keep them separate:

1. **What does this stop?**
2. **What does this merely reorder?**

A timeout on acquisition, a queue depth, a concurrency limiter, a retry budget — several of these
change *who waits and in what order* without changing *how much work happens*. Only a bound the
executor itself honors can shorten the work.

## Cost follows the acts you compose, not the stages you declare

A composition's cost is not proportional to its size. Two plans with identical stage counts can
differ by more than an order of magnitude depending on which acts they invoke and how each act is
bound. A plan of many cheap, well-bound stages is ordinary; a plan of a few unbounded ones is not.

This is why the stage cap is a **shape** guard rather than a cost guard. It bounds what the
planner must build, what the wire must carry, and how large a refusal list can grow. It is not a
cost ceiling and should not be read as one — a plan well under the cap can still be expensive, and
the cap's job is to keep the plan itself tractable, not to price it.

The practical consequence for anyone composing queries: **look at the acts, not the stage count**.
If a plan is slower than you expect, the question is which act is doing the work and what it was
given to work on, not how many stages you wrote.

## Read the ceilings the act declares

Each act declares its own `bound_ceilings` — the maximum a caller may ask for on each bound term.
Most acts cap `Limit` at 50, so a stage that asks for more is clamped to the act's ceiling rather
than honored.

**Ceilings are per-act, and some acts declare none.** `find-resources-with` is deliberately one of
them: it is a filter-driven producer whose natural result is "every visible resource matching
these criteria", and giving it a row ceiling would silently truncate an answer whose whole purpose
is completeness. That is a considered design choice, not an oversight.

The corollary is on the caller. An act with no declared ceiling is bounded by **what you give
it**, so:

- Bind it with filters that actually narrow. An unfiltered producer returns the visible corpus.
- When you feed one producer's output into another stage, remember you are handing the consumer
  whatever the producer found — the consumer's own limit prunes its *output*, not its input.
- Prefer expressing the narrowing in the act that can push it down, rather than producing wide and
  filtering late.

Every stage in a plan executes, whether or not you read it. Naming a stage in `returns` selects
what comes back; it does not select what runs. So an unread stage is not a free stage — if you do
not need it, do not declare it.

## Choosing a number requires measurement, not judgment

Operators own the execution layer, and it is the one place a genuinely runaway statement can be
stopped. Postgres offers `statement_timeout` for this, and it can be attached at several grains:
per-transaction with `SET LOCAL`, per-role with `ALTER ROLE … SET`, or per-function with
`ALTER FUNCTION … SET`. Temper already uses the per-function grain to pin `hnsw.ef_search`, and it
is usually the right instrument here too, because it lets a read-heavy function carry a different
budget from a long-running maintenance write without splitting pools or paths.

Note that a session-level hook such as a pool's `after_connect` is the wrong instrument under a
transaction-mode pooler, where a session is not pinned to a client for the life of a connection.

**Do not pick the number.** A bound chosen without measurement is either theatre or an outage, and
which one you got is not knowable in advance. The methodology matters more than any value this
page could suggest:

- **Install the instrument before you need it.** `pg_stat_statements` collects into shared memory
  from the moment the module is preloaded, and `CREATE EXTENSION` only makes it readable. A
  deployment that adds the extension later may find it already has history.
- **Check `dealloc` before trusting a maximum.** If entries have been evicted, your slowest
  statement is the slowest of what survived — the top of a truncated list, not a maximum. Only
  `dealloc = 0` makes `max_exec_time` mean what it says.
- **Know your window.** Compare `stats_reset` against `pg_postmaster_start_time()`. A window
  shorter than your slowest periodic workload has not yet seen the thing most likely to be killed
  by a new bound.
- **Ask for the slowest *legitimate* query, not the average.** Sizing to a healthy hour's p99
  guarantees you kill the nightly job. The number you need is the slowest thing that is supposed
  to happen.
- **Separate the paths before you separate the numbers.** Surfaces with different duration budgets
  have different honest ceilings; a single global value has to survive the widest spread among
  them, which is usually a bad trade made once for everything.
- **Group by shape, not by name.** A normalized statement text can cover several plans with very
  different costs. A mean across them describes none of them. Compare like shapes, and prefer
  `min`/`stddev` alongside the mean — a slow statement with a high floor is a different problem
  from one with a long tail.
- **Separate the time from the work.** Read `shared_blks_hit` against `shared_blks_read` before
  assuming a slow statement is I/O-bound. A statement that is almost entirely cache hits is
  spending CPU, and the fix for CPU is not the fix for I/O. Note that the `blk_read_time` columns
  read as zero when `track_io_timing` is off, which looks identical to "no time in I/O" — check
  the setting before believing the number.
- **Watch `temp_blks_written`.** It is the usual source of a cost that grows faster than the work
  does. A plan that fits in `work_mem` and one that spills are not on the same curve, so two
  statements with similar buffer counts can differ by orders of magnitude in time. Compositions
  with many stages materialize many intermediate results, which makes them more likely to spill
  than their size alone suggests.

Two properties of the execution environment matter more than they look, and both are worth
checking before concluding that a query shape is at fault: the CPU actually available to the
database, and whether the functions in the path are `PARALLEL SAFE`. A parallel-unsafe function
anywhere in a plan forces the whole statement serial no matter how many workers are configured.

## Verifying that a guard works

A guard you have never seen refuse is a guard you have not verified — you have only failed to
observe it. The useful question is not *"is this bound correct?"* but:

> **What would this have to see in order to refuse?**

Then construct that input and watch it get refused. And confirm the other direction too: a bound
witnessed only by the thing it blocks tells you nothing about whether it kills work that should
have survived. Both directions, or it is not a witness.

The same standard applies to a bound you have chosen from measurement. Show a query designed to
exceed it being cut off, and show a legitimate slow one surviving. One direction is half a test.

## See also

- **[The Trust Boundary](./trust-boundary.md)** — the authentication and system-access gates every
  call crosses before any of this applies.
- **[Telemetry](./telemetry.md)** — the trace and log layers, and what a request looks like from
  the outside.
