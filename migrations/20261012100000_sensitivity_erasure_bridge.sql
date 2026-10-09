-- The sweep-to-erasure bridge (build order 3c): what the erasure surveys read from the sweep.
--
-- Resource erasure spec D10, D11 and its *Rulings from build order 3c* (R1-R4, P5); sensitivity
-- sweep spec D2, D6, D11, Q22, Q34, Q50 and *Where this meets the erasure spec*. Task
-- 01a0e9e8-3acd-7eb2-b729-57cfcba3df04.
--
-- Two survey questions, answered from stored findings only:
--
--   * Does a deriver R's survey names (D8) quote one of R's detected values? A deriver reads
--     `yes`, `no`, `unscanned` or `expired` (D10 as amended by R1). The comparison is over
--     fingerprints already stored: no salt is read, and nothing but ids and states is returned.
--   * Does a block's current revision still hold an open finding (D11's scrub warning)? Since Q34
--     every stored hash is keyed by the salt, so this reads by place (Q22), never by hash (R4).
--
-- No Rust names this schema (sweep witness 12), so each question has a public wrapper the survey
-- service calls. The acts call the plans, never the wrappers: neither value can reach an act. A
-- wrapper whose sensitivity function is absent answers as a sweep that read nothing would.
--
-- Section 1. sensitivity.countable: a finding not ruled false_positive (R3, sweep D6).
-- Section 2. sensitivity.resource_covered: has the sweep read all of a resource (R2).
-- Section 3. sensitivity.deriver_fingerprint_matches (D10; R1, R3; P1, P2).
-- Section 4. sensitivity.block_current_revision_flagged (D11; R4; P6).
-- Section 5. The public wrappers.

-- ---------------------------------------------------------------------------
-- Section 1. A finding counts toward a survey answer unless its current disposition is
-- false_positive. Dispositions are append-only (sweep D6); the current one is the latest in the
-- order the dispositions_latest index keeps (20261002200000). `acknowledged` and `accepted_risk`
-- still count: the value is real.
-- ---------------------------------------------------------------------------
CREATE FUNCTION sensitivity.countable(p_finding uuid) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT coalesce((SELECT d.state FROM sensitivity.dispositions d
                      WHERE d.finding_id = p_finding
                      ORDER BY d.decided_at DESC, d.id DESC
                      LIMIT 1) <> 'false_positive', true);
$$;

COMMENT ON FUNCTION sensitivity.countable(uuid) IS
'Whether a finding counts toward the erasure surveys'' answers: true unless its latest disposition (by decided_at, then id, as dispositions_latest orders them) is false_positive. A value ruled benign is not brought back by a match (erasure rulings from build order 3c, R3).';

-- ---------------------------------------------------------------------------
-- Section 2. Coverage: the stored findings speak for a resource only when the sweep has read all
-- of it. "Off", "detector disabled" and "never scanned" all fail this the same way, and SQL cannot
-- tell them apart: the deployment's opt-in is an environment variable (R2, Q53).
--
-- This is a reading of sensitivity.scan_lane (20261003150000) and of cursor creation
-- (20261003230000), and must change with them. A detector version's two lanes both start at the
-- tick's bound: the head reads keys above its watermark, upward; the backfill reads from its floor
-- (inclusive) or from at or below its watermark, downward, and sets backfill_completed_at at the
-- tail. A key is (coalesce(order_at, '-infinity'), target_id), order_at being NULL on an append-only
-- surface. So a key is covered by a version when it is at or below the head's watermark and above
-- the backfill's floor, or the backfill has completed, or it is at or above the backfill's
-- watermark. Each lane is monotone, so a resource's keys on a surface are covered exactly when its
-- least and greatest are.
--
-- Coverage is per detector id, by any of its versions (P3): a bump disables the old version and
-- its cursors stop, and its fingerprints stay comparable, since minting reads no version.
--
-- A resource's places are its rows in each source view (`src_<surface>`, as scan_lane names
-- them). The ledger's views attribute every row by computation, so its places come from the indexed
-- trail scope instead (_resource_erasure_trail_scope, 20261009100000), a superset of
-- event_resource's attribution: over-reading coverage only errs toward `unscanned`. A place the
-- sweep passed whole (unscanned_places) is never covered. An empty detector set is not covered.
-- ---------------------------------------------------------------------------
CREATE FUNCTION sensitivity.resource_covered(p_resource uuid, p_detectors text[]) RETURNS boolean
LANGUAGE plpgsql STABLE AS $$
DECLARE
    v_surface sensitivity.surfaces;
    v_ledger  boolean;
    v_view    text;
    v_lo_at   timestamptz; v_lo_id uuid;
    v_hi_at   timestamptz; v_hi_id uuid;
    v_skipped boolean;
    v_det     text;
