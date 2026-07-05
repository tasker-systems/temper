# Steward fan-out — deterministic drift-sweep across all cogmaps

**Status:** design (approved for planning)
**Date:** 2026-07-05
**Goal:** Generalize the steward — deterministic drift-sweep across all cogmaps → fan-out invocations (`019f3220`)
**Predecessor:** team-self-cognition-steward-agent-eve-mvp (`019f1ac7`, COMPLETE)
**Related task:** steward guidance-polish (`019f3213`) — lands first-or-alongside; not a design dependency

## 1. Problem

The deployed steward tends a **single** cognitive map pinned by env var `TEMPER_SELF_COGMAP_ID`
(read in `packages/agent-workflows/steward/agent/schedules/steward.ts:16`, mirrored in
`materialize.ts:39`, baked into the per-run prompt). That was the deliberate MVP shortcut. It does
not scale: no new team gets a steward without a redeploy, and there is no notion of *which* maps
actually need attention right now.

This design replaces the env pin with a **deterministic drift-sweep across all team-joined cogmaps**,
then **fans out one isolated steward run per drifted map**. The gate stays substrate-computed (not
model-judged); the model only performs the authored-4 distillation once a map is selected.

## 2. Scope

**In scope:** the deterministic drift sweep over existing team-joined cogmaps; a Postgres-resident
job queue that serializes dispatch safely; the fan-out dispatcher (steward + materialize); deletion
of the env pin.

**Out of scope (separate goals):** auto-birth-of-self-cogmap-per-team; additional ingest sources
(Linear/GitHub); richer prioritization / backpressure / cross-tick budgeting (seams left, not built).

## 3. Key insight — the seam is additive

The prior code map established that every downstream primitive is **already per-cogmap
parameterized**: `steward_ingest_delta`, `steward_advance_watermark`, `materialize_on_threshold`,
`invocation_open` all take a single cogmap ref. The *only* 1:1 coupling is the env pin, read in two
Eve schedules. So this is additive: add a deterministic sweep + a job queue + a fan-out dispatcher;
delete one env var. No rewrite of the authoring path.

### The determinism split (load-bearing)

- **Deterministic (SQL + code):** enumerate candidate cogmaps → run the ingest-delta gate over all
  of them → return the drifted set, ordered. No model.
- **Model (per selected map only):** the authored-4 distillation, exactly as today, in one isolated
  session per drifted map.

### Hard constraint

**One session tends exactly one cogmap.** A job row / session message carries a single `cogmap_id`,
never a set. The fan-out is over the *workflow*, never over the agent's target. Every unit below
enforces this structurally (one job row = one cogmap = one session = one message).

## 4. Components

| Unit | Layer | Responsibility |
|------|-------|----------------|
| `steward_drift_sweep()` | new SQL set-returning fn (temper-substrate) | one pass: for every team-joined cogmap, compute ingest delta vs its own watermark; return rows above threshold, most-drifted-first |
| `kb_workflow_jobs` + fns | new table + SQL fns (temper-substrate) | persona-agnostic durable job queue: enqueue/claim/complete/reap with single-flight dedup + lease-expiry reaping |
| sweep + queue services | temper-services | authz-scoped wrappers; the `DbBackend` write path for queue mutations |
| `POST /api/steward/dispatch` | temper-api | privileged: reap → sweep → enqueue → claim → return claimed jobs |
| `GET /api/steward/sweep` | temper-api | privileged: the drifted set as typed rows (debug/observability; dispatch endpoint uses the fn directly) |
| steward **dispatcher** schedule | Eve `run` handler | hourly: call `/dispatch`, fan out one isolated session per claimed job |
| materialize **dispatcher** schedule | Eve `run` handler | hourly: fan out a self-gating materialize POST per candidate map (no lease) |
| existing steward agent persona | Eve agent | **unchanged** singular authored-4 discipline; map id now arrives in the session message |

**What does NOT change:** the steward's `instructions.md` / `skills/map-stewardship.md` stay singular
("tend *one* map") — correct, because each fan-out session still tends exactly one map. The invocation
envelope stays one-per-session. The authored-4 tool surface is untouched.

## 5. The deterministic drift sweep

**Candidate set:** `SELECT DISTINCT cogmap_id FROM kb_team_cogmaps`. Team-joined maps only — and that
is structural, not arbitrary: a team-less cogmap produces an empty `team_ctx` CTE inside the existing
delta logic, so its delta is always zero. `kb_team_cogmaps` is the same join `steward_ingest_delta`
and `cogmaps_share_a_team` already key on.

