-- Allow slug reuse after soft-delete.
--
-- The original constraint `UNIQUE(slug, kb_context_id)` kept soft-deleted
-- rows reserving their slug, so `temper resource delete <slug>` followed by
-- `temper resource create --slug <slug>` returned 409 Conflict even though
-- `temper resource list` and `temper resource show` no longer surfaced the
-- soft-deleted row to the user. Replace with a partial unique index gated
-- on `is_active = true` so active-row uniqueness is preserved while
-- soft-deleted rows free their slug for reuse.
--
-- Surfaced from 2026-05-03 session; see vault task
-- `2026-05-03-soft-deleted-resources-should-not-block-slug-reuse`.

ALTER TABLE kb_resources
    DROP CONSTRAINT IF EXISTS kb_resources_slug_kb_context_id_key;

CREATE UNIQUE INDEX kb_resources_slug_kb_context_id_active_unique
    ON kb_resources (slug, kb_context_id)
    WHERE is_active = true;
