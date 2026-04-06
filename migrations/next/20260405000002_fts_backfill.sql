-- =============================================================================
-- Backfill FTS search index for all existing resources
-- =============================================================================
-- Populates kb_resource_search_index for every active resource by aggregating
-- title + slug (weight A) and all current chunk content (weight B).
--
-- Safe to re-run: uses ON CONFLICT DO UPDATE (upsert).
-- Separated from schema migration so the schema is available immediately
-- and triggers handle new data even if this backfill is re-run.

INSERT INTO kb_resource_search_index (resource_id, search_vector, search_config, updated)
SELECT
    r.id,
    setweight(to_tsvector('english'::regconfig, COALESCE(r.title, '')), 'A') ||
    setweight(to_tsvector('english'::regconfig, COALESCE(r.slug, '')), 'A') ||
    setweight(to_tsvector('english'::regconfig, COALESCE(body_agg.body, '')), 'B'),
    'english',
    now()
FROM kb_resources r
LEFT JOIN LATERAL (
    SELECT string_agg(cc.content, ' ') AS body
      FROM kb_chunks c
      JOIN kb_chunk_content cc ON cc.chunk_id = c.id
     WHERE c.resource_id = r.id
       AND c.is_current = true
) body_agg ON true
WHERE r.is_active = true
ON CONFLICT (resource_id) DO UPDATE SET
    search_vector = EXCLUDED.search_vector,
    updated = now();