**New set-returning function** — `steward_drift_sweep(p_threshold bigint)`:

```sql
RETURNS TABLE(cogmap_id uuid, watermark uuid, new_resources bigint, new_events bigint)
-- for every team-joined cogmap: read its OWN steward_watermark_event_id,
-- compute the delta since that watermark, keep rows with new_resources >= p_threshold,
-- ORDER BY new_resources DESC   -- most-drifted-first
```

**DRY against the scalar fn.** Today `steward_ingest_delta(p_cogmap, p_watermark)` takes the watermark
as a *param* (the service reads it from `kb_cogmaps` first). The sweep must read each map's own
watermark internally. Rather than copy-paste the counting CTE (the two copies will drift — a
fundamentals rule), **extract the per-cogmap delta computation into one shared SQL expression** that
both the scalar fn and the sweep call. One source of truth for "what counts as drift."

**Authz posture — privileged system read, not user-visibility.** The single-map `steward_ingest_delta`
service gates on `anchor_readable_by_profile` because a *user* asks about *one* map. The sweep is
infrastructure stewarding every team's self-cognition map, driven by the steward app/M2M principal
(the same principal that drives materialize today), analogous to the L0 kernel migration running under
the `system` actor. The sweep endpoint therefore gates on a **system/app principal** and enumerates all
team-joined maps regardless of any individual user's visibility. This is made explicit, not smuggled
through the user-scoped path.

**Scope:** the sweep is steward-specific (it computes *ingest* drift). Materialize deliberately does
not use it — materialize fan-out POSTs to candidate maps and self-gates server-side. One generic job
queue; one steward-specific drift detector.

## 6. The job queue — `kb_workflow_jobs`

### 6.1 What we borrow, and the line we do not cross

tasker-core (sibling repo, production workflow orchestration on Postgres) deliberately splits
**transport (the pgmq extension)** from **durable work-state (hand-rolled `tasker.*` tables)**: pgmq
does visibility-timeout dispatch, but every retry/attempts/dead-letter/staleness decision lives in
hand-rolled SQL. We take the durable half — the part we actually need — and skip pgmq entirely (extra
extension dependency on every Neon target; unneeded at our scale). This is not cargo-culting: the
tables/queries/functions we borrow are well-exercised in tasker-core (thousands of tests, benchmarks,
lock-contention mechanics), and it is our own sibling project, so we intentionally reuse the proven
mechanics and scale-adapt them.

| tasker-core does | We do | Why |
|---|---|---|
| pgmq transport + hand-rolled state tables (two layers) | **one** `kb_workflow_jobs` table | our scale; no DAG/steps to coordinate |
| event-sourced `*_transitions` for status | single `status` enum column | the invocation envelope already audits authored acts; transitions are YAGNI here |
| separate `tasks_dlq` investigation table + resolution workflow | terminal `dead` status + `last_error` on the row | no investigation workflow at our scale; row + logs + envelope suffice |
| `attempts` incremented in Rust app code (their noted gotcha) | increment **in the SQL** claim/fail fns | atomic with the state transition |
| exponential backoff via `next_visible_at` | keep the **column**, don't compute backoff yet | hourly cadence *is* the backoff; column = zero-cost forward-compat for a future fast drainer |
| `FOR UPDATE SKIP LOCKED`, batch, priority DESC | same | proven claim primitive, directly ported |
| unique `identity_hash` dedup | partial unique index on the in-flight tuple | our natural dedup key |

### 6.2 Schema

```sql
CREATE TABLE kb_workflow_jobs (
    id               uuid PRIMARY KEY DEFAULT uuidv7(),
    cogmap_id        uuid NOT NULL REFERENCES kb_cogmaps(id) ON DELETE CASCADE,
    persona          text NOT NULL,              -- Rust enum owns the set ('steward', …)
    dispatch_type    text NOT NULL,              -- 'steward' | 'materialize'
    status           text NOT NULL DEFAULT 'pending', -- pending|in_progress|done|waiting_for_retry|dead
    attempts         int  NOT NULL DEFAULT 0,
    max_attempts     int  NOT NULL DEFAULT 3,
    invocation_id    uuid,                        -- set when the run opens its envelope; ties job↔audit
    enqueued_at      timestamptz NOT NULL DEFAULT now(),
    leased_at        timestamptz,
    lease_expires_at timestamptz,                -- > Vercel function timeout so a live run never looks dead
    next_visible_at  timestamptz NOT NULL DEFAULT now(),  -- forward-compat; hourly cadence = de-facto backoff
    completed_at     timestamptz,
    last_error       text,
    payload          jsonb                        -- minimal: delta counts for logging
);

-- the "idempotent within a band" guarantee: at most ONE active job per (cogmap, persona, dispatch_type)
CREATE UNIQUE INDEX uq_workflow_jobs_in_flight
  ON kb_workflow_jobs (cogmap_id, persona, dispatch_type)
  WHERE status IN ('pending','in_progress','waiting_for_retry');
```

