-- The sweep's door (sensitivity-sweep spec D8, D9; Q45, Q46; build order 3a PR D). The tick returns this
-- run's own facts for the span, as integers only: nothing in the signature can carry text.
-- Rationale: temper-artifacts plans/2026-10-01-sensitivity-sweep-3a-core.md, "PR D".

-- Q49: an idle tick leaves no rows, so the rotation cannot be read off the runs any more. Each
-- surface carries when a tick last visited it, and the claim rotates by that.
ALTER TABLE sensitivity.surfaces ADD COLUMN last_swept_at timestamptz;

-- Q46: the door loops claim → tick inside one call, and stops after a rotation that found nothing.
-- The claim also returns how many surfaces a rotation visits, and rotates by last_swept_at (Q49).
-- C2's body otherwise, verbatim.
DROP FUNCTION sensitivity_sweep_claim(int, int);

CREATE FUNCTION sensitivity_sweep_claim(p_budget_rows int DEFAULT 2000, p_lease_seconds int DEFAULT 330)
RETURNS TABLE (run_id uuid, job_id uuid, surfaces int)
LANGUAGE plpgsql AS $$
DECLARE
    v_pick text;
    v_job  record;
BEGIN
    SELECT s.surface INTO v_pick FROM sensitivity.surfaces s
     WHERE s.enabled
     ORDER BY s.last_swept_at NULLS FIRST, s.surface
     LIMIT 1;
    IF v_pick IS NOT NULL THEN
        PERFORM workflow_job_enqueue_system('sensitivity', 'sensitivity-sweep',
            jsonb_build_object('surface', v_pick, 'budget', p_budget_rows));
    END IF;
    SELECT c.id, c.attempts, c.payload INTO v_job
      FROM workflow_job_claim_system('sensitivity', 'sensitivity-sweep', 1, p_lease_seconds) c;
    IF v_job.id IS NULL THEN
        RETURN;
    END IF;
    INSERT INTO sensitivity.runs (surface, job_id, attempt)
    VALUES (v_job.payload ->> 'surface', v_job.id, v_job.attempts) RETURNING id INTO run_id;
    job_id := v_job.id;
    -- Q46: how many surfaces one rotation visits, so a door that loops can stop after an idle one.
    SELECT count(*)::int INTO surfaces FROM sensitivity.surfaces s WHERE s.enabled;
    RETURN NEXT;
END;
$$;

-- The return type changes, so the function is dropped and created. No deployed binary calls it:
-- this PR's door is its first caller.
DROP FUNCTION sensitivity_sweep_tick(uuid, uuid, bytea, interval, int);

-- 20261003150000's body, verbatim apart from the return and Q49's tail.
CREATE FUNCTION sensitivity_sweep_tick(
    p_run uuid, p_job uuid, p_salt bytea, p_lag interval DEFAULT '5 minutes', p_budget_ms int DEFAULT 20000
) RETURNS TABLE (rows_examined int, hashes_examined int, cache_hits int, new_findings int,
                 cursor_advances int, units_oversize int, failed boolean,
                 outcome smallint, failure smallint,
                 new_findings_head int, new_findings_backfill int, head_holdback_seconds int,
                 sev1 int, sev2 int, sev3 int, sev4 int)
LANGUAGE plpgsql AS $$
DECLARE
    v_job      record;
    v_surface  sensitivity.surfaces;
    v_deadline timestamptz := clock_timestamp() + make_interval(secs => p_budget_ms / 1000.0);
    v_bound_at timestamptz;
    v_bound_id uuid;
    v_budget   int;
    v_head     int;
    v_code     text;
