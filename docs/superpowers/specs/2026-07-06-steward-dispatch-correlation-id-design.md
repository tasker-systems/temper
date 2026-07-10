# Cross-Vercel-app correlation-id visibility for the steward dispatch chain

**Date:** 2026-07-06
**Status:** Design approved; log-layer to implement now, schema stamp spec'd as fast-follow.
**Task:** `019f332e` (directions 2 & 3) · **Goal:** steward generalization `019f3220`
**Siblings shipped:** direction 1 (dispatcher logging) — #272 · direction 4 (map_err NotFound preservation) — #278

## Problem

The steward dispatch chain crosses **three separate Vercel request contexts** with no shared
trace id:

```
steward-agent cron (GET /eve/v1/cron/…)
  → outbound POST /api/steward/dispatch to temper-api (temperkb.io)   [app boundary]
    → (if drifted) a receive(worker,…) agent session                 [app boundary]
      → the agent's invocation_open → a minted invocation_id
```

Observed live while verifying the fan-out (2026-07-05/06):

- The `/dispatch` fetch is **outbound** — its server-side work lands in **temper-api** logs, not
  `steward-agent` logs. The two apps' logs share no key.
- The invocation envelope is minted by the **agent session** (`invocation_open`), *not* by
  temper-api's `/dispatch` (which only runs reap→sweep→enqueue→claim over `kb_workflow_jobs`).
  So there is no single point that sees the whole chain.
- **No correlation id** ties the cron request → the `/dispatch` request → the eventual
  `invocation_id`. A drifted tick can't be followed end-to-end across the two apps; a failed tick
  (cold-start 500 on `/dispatch`, a fetch that never lands, a `receive()` that throws before a
  session starts, an agent that crashes before `invocation_open`) leaves no joinable trace at all.

### Already addressed