`persona` and `dispatch_type` are bounded sets we own → modeled as Rust enums (no stringly-typed
matches over bounded sets). Stored as `text`; the Rust enum is the source of truth for the variants.

### 6.3 Lifecycle

- **New drift episode → new row.** The watermark advanced on the last `done`, so a fresh sweep
  re-drifts only when *genuinely new* writes appear past that watermark. That new work gets a new row.
  (This is the "sweep self-heals / re-enqueues" behavior.)
- **Failing run → same row retried in place.** A crashed session never advances the watermark, so
  there is no new work — the same active row is reaped (lease expired), `attempts++`, back to
  `waiting_for_retry`. The partial-unique index keeps it single. After `attempts >= max_attempts` →
  `dead` + `last_error`, surfaced loudly in logs. (This is tasker-core's poison-job protection — the
  piece pure self-heal lacks: a map that always crashes the steward would otherwise burn a session
  every hour forever.)
- **Completion ties to the natural success signal.** `steward_advance_watermark` finds the one active
  steward job for that cogmap (guaranteed unique by the in-flight index) and transitions it → `done`
  in the same act. Watermark-advance and job-completion become atomic, which permanently closes the
  concurrency race (the watermark only moves on clean completion).

### 6.4 SQL functions

- `workflow_job_enqueue(cogmap, persona, dispatch_type, payload)` — `INSERT … ON CONFLICT DO NOTHING`
  against the in-flight partial-unique index (enqueue dedup).
- `workflow_job_claim(persona, dispatch_type, limit, lease_ttl)` — `FOR UPDATE SKIP LOCKED`,
  `ORDER BY <most-drifted / priority> LIMIT n`; sets `status='in_progress'`, `leased_at`,
  `lease_expires_at`, `attempts++`. Uses a compare-and-swap transition (flip only
  `WHERE status IN ('pending','waiting_for_retry') AND next_visible_at <= now()`) as the double-claim
  guard.
- `workflow_job_complete(cogmap, persona, dispatch_type)` — called by `steward_advance_watermark`;
  transitions the one active job → `done`, sets `completed_at`.
- `workflow_job_reap(lease_grace)` — expired-lease `in_progress` jobs → `waiting_for_retry`
  (or `dead` if `attempts >= max_attempts`); anti-joins already-terminal rows; records `last_error`.

## 7. Dispatch flow

Division of labor: **Rust owns the queue; TS stays thin** (mirrors `materialize.ts` today).

**`POST /api/steward/dispatch`** (privileged, app/system principal) — one call performs the whole
deterministic pass server-side:

1. `workflow_job_reap(...)` — expired leases → retry/dead.
2. `steward_drift_sweep(threshold)` — the drifted set, most-drifted-first.
3. `workflow_job_enqueue(...)` per drifted map — deduped by the in-flight index.
4. `workflow_job_claim(persona='steward', limit=cap, lease_ttl)` — up to N via SKIP LOCKED.
5. Return the claimed jobs (`job_id`, `cogmap_id`, delta counts).

**Eve steward dispatcher schedule** (`run` handler, hourly cron):

1. `POST /api/steward/dispatch` → claimed jobs.
2. For each claimed job: start one independent agent session (via `receive`/session-start), passing a
   **single** `cogmap_id` + `job_id` in the message.
3. **Every clean end completes the job.** The did-work case completes it via `steward_advance_watermark`
   (which calls `workflow_job_complete` atomically). A graceful end that authored nothing (defensive —
   the sweep already gated the map above threshold, so this is rare) still completes the job via
   `workflow_job_complete` directly, so it is never mistaken for a crash. Only an *unclean* end (crash,
   timeout) leaves the job `in_progress`; its lease then expires and the reaper requeues it next tick.

**Eve materialize dispatcher schedule** (`run` handler, hourly cron): enumerate team-joined maps →
fan out a self-gating `POST /api/cognitive-maps/{id}/materialize` per map. No lease (re-materialization
is idempotent; the server no-ops below threshold). Best-effort only.

## 8. Config / environment

