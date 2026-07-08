-- Migration: _rebuild_resource_search_vector aggregates only chunks of LIVE blocks.
--
-- SQL function audit 2026-07-08 (docs/code-reviews/2026-07-08-sql-function-audit.md,
-- finding SQLA-3 / folded-block-leaks-into-fts): the body aggregation gated only
-- c.is_current, missing the NOT b.is_folded join that _recompute_resource_body_hash
-- and every vector/read gate apply. A charter supersede folds the old blocks but the
-- new charter arrives as fresh block ids — nothing flips the folded blocks' chunks
-- off is_current — so superseded charter prose stayed FTS-matchable and the rebuilt
-- search_vector diverged from body_hash (which excludes folded blocks).
--
-- Two beats:
--   1. CREATE OR REPLACE with the live-block join (mirrors _recompute_resource_body_hash),
--      plus a deterministic aggregation order (b.seq, c.chunk_index) so rebuilds are stable.
--   2. Backfill: re-run the rebuild for every resource that has at least one folded
--      block — exactly the population whose stored vectors may carry superseded prose.

CREATE OR REPLACE FUNCTION _rebuild_resource_search_vector(p_resource uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_config varchar(64); v_title text; v_body text;
BEGIN
    SELECT COALESCE((SELECT search_config FROM kb_resource_search_index WHERE resource_id = p_resource),
                    'english') INTO v_config;
    SELECT title INTO v_title FROM kb_resources WHERE id = p_resource;
    IF v_title IS NULL THEN RETURN; END IF;
    SELECT COALESCE(string_agg(cc.content, ' ' ORDER BY b.seq, c.chunk_index), '')
      INTO v_body
      FROM kb_chunks c
      JOIN kb_content_blocks b ON b.id = c.block_id AND NOT b.is_folded
      JOIN kb_chunk_content cc ON cc.chunk_id = c.id
     WHERE c.resource_id = p_resource AND c.is_current;
    INSERT INTO kb_resource_search_index (resource_id, search_vector, search_config, updated)
    VALUES (p_resource,
            setweight(to_tsvector(v_config::regconfig, COALESCE(v_title,'')), 'A')
              || setweight(to_tsvector(v_config::regconfig, v_body), 'B'),
            v_config, now())
    ON CONFLICT (resource_id) DO UPDATE
        SET search_vector = EXCLUDED.search_vector, updated = now();
END;
$$;

-- Backfill stale vectors: any resource with a folded block may have superseded prose
-- baked into its stored search_vector; rebuild each through the fixed function.
DO $$
DECLARE r uuid;
BEGIN
    FOR r IN SELECT DISTINCT resource_id FROM kb_content_blocks WHERE is_folded
    LOOP
        PERFORM _rebuild_resource_search_vector(r);
    END LOOP;
END;
$$;
