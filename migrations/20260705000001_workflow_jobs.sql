-- Persona-agnostic durable job queue for agent dispatch (steward fan-out; goal 019f3220).
--
-- A single hand-rolled table + four SQL primitives, borrowing the sibling tasker-core's DURABLE-state
-- queue mechanics (SKIP LOCKED claim, attempts/max_attempts gating, lease-expiry reaping,
-- partial-unique in-flight dedup) while skipping its pgmq transport + DAG/DLQ apparatus. See
-- docs/superpowers/specs/2026-07-05-steward-fan-out-drift-sweep-design.md §6.
--
-- Deliberate scale-adaptations vs tasker-core (single table, no separate transitions table):
--   * Claim is one batch UPDATE whose locking subquery does FOR UPDATE SKIP LOCKED — claim+flip in a
--     single statement (tasker-core splits claim-by-SELECT from a separate single-row CAS because it
--     carries a transitions state machine; we don't). The subquery's `status IN (...)` filter is the
--     from-state CAS guard; SKIP LOCKED prevents double-claim.
--   * Reaper keys on a real `lease_expires_at < now()` visibility timeout (tasker-core uses a
--     time-in-state threshold as it has no lease column). `WHERE status = 'in_progress'` is the
--     anti-join — terminal rows (done/dead/waiting_for_retry) are already excluded.
--
-- ADDITIVE, additive-only-on-`main`: a new table + four new functions; no existing object altered.

CREATE TABLE kb_workflow_jobs (
    id               uuid PRIMARY KEY DEFAULT uuid_generate_v7(),
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

-- Single-flight: at most ONE active job per (cogmap, persona, dispatch_type). This is the
-- "idempotent within a band" guarantee — a re-enqueue while a job is active is a no-op.
CREATE UNIQUE INDEX uq_workflow_jobs_in_flight
    ON kb_workflow_jobs (cogmap_id, persona, dispatch_type)
    WHERE status IN ('pending', 'in_progress', 'waiting_for_retry');

-- Claim scan support (persona/dispatch_type filter + claimable ordering).
CREATE INDEX idx_workflow_jobs_claimable
    ON kb_workflow_jobs (persona, dispatch_type, next_visible_at)
    WHERE status IN ('pending', 'waiting_for_retry');

-- Enqueue: idempotent within a band. `ON CONFLICT DO NOTHING` (no explicit arbiter needed for
-- DO NOTHING) catches the partial-unique in-flight index, so a re-enqueue returns no row. Wrapping
-- the INSERT in a scalar-returning SQL function turns that empty result into a NULL return, which the
-- caller reads as `None` ("already in-flight").
CREATE FUNCTION workflow_job_enqueue(
    p_cogmap uuid, p_persona text, p_dispatch_type text, p_payload jsonb DEFAULT '{}'::jsonb
) RETURNS uuid LANGUAGE sql AS $$
    INSERT INTO kb_workflow_jobs (cogmap_id, persona, dispatch_type, payload)
    VALUES (p_cogmap, p_persona, p_dispatch_type, p_payload)
    ON CONFLICT DO NOTHING
    RETURNING id;
$$;

-- Claim: FOR UPDATE SKIP LOCKED over claimable rows, oldest-first (FIFO gives cross-tick fairness);
-- flip to in_progress, set the lease, and increment attempts in the same statement (the increment
-- lives in SQL, not app code — avoiding tasker-core's noted app-side-increment gotcha).
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

-- Complete: transition the ONE active job for the tuple → done (the in-flight index guarantees at
-- most one). Called on clean run completion (see advance_steward_watermark) so the watermark advance
-- and the job completion are one atomic "finished this map cleanly" act.
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

-- Reap: expired-lease in_progress jobs → waiting_for_retry, or dead once attempts have reached
-- max_attempts (attempts was already incremented at claim). Returns the count reaped. The
-- `status = 'in_progress'` filter is the anti-join (terminal rows excluded); SKIP LOCKED lets
-- concurrent reapers coexist without double-processing.
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
