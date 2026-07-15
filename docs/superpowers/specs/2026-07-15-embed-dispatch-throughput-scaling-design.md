# Embed-dispatch throughput scaling — loop-drain + shard fan-out

The async-embed drain (issue #299) does not scale to an enterprise pushing thousands
of documents. This spec raises its throughput by three composable, per-deploy-tunable
levers — draining the full invocation wall-clock, using the function's spare vCPU, and
fanning out across N concurrent cron-triggered drainers — without changing the
concurrency-safe queue underneath and without approaching any Vercel or Neon limit.

Ships as two PRs against one design:

- **PR A** — loop-drain + intra-op threads (the headline win; independently valuable).
- **PR B** — shard fan-out (builds on A).

## Problem

The drain is a single serialized ONNX pipeline, invoked once per minute, embedding a
capped batch of chunks per invocation, single-threaded. For a low write rate that is
ample. For an enterprise bulk-loading thousands of resources it is a wall.

### The measured ceiling

Steady-state throughput is:

```
chunks/min  =  (concurrent drainers)  ×  (chunks per invocation)
today       =        1                ×          64              =  64/min
```

- **One cron, one invocation.** `vercel.json` schedules `/api/embed/dispatch` at
  `* * * * *`. The handler (`handlers/embed.rs:95`) runs **exactly one**
  `dispatch_tick` pass — reap → claim `cap` resources → embed ≤ `EMBED_CHUNK_BUDGET`
  chunks across them → complete — and returns. One claim per fire.
- **`EMBED_CHUNK_BUDGET` (64) is the true chunk ceiling**, not `cap`. The budget is
  *one allowance spent across all claimed resources* (`embed.rs` doc comment), so
  claiming 5 or 50 resources still embeds ≤ 64 chunks per invocation.
- **Single-threaded ONNX.** `INTRA_THREADS_DEFAULT = 1`. The `api/internal` function
  has `memory: 3009` → ≈ 1.7 vCPU, of which one thread is used.

### Two diagnostic facts

1. **Production reads ~5 resources/min = 300/hr — exactly `cap`(5) × one claim/min.**
   The chunk budget is not even biting yet; `cap` and the once-per-minute cadence are
   what gate the observed rate. Raising `cap` alone does nothing while the 64-chunk
   budget stands.
2. **Each invocation uses ~2–5 s of its 300 s wall-clock.** One claim of ≤ 64 chunks
   is a few seconds of work; the invocation then returns. The function is **~98 % idle
   relative to its own `maxDuration`.** This is a larger, cheaper lever than fan-out and
   is available before touching concurrency at all.

### What this problem is not

- **Not a compute-per-chunk problem.** The #451 post-mortem's throttled bench
  (`crates/temper-ingest/bench/`, 1.5 vCPU) measured cold load 0.38 s, warm embed
  ~5 ms. The model is not too heavy for the serverless CPU.
- **Not a queue-safety problem.** `workflow_job_claim_resource`
  (`migrations/20260707000001_workflow_jobs_resource_scope.sql:70`) already claims with
  `FOR UPDATE SKIP LOCKED`. The queue was built for horizontal drain concurrency; the
  system has simply never run more than one drainer.
- **Not a Neon or Vercel limits problem** — see *Constraints verified* below.

## Design

Three levers over the same queue. Every knob is per-deploy (env or `vercel.json`), so
an enterprise tunes its own throughput without a rebuild.

```
chunks/min  ≈  N shards  ×  (deadline_s / seconds_per_claim)  ×  chunks_per_claim  ×  thread_speedup
```

### Constraints verified (2026-07-15)

| Constraint | Finding | Binding at judicious N? |
|---|---|---|
| Vercel crons | 100 per project on **all** plans (raised Jan 2026); minute-granularity on Enterprise. 2 used today → ~98 shard slots free. | No |
| Vercel fn concurrency | Auto-scales; high Enterprise ceiling. | No |
| Neon pooled endpoint (PgBouncer) | 10 000 client conns; server pool ≈ 0.9 × max_connections ≈ ~377 active txns at 1 CU (prod = PG 17). | No |
| Pool config | Every Vercel fn = `max_connections(5)`, `acquire_timeout(8 s)`. Serial drain uses ~1–2 active conns despite the cap of 5. | No |

At a judicious N (2–4), effective concurrency is a handful of drainers each holding
~1–2 active transactions — orders of magnitude under both the PgBouncer client ceiling
and the active-transaction server pool.

### Lever 1 — loop-drain (PR A)

Change `dispatch_tick` from claim-once-and-return to a claim loop bounded by the
per-invocation deadline:

```
reap once                                # stale-lease sweep, before the loop (leases are 600 s)
start = now
while now - start < deadline:
    claimed = claim(cap)                 # FOR UPDATE SKIP LOCKED
    if claimed is empty: break           # queue drained → stop early
    budget = resolve_chunk_budget()      # fresh per-claim allowance each iteration
    for job in claimed:
        embed job with the remaining budget; complete or re-enqueue as today
```

`reap` runs **once before the loop**, not per iteration: leases are 600 s
(`DEFAULT_EMBED_LEASE_SECONDS`), far longer than a ~55 s invocation, so a single sweep
at entry is sufficient and avoids redundant work each iteration.

This fills the wall-clock instead of returning after one claim. It is **deadline-safe
by construction**: the deadline is checked between claims, and each claim embeds
≤ `EMBED_CHUNK_BUDGET` chunks in one bounded `embed_texts` call.

**Loop-drain retires the single-large-resource "budget cliff."** The earlier concern
was that raising the per-invocation budget would let one large resource pull the whole
budget into a single uninterruptible `embed_texts` call that overruns the deadline
(which is checked only *between* jobs). Loop-drain makes that moot: throughput comes
from *more iterations*, not *bigger batches*. Per-claim budget stays modest (64), so no
single inference call is ever large, and the loop's per-claim granularity **is** the
deadline-check granularity. Prod's largest resource (939 chunks) still embeds 64 and
re-enqueues; the loop simply re-claims it on a later iteration — progress stays
monotonic exactly as today.

### Lever 2 — intra-op threads (PR A)

Set `TEMPER_ONNX_INTRA_THREADS=2` on the deploy to use the function's ≈ 1.7 vCPU within
each invocation. Near-free ~1.5–2× per invocation; pure env knob, no code change beyond
confirming the deploy sets it. `set_intra_op_threads` already exists
(`temper-ingest/src/embed.rs`).

### Lever 3 — shard fan-out (PR B)

Add N cron entries for `/api/embed/dispatch?shard=k` (`k = 0..N-1`), all at
`* * * * *`. Each fires its own function instance every minute; each is an independent
loop-draining drainer.

**The `shard` param carries no logic.** Because the claim is `FOR UPDATE SKIP LOCKED`,
the drainers partition the queue automatically by racing on the lock — no modulo, no
hash, no coordination. The param exists only to make N *distinct* cron lines. Changing N
is adding or removing cron entries; nothing is baked into the claim. A shard that finds
the queue empty claims nothing and returns fast (cheap).

Build-time check: confirm Vercel accepts a query string in a cron `path`. If not, use a
path segment (`/api/embed/dispatch/:shard`) — same design, different routing.

### Concurrency model — Option A, predictable N (chosen)

With loop-drain, the per-invocation **deadline** (already the env knob
`TEMPER_EMBED_DISPATCH_DEADLINE_SECONDS`, currently 200 s) stops meaning "when to stop
claiming in a single pass" and starts meaning **"how long each invocation lives."**
Combined with the one-minute cadence, that sets the concurrency model.

**We choose Option A: deadline ≈ 55 s.** Each invocation finishes inside its minute, so
back-to-back fires *tile* the timeline — **exactly N drainers running at any instant**,
cost linear in N, Neon connections bounded at ~N×2. Scaling is purely "add shards." Even
at 55 s, loop-drain does ~25× a single claim.

- `maxDuration: 300` stays as a pure safety ceiling.
- The deadline env stays retunable per-deploy, so a specific deploy can move toward
  Option B (below) without a code change.

Option B (deadline ≈ 250 s, ~4× stacking per shard) is documented in *Deferred* as the
revisit path if per-shard throughput ever needs to rise faster than N.

### Bounds that stay meaningful

| Knob | Old meaning | New meaning under loop-drain |
|---|---|---|
| `TEMPER_EMBED_DISPATCH_DEADLINE_SECONDS` | when to stop claiming in one pass (default 200) | invocation lifetime; **default retuned to ~55 s** for Option A |
| `cap` (`DEFAULT_EMBED_DISPATCH_CAP`, 5) | resources per pass | resources per **claim** (per loop iteration) |
| `EMBED_CHUNK_BUDGET` (64) | chunks per invocation | chunks per **claim** — still bounds one `embed_texts` call, still re-enqueues large resources |
| `TEMPER_EMBED_BATCH` (32) | ONNX window (memory) | unchanged |
| warm cron (`/api/embed/warm`, every 2 min) | keep an instance hot | unchanged — frequent shards keep instances warm anyway |

## The measurement prerequisite

Every default above should be *sized* from one number that is currently unmeasured
(task `019f5892`, cited three times in the code as the reason the budget is
conservative): **real ms/chunk at 510-token chunks on the deploy's vCPU slice.** The
"~5 ms warm" figure was a 1-token `"warm"` string, not representative.

The throttled Docker bench already exists (`crates/temper-ingest/bench/`, PR #455).
Before merging PR A, run it at realistic 510-token chunks to derive:

- the ~55 s deadline's real per-invocation chunk yield, and
- whether `INTRA_THREADS=2` gives the expected ~1.5–2×.

This is the post-mortem's own lesson applied: measure, don't reason from symptoms.

## Decisions

1. **Scale the clean, linear dimension (N shards), not the emergent, stacked one.**
   Option A gives concurrency = N exactly, cost linear in N, connections bounded —
   which is what "effective, not overkill" requires.
2. **Shards partition via SKIP-LOCKED, not via a modulo predicate.** The queue already
   does this; a shard param with no logic is the whole mechanism.
3. **Loop-drain first, threads with it, shards second.** PR A is the ~25×+ win and
   ships alone; PR B multiplies it by N.
4. **Keep per-claim budget modest (64).** Loop-drain makes raising it unnecessary and
   avoids re-opening the single-large-resource cliff.
5. **Size defaults from the bench measurement, not from guesses.**

## Rejected

- **Managed / external embedding API.** Breaks the deepest invariant in the codebase:
  the sha256 model-identity gate assumes bge-base-768 embedded *identically* on every
  surface, CLI client-side included. A provider means a different model → re-embed the
  entire index and diverge the CLI. Rejected on architecture, not effort.
- **Deterministic `hashtext(id) % N = k` sharding.** Buys nothing over SKIP-LOCKED
  (which already avoids lock contention) and would bake N into the claim function, so
  changing N would require coordinating the modulo. Documented as a future refinement
  only if lock contention is ever observed at very high N — not expected.
- **A mid-resource deadline check inside `embed_resource_chunks`.** This was the fix for
  the single-large-resource budget cliff *in a single-claim world*. Loop-drain retires
  the cliff entirely, so this added complexity is unnecessary.

## Deferred

- **Option B — max per-shard throughput (deadline ≈ 250 s, ~4× stacking).** Each
  invocation runs near `maxDuration`, so a minute-cadence cron stacks ~4 overlapping
  invocations per shard → effective concurrency ~4N. More throughput per shard (fewer
  shards), but concurrency is emergent and a burst transiently spikes connections/cost.
  Revisit only if N-scaling under Option A proves insufficient before the ~98-cron
  headroom is a concern. Switching is a per-deploy env change
  (`TEMPER_EMBED_DISPATCH_DEADLINE_SECONDS`), no code.
- **Raising per-claim `EMBED_CHUNK_BUDGET` above 64.** Once ms/chunk is measured and
  loop-drain is in production, a larger per-claim batch could reduce claim round-trips.
  Only worth it if claim overhead shows up in the bench; loop-drain likely makes it
  irrelevant.
- **A dedicated persistent embedding worker (off Vercel).** The architecturally correct
  host for sustained heavy batch compute, and additive over this design (a worker is
  just another SKIP-LOCKED drainer). Out of scope here; revisit if Vercel concurrency
  economics ever fail at the enterprise's real target rate.

## Open questions and risks

- **ms/chunk is unmeasured.** The whole sizing rests on the bench measurement above; do
  it before locking PR A's deadline default.
- **Query-string cron paths.** Confirm Vercel accepts `?shard=k` in a cron `path`;
  fall back to a path segment if not (PR B).
- **Cost.** Both levers increase Vercel compute spend — you are now *using* invocations
  you already scheduled. Size N to the target rate, not the maximum. Worth an explicit
  spend-management note for the enterprise deploy.
- **Cold starts under short invocations.** At a ~55 s deadline an instance may scale to
  zero between fires and re-pay the model load (~0.38 s, cheap post-LFS-fix); the warm
  cron mitigates. ~0.7 % overhead — acceptable.
- **Target rate is unstated.** Enterprise currently ingests ~300 resources/hr. The
  concrete docs/hr (or chunks/hr) target would let us pick N precisely rather than
  "start at 2–4 and measure."

## PR plan

- **PR A — loop-drain + threads.** `dispatch_tick` claim loop; retune the deadline
  default to ~55 s; deploy sets `TEMPER_ONNX_INTRA_THREADS=2`. Tests: loop drains
  multiple claims until the deadline; deadline stops the loop; empty queue breaks early;
  large-resource re-enqueue still monotonic. Run the bench first to set the default.
- **PR B — shard fan-out.** N `?shard=k` cron entries in `vercel.json`; the (cosmetic)
  shard param; verify query-string cron paths; a test that two concurrent
  `dispatch_tick` calls claim disjoint sets; N-sizing guidance in the deploy docs.