- **Direction 1 — dispatcher logging (#272).** The cron `run` handler
  (`packages/agent-workflows/steward/agent/schedules/steward.ts`) already logs its outcome on the
  **steward-agent** side, including the no-drift case:
  `[steward-dispatch] claimed N job(s) … (no drift)`, and `catch` → `[steward-dispatch] tick failed`.
  A healthy no-op is therefore already observable. This spec **enriches** those existing lines with
  the correlation id rather than adding a new logging surface.
- **Direction 4 — map_err (#278).** The steward MCP `map_err` now preserves the `NotFound` payload,
  so an "event not found" no longer masquerades as "cognitive map not found".

This spec covers **directions 2 (correlation id) and 3 (Vercel-native evaluation).**

## Key decision: logs are the primary trace; the schema stamp is convenience

An in-schema `correlation_id` (e.g. on the invocation envelope) only exists for ticks that
**successfully reach the DB write**. The failures we most need to see all happen *before* any
invocation row exists — a Vercel cold-start 500, an outbound fetch that never lands, an eve
`receive()` that throws before a session starts (the exact bug this saga chased), an agent that
dies before `invocation_open`. A durable stamp is **blind precisely at the infra boundary**, which
is the failure-prone, silent-by-default part of the chain.

Therefore the load-bearing observability lives in the **logs**, emitted at every boundary crossing
*before* the risky hop, so a failure after that point still leaves the id in a log at the last
boundary it reached. The in-schema stamp is a pure **convenience layer** on top — useful for
`invocation_show` queries on ticks that *did* succeed, never the thing relied on when something
breaks.

- **Now:** boundary-logging correlation id + `x-vercel-id` bridge. Infra-resilient. No schema change.
- **Fast-follow (separate task):** `kb_workflow_jobs.correlation_id` + `kb_invocations.correlation_id`
  (additive migrations), with the invocation inheriting the id **server-side** from its active job —
  not via a model-passed param. Adds a direct DB-side join for successful ticks.

## Design — the "now" work (boundary logging)

### Correlation id

- **Mint** `correlationId = crypto.randomUUID()` at the top of the cron `run` handler (Node 24
  native, zero-dep). A **v4** UUID is deliberate here: a *log* correlation key needs uniqueness, not
  v7 time-sortability — log timestamps already order the trace, and the fast-follow's UUID column
  accepts any UUID. (The steward toolchain ships no uuid package; pulling one in for sortability we
  don't need is not worth the dependency.)
- **Log it first, before the outbound fetch:** `[steward-dispatch] tick {correlationId} starting`.
  This is the anchor line — every later line and every failure references it.

### Carrying it across the app boundary — a header, not a body field

Send the id to `/dispatch` as an **`x-steward-correlation-id`** request header.

- It is transport/tracing metadata, not domain data, so it does **not** belong in the
  `DispatchTickRequest` body — the typed request contract stays unchanged (the cron keeps sending
  `{}` as the body; server defaults still apply).
- temper-api reads it with an Axum `HeaderMap` extractor in `handlers::steward::dispatch` and logs
  on handler entry: `[steward-dispatch] received tick {correlationId}` (structured field, not string
  interpolation, per the pino/structured-logging rule on the TS side and `tracing` fields on the
  Rust side).
- A missing/blank header is tolerated (logs `received tick <none>`); the header is additive and
  never required for `/dispatch` to function.

### Enriching the existing cron lines

The `#272` lines gain the id (no new log sites):

- success: `[steward-dispatch] tick {correlationId}: claimed N job(s): {cogmapIds}` (also include the
  `jobId`s, so the deterministic `tick → job → cogmap` chain is fully in the steward-agent log).
- failure: `catch` → `[steward-dispatch] tick {correlationId} failed: {err}`.

So a `/dispatch` that 500s, a fetch that never lands, or a `receive()` that throws is now pinned to a
specific `correlationId` instead of vanishing inside `waitUntil`.

### The `x-vercel-id` bridge (direction 3)

At each hop, log Vercel's own `x-vercel-id` next to our `correlationId`:

- cron logs the `/dispatch` **response**'s `x-vercel-id`
  (`res.headers.get("x-vercel-id")` → `[steward-dispatch] tick {correlationId} dispatch vercel-id {x}`);
- temper-api logs its **inbound** `x-vercel-id`.

Our id threads the whole chain in app logs; the `x-vercel-id` is the **escape hatch** into Vercel's
per-request infra observability for any single hop that fails *inside* the platform before our code
runs.

### What "now" delivers, deterministically (no model reliance)

Across the two apps' logs, joinable by `correlationId`:

```
steward-agent:  tick {id} starting
steward-agent:  tick {id}: claimed N: job {jobId} / cogmap {cogmapId}   (or "(no drift)")
temper-api:     received tick {id}   (+ inbound x-vercel-id)
steward-agent:  tick {id} failed: …  (on any boundary failure)
```

The exact tie to a specific `invocation_id` is already *practically* reachable (cogmap id + tick
timestamp → `invocation_list` on that map), and becomes a **direct** join with the fast-follow. The
"now" phase deliberately does **not** rely on the model logging anything — every line above is emitted
by deterministic cron/handler code.

## Vercel-native evaluation (direction 3) — conclusion

The **app-level id is primary.** W3C `traceparent` / OTel trace propagation does **not** auto-cross
the cron → `receive(worker)` session boundary — the agent session is a *separate* eve-started Vercel
invocation, not a child of the cron request's trace — so platform trace propagation cannot span the
chain on its own. `x-vercel-id` is used only as the **bridge** from our id into Vercel's per-request
view, not as the trace itself. The design therefore takes **no dependency on OTel/traceparent**; if
Vercel later propagates a trace across the eve channel boundary, it can be logged alongside
`correlationId` as another bridge key without changing the primary design.

## Fast-follow — SHIPPED (task `019f4be3`, migration `20260710000010_steward_tick_correlation`)

Implemented as spec'd below, with one correction the implementation forced and one question it settled.

**Correction.** Item 1 said "set at claim time in `handlers::steward::dispatch`". The handler cannot do
it: `/dispatch` fires **zero** `kb_events` — reap→sweep→enqueue→claim touch only `kb_workflow_jobs`, and
`drift_sweep` is a pure read. The stamp therefore rides *inside* `workflow_job_claim` (which grew a
`p_correlation` argument), not as a follow-up write. It is set **unconditionally** on each claim: a
re-claim after a reap belongs to the tick that claimed it, not the one that lost its lease.

**Settled: act grain vs run grain.** A steward tick is **one dispatch act plus N run-grain sessions**,
not one act. `kb_events.correlation_id` stays act-grain (a block's event stream); a session's writes are
already run-correlated by `invocation_id`. So the tick id stops at the invocation, and an act joins to
its tick through `kb_events.invocation_id → kb_invocations.correlation_id`. Stamping the tick onto every
session event would buy a one-hop join and destroy a distinction the ledger already asserts.

A consequence: the fan-out prompt **no longer mentions the correlation id**. P3 exposed `correlation_id`
on the MCP write tools' `ActInput`, so telling the model the tick id invited exactly the grain collapse
above. Inheritance is server-side; the agent is told nothing and passes nothing.

**Replay.** `_project_delegated_launch` reads the correlation off the **event**, never the job table
(`NULLIF(correlation_id, p_event)` — an uncorrelated event self-roots to its own id). Replay rebuilds
`kb_invocations.correlation_id` from the ledger alone, with no job row in existence. A projection that
re-read the queue passes every other test and is caught only by
`tick_correlation_survives_replay_without_the_job_table`.

The original spec follows, for the record. Additive-only, safe on `main`.

1. **Migration — `kb_workflow_jobs.correlation_id UUID NULL`.** Set at **claim** time in
   `handlers::steward::dispatch` from the `x-steward-correlation-id` header, so every claimed job
   row records the tick that claimed it.
2. **Migration — `kb_invocations.correlation_id UUID NULL`.**
3. **Server-side inheritance, not a model-passed param.** When `invocation_open` fires for a cogmap
   that has an **active claimed job**, the invocation inherits that job's `correlation_id`
   automatically (the job is in `claimed`/running state for the duration of the tick, so it is
   findable). This is deliberately **not** a new `correlation_id` argument on `invocation_open`: a
   model-passed param is the fragile path a weaker model skips (cf. the 2026-07-05 minimax review),
   whereas server-side inheritance is deterministic and needs nothing from the agent.
   - Edge case: a **manual** `invocation_open` with no active job simply gets `NULL` — correct, since
     there is no tick to correlate to.
4. **Surface** `correlation_id` on `DispatchTickResponse` (echo), `invocation_show`, and
   `invocation_list`, so a tick's whole chain is joinable in temper's data layer, not only in logs.

Surfaces touched by the fast-follow: two additive migrations, `handlers::steward::dispatch` (claim
write), the `invocation_open` service/backend path (inheritance read+write), `temper-core` response
types, the MCP invocation tools, and `.sqlx` cache regeneration. That breadth is exactly why it is a
**separate** unit from the log-layer change.

## Acceptance mapping

| Acceptance criterion (task 019f332e) | Met by |
|---|---|
| A no-op tick leaves an observable `steward-agent` log line | Already (#272); enriched with `correlationId` here |
| A drifted tick's cron → `/dispatch` → `invocation_id` joinable by one correlation id | "Now": cron→/dispatch→job→cogmap in logs; **direct** to `invocation_id` with the fast-follow |
| Steward MCP `map_err` preserves NotFound detail | Shipped (#278) |

## Scope & surfaces — the "now" change

- `packages/agent-workflows/steward/agent/schedules/steward.ts` — mint id, log anchor/result/failure
  lines with the id, send the header, log the response `x-vercel-id`.
- `crates/temper-api/src/handlers/steward.rs` — read `x-steward-correlation-id` via `HeaderMap`, log
  `received tick {id}` + inbound `x-vercel-id` on entry. No change to `DispatchTickRequest`.
- No migration, no `.sqlx` change, no `invocation_open` change in the "now" phase.

This is a small, self-contained change spanning one TS file and one Rust handler — a natural single
PR (PR C in this session's set), or the whole thing can be captured as the fast-follow task if not
implemented now.
