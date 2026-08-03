-- Generalize kb_workflow_jobs from (cogmap | resource) scope to (cogmap | resource | context) scope —
-- the ANCHOR scope the region clocks need, so region materialization can leave the request path.
--
-- Goal 019fc46c-5731-75d2-b00f-891069ce6f31, clause `a-write-returns-without-waiting-on-projection`.
--
-- ── Why a context column, and not a polymorphic (anchor_table, anchor_id) pair ──────────────────
-- `kb_events` addresses an anchor polymorphically (producing_anchor_table/_id, no FK) and that is
-- right THERE: kb_events is append-only by design — there is no delete mechanism and must not be — so
-- the FK a polymorphic pair forfeits would protect nothing. `kb_workflow_jobs` rows DO get deleted:
-- both existing scopes carry `ON DELETE CASCADE` precisely so a deleted cogmap or resource takes its
-- queued jobs with it. Copying the events shape here would silently drop that, leaving jobs pointing
-- at anchors that no longer exist. So the typed column earns its place for the same reason
-- 20260707000001 gave it to `resource_id`: FK, cascade, CHECK, and its own partial-unique index.
--
-- ── Why the region clocks need an anchor scope at all ───────────────────────────────────────────
-- `region_clocks::tick` is keyed on a `HomeAnchor` — context OR cogmap. `cogmap_id` already covers
-- half of that; contexts had no representable scope, so a context-anchored job could not be enqueued
-- at all. Two of the four anchors carrying production materialization load are contexts, so a
-- cogmap-only detachment would have left half the load on the request path.
--
-- ADDITIVE, additive-only-on-`main`. Every element permits strictly more than the current schema and
-- no existing reader or writer changes behaviour:
--   * a new nullable column with an FK — invisible to a binary that never sets it;
--   * the one-scope CHECK is WIDENED (2 columns → 3). Every existing row has exactly one of
--     (cogmap_id, resource_id) set and therefore satisfies the 3-column form unchanged; a widened
--     CHECK accepts a strict superset of what the current one accepts, so no in-flight write can be
--     rejected by it. Same argument 20260707000001 made for dropping a NOT NULL;
--   * a new partial-unique index over rows the old binary never creates (context_id IS NOT NULL);
--   * three new functions. The steward (cogmap) and embed (resource) paths are untouched.
-- A binary predating this migration ignores the column entirely and keeps working.

-- A job is scoped to EXACTLY ONE of: a cogmap (steward), a resource (embed), a context (region).
ALTER TABLE kb_workflow_jobs
    ADD COLUMN context_id uuid REFERENCES kb_contexts(id) ON DELETE CASCADE;

COMMENT ON COLUMN kb_workflow_jobs.context_id IS
    'Context scope for context-anchored jobs (region materialization). Mutually exclusive with '
    'cogmap_id and resource_id (ck_workflow_jobs_one_scope).';

ALTER TABLE kb_workflow_jobs DROP CONSTRAINT ck_workflow_jobs_one_scope;
ALTER TABLE kb_workflow_jobs
    ADD CONSTRAINT ck_workflow_jobs_one_scope
    CHECK (num_nonnulls(cogmap_id, resource_id, context_id) = 1);

-- Single-flight for context-anchored jobs: at most ONE active job per
-- (context_id, persona, dispatch_type). The twin of uq_workflow_jobs_in_flight (cogmap) and
-- uq_workflow_jobs_in_flight_resource. The `context_id IS NOT NULL` predicate keeps the other two
-- scopes out of this index; they keep their own.
--
-- This index is what makes N settling-worthy arrivals on one context collapse to ONE job — the
-- enqueue below is ON CONFLICT DO NOTHING, so a re-enqueue while a job is active is a no-op.
CREATE UNIQUE INDEX uq_workflow_jobs_in_flight_context
    ON kb_workflow_jobs (context_id, persona, dispatch_type)
    WHERE context_id IS NOT NULL
      AND status IN ('pending', 'in_progress', 'waiting_for_retry');

