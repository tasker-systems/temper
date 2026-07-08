-- Generalize kb_workflow_jobs from cogmap-scoped to also resource-scoped (issue #299: async embedding).
--
-- The queue (20260705000001) keys a job on (cogmap_id, persona, dispatch_type) with cogmap_id NOT NULL.
-- Embedding jobs are scoped to a RESOURCE, not a cogmap — a context-homed resource has no cogmap — so
-- this migration adds a first-class `resource_id` scope alongside the existing cogmap scope. A resource
-- column (vs the issue's original "carry resource_id in the jsonb payload" sketch) is the better key: it
-- gets an FK (cascade-delete a resource's jobs), a CHECK (exactly one scope), and its own partial-unique
-- in-flight index — the "at most one pending embed per resource" single-flight the supersede-on-update
-- story relies on. The payload column stays dormant; the scope is a typed column, not jsonb.
--
-- ADDITIVE, additive-only-on-`main`: relaxes one NOT NULL (permits more), adds a nullable column, a CHECK
-- satisfied by all existing rows (they have cogmap_id set, resource_id NULL ⇒ exactly one scope), a new
-- partial-unique index, and three new functions. The steward path (cogmap-keyed enqueue/claim/complete)
-- is untouched — the new resource-keyed functions run in parallel.

-- A job is scoped to EITHER a cogmap (steward) OR a resource (embed), never both, never neither.
ALTER TABLE kb_workflow_jobs ALTER COLUMN cogmap_id DROP NOT NULL;
ALTER TABLE kb_workflow_jobs
    ADD COLUMN resource_id uuid REFERENCES kb_resources(id) ON DELETE CASCADE;
ALTER TABLE kb_workflow_jobs
    ADD CONSTRAINT ck_workflow_jobs_one_scope CHECK (num_nonnulls(cogmap_id, resource_id) = 1);

COMMENT ON COLUMN kb_workflow_jobs.resource_id IS
    'Resource scope for resource-keyed jobs (embed). Mutually exclusive with cogmap_id (ck_workflow_jobs_one_scope).';

-- Single-flight for resource-keyed jobs: at most ONE active job per (resource, persona, dispatch_type).
-- The `resource_id IS NOT NULL` predicate keeps cogmap-scoped rows (resource_id NULL) out of this index;
-- they keep their own uq_workflow_jobs_in_flight on (cogmap_id, persona, dispatch_type).
CREATE UNIQUE INDEX uq_workflow_jobs_in_flight_resource
    ON kb_workflow_jobs (resource_id, persona, dispatch_type)
    WHERE resource_id IS NOT NULL
      AND status IN ('pending', 'in_progress', 'waiting_for_retry');

-- Enqueue a resource-keyed job — the resource twin of workflow_job_enqueue. Idempotent within a band:
-- ON CONFLICT DO NOTHING catches the resource in-flight index, so a re-enqueue while a job is active
-- returns no row (caller reads NULL as "already in-flight"). This also gives supersede-on-update for
-- free: a create-then-quick-update re-enqueues and dedups against the still-in-flight embed.
CREATE FUNCTION workflow_job_enqueue_resource(
    p_resource uuid, p_persona text, p_dispatch_type text, p_payload jsonb DEFAULT '{}'::jsonb
) RETURNS uuid LANGUAGE sql AS $$
    INSERT INTO kb_workflow_jobs (resource_id, persona, dispatch_type, payload)
    VALUES (p_resource, p_persona, p_dispatch_type, p_payload)
    ON CONFLICT DO NOTHING
    RETURNING id;
$$;

-- Claim resource-keyed jobs — the resource twin of workflow_job_claim, returning `resource_id` (the
-- scope the worker needs) instead of `cogmap_id`. Same FOR UPDATE SKIP LOCKED claim+flip+increment in
-- one statement, FIFO ordering, lease set. The `resource_id IS NOT NULL` guard makes the returned
-- resource_id non-null and keeps this claim disjoint from the steward (cogmap) claim.
CREATE FUNCTION workflow_job_claim_resource(
    p_persona text, p_dispatch_type text, p_limit int, p_lease_seconds int
) RETURNS TABLE(id uuid, resource_id uuid, attempts int, payload jsonb)
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
            AND c.resource_id IS NOT NULL
            AND c.status IN ('pending', 'waiting_for_retry')
            AND c.next_visible_at <= now()
          ORDER BY c.enqueued_at
          LIMIT p_limit
          FOR UPDATE SKIP LOCKED
     )
    RETURNING j.id, j.resource_id, j.attempts, j.payload;
$$;

-- Complete the ONE active resource-keyed job for the tuple → done (the resource in-flight index
-- guarantees at most one). The resource twin of workflow_job_complete.
CREATE FUNCTION workflow_job_complete_resource(
    p_resource uuid, p_persona text, p_dispatch_type text
) RETURNS uuid LANGUAGE sql AS $$
    UPDATE kb_workflow_jobs
       SET status = 'done', completed_at = now()
     WHERE resource_id = p_resource
       AND persona = p_persona
       AND dispatch_type = p_dispatch_type
       AND status IN ('pending', 'in_progress', 'waiting_for_retry')
    RETURNING id;
$$;