BEGIN
    IF coalesce(cardinality(p_detectors), 0) = 0 THEN
        RETURN false;
    END IF;
    FOR v_surface IN
        SELECT * FROM sensitivity.surfaces s WHERE s.enabled AND s.cursor_kind IS NOT NULL
    LOOP
        v_ledger := v_surface.surface IN ('kb_events.payload', 'kb_events.metadata');
        v_view := 'src_' || replace(v_surface.surface, '.', '__');
        IF v_ledger THEN
            v_lo_at := NULL;
            v_hi_at := NULL;
            SELECT t.event_id INTO v_lo_id
              FROM public._resource_erasure_trail_scope(p_resource) t ORDER BY t.event_id LIMIT 1;
            SELECT t.event_id INTO v_hi_id
              FROM public._resource_erasure_trail_scope(p_resource) t ORDER BY t.event_id DESC LIMIT 1;
            SELECT EXISTS (SELECT 1 FROM public._resource_erasure_trail_scope(p_resource) t
                             JOIN sensitivity.unscanned_places u
                               ON u.surface = v_surface.surface AND u.target_id = t.event_id)
              INTO v_skipped;
        ELSE
            EXECUTE format('SELECT order_at, target_id FROM sensitivity.%I WHERE resource_id = $1 '
                           'ORDER BY coalesce(order_at, ''-infinity''), target_id LIMIT 1', v_view)
               INTO v_lo_at, v_lo_id USING p_resource;
            EXECUTE format('SELECT order_at, target_id FROM sensitivity.%I WHERE resource_id = $1 '
                           'ORDER BY coalesce(order_at, ''-infinity'') DESC, target_id DESC LIMIT 1', v_view)
               INTO v_hi_at, v_hi_id USING p_resource;
            EXECUTE format('SELECT EXISTS (SELECT 1 FROM sensitivity.%I v '
                           'JOIN sensitivity.unscanned_places u ON u.surface = $2 AND u.target_id = v.target_id '
                           'WHERE v.resource_id = $1)', v_view)
               INTO v_skipped USING p_resource, v_surface.surface;
        END IF;
        CONTINUE WHEN v_lo_id IS NULL;
        IF v_skipped THEN
            RETURN false;
        END IF;
        FOREACH v_det IN ARRAY p_detectors LOOP
            IF NOT EXISTS (
                SELECT 1
                  FROM sensitivity.cursors h
                  JOIN sensitivity.cursors b
                    ON b.surface = h.surface AND b.detector_id = h.detector_id
                   AND b.detector_version = h.detector_version AND b.lane = 'backfill'
                 WHERE h.surface = v_surface.surface AND h.lane = 'head' AND h.detector_id = v_det
                   AND (coalesce(v_hi_at, '-infinity'), v_hi_id)
                       <= (coalesce(h.watermark_at, '-infinity'), h.watermark_id)
                   AND ((coalesce(v_lo_at, '-infinity'), v_lo_id)
                            > (coalesce(b.backfill_floor_at, '-infinity'), b.backfill_floor_id)
                        OR b.backfill_completed_at IS NOT NULL
                        OR (b.watermark_id IS NOT NULL
                            AND (coalesce(v_lo_at, '-infinity'), v_lo_id)
                                >= (coalesce(b.watermark_at, '-infinity'), b.watermark_id))))
            THEN
                RETURN false;
            END IF;
        END LOOP;
    END LOOP;
    RETURN true;
