-- Backfill kb_resource_revisions from existing kb_resource_audits (chunk-producing
-- actions only) and populate kb_chunks.first_revision_id / superseded_revision_id
-- from the audit timeline.
--
-- Strategy:
--   1. Synthesize one revision per (resource_id, audit) where action is chunk-
--      producing. Revision.created = audit.created so timeline lookups align.
--   2. For each chunk, pick the nearest-preceding revision (by created).
--   3. For non-current chunks, pick the earliest-following revision as the
--      superseder.
--   4. For any chunk still unassigned (resources with no chunk-producing audit
--      history -- should be rare/zero pre-release), synthesize a revision from
--      the chunk's own created + content_hash.
--   5. Recompute chunk_count per revision.

BEGIN;

-- Step 1: synthesize revisions from chunk-producing audits.
INSERT INTO kb_resource_revisions (id, resource_id, audit_id, body_hash, chunk_count, created)
SELECT uuidv7(), a.resource_id, a.id, a.body_hash, 0, a.created
  FROM kb_resource_audits a
 WHERE a.action IN ('create', 'update_body');

-- Step 2: chunks' first_revision_id = nearest-preceding revision.
UPDATE kb_chunks c
   SET first_revision_id = (
       SELECT r.id FROM kb_resource_revisions r
        WHERE r.resource_id = c.resource_id
          AND r.created <= c.created
        ORDER BY r.created DESC
        LIMIT 1
   );

-- Step 3: non-current chunks get earliest-following revision as superseder.
UPDATE kb_chunks c
   SET superseded_revision_id = (
       SELECT r.id FROM kb_resource_revisions r
        WHERE r.resource_id = c.resource_id
          AND r.created > c.created
        ORDER BY r.created ASC
        LIMIT 1
   )
 WHERE c.is_current = false;

-- Step 4: fallback -- chunks with no preceding audit get a synthetic revision.
-- One revision per orphan-chunk cohort (grouped by resource).
WITH orphans AS (
    SELECT resource_id, MIN(created) AS first_chunk_created,
           MIN(content_hash) AS sample_hash,
           COUNT(*)::INT AS n
      FROM kb_chunks
     WHERE first_revision_id IS NULL
     GROUP BY resource_id
),
inserted AS (
    INSERT INTO kb_resource_revisions (id, resource_id, audit_id, body_hash, chunk_count, created)
    SELECT uuidv7(), o.resource_id, NULL, o.sample_hash, o.n, o.first_chunk_created
      FROM orphans o
    RETURNING id, resource_id
)
UPDATE kb_chunks c
   SET first_revision_id = i.id
  FROM inserted i
 WHERE c.resource_id = i.resource_id
   AND c.first_revision_id IS NULL;

-- Step 5: recompute chunk_count per revision now that all links exist.
UPDATE kb_resource_revisions r
   SET chunk_count = (
       SELECT COUNT(*) FROM kb_chunks c
        WHERE c.first_revision_id = r.id
   );

COMMIT;