-- ── Anchor-scoped enqueue / claim / complete ────────────────────────────────────────────────────
-- These take the anchor as a (cogmap, context) pair with exactly one non-null, mirroring HomeAnchor's
-- closed two-variant shape. One function pair rather than two keeps the caller from branching on
-- anchor kind at every call site; the CHECK constraint enforces the exclusivity the pair implies.

-- Enqueue: idempotent within a band, exactly as the cogmap and resource twins are. Returns NULL when
-- a job for this (anchor, persona, dispatch_type) is already in flight — the caller reads that as
-- "already queued", never as an error.
CREATE FUNCTION workflow_job_enqueue_anchor(
    p_cogmap uuid, p_context uuid, p_persona text, p_dispatch_type text,
    p_payload jsonb DEFAULT '{}'::jsonb
) RETURNS uuid LANGUAGE sql AS $$
    INSERT INTO kb_workflow_jobs (cogmap_id, context_id, persona, dispatch_type, payload)
    VALUES (p_cogmap, p_context, p_persona, p_dispatch_type, p_payload)
    ON CONFLICT DO NOTHING
    RETURNING id;
$$;

-- Claim anchor-keyed jobs — the anchor twin of workflow_job_claim / workflow_job_claim_resource,
-- returning BOTH scope columns so the worker can rebuild the HomeAnchor. Same FOR UPDATE SKIP LOCKED
-- claim+flip+increment in one statement, FIFO ordering, lease set.
--
-- The `num_nonnulls(cogmap_id, context_id) = 1` guard is what keeps this claim disjoint from the
-- embed (resource) claim. It does NOT need to be disjoint from the steward claim by scope — both are
-- cogmap-shaped — and is not: persona/dispatch_type separate them, which is the same mechanism that
-- lets an auditor job and a steward job coexist over one cogmap (see Persona::Auditor).
CREATE FUNCTION workflow_job_claim_anchor(
    p_persona text, p_dispatch_type text, p_limit int, p_lease_seconds int
) RETURNS TABLE(id uuid, cogmap_id uuid, context_id uuid, attempts int, payload jsonb)
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
            AND num_nonnulls(c.cogmap_id, c.context_id) = 1
            AND c.status IN ('pending', 'waiting_for_retry')
            AND c.next_visible_at <= now()
          ORDER BY c.enqueued_at
          LIMIT p_limit
          FOR UPDATE SKIP LOCKED
     )
    RETURNING j.id, j.cogmap_id, j.context_id, j.attempts, j.payload;
$$;

-- Complete the ONE active anchor-keyed job for the tuple → done. `IS NOT DISTINCT FROM` (not `=`)
-- because one of the two scope columns is always NULL and `NULL = NULL` is NULL, which would match
-- no row and leave every completed job in flight until its lease expired.
CREATE FUNCTION workflow_job_complete_anchor(
    p_cogmap uuid, p_context uuid, p_persona text, p_dispatch_type text
) RETURNS uuid LANGUAGE sql AS $$
    UPDATE kb_workflow_jobs
       SET status = 'done', completed_at = now()
     WHERE cogmap_id IS NOT DISTINCT FROM p_cogmap
       AND context_id IS NOT DISTINCT FROM p_context
       AND persona = p_persona
       AND dispatch_type = p_dispatch_type
       AND status IN ('pending', 'in_progress', 'waiting_for_retry')
    RETURNING id;
$$;

SELECT declare_migration(
    20260802000020,
    'additive',
    'kb_workflow_jobs gains a nullable context_id scope (FK kb_contexts, ON DELETE CASCADE), the one-scope CHECK widens from 2 to 3 columns, a partial-unique in-flight index on (context_id, persona, dispatch_type), and three anchor-scoped functions (enqueue/claim/complete). Additive: the widened CHECK accepts a strict superset of the current one and every existing row already satisfies it; the new column and index are invisible to a binary that never sets context_id; the steward (cogmap) and embed (resource) paths are untouched. Enables region materialization to leave the request path (goal 019fc46c).'
);
