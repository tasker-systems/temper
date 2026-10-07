-- The sweep's two other stores of salt-keyed digests are bounded by the erasure expiry too: the
-- memo keeps a clean unit's hash for 30 days, and an erased resource's place observations go on
-- the next expiry call.
--
-- 20261004000010 expires a finding's digests once its erased place has been closed for 30 days
-- (sweep D11, ruled 2026-10-03), and deletes the memo rows carrying that finding's hash. Two
-- stores of sha256(salt || unit) fell outside it, and each is the same confirmation oracle over
-- erased content for whoever holds the salt (v0.6.0 security review §4.3):
--
--   * sensitivity.memo holds the hash of every unit a detector version found nothing in. A unit
--     with no finding is never in the expiry's due set, so its hash outlived the content's
--     erasure. A memo row names no place, and this database holds no salt, so an erased place can
--     never be traced to its memo rows: an emptied block's '' says nothing about the hash of what
--     it held. The bound is therefore age. A memo row is deleted 30 days after it was written,
--     whatever it hashes, which is never longer than a finding on the same erased place keeps its
--     digests. The memo is a cache (D4 rule 2): a deleted row costs at most one re-scan of that
--     unit wherever it next appears. Rows written before this migration carry no age, read as
--     -infinity, and are due at once.
--   * sensitivity.place_observations holds the last hash each mutable place was read with.
--     20261004000010 reasoned that it "needs nothing", since the act moves kb_resources.updated
--     and the next head tick records the husk's hash. That holds only while some detector is
--     enabled, and 20261004140000 turns every detector off by default, so with no tick the
--     pre-erasure hash stays. Every observation of an erased resource is now deleted, the husk's
--     included: the write doors refuse an erased id, so the place cannot change again and its
--     observation has nothing left to tell a recurrence from. Not windowed, because an
--     observation has no correlation value to keep. Not compared against erased_at either: a tick
--     whose transaction began after the act's but read before the act committed records the
--     pre-erasure hash at a time after erased_at. Only the kb_resources surfaces are
--     mutable_timestamp, and only they write observations;
--     every_mutable_surface_is_a_resource_surface holds a new one to an arm here.
--
--   * The planner's statistics keep copies of the hashes themselves: a column's histogram holds
--     up to a hundred sampled values, readable in pg_stats by any role that may read the column,
--     and it outlives the rows it sampled until the next ANALYZE replaces it, which on a quiet
--     table may never come. A histogram of keyed hashes tells the planner nothing it uses, so the
--     four hash columns keep none: their statistics target goes to 0, and an ALTER COLUMN TYPE to
--     the type they already have removes what ANALYZE has gathered so far. It rewrites neither
--     the table nor an index (the type is unchanged), and with the target at 0 no ANALYZE gathers
--     them again.
--
-- Both run on every call of expire_erased_fingerprints, so on every door call whether or not the
-- deployment sweeps (Q53), and an install whose detectors are off holds no new row to delete. The
-- memo is deleted in batches of 50,000, oldest first, so an install with a large memo from before
-- this migration clears it over several door calls rather than in one statement.

ALTER TABLE sensitivity.memo ADD COLUMN memoized_at timestamptz NOT NULL DEFAULT '-infinity';
ALTER TABLE sensitivity.memo ALTER COLUMN memoized_at SET DEFAULT now();

CREATE INDEX memo_by_memoized_at ON sensitivity.memo (memoized_at);

COMMENT ON COLUMN sensitivity.memo.memoized_at IS
$c$When the row was written. expire_erased_fingerprints deletes a row 30 days after it, since a memo
row cannot be traced to the place its unit came from. -infinity for a row written before the column
existed, which is due at once.$c$;

ALTER TABLE sensitivity.memo
    ALTER COLUMN content_hash SET STATISTICS 0, ALTER COLUMN content_hash TYPE text;
ALTER TABLE sensitivity.place_observations
    ALTER COLUMN content_hash SET STATISTICS 0, ALTER COLUMN content_hash TYPE text;
