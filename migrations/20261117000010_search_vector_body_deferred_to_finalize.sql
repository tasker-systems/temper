-- A segmented ingest's search vector takes its body text once, at finalize — not on every append.
--
-- Every block append ends in `_project_blocks`, whose tail rebuilt the resource's search vector by
-- string_agg-ing EVERY live chunk's text and running to_tsvector over all of it. The append's cost
-- therefore grew with the resource's size and a segmented ingest was quadratic: at 4000 blocks of
-- ~500 bytes the rebuild was ~106 ms of a ~96 ms-average append window, against ~3 ms for the
-- body-hash recompute beside it (measured; laptop, ephemeral database).
--
-- THE GUARD IS SAFE ONLY BECAUSE NO READER SEES AN IN-PROGRESS VECTOR. Every search arm reads through
-- `kb_resources_live` (`is_active AND ingest_state = 'complete'`, 20260808000020). A new reader of
-- `kb_resource_search_index` that skips that view would find title-only vectors for resources still
-- arriving — it must carry the same gate.
--
-- What stays the same while an ingest is in progress: the row is still upserted, with title,
-- keywords, tags and descriptor, so its existence and the erasure act's `search_vector <> ''`
-- accounting (20261004150000) are unchanged. Only the weight-B body is withheld.
--
-- `_project_resource_finalized` now rebuilds once after flipping the state to `complete`, inside the
-- finalize transaction, so a resource is never observed complete with a body-less vector. Replay runs
-- the same two projectors, so the replayed vector equals the live one.
--
-- The body hash is NOT deferred: every append response returns it (`landed_blocks`) and an MCP caller
-- echoes it back at finalize, so it must be correct between appends.
--
-- Both bodies are copied verbatim from their live definitions (20260711000060, and 20260929040730's
-- schema-qualified `public._project_resource_finalized`, which carries the erasure write guard); the
-- only edits are the in-progress guard and the trailing rebuild. Find a live definition by the
-- function name with and without `public.`: a search for one spelling misses the other.

CREATE OR REPLACE FUNCTION _rebuild_resource_search_vector(p_resource uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_config varchar(64); v_title text; v_body text;
        v_keywords text; v_tags text; v_descriptor text;
        v_ingest_state text;
BEGIN
    SELECT COALESCE((SELECT search_config FROM kb_resource_search_index WHERE resource_id = p_resource),
                    'english') INTO v_config;
    SELECT title, ingest_state INTO v_title, v_ingest_state FROM kb_resources WHERE id = p_resource;
    IF v_title IS NULL THEN RETURN; END IF;
    -- A still-arriving body is not indexed: no search arm admits an in-progress resource, and
    -- aggregating the whole body here on every append made a segmented ingest quadratic.
    -- `_project_resource_finalized` rebuilds with the body once the ingest completes.
    IF v_ingest_state = 'in_progress' THEN
        v_body := '';
    ELSE
        SELECT COALESCE(string_agg(cc.content, ' ' ORDER BY b.seq, c.chunk_index), '')
          INTO v_body
          FROM kb_chunks c
          JOIN kb_content_blocks b ON b.id = c.block_id AND NOT b.is_folded
          JOIN kb_chunk_content cc ON cc.chunk_id = c.id
         WHERE c.resource_id = p_resource AND c.is_current;
    END IF;

    -- open_meta convention v1: keywords (array of strings, space-joined, or a bare string) at weight C.
    SELECT COALESCE(
        (SELECT CASE jsonb_typeof(property_value)
                  WHEN 'array'  THEN (SELECT string_agg(e, ' ')
                                        FROM jsonb_array_elements_text(property_value) e)
                  WHEN 'string' THEN property_value #>> '{}'
                  ELSE '' END
           FROM kb_properties
          WHERE owner_table = 'kb_resources' AND owner_id = p_resource
            AND property_key = 'keywords' AND NOT is_folded
          LIMIT 1),
        '') INTO v_keywords;
    -- open_meta convention v2: tags (array of strings, space-joined, or a bare string) at weight C.
    SELECT COALESCE(
        (SELECT CASE jsonb_typeof(property_value)
                  WHEN 'array'  THEN (SELECT string_agg(e, ' ')
                                        FROM jsonb_array_elements_text(property_value) e)
                  WHEN 'string' THEN property_value #>> '{}'
                  ELSE '' END
           FROM kb_properties
          WHERE owner_table = 'kb_resources' AND owner_id = p_resource
            AND property_key = 'tags' AND NOT is_folded
          LIMIT 1),
        '') INTO v_tags;
    -- open_meta convention v1: descriptor (a JSON string) at weight D.
    SELECT COALESCE(
        (SELECT property_value #>> '{}'
           FROM kb_properties
          WHERE owner_table = 'kb_resources' AND owner_id = p_resource
            AND property_key = 'descriptor' AND NOT is_folded
            AND jsonb_typeof(property_value) = 'string'
          LIMIT 1),
        '') INTO v_descriptor;

    INSERT INTO kb_resource_search_index (resource_id, search_vector, search_config, updated)
    VALUES (p_resource,
            setweight(to_tsvector(v_config::regconfig, COALESCE(v_title,'')), 'A')
              || setweight(to_tsvector(v_config::regconfig, v_body), 'B')
              || setweight(to_tsvector(v_config::regconfig, COALESCE(v_keywords,'')), 'C')
              || setweight(to_tsvector(v_config::regconfig, COALESCE(v_tags,'')), 'C')
              || setweight(to_tsvector(v_config::regconfig, COALESCE(v_descriptor,'')), 'D'),
            v_config, now())
    ON CONFLICT (resource_id) DO UPDATE
        SET search_vector = EXCLUDED.search_vector, updated = now();
END;
$$;

COMMENT ON FUNCTION _rebuild_resource_search_vector(uuid) IS
$c$Rebuilds a resource's FTS vector: title (A), live body (B), keywords and tags (C), descriptor (D).
While the resource's ingest_state is in_progress the body is withheld (20261117000010): no search arm
admits an in-progress resource, and _project_resource_finalized rebuilds with the body at finalize.$c$;

CREATE OR REPLACE FUNCTION _project_resource_finalized(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM _resource_write_guard((p_payload->>'resource_id')::uuid);
    UPDATE kb_resources SET ingest_state = 'complete'
     WHERE id = (p_payload->>'resource_id')::uuid;
    -- The body the in-progress guard withheld, indexed once, in the same transaction as the flip.
    PERFORM _rebuild_resource_search_vector((p_payload->>'resource_id')::uuid);
END;
$$;

COMMENT ON FUNCTION _project_resource_finalized(uuid, jsonb) IS
$c$Marks a segmented ingest complete, then rebuilds its search vector with the body text that was
withheld while it was in progress (20261117000010).$c$;

SELECT declare_migration(
    20261117000010,
    'additive',
    'CREATE OR REPLACE of _rebuild_resource_search_vector and _project_resource_finalized with unchanged signatures; no table, column or index changes. A binary from before this migration calls neither function directly with different arguments, and every search read it makes goes through kb_resources_live, which never admits the in-progress resources whose vector now omits body text. A binary from after it against the old functions gets the old, slower but complete vectors.'
);
