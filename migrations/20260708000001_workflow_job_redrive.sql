-- Re-drive dead resource-keyed jobs (issue #299, Phase 4). A `dead` embed job (reaper-exhausted its
-- retry budget) leaves its resource FTS-only — chunk text landed but the vector never did. Re-drive is
-- the recovery path: re-enqueue a fresh embed job per resource that has a dead job, so the next
-- dispatch tick claims and embeds it. Operator-triggered (`/api/embed/dispatch?redrive=true`), not an
-- automatic reaper cadence — a persistently-failing resource stays observably `dead` until asked,
-- rather than churning in a dead→redrive→dead loop.
--
-- Re-enqueue (a new pending row) rather than resurrecting the dead row in place: the dead rows stay as
-- an accountability trail, and a fresh INSERT reuses the exact single-flight semantics of
-- workflow_job_enqueue_resource — ON CONFLICT DO NOTHING against uq_workflow_jobs_in_flight_resource, so
-- a resource that already has a live job is skipped (no duplicate active job). SELECT DISTINCT collapses
-- multiple dead rows for one resource to a single re-enqueue. Schema-agnostic like its sibling queue
-- primitives: it re-enqueues by job state alone; the embed itself is idempotent (a resource with no
-- NULL-embedding current chunks embeds zero and completes cleanly), so re-driving an already-repaired
-- resource is a cheap no-op rather than a correctness hazard.
--
-- ADDITIVE, additive-only-on-`main`: one new function; no existing object altered.

CREATE FUNCTION workflow_job_redrive_resource(
    p_persona text, p_dispatch_type text, p_limit int
) RETURNS TABLE(id uuid, resource_id uuid) LANGUAGE sql AS $$
    INSERT INTO kb_workflow_jobs (resource_id, persona, dispatch_type)
    SELECT d.resource_id, p_persona, p_dispatch_type
      FROM (
          SELECT DISTINCT j.resource_id
            FROM kb_workflow_jobs j
           WHERE j.persona = p_persona
             AND j.dispatch_type = p_dispatch_type
             AND j.resource_id IS NOT NULL
             AND j.status = 'dead'
           ORDER BY j.resource_id
           LIMIT p_limit
      ) d
    ON CONFLICT DO NOTHING
    RETURNING id, resource_id;
$$;
