-- Steward tick correlation, from the cron header to the ledger (task 019f4be3 — P4 of the temper-rb goal).
--
-- The fast-follow spec'd in docs/superpowers/specs/2026-07-06-steward-dispatch-correlation-id-design.md
-- ("Fast-follow (spec'd, deferred to a follow-up task)"). P3 (20260709000050) made
-- `kb_events.correlation_id` caller-settable; the steward already mints a per-tick id and sends it as
-- `x-steward-correlation-id`, but only into logs. This threads it into data.
--
-- ── The grain decision (the task's open question) ────────────────────────────
-- One steward tick is ONE DISPATCH ACT PLUS N RUN-GRAIN SESSIONS — not one act.
--
-- `/api/steward/dispatch` fires ZERO kb_events: reap→sweep→enqueue→claim touch only kb_workflow_jobs
-- (a queue), and drift_sweep is a pure read. So there are no "dispatch ledger writes" to share a
-- correlation_id; the tick's durable trace is the claimed JOB rows. Each claimed job then spawns one
-- agent session, whose writes are already correlated at run grain by `kb_events.invocation_id`. That
-- is the right grain for "an agent working session" — and `kb_events.correlation_id` is reserved for
-- act grain ("groups a multi-event act", canonical_schema:472), e.g. a block's event stream.
--
-- So we thread the tick id down to the invocation and STOP. An event joins to its tick through its
-- invocation, not by carrying the tick id directly:
--
--     kb_events.invocation_id → kb_invocations.correlation_id = the tick
--
-- Collapsing the two — stamping the tick id onto every session event's correlation_id — would make
-- act grain and run grain indistinguishable for every agent-authored write. It buys a one-hop join and
-- destroys a distinction the ledger already asserts. We keep both grains.
--
-- ── Why this needs nothing from the model ────────────────────────────────────
-- The claimed job row is the carrier. `invocation_open` reads the active job for its cogmap and
-- inherits that job's correlation_id server-side. This is deliberately NOT a new `correlation_id`
-- argument on `invocation_open`: a model-passed correlator per tool call is the fragile path a weaker
-- model silently skips. Nothing is asked of the agent.
--
-- Correlation is a correlation aid, NEVER authorization. Nothing gates on it. An absent or unparseable
-- `x-steward-correlation-id` yields NULL and self-roots, byte-identical to today.

-- ── Columns ─────────────────────────────────────────────────────────────────
ALTER TABLE kb_workflow_jobs ADD COLUMN correlation_id uuid;
ALTER TABLE kb_invocations   ADD COLUMN correlation_id uuid;

COMMENT ON COLUMN kb_workflow_jobs.correlation_id IS
    'The dispatch tick that CLAIMED this job (x-steward-correlation-id). Set at claim, unconditionally '
    '— a re-claim after a reap belongs to the new tick, not the one that lost its lease. NULL when the '
    'claimer sent no header. Provenance only; nothing gates on it.';
COMMENT ON COLUMN kb_invocations.correlation_id IS
    'The dispatch tick this run was spawned by, inherited server-side at invocation_open from the '
    'active claimed job for the originating cogmap. NULL for a manual open with no active job — '
    'correct, since there is no tick to correlate to. Join an act to its tick via '
    'kb_events.invocation_id → here.';

-- The tick→runs lookup ("show me every session this tick spawned").
CREATE INDEX idx_kb_invocations_correlation ON kb_invocations(correlation_id);

-- ── Claim stamps the tick ───────────────────────────────────────────────────
-- Adding a parameter changes a function's identity, so CREATE OR REPLACE would leave a second,
-- ambiguous overload callable; DROP + CREATE truly replaces (same mechanics as 20260709000050).
-- `p_correlation` is appended LAST with a DEFAULT, so the deployed 4-arg positional call site
-- (`workflow_job_claim($1,$2,$3,$4)`) keeps resolving during the migrate→deploy window. Body is
-- otherwise verbatim from 20260705000001:68-89. No view, trigger, or SQL function calls it.
DROP FUNCTION workflow_job_claim(text, text, int, int);
CREATE FUNCTION workflow_job_claim(
    p_persona text, p_dispatch_type text, p_limit int, p_lease_seconds int,
    p_correlation uuid DEFAULT NULL
) RETURNS TABLE(id uuid, cogmap_id uuid, attempts int, payload jsonb)
LANGUAGE sql AS $$
    UPDATE kb_workflow_jobs j
       SET status = 'in_progress',
           leased_at = now(),
           lease_expires_at = now() + make_interval(secs => p_lease_seconds),
           attempts = j.attempts + 1,
           correlation_id = p_correlation
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

-- ── invocation_open inherits the tick from its claimed job ──────────────────
-- Signature unchanged → CREATE OR REPLACE, and no Rust call site moves. Body verbatim from
-- 20260624000002:1227-1242 with the correlation lookup added and forwarded to the event sink.
--
-- The lookup: the single-flight index guarantees at most one active job per
-- (cogmap_id, persona, dispatch_type), but a cogmap could in principle hold active jobs for several
-- personas at once, so this takes the most recently leased one. A manual open (no active job) leaves
-- v_corr NULL → `_event_append`'s COALESCE(p_correlation, v_ev) self-roots the event exactly as before.
CREATE OR REPLACE FUNCTION invocation_open(p_payload jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_inv uuid := (p_payload->>'invocation_id')::uuid;
        v_orig uuid := (p_payload->>'originating_cogmap_id')::uuid;
        v_parent uuid := (p_payload->>'parent_cogmap_id')::uuid;
        v_corr uuid;
        v_ev uuid;
BEGIN
    IF v_parent IS NOT NULL AND NOT cogmaps_share_a_team(v_parent, v_orig) THEN
        RAISE EXCEPTION 'delegation gate: cogmaps % and % share no team', v_parent, v_orig;
    END IF;
    SELECT j.correlation_id INTO v_corr
      FROM kb_workflow_jobs j
     WHERE j.cogmap_id = v_orig
       AND j.status = 'in_progress'
       AND j.correlation_id IS NOT NULL
     ORDER BY j.leased_at DESC
     LIMIT 1;
    v_ev := _event_append('delegated_launch', p_emitter, 'kb_cogmaps', v_orig, p_payload,
                          p_invocation => v_inv, p_correlation => v_corr);
    PERFORM _project_delegated_launch(v_ev, p_payload);
    RETURN v_inv;
END;
$$;

-- ── The projection reads the tick off the EVENT, never the job table ────────
-- Replay-stable by construction: this rebuilds kb_invocations.correlation_id from the ledger alone
-- (replay.rs:378 replays `_project_delegated_launch($1,$2)` — signature unchanged). Reading the job
-- table here instead would make replay depend on mutable queue state and reproduce a different value.
--
-- `_event_append` self-roots an uncorrelated event to its OWN id, so `correlation_id = p_event` means
-- "no tick"; NULLIF maps that back to NULL. A real tick id is a freshly minted v4 from the cron and
-- cannot collide with the event's v7 id.
CREATE OR REPLACE FUNCTION _project_delegated_launch(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_occurred timestamptz;
        v_corr uuid;
BEGIN
    SELECT occurred_at, NULLIF(correlation_id, p_event)
      INTO v_occurred, v_corr
      FROM kb_events WHERE id = p_event;

    INSERT INTO kb_invocations (id, opened_by_event_id, status, trigger_kind,
        originating_cogmap_id, parent_cogmap_id, scoped_entity_id, telos_resource_id, opened_at,
        correlation_id)
    SELECT (p_payload->>'invocation_id')::uuid, p_event, 'open', p_payload->>'trigger_kind',
           (p_payload->>'originating_cogmap_id')::uuid, (p_payload->>'parent_cogmap_id')::uuid,
           (p_payload->>'scoped_entity_id')::uuid, c.telos_resource_id, v_occurred,
           v_corr
    FROM kb_cogmaps c WHERE c.id = (p_payload->>'originating_cogmap_id')::uuid;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'delegated_launch: originating cogmap % not found',
            (p_payload->>'originating_cogmap_id')::uuid;
    END IF;
END;
$$;