- **Delete `TEMPER_SELF_COGMAP_ID`** from both schedules. Map identity flows from the sweep, never an
  env pin. This single deletion is the single→multi flip.
- Unchanged: `TEMPER_MCP_URL`, `TEMPER_API_URL`, the M2M/Connect/token auth chain.
- New: a **per-tick dispatch cap** (env/config, default ~10) — the minimal budget guard. The sweep
  orders most-drifted-first, so the cap is meaningful. Richer prioritization/backpressure stays
  deferred (the `next_visible_at` column + a decoupled drainer are the pre-cut seams).
- New: lease TTL config (must exceed the Vercel function timeout — 300s default — so a live run never
  looks dead).

## 9. Backward compatibility

Additive-only-on-`main`, clean:

- New table + functions + endpoints — no destructive change, no prod data migration (queue starts
  empty).
- The scalar `steward_ingest_delta` and the single-map `GET /api/steward/{cogmap}/delta` +
  `POST /api/steward/{cogmap}/watermark` endpoints **stay** (refactored to share the delta CTE) —
  still useful for single-map ops/debug.
- The env-pinned schedules are *replaced* by the dispatchers — a change in the `steward/` package, not
  a schema migration. Deploy swaps them.

## 10. Testing

**Intentional-borrow methodology (first implementation pass).** Before writing the queue SQL, a
dedicated study-and-port pass over tasker-core's *specific* battle-tested mechanics — cited by
`file:line`, ported with attribution comments, scale-adapted:

- the `FOR UPDATE SKIP LOCKED … ORDER BY … LIMIT n` claim query shape (lock-contention ordering) —
  `tasker.get_next_ready_tasks`, `migrations/20260110000003_sql_functions.sql:645-752`;
- the compare-and-swap transition (flip status only `WHERE status = expected`, gate on rows-affected)
  as the double-claim guard — `tasker.transition_task_state_atomic`, same file `:1682-1734`;
- the reaper anti-join against already-terminal jobs + state-specific staleness cutoff —
  `tasker.detect_and_transition_stale_tasks` / `get_stale_tasks_for_dlq`, same file `:315-403`, `:926-982`;
- `ON CONFLICT DO NOTHING` on a partial-unique dedup index — `idx_dlq_unique_pending_task`,
  `migrations/20260110000002_constraints_and_indexes.sql:150`.

We port these **primitives**, not the DAG/steps/transitions/DLQ-investigation apparatus — that is the
cargo-cult line.

**Test coverage (borrowing tasker-core's scenarios):**

- SQL fns via `#[sqlx::test]` ephemeral DBs: enqueue-dedup (double-enqueue → one row); concurrent-claim
  (two claimers, SKIP LOCKED → no double-claim); complete (watermark-advance → job `done`); reap
  (lease expiry → `waiting_for_retry`, then `dead` at `max_attempts`).
- Service-layer tests for the sweep (privileged posture) and the dispatch composition.
- e2e driving the real `POST /api/steward/dispatch` tick end-to-end (sweep → enqueue → claim), per the
  "e2e at the production caller's level" discipline.
- Cache regen: `cargo sqlx prepare --workspace -- --all-features`, plus the per-crate rituals for any
  test-target queries (`prepare-services`, `prepare-api`, `prepare-e2e`).

## 11. Open items deferred (seams left, not built)

- **Prioritization / backpressure / budget** beyond the per-tick cap → the `next_visible_at` column +
  a decoupled high-frequency drainer (no schema change needed to add later).
- **Auto-birth-of-self-cogmap-per-team** → separate goal.
- **Additional ingest sources (Linear/GitHub)** → separate goal.
- **Per-map model/effort selection** (minimax vs Claude) → interacts with guidance-polish `019f3213`.

## 12. Component-by-component build order (for the plan)

1. Study-and-port pass over tasker-core (methodology above).
2. `kb_workflow_jobs` table + the four SQL fns + `#[sqlx::test]` coverage.
3. `steward_drift_sweep` fn + shared-delta refactor of `steward_ingest_delta`.
4. Services (sweep + queue) + `DbBackend` write wiring + `workflow_job_complete` hook into
   `steward_advance_watermark`.
5. `POST /api/steward/dispatch` (+ `GET /api/steward/sweep`) endpoints, privileged posture, ts-rs types.
6. e2e for the dispatch tick.
7. Eve dispatcher schedules (steward + materialize); delete the env pin; per-tick cap + lease TTL config.
8. Deploy; observe a tick; confirm fan-out + single-flight + reaping on the live instance.
