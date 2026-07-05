# Steward Fan-Out — Drift-Sweep Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the single env-pinned steward target (`TEMPER_SELF_COGMAP_ID`) with a deterministic drift-sweep across all team-joined cogmaps that fans out one isolated steward run per drifted map.

**Architecture:** A persona-agnostic Postgres job queue (`kb_workflow_jobs`) serializes dispatch with single-flight dedup + lease-expiry reaping (patterns ported from the sibling `tasker-core`'s hand-rolled queue). A deterministic SQL sweep names the drifted set (reusing the existing per-cogmap `steward_ingest_delta` via LATERAL). A privileged dispatch command composes reap→sweep→enqueue→claim server-side; the Eve dispatcher stays thin (call the endpoint, fan out one session per claimed job). Additive-only-on-`main`.

**Tech Stack:** Rust (temper-core types, temper-substrate migrations, temper-services `DbBackend` + services, temper-api handlers), PostgreSQL 18/17 (sqlx compile-checked macros), TypeScript/Eve (agent-workflows/steward).

**Design spec:** `docs/superpowers/specs/2026-07-05-steward-fan-out-drift-sweep-design.md`

## Global Constraints

- **Additive-only-on-`main`.** New table + `CREATE FUNCTION` / `CREATE OR REPLACE FUNCTION` only. No `DROP`, no destructive `ALTER`. New migrations under `migrations/`, sqlx-based.
- **SQL macros are compile-checked.** After ANY change to a `sqlx::query!` / `query_as!` / `query_scalar!` macro or the schema it hits: regenerate the cache with `cargo sqlx prepare --workspace -- --all-features`, and for test-target queries run the per-crate ritual (`cargo make prepare-services`, `cargo make prepare-api`, `cargo make prepare-e2e`). Commit the `.sqlx/` changes. `cargo make check` runs `SQLX_OFFLINE=true` — it is the honest local probe.
- **Persona / dispatch-type are bounded sets we own → Rust enums**, serialized to `text` columns (no Postgres enum — adding a variant must not require a migration). No stringly-typed matches over these sets.
- **Reads scope through `anchor_readable_by_profile`** (or equivalent) — including the sweep. No system bypass; the steward app-principal's broad read comes from grants / `access_mode=open`, still passing through the gate.
- **Writes route through the `Backend` trait / `DbBackend`.** Surfaces dispatch one operations command per inbound call. Never inline `sqlx::query!()` write persistence in a handler.
- **Typed structs over inline JSON**; **params structs** for >5 domain params; **auth before writes**.
- **Test DB gate:** every `#[sqlx::test]` file needs `#![cfg(feature = "test-db")]` (or the module is `#[cfg(all(test, feature = "test-db"))]`, matching `steward_service.rs`).
- **Feature flags on new temper-core types:** mirror `steward.rs` — `#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]` + `ts(export, export_to = "...")`, `#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]`, `#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]` where the type crosses that surface.
- **Constants live in one place:** new tuning constants (lease TTL, dispatch cap default) go in `temper-core/src/types/workflow_job.rs` alongside a doc comment, mirroring `DEFAULT_STEWARD_INGEST_THRESHOLD`.

---

## File Structure

**Task 1 — queue foundation**
- Create: `migrations/20260705000001_workflow_jobs.sql` — `kb_workflow_jobs` table + partial-unique in-flight index + 4 SQL fns.
- Create: `crates/temper-core/src/types/workflow_job.rs` — `Persona`, `DispatchType`, `JobStatus` enums; `ClaimedJob`; constants.
- Modify: `crates/temper-core/src/types/mod.rs` — register the module.
- Create: `crates/temper-services/src/services/workflow_job_service.rs` — thin `enqueue`/`claim`/`complete`/`reap` helpers (called by `DbBackend`; exercised by tests).
- Modify: `crates/temper-services/src/services/mod.rs` — register the module.

**Task 2 — drift sweep + candidates**
- Create: `migrations/20260705000002_steward_drift_sweep.sql` — `steward_drift_sweep` + `steward_candidate_cogmaps` fns (both LATERAL-reuse / scope by principal).
- Modify: `crates/temper-core/src/types/steward.rs` — add `DriftSweepRow`.
- Modify: `crates/temper-services/src/services/steward_service.rs` — add `drift_sweep` + `candidate_cogmaps` reads.
- Modify: `crates/temper-api/src/handlers/steward.rs` — add `sweep` + `candidates` GET handlers.
- Modify: `crates/temper-api/src/routes.rs` — register the two GET routes.

**Task 3 — dispatch composite + completion hook + e2e**
- Modify: `crates/temper-workflow/src/operations/backend.rs` — `StewardDispatchTick` command + `ClaimedJobs` output + `Backend` trait method.
- Modify: `crates/temper-services/src/backend/db_backend.rs` — `steward_dispatch_tick` impl; hook `workflow_job_complete` into `advance_steward_watermark`.
- Modify: `crates/temper-core/src/types/steward.rs` — `DispatchTickRequest`, `DispatchTickResponse`.
- Modify: `crates/temper-api/src/handlers/steward.rs` — `dispatch` POST handler.
- Modify: `crates/temper-api/src/routes.rs` — register the POST route.
- Create: `tests/e2e/tests/steward_dispatch_test.rs` — drive `POST /api/steward/dispatch` end-to-end.

**Task 4 — Eve dispatchers + env-pin removal**
- Rewrite: `packages/agent-workflows/steward/agent/schedules/steward.ts` — code `run` dispatcher; delete `TEMPER_SELF_COGMAP_ID`.
- Rewrite: `packages/agent-workflows/steward/agent/schedules/materialize.ts` — fan out over candidates; delete `TEMPER_SELF_COGMAP_ID`.
- Modify: `packages/agent-workflows/steward/agent/instructions.md` / relevant docs — note map id arrives per-session (only if a stale singular claim needs correcting; verify first).

---

## Task 1: The `kb_workflow_jobs` queue

**Files:**
- Create: `migrations/20260705000001_workflow_jobs.sql`
- Create: `crates/temper-core/src/types/workflow_job.rs`
- Modify: `crates/temper-core/src/types/mod.rs`
- Create: `crates/temper-services/src/services/workflow_job_service.rs`
- Modify: `crates/temper-services/src/services/mod.rs`

**Interfaces:**
- Produces (SQL): `workflow_job_enqueue(uuid, text, text, jsonb) → uuid`; `workflow_job_claim(text, text, int, int) → TABLE(id uuid, cogmap_id uuid, attempts int, payload jsonb)`; `workflow_job_complete(uuid, text, text) → uuid`; `workflow_job_reap(text) → int`.
- Produces (Rust): `workflow_job_service::{enqueue, claim, complete, reap}`; `ClaimedJob { id: Uuid, cogmap_id: Uuid, attempts: i32, payload: serde_json::Value }`; `Persona::Steward`, `DispatchType::Steward`, `DEFAULT_STEWARD_LEASE_SECONDS: i32 = 600`, `DEFAULT_STEWARD_DISPATCH_CAP: i64 = 10`.

- [ ] **Step 1: Study-and-port pass over tasker-core (no code yet).**

Read these exact functions in `/Users/petetaylor/projects/tasker-systems/tasker-core` and confirm our fn bodies below match their proven mechanics (lock ordering, SKIP LOCKED, the CAS status guard, dedup). Adjust the SQL in Step 2 only if a mechanic differs:
- Claim: `tasker.get_next_ready_tasks` — `migrations/20260110000003_sql_functions.sql:645-752` (the `FOR UPDATE SKIP LOCKED … ORDER BY … LIMIT n` shape).
- CAS transition guard: `tasker.transition_task_state_atomic` — same file `:1682-1734` (flip only `WHERE to_state = expected`).
- Reaper: `tasker.detect_and_transition_stale_tasks` / `get_stale_tasks_for_dlq` — `:315-403`, `:926-982` (expired-lease detection + anti-join terminal rows).
- Dedup: partial-unique `idx_dlq_unique_pending_task` + `ON CONFLICT DO NOTHING` — `migrations/20260110000002_constraints_and_indexes.sql:150`.

Port the **primitives**, not the DAG/steps/transitions/DLQ-investigation apparatus.

- [ ] **Step 2: Write the migration.**

Create `migrations/20260705000001_workflow_jobs.sql`:

```sql
-- Persona-agnostic durable job queue for agent dispatch (steward fan-out; goal 019f3220).
--
-- A single hand-rolled table + four SQL primitives, borrowing tasker-core's DURABLE-state
-- mechanics (SKIP LOCKED claim, attempts/max_attempts gating, lease-expiry reaping, partial-unique
-- in-flight dedup) while skipping its pgmq transport + DAG/DLQ apparatus (scale-adapted, not
-- cargo-culted). See docs/superpowers/specs/2026-07-05-steward-fan-out-drift-sweep-design.md §6.
--
-- ADDITIVE, additive-only-on-`main`: a new table + four new functions; no existing object altered.

CREATE TABLE kb_workflow_jobs (
    id               uuid PRIMARY KEY DEFAULT uuidv7(),
    cogmap_id        uuid NOT NULL REFERENCES kb_cogmaps(id) ON DELETE CASCADE,
    persona          text NOT NULL,
    dispatch_type    text NOT NULL,
    status           text NOT NULL DEFAULT 'pending',
    attempts         int  NOT NULL DEFAULT 0,
    max_attempts     int  NOT NULL DEFAULT 3,
    invocation_id    uuid,
    enqueued_at      timestamptz NOT NULL DEFAULT now(),
    leased_at        timestamptz,
    lease_expires_at timestamptz,
    next_visible_at  timestamptz NOT NULL DEFAULT now(),
    completed_at     timestamptz,
    last_error       text,
    payload          jsonb NOT NULL DEFAULT '{}'::jsonb
);

COMMENT ON TABLE kb_workflow_jobs IS
    'Durable agent-dispatch queue (goal 019f3220). At most one ACTIVE job per '
    '(cogmap_id, persona, dispatch_type) — the single-flight guarantee — via the partial-unique index.';

-- Single-flight: at most ONE active job per (cogmap, persona, dispatch_type).
CREATE UNIQUE INDEX uq_workflow_jobs_in_flight
    ON kb_workflow_jobs (cogmap_id, persona, dispatch_type)
    WHERE status IN ('pending', 'in_progress', 'waiting_for_retry');

-- Claim scan support.
CREATE INDEX idx_workflow_jobs_claimable
    ON kb_workflow_jobs (persona, dispatch_type, next_visible_at)
    WHERE status IN ('pending', 'waiting_for_retry');

-- Enqueue: idempotent within a band. ON CONFLICT on the in-flight index makes a re-enqueue a no-op
-- (returns NULL) while a job is already active for the tuple.
CREATE FUNCTION workflow_job_enqueue(
    p_cogmap uuid, p_persona text, p_dispatch_type text, p_payload jsonb DEFAULT '{}'::jsonb
) RETURNS uuid LANGUAGE sql AS $$
    INSERT INTO kb_workflow_jobs (cogmap_id, persona, dispatch_type, payload)
    VALUES (p_cogmap, p_persona, p_dispatch_type, p_payload)
    ON CONFLICT DO NOTHING
    RETURNING id;
$$;

-- Claim: FOR UPDATE SKIP LOCKED over claimable rows, oldest-first; flip to in_progress, set the
-- lease, and increment attempts in the same statement (the increment lives in SQL, not app code).
CREATE FUNCTION workflow_job_claim(
    p_persona text, p_dispatch_type text, p_limit int, p_lease_seconds int
) RETURNS TABLE(id uuid, cogmap_id uuid, attempts int, payload jsonb)
LANGUAGE sql AS $$
    UPDATE kb_workflow_jobs j
       SET status = 'in_progress',
           leased_at = now(),
           lease_expires_at = now() + make_interval(secs => p_lease_seconds),
           attempts = j.attempts + 1
     WHERE j.id IN (
         SELECT c.id
           FROM kb_workflow_jobs c
          WHERE c.persona = p_persona
            AND c.dispatch_type = p_dispatch_type
            AND c.status IN ('pending', 'waiting_for_retry')
            AND c.next_visible_at <= now()
          ORDER BY c.enqueued_at
          LIMIT p_limit
          FOR UPDATE SKIP LOCKED
     )
    RETURNING j.id, j.cogmap_id, j.attempts, j.payload;
$$;

-- Complete: transition the ONE active job for the tuple → done (the in-flight index guarantees
-- uniqueness). Called on clean run completion (see advance_steward_watermark).
CREATE FUNCTION workflow_job_complete(
    p_cogmap uuid, p_persona text, p_dispatch_type text
) RETURNS uuid LANGUAGE sql AS $$
    UPDATE kb_workflow_jobs
       SET status = 'done', completed_at = now()
     WHERE cogmap_id = p_cogmap
       AND persona = p_persona
       AND dispatch_type = p_dispatch_type
       AND status IN ('pending', 'in_progress', 'waiting_for_retry')
    RETURNING id;
$$;

-- Reap: expired-lease in_progress jobs → waiting_for_retry, or dead once attempts have hit
-- max_attempts (attempts was already incremented at claim). Returns the count reaped.
CREATE FUNCTION workflow_job_reap(p_error text DEFAULT 'lease expired') RETURNS int
LANGUAGE sql AS $$
    WITH expired AS (
        SELECT id, attempts, max_attempts
          FROM kb_workflow_jobs
         WHERE status = 'in_progress'
           AND lease_expires_at < now()
         FOR UPDATE SKIP LOCKED
    ), updated AS (
        UPDATE kb_workflow_jobs j
           SET status = CASE WHEN e.attempts >= e.max_attempts THEN 'dead' ELSE 'waiting_for_retry' END,
               last_error = p_error,
               lease_expires_at = NULL,
               completed_at = CASE WHEN e.attempts >= e.max_attempts THEN now() ELSE NULL END
          FROM expired e
         WHERE j.id = e.id
        RETURNING j.id
    )
    SELECT count(*)::int FROM updated;
$$;
```

- [ ] **Step 3: Write the temper-core types.**

Create `crates/temper-core/src/types/workflow_job.rs`:

```rust
//! Types for the persona-agnostic agent-dispatch job queue (`kb_workflow_jobs`, goal 019f3220).
//!
//! The queue serializes fan-out steward runs: at most one active job per
//! (cogmap, persona, dispatch_type). `Persona` and `DispatchType` are bounded sets we own — Rust
//! enums (serialized to `text`), so a new variant is a code change, never a migration.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Lease duration for a claimed job. MUST exceed the Vercel function timeout (300s default) so a
/// genuinely-running steward session never looks dead to the reaper.
pub const DEFAULT_STEWARD_LEASE_SECONDS: i32 = 600;

/// Default number of drifted maps dispatched per tick — the minimal budget guard. The sweep orders
/// most-drifted-first, so the cap is meaningful; richer prioritization is deferred.
pub const DEFAULT_STEWARD_DISPATCH_CAP: i64 = 10;

/// Which agent persona a queued job is for. One variant today; the queue is persona-agnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Persona {
    Steward,
}

impl Persona {
    pub fn as_str(self) -> &'static str {
        match self {
            Persona::Steward => "steward",
        }
    }
}

/// The kind of dispatch a job represents. Only `Steward` is queued today (materialize fans out
/// lease-free); the column is forward-looking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchType {
    Steward,
}

impl DispatchType {
    pub fn as_str(self) -> &'static str {
        match self {
            DispatchType::Steward => "steward",
        }
    }
}

/// A job claimed for dispatch — the caller starts exactly one agent session per `ClaimedJob`,
/// carrying its single `cogmap_id`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "steward.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimedJob {
    pub id: Uuid,
    pub cogmap_id: Uuid,
    pub attempts: i32,
}
```

Register in `crates/temper-core/src/types/mod.rs` (add `pub mod workflow_job;` in module order).

- [ ] **Step 4: Write the service helpers with failing tests.**

Create `crates/temper-services/src/services/workflow_job_service.rs`:

```rust
//! Thin wrappers over the `kb_workflow_jobs` SQL primitives (goal 019f3220). `DbBackend` composes
//! these into the dispatch tick; tests exercise them directly. Auth is NOT here — these are queue
//! primitives; the dispatch command that composes them carries the auth gate.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;
use temper_core::types::workflow_job::ClaimedJob;

/// Enqueue a job for `(cogmap, persona, dispatch_type)`. Returns `Some(id)` when a new row was
/// created, `None` when one is already in-flight for the tuple (the single-flight dedup).
pub async fn enqueue(
    pool: &PgPool,
    cogmap_id: Uuid,
    persona: &str,
    dispatch_type: &str,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"SELECT workflow_job_enqueue($1, $2, $3, '{}'::jsonb) AS "id: Uuid""#,
        cogmap_id,
        persona,
        dispatch_type,
    )
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Claim up to `limit` claimable jobs, leasing each for `lease_seconds`.
pub async fn claim(
    pool: &PgPool,
    persona: &str,
    dispatch_type: &str,
    limit: i32,
    lease_seconds: i32,
) -> ApiResult<Vec<ClaimedJob>> {
    let rows = sqlx::query!(
        r#"
        SELECT id AS "id!: Uuid", cogmap_id AS "cogmap_id!: Uuid", attempts AS "attempts!: i32"
          FROM workflow_job_claim($1, $2, $3, $4)
        "#,
        persona,
        dispatch_type,
        limit,
        lease_seconds,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ClaimedJob { id: r.id, cogmap_id: r.cogmap_id, attempts: r.attempts })
        .collect())
}

/// Transition the one active job for the tuple → done. Returns the job id if one was active.
pub async fn complete(
    pool: &PgPool,
    cogmap_id: Uuid,
    persona: &str,
    dispatch_type: &str,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"SELECT workflow_job_complete($1, $2, $3) AS "id: Uuid""#,
        cogmap_id,
        persona,
        dispatch_type,
    )
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Reap expired-lease jobs → retry (or dead at max attempts). Returns the count reaped.
pub async fn reap(pool: &PgPool, error: &str) -> ApiResult<i32> {
    let n = sqlx::query_scalar!(r#"SELECT workflow_job_reap($1) AS "n!: i32""#, error)
        .fetch_one(pool)
        .await?;
    Ok(n)
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;

    async fn a_cogmap(pool: &PgPool) -> Uuid {
        let telos: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ('telos', '') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query_scalar("INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('m', $1) RETURNING id")
            .bind(telos)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn status_of(pool: &PgPool, id: Uuid) -> String {
        sqlx::query_scalar("SELECT status FROM kb_workflow_jobs WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn enqueue_dedup_keeps_one_active(pool: PgPool) {
        let c = a_cogmap(&pool).await;
        let first = enqueue(&pool, c, "steward", "steward").await.unwrap();
        let second = enqueue(&pool, c, "steward", "steward").await.unwrap();
        assert!(first.is_some(), "first enqueue creates a row");
        assert!(second.is_none(), "second is a no-op while the first is in-flight");
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_workflow_jobs WHERE cogmap_id = $1")
            .bind(c)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn claim_leases_and_increments_attempts(pool: PgPool) {
        let c = a_cogmap(&pool).await;
        enqueue(&pool, c, "steward", "steward").await.unwrap();
        let claimed = claim(&pool, "steward", "steward", 10, 600).await.unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].cogmap_id, c);
        assert_eq!(claimed[0].attempts, 1, "attempts incremented at claim");
        assert_eq!(status_of(&pool, claimed[0].id).await, "in_progress");
        // A second claim finds nothing — it is no longer claimable.
        let again = claim(&pool, "steward", "steward", 10, 600).await.unwrap();
        assert!(again.is_empty(), "in_progress is not re-claimable");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn complete_marks_done_and_frees_the_slot(pool: PgPool) {
        let c = a_cogmap(&pool).await;
        enqueue(&pool, c, "steward", "steward").await.unwrap();
        claim(&pool, "steward", "steward", 10, 600).await.unwrap();
        let done = complete(&pool, c, "steward", "steward").await.unwrap();
        assert!(done.is_some());
        // Slot freed: a fresh drift episode can enqueue again.
        let reenq = enqueue(&pool, c, "steward", "steward").await.unwrap();
        assert!(reenq.is_some(), "done row does not block the in-flight index");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reap_expired_lease_retries_then_dead(pool: PgPool) {
        let c = a_cogmap(&pool).await;
        enqueue(&pool, c, "steward", "steward").await.unwrap();
        // Claim with a already-past lease (negative seconds → lease_expires_at in the past).
        let claimed = claim(&pool, "steward", "steward", 10, -1).await.unwrap();
        let id = claimed[0].id;
        // attempts=1, max=3 → reap sends it to waiting_for_retry.
        assert_eq!(reap(&pool, "boom").await.unwrap(), 1);
        assert_eq!(status_of(&pool, id).await, "waiting_for_retry");
        // Two more claim+reap cycles (attempts 2, then 3) → dead at attempts >= max_attempts.
        claim(&pool, "steward", "steward", 10, -1).await.unwrap();
        reap(&pool, "boom").await.unwrap();
        claim(&pool, "steward", "steward", 10, -1).await.unwrap();
        reap(&pool, "boom").await.unwrap();
        assert_eq!(status_of(&pool, id).await, "dead", "attempts hit max_attempts → dead");
    }
}
```

Register in `crates/temper-services/src/services/mod.rs` (add `pub mod workflow_job_service;`).

- [ ] **Step 5: Run the tests — verify they fail (fns/table not yet applied to the compile-check DB).**

Run: `export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development && cargo make docker-up && cargo sqlx migrate run` then `cargo nextest run -p temper-services --features test-db workflow_job_service`
Expected: compile OR test failure until Step 2's migration is applied to the dev DB and the cache is generated.

- [ ] **Step 6: Regenerate the sqlx cache.**

Run: `cargo sqlx prepare --workspace -- --all-features`
Then `cargo make prepare-services`.
Expected: `.sqlx/` entries added for the new `query!` macros.

- [ ] **Step 7: Run the tests — verify they pass.**

Run: `cargo nextest run -p temper-services --features test-db workflow_job_service`
Expected: 4 tests PASS.

- [ ] **Step 8: `cargo make check`, then commit.**

```bash
cargo make check
git add migrations/20260705000001_workflow_jobs.sql \
        crates/temper-core/src/types/workflow_job.rs crates/temper-core/src/types/mod.rs \
        crates/temper-services/src/services/workflow_job_service.rs \
        crates/temper-services/src/services/mod.rs .sqlx crates/temper-services/.sqlx
git commit -m "feat(steward): kb_workflow_jobs queue — enqueue/claim/complete/reap primitives

Persona-agnostic Postgres dispatch queue with single-flight dedup + lease-expiry
reaping, borrowing tasker-core's hand-rolled queue mechanics (SKIP LOCKED claim,
attempts/max_attempts → dead), skipping pgmq/DAG/DLQ. Goal 019f3220.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Drift sweep + candidate enumeration

**Files:**
- Create: `migrations/20260705000002_steward_drift_sweep.sql`
- Modify: `crates/temper-core/src/types/steward.rs`
- Modify: `crates/temper-services/src/services/steward_service.rs`
- Modify: `crates/temper-api/src/handlers/steward.rs`
- Modify: `crates/temper-api/src/routes.rs`

**Interfaces:**
- Consumes: existing `steward_ingest_delta(uuid, uuid)` (Task-independent; already shipped).
- Produces (SQL): `steward_drift_sweep(p_principal uuid, p_threshold bigint) → TABLE(cogmap_id uuid, watermark uuid, new_resources bigint, new_events bigint)`; `steward_candidate_cogmaps(p_principal uuid) → TABLE(cogmap_id uuid)`.
- Produces (Rust): `DriftSweepRow { cogmap_id, watermark: Option<Uuid>, new_resources, new_events }`; `steward_service::{drift_sweep, candidate_cogmaps}`; `GET /api/steward/sweep`, `GET /api/steward/candidates`.

- [ ] **Step 1: Write the migration.**

Create `migrations/20260705000002_steward_drift_sweep.sql`:

```sql
-- Deterministic drift sweep across all team-joined cogmaps (goal 019f3220).
--
-- REUSE, not re-implement: the sweep calls the existing per-cogmap `steward_ingest_delta` via a
-- LATERAL join (DRY — one source of truth for "what counts as drift"), reading each map's own
-- watermark internally. Candidate set = team-joined cogmaps (kb_team_cogmaps), scoped through
-- `anchor_readable_by_profile(principal, ...)` — the same read gate every other query uses; the
-- steward app-principal's broad read comes from grants / access_mode=open, not a bypass.
--
-- ADDITIVE: two new functions; nothing altered.

CREATE FUNCTION steward_candidate_cogmaps(p_principal uuid)
RETURNS TABLE(cogmap_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT DISTINCT tc.cogmap_id
      FROM kb_team_cogmaps tc
     WHERE anchor_readable_by_profile(p_principal, 'kb_cogmaps', tc.cogmap_id);
$$;

CREATE FUNCTION steward_drift_sweep(p_principal uuid, p_threshold bigint)
RETURNS TABLE(cogmap_id uuid, watermark uuid, new_resources bigint, new_events bigint)
LANGUAGE sql STABLE AS $$
    SELECT m.cogmap_id,
           cm.steward_watermark_event_id AS watermark,
           d.new_resources,
           d.new_events
      FROM steward_candidate_cogmaps(p_principal) m
      JOIN kb_cogmaps cm ON cm.id = m.cogmap_id
      CROSS JOIN LATERAL steward_ingest_delta(m.cogmap_id, cm.steward_watermark_event_id) d
     WHERE d.new_resources >= p_threshold
     ORDER BY d.new_resources DESC;
$$;
```

- [ ] **Step 2: Add the `DriftSweepRow` type.**

Append to `crates/temper-core/src/types/steward.rs`:

```rust
/// One drifted cogmap in a sweep result — the map plus its ingest delta since its own watermark.
/// Ordered most-drifted-first by the sweep.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "steward.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftSweepRow {
    pub cogmap_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watermark: Option<Uuid>,
    pub new_resources: i64,
    pub new_events: i64,
}
```

- [ ] **Step 3: Add service reads with failing tests.**

Append to `crates/temper-services/src/services/steward_service.rs` (before the `tests` module) — reuse the module's existing `seed`/`add_event`/`grant_cogmap_write` helpers in the tests:

```rust
/// Sweep all team-joined cogmaps the principal can read, returning those whose ingest delta clears
/// `threshold`, most-drifted-first. The privileged case (steward app-principal) simply has broad
/// read; the gate is the same `anchor_readable_by_profile` every read uses.
pub async fn drift_sweep(
    pool: &PgPool,
    principal: ProfileId,
    threshold: Option<i64>,
) -> ApiResult<Vec<temper_core::types::steward::DriftSweepRow>> {
    use temper_core::types::steward::DriftSweepRow;
    let threshold = threshold.unwrap_or(DEFAULT_STEWARD_INGEST_THRESHOLD);
    let rows = sqlx::query!(
        r#"
        SELECT cogmap_id   AS "cogmap_id!: Uuid",
               watermark   AS "watermark: Uuid",
               new_resources AS "new_resources!: i64",
               new_events    AS "new_events!: i64"
          FROM steward_drift_sweep($1, $2)
        "#,
        *principal,
        threshold,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| DriftSweepRow {
            cogmap_id: r.cogmap_id,
            watermark: r.watermark,
            new_resources: r.new_resources,
            new_events: r.new_events,
        })
        .collect())
}

