-- A place an erasure act emptied keeps its findings' salt-keyed digests for 30 days, then gives
-- them up.
--
-- Sweep D11 keeps a finding's fingerprints past the erasure of the content they came from, so an
-- operator can ask which other resources carry the same value. The cost D11 names is a
-- confirmation oracle: whoever holds the salt can brute-force a small-space value (an SSN is about
-- 10^9 guesses) from its fingerprint, and a short unit (a title) from the finding's keyed
-- content_hash. For an erased place, that is a recoverable copy of what the act removed.
-- Ruled 2026-10-03 with Pete (resource erasure task 01a0fedb-bfe6-7783-bf71-5e030bd6a806, item 3):
-- correlation's value is front-loaded, so the digests are kept for a window and then dropped.
--
-- A finding gives them up when it is ERASED and CLOSED, and has been seen so for 30 days.
--   * Erased means the resource erasure act reached its place (sensitivity.erased_place_findings).
--     That is not findings.resource_id alone: the act wipes an edge's label and its edge-owned
--     properties at EITHER end, while the scan records an edge label's resource as its source
--     and an edge-owned property's as NULL; and it deletes a remote source it orphans, whose
--     finding carries no resource at all. Only the act deletes a kb_remote_sources row.
--     It also covers the two acts that empty content while the resource lives on: the block
--     history scrub (the act's own body, narrowed to named blocks' history) and the principal
--     erasure act (governed block and chunk content). Only an erasure body ever sets
--     kb_block_content.content or kb_chunk_content.content to '' or kb_chunks.header_path to
--     NULL, so on those three surfaces a place closed as content_empty is an erased place.
--   * Closed is sensitivity.place_closure. A finding still open sits on a place that still holds
--     the value in plain text (a ledger path until erasure cut 2, a block-owned property the act
--     does not reach, a remote source another resource still cites), so its digests add nothing.
--   * The window runs from the first scanning tick that commits after it sees the finding erased
--     and closed, recorded in sensitivity.erased_closures. A missing remote-source row has no
--     erased_at to read, so every arm uses the same clock. It can only start late, never early: a
--     tick whose scan fails rolls the step back with it, and a stopped sweep never starts it.
--   * When the window ends, closure is read again; a finding open again keeps its digests.
-- Its fingerprints are deleted, its content_hash is replaced by a digest of the finding's own id,
-- which says nothing about the unit, and every sensitivity.memo row carrying the original hash is
-- deleted first. The memo holds the keyed hash of each unit a detector found nothing in, which
-- includes a unit another detector did find something in, so leaving it would keep the same oracle
-- with only the pointer gone. The memo is a cache: a deleted row costs a re-scan of that unit
-- wherever it next appears. place_observations needs nothing: the act moves the place's order key
-- (kb_resources.updated), so the next head tick re-reads it and records the husk's hash.
-- fingerprint_state reads 'expired'. The row itself stays:
-- the pointer, category, severity and closure are the content-free record of what was found, and
-- sweep §8 forbids a finding silently vanishing. Closure is unchanged by the rewrite: an erased
-- place closes as row_missing, sentinel or content_empty, which place_closure decides before it
-- compares hashes, and a finding closed as changed stays changed, since no observation can equal
-- the replacement.
--
-- Ledger findings are eligible by findings.resource_id only. Erasure cut 2, which first closes
-- them, adds an arm for its own trail scope here (an event about an edge into R names its source).
--
-- The sweep reads the erasure's state; the act never names this schema (3c's posture, where the
-- act behaves identically with the sweep uninstalled).

ALTER TABLE sensitivity.findings DROP CONSTRAINT findings_fingerprint_state_check;
ALTER TABLE sensitivity.findings ADD CONSTRAINT findings_fingerprint_state_check
    CHECK (fingerprint_state IN ('complete', 'truncated', 'not_fingerprinted', 'expired'));

CREATE TABLE sensitivity.erased_closures (
    finding_id     uuid PRIMARY KEY REFERENCES sensitivity.findings (id),
    closed_seen_at timestamptz NOT NULL DEFAULT now()
);

