-- Retention sweep. Three dials, evaluated jointly:
--   1. Referential pin (enforced by FKs): a revision referenced by any
--      kb_chunks.first_revision_id or .superseded_revision_id cannot be
--      deleted. We pre-filter candidates to skip the DELETE attempt.
--   2. Per-resource keep-last-N: the N most recent revisions per resource
--      are pinned regardless of age.
--   3. Age ceiling: revisions younger than p_age_ceiling_days are pinned.
--
-- Practical note: because Phase C does not garbage-collect kb_chunks, dial
-- (1) is dominant in practice. Any revision that ever minted or superseded
-- a chunk stays pinned forever. The sweep therefore collects only chunkless
-- revisions — orphans from the backfill fallback (migration 0008 step 4)
-- and potential future sources of chunkless revisions. Operators should
-- not expect large deletion counts until chunk GC lands.
--
-- Returns the count of revisions deleted.
CREATE OR REPLACE FUNCTION sweep_orphaned_revisions(
    p_keep_last_n      INT DEFAULT 10,
    p_age_ceiling_days INT DEFAULT 90
) RETURNS INT
LANGUAGE plpgsql AS $$
DECLARE
    v_deleted INT;
BEGIN
    WITH ranked AS (
        SELECT r.id,
               r.resource_id,
               r.created,
               row_number() OVER (PARTITION BY r.resource_id ORDER BY r.created DESC) AS rn
          FROM kb_resource_revisions r
    ),
    candidates AS (
        SELECT r.id
          FROM ranked r
         WHERE r.rn > p_keep_last_n
           AND r.created < now() - (p_age_ceiling_days || ' days')::interval
           AND NOT EXISTS (SELECT 1 FROM kb_chunks c WHERE c.first_revision_id = r.id)
           AND NOT EXISTS (SELECT 1 FROM kb_chunks c WHERE c.superseded_revision_id = r.id)
    ),
    deleted AS (
        DELETE FROM kb_resource_revisions
         WHERE id IN (SELECT id FROM candidates)
        RETURNING id
    )
    SELECT COUNT(*)::INT INTO v_deleted FROM deleted;

    RETURN v_deleted;
END;
$$;