BEGIN
    SELECT j.payload INTO v_job FROM kb_workflow_jobs j
      JOIN sensitivity.runs r ON r.id = p_run AND r.job_id = j.id AND r.attempt = j.attempts
                             AND r.outcome IS NULL AND r.surface = j.payload ->> 'surface'
     WHERE j.id = p_job AND j.persona = 'sensitivity' AND j.status = 'in_progress'
       AND j.lease_expires_at > now();
    IF NOT FOUND THEN
        RETURN;
    END IF;
    SELECT s.* INTO v_surface FROM sensitivity.surfaces s WHERE s.surface = v_job.payload ->> 'surface';
    IF p_salt IS NULL OR octet_length(p_salt) < 16 THEN
        v_code := 'salt_missing';
    ELSIF NOT v_surface.enabled
       OR to_regclass(format('sensitivity.%I', 'src_' || replace(v_surface.surface, '.', '__'))) IS NULL THEN
        UPDATE sensitivity.runs SET outcome = 'idle', finished_at = clock_timestamp() WHERE id = p_run;
    ELSE
        -- D4 rule 1: never past now() - lag, nor past the oldest transaction open here. Any open
        -- transaction may yet commit a row stamped before it wrote (Witness 5); the hold is
        -- recorded, so a stalled head never reads as quiet.
        SELECT least(now(), coalesce(min(a.xact_start), now())) - p_lag INTO v_bound_at
          FROM pg_stat_activity a
         WHERE a.datname = current_database() AND a.pid <> pg_backend_pid() AND a.xact_start IS NOT NULL;
        v_bound_id := sensitivity.v7_floor(v_bound_at);
        UPDATE sensitivity.runs
           SET head_holdback_seconds = least(ceil(extract(epoch FROM (now() - p_lag) - v_bound_at)), 100000)
         WHERE id = p_run;
        BEGIN
            -- A new detector version starts both lanes at the bound: its head goes forward, its
            -- backfill goes down from the same point (D4 rule 2).
            INSERT INTO sensitivity.cursors (surface, cursor_kind, detector_id, detector_version, lane,
                watermark_at, watermark_id, backfill_floor_at, backfill_floor_id)
            SELECT v_surface.surface, v_surface.cursor_kind, d.id, d.version, l.lane,
                   CASE WHEN l.lane = 'head' AND v_surface.cursor_kind = 'mutable_timestamp' THEN v_bound_at END,
                   CASE WHEN l.lane = 'head' THEN m.floor_id END,
                   CASE WHEN l.lane = 'backfill' AND v_surface.cursor_kind = 'mutable_timestamp' THEN v_bound_at END,
                   CASE WHEN l.lane = 'backfill' THEN m.floor_id END
              FROM sensitivity.detectors d
             CROSS JOIN (VALUES ('head'), ('backfill')) l(lane)
             CROSS JOIN (SELECT CASE WHEN v_surface.cursor_kind = 'mutable_timestamp'
                                     THEN 'ffffffff-ffff-ffff-ffff-ffffffffffff'::uuid ELSE v_bound_id END) m(floor_id)
             WHERE d.enabled
            ON CONFLICT DO NOTHING;
            -- The backfill keeps half the budget while it has history left (D4: its own budget).
            v_budget := least((v_job.payload ->> 'budget')::int, 100000);
            v_head := CASE WHEN EXISTS (
                          SELECT 1 FROM sensitivity.cursors c
                            JOIN sensitivity.detectors d ON d.id = c.detector_id AND d.version = c.detector_version
                           WHERE c.surface = v_surface.surface AND c.lane = 'backfill' AND d.enabled
                             AND c.backfill_completed_at IS NULL)
                      THEN v_budget / 2 ELSE v_budget END;
            v_head := sensitivity.scan_lane(p_run, v_surface, 'head', p_salt, v_bound_at, v_bound_id,
                                            v_head, v_deadline);
            PERFORM sensitivity.scan_lane(p_run, v_surface, 'backfill', p_salt, v_bound_at, v_bound_id,
                                          v_budget - v_head, v_deadline);
            UPDATE sensitivity.runs SET outcome = 'scanned', finished_at = clock_timestamp() WHERE id = p_run;
        EXCEPTION
            -- Never SQLERRM: a message can quote the row that raised it (D8 constraint 2). OTHERS
            -- does not match query_canceled, so the timeout is caught by name.
            WHEN query_canceled THEN
                v_code := 'statement_timeout';
            WHEN OTHERS THEN
                v_code := CASE WHEN SQLSTATE = '2201B' THEN 'detector_pattern_invalid'
                               WHEN SQLSTATE LIKE '23%' THEN 'store_constraint'
                               ELSE 'scan_failed' END;
        END;
    END IF;
    failed := v_code IS NOT NULL;
    IF failed THEN
        UPDATE sensitivity.runs SET outcome = 'failed', finished_at = clock_timestamp() WHERE id = p_run;
        PERFORM workflow_job_fail_system(p_job, 'sensitivity', 'sensitivity-sweep', v_code);
    ELSE
        PERFORM workflow_job_complete_system(p_job, 'sensitivity', 'sensitivity-sweep');
    END IF;
    -- Q45: the outcome and the failure are numbered, so the signature stays free of text (D8).
    -- Their codes are mirrored in Rust (sensitivity_sweep_service), and a test holds the two equal.
    failure := CASE v_code WHEN 'salt_missing' THEN 1 WHEN 'statement_timeout' THEN 2
                           WHEN 'detector_pattern_invalid' THEN 3 WHEN 'store_constraint' THEN 4
                           WHEN 'scan_failed' THEN 5 END;
    SELECT r.rows_examined, r.hashes_examined, r.cache_hits, r.new_findings, r.cursor_advances, r.units_oversize,
           CASE r.outcome WHEN 'scanned' THEN 1 WHEN 'idle' THEN 2 WHEN 'failed' THEN 3 END,
           r.new_findings_head, r.new_findings_backfill, r.head_holdback_seconds,
           coalesce((r.by_severity ->> '1')::int, 0), coalesce((r.by_severity ->> '2')::int, 0),
           coalesce((r.by_severity ->> '3')::int, 0), coalesce((r.by_severity ->> '4')::int, 0)
      INTO rows_examined, hashes_examined, cache_hits, new_findings, cursor_advances, units_oversize,
           outcome, new_findings_head, new_findings_backfill, head_holdback_seconds,
           sev1, sev2, sev3, sev4
      FROM sensitivity.runs r WHERE r.id = p_run;
    -- Q49: the visit is recorded on the surface, whatever the tick found. A tick that changed
    -- nothing and saw nothing worth keeping leaves no rows: not its run, not its finished job. A
    -- held-back head is kept, because a stalled head is a signal, not quiet.
    UPDATE sensitivity.surfaces SET last_swept_at = clock_timestamp()
     WHERE surface = v_job.payload ->> 'surface';
    IF NOT failed AND rows_examined = 0 AND hashes_examined = 0 AND new_findings = 0
       AND cursor_advances = 0 AND units_oversize = 0 AND head_holdback_seconds = 0 THEN
        DELETE FROM sensitivity.runs WHERE id = p_run;
        DELETE FROM kb_workflow_jobs WHERE id = p_job AND persona = 'sensitivity' AND status = 'done';
    END IF;
    RETURN NEXT;
END;
$$;

SELECT declare_migration(
    20261003230000,
    'additive',
    'The sensitivity sweep''s door (spec D8, D9; Q45, Q46). DROP + CREATE of sensitivity_sweep_claim with the same parameters, returning surfaces (the enabled-surface count) after run_id and job_id; its body is 20261003150000''s plus that one read. DROP + CREATE of sensitivity_sweep_tick with the same parameters and a wider RETURNS TABLE: the seven prior columns keep their names, types and order, followed by outcome, failure, new_findings_head, new_findings_backfill, head_holdback_seconds and sev1-sev4, all integers. The body is 20261003150000''s verbatim except the final read and Q49''s tail: it stamps sensitivity.surfaces.last_swept_at (a new nullable column the claim now rotates by), and a tick that examined, found, advanced and held back nothing deletes its own run row and its finished job row. No deployed binary calls either function: the door that calls them ships with this migration. The one schema change is that nullable column; no constraint or grant changes.'
);