-- finding_closure reads findings.content_hash, and a column a view reads cannot change type, so
-- the view is dropped and recreated around it, 20261003150000's definition verbatim.
DROP VIEW sensitivity.finding_closure;
ALTER TABLE sensitivity.findings
    ALTER COLUMN content_hash SET STATISTICS 0, ALTER COLUMN content_hash TYPE text;
CREATE VIEW sensitivity.finding_closure AS
    SELECT c.finding_id, c.closed_by
      FROM (SELECT f.id AS finding_id, sensitivity.place_closure(f.surface, f.target_id, f.content_hash) AS closed_by
              FROM sensitivity.findings f) c
     WHERE c.closed_by IS NOT NULL;
ALTER TABLE sensitivity.finding_fingerprints
    ALTER COLUMN fingerprint SET STATISTICS 0, ALTER COLUMN fingerprint TYPE bytea;

-- 20261004000010's body, verbatim except for the two DELETEs after the finding steps.
CREATE OR REPLACE FUNCTION sensitivity.expire_erased_fingerprints(p_window interval DEFAULT '30 days')
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

    -- Last, and never waiting: each skips rows another call holds, so neither waits on one, and
    -- the finding steps above have taken their locks before these take any.
    -- A clean unit's hash lives as long as the window, whatever place it came from; oldest first.
    DELETE FROM sensitivity.memo m
     WHERE (m.content_hash, m.detector_id, m.detector_version) IN (
            SELECT d.content_hash, d.detector_id, d.detector_version FROM sensitivity.memo d
             WHERE d.memoized_at <= now() - p_window
             ORDER BY d.memoized_at
             LIMIT 50000
               FOR UPDATE SKIP LOCKED);

    -- An erased resource's place observations, the husk's included.
    DELETE FROM sensitivity.place_observations o
     WHERE (o.surface, o.target_id) IN (
            SELECT po.surface, po.target_id FROM sensitivity.place_observations po
              JOIN kb_resources r ON r.id = po.target_id
             WHERE po.surface IN ('kb_resources.title', 'kb_resources.origin_uri')
               AND r.erased_at IS NOT NULL
               FOR UPDATE OF po SKIP LOCKED);

    RETURN v_n;
END;
$$;

COMMENT ON FUNCTION sensitivity.expire_erased_fingerprints(interval) IS
'Starts the window for every closed finding on a place an erasure act emptied or reached (sensitivity.erased_place_findings), and once a finding has been seen so for p_window (default 30 days, ruled 2026-10-03) and is still closed, deletes the memo rows carrying its keyed content_hash, drops its fingerprints and replaces that hash; fingerprint_state reads expired and the finding row stays. Then deletes up to 50,000 memo rows written more than p_window ago, oldest first, and every place observation of an erased resource, each skipping rows another call holds. Run on every door call through sensitivity_expire_erased_fingerprints, opted in or not (Q53), and by every scanning sensitivity_sweep_tick. Returns the number of findings expired.';

SELECT declare_migration(
    20261007000010,
    'additive',
    'Bounds the sweep''s two other stores of salt-keyed digests by the erasure expiry (v0.6.0 security review §4.3). Adds sensitivity.memo.memoized_at (NOT NULL, -infinity for existing rows, now() for new ones) and its index; sets the statistics target of the four hash columns (memo, place_observations and findings content_hash, finding_fingerprints fingerprint) to 0 and removes their gathered statistics with a same-type ALTER COLUMN TYPE, which rewrites no table or index (sensitivity.finding_closure, which reads one of them, is dropped and recreated verbatim around it); and CREATE OR REPLACEs sensitivity.expire_erased_fingerprints with the same signature and return: it now deletes memo rows older than its window, 50,000 per call, oldest first, and every place observation of an erased resource, after its unchanged finding steps. Additive: no column changes type or nullability, the scan''s INSERT into the memo names its three columns and gets the new one''s default, no Rust names the sensitivity schema (the grep gate holds it), and the deployed door reads the same int.'
);