END;
$$;

COMMENT ON FUNCTION sensitivity.resource_covered(uuid, text[]) IS
'Whether every place of the resource on every enabled surface has been read by every detector id in p_detectors (by some version''s head and backfill cursors, as scan_lane advances them), with none passed whole in unscanned_places. The ledger''s places are the resource''s trail scope. False for an empty detector set (erasure rulings from build order 3c, R2).';

-- ---------------------------------------------------------------------------
-- Section 3. Does a deriver quote one of R's detected values (D10)?
--
--   * R's side is R's countable findings, open or closed (R3): after a scrub or an erasure R's
--     findings are closed, and their fingerprints are what sweep D11 kept.
--   * A deriver's side is its countable findings with no closure (R3): a closed place no longer
--     holds the value.
--   * The states, first that holds wins (P2):
--       yes        a fingerprint in common;
--       expired    some of R's fingerprints are gone (Q50's expiry), capped (`truncated`) or never
--                  minted (`not_fingerprinted`), or the deriver's are, for a detector R's side
--                  holds (R1). Only when R holds a finding: with none, nothing of R's was lost, and
--                  the deriver's own capped finding is not this survey's to disclose. This is permanent, so it outranks `unscanned`: no later sweep can
--                  make `no` trustworthy;
--       unscanned  the sweep has not read all of R or all of the deriver, for the detectors R's
--                  findings came from; with no finding on R, for every enabled fingerprinting
--                  detector that has cursors. A disabled one stops reading, so waiting on it
--                  would leave content written since unscanned for good. None at all means the
--                  sweep never ran, or nothing is enabled (R2);
--       no         otherwise.
--
-- p_derivers is the survey plan's own deriver list (P1): this function never re-derives D8's
-- predicate. One row per distinct deriver other than R. Nothing but ids and states leaves it.
-- ---------------------------------------------------------------------------
CREATE FUNCTION sensitivity.deriver_fingerprint_matches(p_resource uuid, p_derivers uuid[])
RETURNS TABLE (deriver_id uuid, state text)
LANGUAGE plpgsql STABLE SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_detectors text[];
    v_r_holds   boolean;
    v_r_lost    boolean;
    v_r_covered boolean;
BEGIN
    SELECT array_agg(DISTINCT f.detector_id), bool_or(f.fingerprint_state <> 'complete')
      INTO v_detectors, v_r_lost
      FROM sensitivity.findings f
     WHERE f.resource_id = p_resource AND sensitivity.countable(f.id);
    v_r_holds := v_detectors IS NOT NULL;
    IF NOT v_r_holds THEN
        SELECT array_agg(DISTINCT c.detector_id) INTO v_detectors
          FROM sensitivity.cursors c
          JOIN sensitivity.detectors d ON d.id = c.detector_id
         WHERE d.fingerprinted AND d.enabled;
        v_r_lost := false;
    END IF;
    v_r_covered := sensitivity.resource_covered(p_resource, v_detectors);

    RETURN QUERY
    SELECT x.id,
           CASE
               WHEN EXISTS (
                   SELECT 1
                     FROM sensitivity.findings rf
                     JOIN sensitivity.finding_fingerprints rp ON rp.finding_id = rf.id
                     JOIN sensitivity.finding_fingerprints dp ON dp.fingerprint = rp.fingerprint
                     JOIN sensitivity.findings df ON df.id = dp.finding_id
                    WHERE rf.resource_id = p_resource AND sensitivity.countable(rf.id)
                      AND df.resource_id = x.id AND sensitivity.countable(df.id)
                      AND NOT EXISTS (SELECT 1 FROM sensitivity.finding_closure c
                                       WHERE c.finding_id = df.id))
                   THEN 'yes'
               WHEN v_r_lost OR (v_r_holds AND EXISTS (
                   SELECT 1
                     FROM sensitivity.findings df
                    WHERE df.resource_id = x.id AND df.detector_id = ANY (v_detectors)
                      AND df.fingerprint_state <> 'complete' AND sensitivity.countable(df.id)
                      AND NOT EXISTS (SELECT 1 FROM sensitivity.finding_closure c
                                       WHERE c.finding_id = df.id)))
                   THEN 'expired'
               WHEN NOT v_r_covered OR NOT sensitivity.resource_covered(x.id, v_detectors)
                   THEN 'unscanned'
               ELSE 'no'
           END
      FROM (SELECT DISTINCT u.id FROM unnest(p_derivers) AS u(id)
             WHERE u.id IS DISTINCT FROM p_resource) x;