/// All team-joined cogmaps the principal can read (materialize fan-out candidate set).
pub async fn candidate_cogmaps(pool: &PgPool, principal: ProfileId) -> ApiResult<Vec<Uuid>> {
    let ids = sqlx::query_scalar!(
        r#"SELECT cogmap_id AS "id!: Uuid" FROM steward_candidate_cogmaps($1)"#,
        *principal,
    )
    .fetch_all(pool)
    .await?;
    Ok(ids)
}
```

Add to the `tests` module:

```rust
    #[sqlx::test(migrations = "../../migrations")]
    async fn sweep_returns_only_drifted_maps_most_drifted_first(pool: PgPool) {
        let s = seed(&pool).await;
        // 6 resource_created in the team context → above default threshold 5.
        for _ in 0..6 {
            add_event(&pool, s.entity, "resource_created", s.ctx).await;
        }
        let rows = drift_sweep(&pool, s.member.into(), None).await.unwrap();
        assert_eq!(rows.len(), 1, "the one drifted, readable, team-joined map");
        assert_eq!(rows[0].cogmap_id, s.cogmap);
        assert_eq!(rows[0].new_resources, 6);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn sweep_excludes_below_threshold_and_unreadable(pool: PgPool) {
        let s = seed(&pool).await;
        for _ in 0..2 {
            add_event(&pool, s.entity, "resource_created", s.ctx).await;
        }
        // Below threshold for the member.
        assert!(drift_sweep(&pool, s.member.into(), None).await.unwrap().is_empty());
        // Outsider cannot read the map → never a candidate, even above threshold.
        for _ in 0..6 {
            add_event(&pool, s.entity, "resource_created", s.ctx).await;
        }
        assert!(drift_sweep(&pool, s.outsider.into(), None).await.unwrap().is_empty());
        assert_eq!(drift_sweep(&pool, s.member.into(), None).await.unwrap().len(), 1);
    }
```

- [ ] **Step 4: Add the API handlers.**

Append to `crates/temper-api/src/handlers/steward.rs`:

```rust
#[utoipa::path(
    get,
    path = "/api/steward/sweep",
    tag = "Steward",
    params(("threshold" = Option<i64>, Query, description = "Ingest threshold (default applies when omitted)")),
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Drifted team-joined cogmaps, most-drifted-first", body = Vec<DriftSweepRow>))
)]
pub async fn sweep(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<DeltaQuery>,
) -> ApiResult<Json<Vec<DriftSweepRow>>> {
    let rows = steward_service::drift_sweep(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        q.threshold,
    )
    .await?;
    Ok(Json(rows))
}