-- The findings, not yet expired or clocked, whose place an erasure act emptied or reached: one
-- arm per reach. Narrowed first, so a tick never re-walks what it has already handled.
CREATE FUNCTION sensitivity.erased_place_findings() RETURNS TABLE (finding_id uuid)
LANGUAGE sql STABLE AS $$
    WITH f AS (
        SELECT f.* FROM sensitivity.findings f
         WHERE f.fingerprint_state <> 'expired'
           AND NOT EXISTS (SELECT 1 FROM sensitivity.erased_closures c WHERE c.finding_id = f.id)
    )
    -- A place the scan attributes to an erased resource: its title, origin_uri, blocks, chunks,
    -- citation reasons, and its own and its blocks' properties.
    SELECT f.id FROM f
      JOIN kb_resources r ON r.id = f.resource_id
     WHERE r.erased_at IS NOT NULL
    UNION
    -- An edge's label, with an erased resource at either end (act step 9c).
    SELECT f.id FROM f
      JOIN kb_edges e ON e.id = f.target_id
     WHERE f.surface = 'kb_edges.label'
       AND EXISTS (SELECT 1 FROM kb_resources r
                    WHERE r.erased_at IS NOT NULL
                      AND ((e.source_table = 'kb_resources' AND r.id = e.source_id)
                        OR (e.target_table = 'kb_resources' AND r.id = e.target_id)))
    UNION
    -- An edge-owned property's key or value, on such an edge (act step 9d).
    SELECT f.id FROM f
      JOIN kb_properties p ON p.id = f.target_id AND p.owner_table = 'kb_edges'
      JOIN kb_edges e ON e.id = p.owner_id
     WHERE f.surface IN ('kb_properties.property_key', 'kb_properties.property_value')
       AND EXISTS (SELECT 1 FROM kb_resources r
                    WHERE r.erased_at IS NOT NULL
                      AND ((e.source_table = 'kb_resources' AND r.id = e.source_id)
                        OR (e.target_table = 'kb_resources' AND r.id = e.target_id)))
    UNION
    -- Block and chunk content an erasure act emptied while the resource lives on: the scrub's
    -- prior revisions and the principal act's governed content (content_empty, never row_missing).
    SELECT f.id FROM f
     WHERE f.surface IN ('kb_block_content.content', 'kb_chunk_content.content', 'kb_chunks.header_path')
       AND sensitivity.place_closure(f.surface, f.target_id, f.content_hash) = 'content_empty'
    UNION
    -- A remote source the act orphaned and deleted (act step 9e, the only deleter).
    SELECT f.id FROM f
     WHERE f.surface = 'kb_remote_sources.uri'
       AND NOT EXISTS (SELECT 1 FROM kb_remote_sources s WHERE s.id = f.target_id);
$$;

CREATE FUNCTION sensitivity.expire_erased_fingerprints(p_window interval DEFAULT '30 days')
RETURNS int LANGUAGE plpgsql AS $$
DECLARE
    v_n int;
BEGIN
    INSERT INTO sensitivity.erased_closures (finding_id)
    SELECT f.id
      FROM sensitivity.erased_place_findings() e
      JOIN sensitivity.findings f ON f.id = e.finding_id
     WHERE sensitivity.place_closure(f.surface, f.target_id, f.content_hash) IS NOT NULL
    ON CONFLICT (finding_id) DO NOTHING;

    -- Locked in id order, so two ticks expiring the same findings cannot deadlock.
    WITH due AS (
        SELECT f.id, f.content_hash FROM sensitivity.erased_closures c
          JOIN sensitivity.findings f ON f.id = c.finding_id
         WHERE c.closed_seen_at <= now() - p_window
           AND f.fingerprint_state <> 'expired'
           AND sensitivity.place_closure(f.surface, f.target_id, f.content_hash) IS NOT NULL
         ORDER BY f.id
           FOR UPDATE OF f
    ), memo_gone AS (
        DELETE FROM sensitivity.memo m USING due WHERE m.content_hash = due.content_hash
    ), gone AS (
        DELETE FROM sensitivity.finding_fingerprints ff USING due WHERE ff.finding_id = due.id
    )
    UPDATE sensitivity.findings f
       SET fingerprint_state = 'expired',
           content_hash = encode(sha256(convert_to('expired:' || f.id::text, 'UTF8')), 'hex')
      FROM due
     WHERE f.id = due.id;
    GET DIAGNOSTICS v_n = ROW_COUNT;
    RETURN v_n;
END;
$$;

COMMENT ON FUNCTION sensitivity.expire_erased_fingerprints(interval) IS
'Starts the window for every closed finding on a place an erasure act emptied or reached (sensitivity.erased_place_findings), then, once a finding has been seen so for p_window (default 30 days, ruled 2026-10-03) and is still closed, deletes the memo rows carrying its keyed content_hash, drops its fingerprints and replaces that hash. fingerprint_state reads expired; the finding row stays. Run by every scanning sensitivity_sweep_tick. Returns the number of findings expired.';

-- 20261003150000's body, verbatim except for the one PERFORM below.

CREATE OR REPLACE FUNCTION sensitivity_sweep_tick(
    p_run uuid, p_job uuid, p_salt bytea, p_lag interval DEFAULT '5 minutes', p_budget_ms int DEFAULT 20000
) RETURNS TABLE (rows_examined int, hashes_examined int, cache_hits int, new_findings int,
                 cursor_advances int, units_oversize int, failed boolean)
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
            -- A closed finding on a place the erasure act reached gives up its salt-keyed digests
            -- once the window has passed (sensitivity.expire_erased_fingerprints).
            PERFORM sensitivity.expire_erased_fingerprints();
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
    SELECT r.rows_examined, r.hashes_examined, r.cache_hits, r.new_findings, r.cursor_advances, r.units_oversize
      INTO rows_examined, hashes_examined, cache_hits, new_findings, cursor_advances, units_oversize
      FROM sensitivity.runs r WHERE r.id = p_run;
    RETURN NEXT;
END;
$$;


SELECT declare_migration(
    20261003210000,
    'additive',
    'Findings on places an erasure act emptied (the resource act, the block history scrub, the principal act) give up their salt-keyed digests after 30 days (sensitivity sweep D11, ruled 2026-10-03). Adds sensitivity.erased_closures, sensitivity.erased_place_findings and sensitivity.expire_erased_fingerprints (which also deletes the memo rows of the hashes it retires), and widens findings_fingerprint_state_check with ''expired''. CREATE OR REPLACEs sensitivity_sweep_tick with the same signature and return shape; its body is 20261003150000''s verbatim plus one PERFORM of the new function. Additive: no deployed binary names the sensitivity schema (the grep gate holds it), the tick''s signature is unchanged, and a widened CHECK admits every row it admitted before.'
);