END;
$$;

COMMENT ON FUNCTION sensitivity.deriver_fingerprint_matches(uuid, uuid[]) IS
'For each deriver in p_derivers (the erasure survey plan''s own list), whether it quotes one of R''s detected values: yes (a stored fingerprint in common between R''s countable findings, open or closed, and the deriver''s open countable ones), expired (a fingerprint the comparison needs is gone, capped or never minted), unscanned (the sweep has not read all of R or the deriver), or no; first that holds wins. Reads stored fingerprints only, never the salt, and returns ids and states only (erasure spec D10, rulings from build order 3c).';

-- ---------------------------------------------------------------------------
-- Section 4. Does a block's current revision still hold a finding (D11's scrub warning)? By
-- place (R4): an open, countable finding on the current revision's content, or on the content or
-- header path of one of the block's current chunks. A folded block has no current revision, and
-- the scrub empties it whole (P6). A current revision the sweep has not read yet holds no finding,
-- so it gives no warning: the warning confirms a leak, never cleanliness (R2).
-- ---------------------------------------------------------------------------
CREATE FUNCTION sensitivity.block_current_revision_flagged(p_block uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
    SELECT EXISTS (
        SELECT 1
          FROM kb_content_blocks b
          JOIN sensitivity.findings f
            ON (f.surface = 'kb_block_content.content' AND f.target_id = b.current_revision_id)
            OR (f.surface IN ('kb_chunk_content.content', 'kb_chunks.header_path')
                AND f.target_id IN (SELECT c.id FROM kb_chunks c
                                     WHERE c.block_id = b.id AND c.is_current))
         WHERE b.id = p_block AND NOT b.is_folded
           AND sensitivity.countable(f.id)
           AND NOT EXISTS (SELECT 1 FROM sensitivity.finding_closure c WHERE c.finding_id = f.id));
$$;

COMMENT ON FUNCTION sensitivity.block_current_revision_flagged(uuid) IS
'Whether the block''s current revision, or one of its current chunks, holds an open countable finding: the scrub survey''s "you have not edited it out yet". False for a folded block, and for a current revision the sweep has not read. Boolean only (erasure spec D11, rulings from build order 3c, R4).';

-- ---------------------------------------------------------------------------
-- Section 5. The doors the survey service calls. Survey-only: no act calls them. Without the
-- sweep's function each answers as a sweep that read nothing would, so a survey always renders.
--
-- Each takes the ids to compare from its caller, so each is a pairwise confirmation oracle over
-- any two resources: do they share a detected value, has the sweep read them. Two fences hold it:
--   * the grep gate (sensitivity_schema_unreachable_test) names these two functions too, and
--     allows them only in the two survey services, which sit behind the system-admin gate;
--   * EXECUTE is revoked from PUBLIC on all six functions (Section 6). One role migrates and serves
--     today (Q17), so this changes nothing yet. If roles are ever split, the app role is granted
--     EXECUTE on these two wrappers and nothing in the schema: they are SECURITY DEFINER, so they
--     reach the sweep without the role holding USAGE on it (D10: "a REVOKE on the app role does
--     not break the call").
-- ---------------------------------------------------------------------------
CREATE FUNCTION resource_erasure_deriver_fingerprints(p_resource uuid, p_derivers uuid[])
RETURNS TABLE (deriver_id uuid, state text)
LANGUAGE plpgsql STABLE SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
BEGIN
    IF to_regproc('sensitivity.deriver_fingerprint_matches') IS NULL THEN
        RETURN QUERY
        SELECT DISTINCT u.id, 'unscanned'::text
          FROM unnest(p_derivers) AS u(id)
         WHERE u.id IS DISTINCT FROM p_resource;
        RETURN;
    END IF;
    RETURN QUERY
    SELECT m.deriver_id, m.state
      FROM sensitivity.deriver_fingerprint_matches(p_resource, p_derivers) m;
END;
$$;

COMMENT ON FUNCTION resource_erasure_deriver_fingerprints(uuid, uuid[]) IS
'The erasure survey''s deriver annotation (D10): sensitivity.deriver_fingerprint_matches over the plan''s derivers, or unscanned for each when that function is absent. Survey only; the act never calls it.';

CREATE FUNCTION block_history_scrub_flagged_blocks(p_resource uuid, p_blocks uuid[])
RETURNS uuid[]
LANGUAGE plpgsql STABLE SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
BEGIN
    IF to_regproc('sensitivity.block_current_revision_flagged') IS NULL THEN
        RETURN '{}'::uuid[];
    END IF;
    RETURN coalesce((
        SELECT array_agg(b.id ORDER BY u.n)
          FROM unnest(p_blocks) WITH ORDINALITY AS u(id, n)
          JOIN kb_content_blocks b ON b.id = u.id AND b.resource_id = p_resource
         WHERE sensitivity.block_current_revision_flagged(b.id)), '{}'::uuid[]);
END;
$$;

COMMENT ON FUNCTION block_history_scrub_flagged_blocks(uuid, uuid[]) IS
'The scrub survey''s warning (D11): the named blocks of the resource, in the order named, whose current revision still holds an open finding (sensitivity.block_current_revision_flagged). Empty when that function is absent. Survey only; the act never calls it.';

-- ---------------------------------------------------------------------------
-- Section 6. No role reaches these by default (PostgreSQL grants EXECUTE to PUBLIC).
-- ---------------------------------------------------------------------------
REVOKE EXECUTE ON FUNCTION sensitivity.countable(uuid) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION sensitivity.resource_covered(uuid, text[]) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION sensitivity.deriver_fingerprint_matches(uuid, uuid[]) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION sensitivity.block_current_revision_flagged(uuid) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION resource_erasure_deriver_fingerprints(uuid, uuid[]) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION block_history_scrub_flagged_blocks(uuid, uuid[]) FROM PUBLIC;

SELECT declare_migration(
    20261012100000,
    'additive',
    'New functions only; no existing signature, body, table or view changes. In the sensitivity schema: countable, resource_covered, deriver_fingerprint_matches and block_current_revision_flagged (the last two SECURITY DEFINER with search_path set). In public: resource_erasure_deriver_fingerprints and block_history_scrub_flagged_blocks (SECURITY DEFINER with search_path set), the doors the erasure and block scrub survey services call. EXECUTE on all six is revoked from PUBLIC; the owning role, which also serves, keeps it. They read stored findings, fingerprints, dispositions and cursors, never the salt, and return ids, states and booleans only. No act calls them: resource_erasure_execute and block_history_scrub_execute read the plans, which are unchanged. Additive: no deployed binary names the sensitivity schema (the grep gate holds it).'
);