#[utoipa::path(
    get,
    path = "/api/steward/candidates",
    tag = "Steward",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Readable team-joined cogmap ids", body = Vec<Uuid>))
)]
pub async fn candidates(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<Vec<Uuid>>> {
    let ids = steward_service::candidate_cogmaps(&state.pool, ProfileId::from(auth.0.profile.id)).await?;
    Ok(Json(ids))
}
```

Add `DriftSweepRow` to the `use temper_core::types::steward::{...}` import line.

- [ ] **Step 5: Register routes.**

In `crates/temper-api/src/routes.rs`, next to the existing steward routes (near `:216-219`), add:

```rust
        .route("/api/steward/sweep", get(handlers::steward::sweep))
        .route("/api/steward/candidates", get(handlers::steward::candidates))
```

(Ensure `get` is imported in that module — it already is for the existing `/delta` GET.)

- [ ] **Step 6: Apply migration, regenerate cache, run tests.**

```bash
cargo sqlx migrate run
cargo sqlx prepare --workspace -- --all-features && cargo make prepare-services && cargo make prepare-api
cargo nextest run -p temper-services --features test-db steward_service
```
Expected: the two new sweep tests PASS alongside the existing ingest tests.

- [ ] **Step 7: `cargo make check`, then commit.**

```bash
cargo make check
git add migrations/20260705000002_steward_drift_sweep.sql crates/temper-core/src/types/steward.rs \
        crates/temper-services/src/services/steward_service.rs \
        crates/temper-api/src/handlers/steward.rs crates/temper-api/src/routes.rs \
        .sqlx crates/temper-services/.sqlx crates/temper-api/.sqlx
git commit -m "feat(steward): drift sweep + candidate enumeration over team-joined cogmaps

steward_drift_sweep reuses steward_ingest_delta via LATERAL (DRY), scoped through
anchor_readable_by_profile. GET /api/steward/sweep + /candidates. Goal 019f3220.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Dispatch composite + completion hook + e2e

**Files:**
- Modify: `crates/temper-workflow/src/operations/backend.rs`
- Modify: `crates/temper-services/src/backend/db_backend.rs`
- Modify: `crates/temper-core/src/types/steward.rs`
- Modify: `crates/temper-api/src/handlers/steward.rs`
- Modify: `crates/temper-api/src/routes.rs`
- Create: `tests/e2e/tests/steward_dispatch_test.rs`

**Interfaces:**
- Consumes: `workflow_job_service::{enqueue, claim, reap}`, `steward_service::drift_sweep`, `DEFAULT_STEWARD_LEASE_SECONDS`, `DEFAULT_STEWARD_DISPATCH_CAP`, `ClaimedJob`.
- Produces: `StewardDispatchTick { threshold: Option<i64>, cap: Option<i64>, origin: Surface }` command; `Backend::steward_dispatch_tick(cmd) → CommandOutput<Vec<ClaimedJob>>`; `DispatchTickRequest`, `DispatchTickResponse { claimed: Vec<ClaimedJob> }`; `POST /api/steward/dispatch`.

- [ ] **Step 1: Define the command + trait method.**

In `crates/temper-workflow/src/operations/backend.rs`, mirror `AdvanceStewardWatermark` (its command struct + `Backend` trait method near `:149`):

```rust
/// Compose one deterministic steward-dispatch pass: reap stale jobs, sweep drifted maps, enqueue
/// (deduped), and claim up to `cap` for fan-out. Auth: the principal's readable candidate set gates
/// the sweep; only steward jobs are enqueued/claimed.
#[derive(Debug, Clone)]
pub struct StewardDispatchTick {
    pub threshold: Option<i64>,
    pub cap: Option<i64>,
    pub origin: Surface,
}
```

Add to the `Backend` trait:

```rust
    async fn steward_dispatch_tick(
        &self,
        cmd: StewardDispatchTick,
    ) -> Result<CommandOutput<Vec<temper_core::types::workflow_job::ClaimedJob>>, TemperError>;
```

- [ ] **Step 2: Implement it in `DbBackend` + hook completion into watermark-advance.**

In `crates/temper-services/src/backend/db_backend.rs`, add the impl (uses `self.profile_id` as the sweep principal):

```rust
    async fn steward_dispatch_tick(
        &self,
        cmd: StewardDispatchTick,
    ) -> Result<CommandOutput<Vec<temper_core::types::workflow_job::ClaimedJob>>, TemperError> {
        use crate::services::{steward_service, workflow_job_service};
        use temper_core::types::workflow_job::{DEFAULT_STEWARD_DISPATCH_CAP, DEFAULT_STEWARD_LEASE_SECONDS};

        // 1. Reap stale leases (crashed runs → retry/dead) before claiming.
        workflow_job_service::reap(&self.pool, "lease expired").await.map_err(api_err_from)?;

        // 2. Deterministic sweep over readable team-joined maps.
        let drifted = steward_service::drift_sweep(&self.pool, self.profile_id, cmd.threshold)
            .await
            .map_err(api_err_from)?;

        // 3. Enqueue each drifted map (deduped by the in-flight index).
        for row in &drifted {
            workflow_job_service::enqueue(&self.pool, row.cogmap_id, "steward", "steward")
                .await
                .map_err(api_err_from)?;
        }

        // 4. Claim up to cap for fan-out.
        let cap = cmd.cap.unwrap_or(DEFAULT_STEWARD_DISPATCH_CAP) as i32;
        let claimed = workflow_job_service::claim(
            &self.pool, "steward", "steward", cap, DEFAULT_STEWARD_LEASE_SECONDS,
        )
        .await
        .map_err(api_err_from)?;

        Ok(CommandOutput::new(claimed))
    }
```

(Match the existing error-conversion helper used in this file — `api_err` / `ApiError::from`; the sketch's `api_err_from` stands for whatever the file already uses to turn an `ApiResult` error into `TemperError`. Verify against a neighboring method.)

Then hook completion into `advance_steward_watermark` — after the `UPDATE kb_cogmaps SET steward_watermark_event_id` (`:1725-1732`), before the `Ok(...)`:

```rust
        // A clean watermark advance IS steward-run completion — complete the active job atomically
        // so the concurrency race closes (the watermark only moves on clean completion).
        crate::services::workflow_job_service::complete(&self.pool, *cmd.cogmap, "steward", "steward")
            .await
            .map_err(api_err_from)?;
```

- [ ] **Step 3: Add the request/response types.**

Append to `crates/temper-core/src/types/steward.rs`:

```rust
use crate::types::workflow_job::ClaimedJob;

/// Request body for `POST /api/steward/dispatch`. Both optional — server defaults apply.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct DispatchTickRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap: Option<i64>,
}

/// Response for a dispatch tick — the jobs claimed for fan-out (one session per entry).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "steward.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchTickResponse {
    pub claimed: Vec<ClaimedJob>,
}
```

- [ ] **Step 4: Add the POST handler + route.**

Append to `crates/temper-api/src/handlers/steward.rs` (add the imports: `DispatchTickRequest`, `DispatchTickResponse`, `StewardDispatchTick`):

```rust
#[utoipa::path(
    post,
    path = "/api/steward/dispatch",
    tag = "Steward",
    security(("bearer_auth" = [])),
    request_body = DispatchTickRequest,
    responses((status = 200, description = "Jobs claimed for fan-out", body = DispatchTickResponse))
)]
pub async fn dispatch(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<DispatchTickRequest>,
) -> ApiResult<Json<DispatchTickResponse>> {
    let cmd = StewardDispatchTick { threshold: req.threshold, cap: req.cap, origin: Surface::ApiHttp };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend.steward_dispatch_tick(cmd).await.map_err(ApiError::from)?;
    Ok(Json(DispatchTickResponse { claimed: out.value }))
}
```

In `routes.rs` add: `.route("/api/steward/dispatch", post(handlers::steward::dispatch))` (import `post` if not already).

- [ ] **Step 5: Write the e2e test.**

Create `tests/e2e/tests/steward_dispatch_test.rs` — drive the real endpoint through the Axum server + Postgres. Follow the harness pattern in `tests/e2e/tests/common/` (spawn server, mint JWT for a profile that owns/reads a team-joined cogmap, seed team + cogmap + team-context + N `resource_created` events above threshold), then:

```rust
// POST /api/steward/dispatch → expect one claimed job for the drifted map.
let resp = client.post(format!("{base}/api/steward/dispatch")).bearer(&jwt).json(&serde_json::json!({})).send().await;
assert_eq!(resp.status(), 200);
let body: DispatchTickResponse = resp.json().await;
assert_eq!(body.claimed.len(), 1);
assert_eq!(body.claimed[0].cogmap_id, cogmap_id);
// A second immediate dispatch claims nothing (single-flight — the first is in_progress).
let resp2 = client.post(...).json(&serde_json::json!({})).send().await;
assert!(resp2.json::<DispatchTickResponse>().await.claimed.is_empty());
```

(Match the exact harness helpers + JWT fixture usage from a sibling e2e test, e.g. an existing steward or cogmap e2e; do not invent harness APIs — grep `tests/e2e/tests/common/` first.)

- [ ] **Step 6: Regenerate caches, run tests.**

```bash
cargo sqlx prepare --workspace -- --all-features && cargo make prepare-services && cargo make prepare-api && cargo make prepare-e2e
cargo nextest run -p temper-services --features test-db
cargo make test-e2e -- steward_dispatch
```
Expected: dispatch composite unit path + e2e PASS.

- [ ] **Step 7: `cargo make check`, then commit.**

```bash
cargo make check
git add crates/temper-workflow/src/operations/backend.rs crates/temper-services/src/backend/db_backend.rs \
        crates/temper-core/src/types/steward.rs crates/temper-api/src/handlers/steward.rs \
        crates/temper-api/src/routes.rs tests/e2e/tests/steward_dispatch_test.rs \
        .sqlx crates/temper-services/.sqlx crates/temper-api/.sqlx tests/e2e/.sqlx
git commit -m "feat(steward): dispatch composite (reap→sweep→enqueue→claim) + completion hook

POST /api/steward/dispatch runs one deterministic pass and returns claimed jobs;
advance_steward_watermark now atomically completes the active job (closes the race).
e2e drives the endpoint. Goal 019f3220.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Eve dispatchers + env-pin removal

**Files:**
- Rewrite: `packages/agent-workflows/steward/agent/schedules/steward.ts`
- Rewrite: `packages/agent-workflows/steward/agent/schedules/materialize.ts`
- Verify/adjust: `packages/agent-workflows/steward/agent/instructions.md` (only if a stale singular claim conflicts with per-session id delivery).

**Interfaces:**
- Consumes: `POST /api/steward/dispatch` → `{ claimed: [{ id, cogmap_id, attempts }] }`; `GET /api/steward/candidates` → `[uuid]`; `POST /api/cognitive-maps/{id}/materialize`.
- Note: run tooling from inside `steward/` (`cd steward && npm install`), never the repo root (workspace-isolated Eve project).

- [ ] **Step 1: Study the Eve fan-out primitive.**

Read `packages/agent-workflows/steward/node_modules/eve/docs/schedules.mdx` (the `run` handler + `receive`) and `patterns/dynamic-scheduling.md` (the `jobs.map(job => receive(...))` fan-out). Confirm how to start one durable session per claimed job (via `receive(...)` in the `run` handler). This decides the exact session-start call in Step 2.

- [ ] **Step 2: Rewrite `steward.ts` as a code dispatcher.**

Replace the file — delete the `TEMPER_SELF_COGMAP_ID` pin and the markdown prompt; keep the `temperToken()` / `requireEnv` helpers (copy the proven pair from `materialize.ts`). The `run` handler:
1. `POST ${TEMPER_API_URL}/api/steward/dispatch` with `{}` (server defaults for threshold + cap).
2. For each `claimed` job, start one session carrying a **single** `cogmap_id` + `job_id`, with a prompt that instructs the model to run the authored-4 over THAT one map (load the map-stewardship skill, open the envelope, distill, then `steward_advance_watermark` — which completes the job).

```ts
async run({ receive, waitUntil }) {
  waitUntil((async () => {
    const apiUrl = requireEnv("TEMPER_API_URL").replace(/\/+$/, "");
    const token = await temperToken();
    const res = await fetch(`${apiUrl}/api/steward/dispatch`, {
      method: "POST",
      headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
      body: "{}",
    });
    if (!res.ok) throw new Error(`dispatch failed: ${res.status} ${await res.text()}`);
    const { claimed } = (await res.json()) as { claimed: { id: string; cogmap_id: string }[] };
    await Promise.all(
      claimed.map((job) =>
        receive(/* the eve/self channel */, {
          message:
            `Run one steward tick over cognitive map ${job.cogmap_id} (job ${job.id}). ` +
            `Pass this SINGLE cogmap id as the \`cogmap\` argument to every temper tool. ` +
            `Load the map-stewardship skill, open the invocation envelope, read the telos, ` +
            `distill new/changed sources with the authored-4 (create/assert/facet/fold), then ` +
            `advance the watermark (which completes this job) and close the envelope.`,
          // auth: appAuth / the app-subject auth mirroring the connection
        }),
      ),
    );
  })());
},
```

(Resolve the exact `receive` channel + `auth` argument from Step 1's reading — mirror the connection's app-subject auth. Do NOT fan out multiple ids into one session: one `receive` per claimed job, one id each.)

- [ ] **Step 3: Rewrite `materialize.ts` to fan out over candidates.**

Replace the single `cogmapId = requireEnv("TEMPER_SELF_COGMAP_ID")` with an enumeration: `GET ${apiUrl}/api/steward/candidates` → `string[]`, then `Promise.all` a self-gating `POST /api/cognitive-maps/${id}/materialize` per id (empty body; the server no-ops below threshold — no lease needed). Keep the `temperToken()` / `requireEnv` helpers.

```ts
async function materializeTick(): Promise<void> {
  const apiUrl = requireEnv("TEMPER_API_URL").replace(/\/+$/, "");
  const token = await temperToken();
  const list = await fetch(`${apiUrl}/api/steward/candidates`, {
    headers: { authorization: `Bearer ${token}` },
  });
  if (!list.ok) throw new Error(`candidates failed: ${list.status} ${await list.text()}`);
  const ids = (await list.json()) as string[];
  await Promise.all(ids.map(async (id) => {
    const res = await fetch(`${apiUrl}/api/cognitive-maps/${id}/materialize`, {
      method: "POST",
      headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
      body: "{}",
    });
    if (!res.ok) throw new Error(`materialize ${id} failed: ${res.status} ${await res.text()}`);
  }));
}
```

- [ ] **Step 4: Typecheck the Eve project.**

```bash
cd packages/agent-workflows/steward && npm install && npx tsc --noEmit
```
Expected: no type errors.

- [ ] **Step 5: Verify instructions.md needs no change.**

Grep `packages/agent-workflows/steward/agent/instructions.md` for any claim that the map id is env-pinned or that there is exactly "one map" at the infra level. The *session* still tends one map (correct), so the authoring persona text is fine; only correct a sentence that asserts the target comes from an env var. If nothing asserts that, make NO change.

- [ ] **Step 6: Commit.**

```bash
git add packages/agent-workflows/steward/agent/schedules/steward.ts \
        packages/agent-workflows/steward/agent/schedules/materialize.ts
# include instructions.md only if Step 5 required an edit
git commit -m "feat(steward): fan-out dispatchers replace the env-pinned single map

steward.ts POSTs /api/steward/dispatch then starts one isolated session per claimed
job (single cogmap id each); materialize.ts fans out over /candidates. Deletes
TEMPER_SELF_COGMAP_ID — the single→multi flip. Goal 019f3220.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 7: Deploy + observe (post-merge, operator step).**

After merge, deploy the steward Eve project; watch one tick under Vercel **Observability → Cron Jobs / Logs**. Confirm: `/dispatch` returns claimed jobs, one session starts per drifted map, `kb_workflow_jobs` rows transition `pending → in_progress → done`, and a killed session's row reaps to `waiting_for_retry` next tick. (This is the spec §12 step 8 live verification.)

---

## Self-Review

**Spec coverage:**
- §5 sweep → Task 2 (`steward_drift_sweep`, LATERAL reuse, `anchor_readable_by_profile` scoping). ✓
- §6 queue → Task 1 (table, 4 fns, single-flight index, lifecycle via tests). ✓
- §7 dispatch flow → Task 3 (composite command, reap→sweep→enqueue→claim) + Task 4 (thin Eve dispatcher, one session per job). ✓
- §7 completion signal → Task 3 Step 2 (watermark-advance → `workflow_job_complete`). ✓
- §8 config → Task 1 constants (`DEFAULT_STEWARD_LEASE_SECONDS`, `DEFAULT_STEWARD_DISPATCH_CAP`) + Task 3 request params. ✓
- §8 env-pin deletion → Task 4. ✓
- §9 backward-compat → additive migrations; scalar `steward_ingest_delta` + single-map endpoints untouched. ✓
- §10 intentional-borrow → Task 1 Step 1 (study-and-port); test scenarios (dedup/claim/complete/reap) → Task 1 Step 4. ✓
- §10 e2e at production caller → Task 3 Step 5. ✓
- Materialize generalization (§4/§7) → Task 2 (`/candidates`) + Task 4 Step 3. ✓

**Refinements vs spec (both strictly better, no scope change):**
1. Sweep **reuses `steward_ingest_delta` via LATERAL** rather than extracting a shared CTE — true function reuse, one source of truth.
2. Sweep scopes through **`anchor_readable_by_profile`** (the profile-scoping fundamental) with the steward app-principal, rather than a system bypass — the "privileged" read is a broadly-granted principal, not an ungated path.

**Placeholder scan:** the one deferred detail is the exact Eve `receive` channel/auth argument (Task 4 Step 2) and the exact e2e harness helpers (Task 3 Step 5) — both are explicit "read X first, mirror the proven pattern" steps with the source named, not open TODOs. The `api_err_from` helper name in Task 3 Step 2 is flagged to reconcile against the file's existing error-conversion helper.

**Type consistency:** `ClaimedJob { id, cogmap_id, attempts }` defined in Task 1, consumed unchanged in Tasks 3–4. `DriftSweepRow` defined Task 2, consumed Task 2 handler. Fn names `enqueue/claim/complete/reap` consistent across service (Task 1) and backend (Task 3). SQL fn signatures in the migrations match the `sqlx::query!` bindings in the services.
