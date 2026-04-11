-- Add heading_depth to kb_chunks so reconstitution can rebuild markdown headings.
-- depth 0 = no heading (preamble text), 1 = #, 2 = ##, etc.
ALTER TABLE kb_chunks ADD COLUMN heading_depth SMALLINT NOT NULL DEFAULT 0;

-- Recreate the view to include heading_depth.
-- The current definition (from 20260401000002) joins kb_chunk_content for content.
DROP VIEW IF EXISTS kb_current_chunks;
CREATE VIEW kb_current_chunks AS
SELECT c.id, c.resource_id, c.chunk_index, c.version, c.header_path, c.heading_depth,
       cc.content, c.content_hash, c.embedding, c.created
  FROM kb_chunks c
  LEFT JOIN kb_chunk_content cc ON cc.chunk_id = c.id
 WHERE c.is_current = true
 ORDER BY c.resource_id, c.chunk_index;

-- Update persist_resource_chunks to include heading_depth
CREATE OR REPLACE FUNCTION persist_resource_chunks(
    p_resource_id UUID,
    p_chunks      JSONB
) RETURNS INT
LANGUAGE plpgsql AS $$
DECLARE
    v_count INT;
BEGIN
    PERFORM set_config('temper.skip_search_rebuild', 'true', true);

    WITH chunk_data AS (
        SELECT
            gen_random_uuid() AS chunk_id,
            p_resource_id AS resource_id,
            (elem->>'chunk_index')::INT AS chunk_index,
            COALESCE(elem->>'header_path', '') AS header_path,
            COALESCE((elem->>'heading_depth')::SMALLINT, 0) AS heading_depth,
            elem->>'content' AS content,
            elem->>'content_hash' AS content_hash,
            elem->>'embedding' AS embedding_str
        FROM jsonb_array_elements(p_chunks) AS elem
    ),
    inserted_chunks AS (
        INSERT INTO kb_chunks (
            id, resource_id, chunk_index, version, header_path, heading_depth,
            content_hash, embedding, is_current
        )
        SELECT
            cd.chunk_id, cd.resource_id, cd.chunk_index, 1, cd.header_path, cd.heading_depth,
            cd.content_hash, cd.embedding_str::vector, true
        FROM chunk_data cd
        RETURNING id, chunk_index
    ),
    inserted_content AS (
        INSERT INTO kb_chunk_content (chunk_id, content)
        SELECT cd.chunk_id, cd.content
        FROM chunk_data cd
        RETURNING chunk_id
    )
    SELECT COUNT(*) INTO v_count FROM inserted_chunks;

    PERFORM rebuild_resource_search_vector(p_resource_id);
    PERFORM set_config('temper.skip_search_rebuild', '', true);

    RETURN v_count;
END;
$$;

-- Update replace_resource_chunks to include heading_depth
CREATE OR REPLACE FUNCTION replace_resource_chunks(
    p_resource_id UUID,
    p_chunks      JSONB
) RETURNS INT
LANGUAGE plpgsql AS $$
DECLARE
    v_count INT;
BEGIN
    PERFORM set_config('temper.skip_search_rebuild', 'true', true);

    UPDATE kb_chunks
       SET is_current = false
     WHERE resource_id = p_resource_id
       AND is_current = true;

    WITH chunk_data AS (
        SELECT
            gen_random_uuid() AS chunk_id,
            p_resource_id AS resource_id,
            (elem->>'chunk_index')::INT AS chunk_index,
            COALESCE(elem->>'header_path', '') AS header_path,
            COALESCE((elem->>'heading_depth')::SMALLINT, 0) AS heading_depth,
            elem->>'content' AS content,
            elem->>'content_hash' AS content_hash,
            elem->>'embedding' AS embedding_str
        FROM jsonb_array_elements(p_chunks) AS elem
    ),
    inserted_chunks AS (
        INSERT INTO kb_chunks (
            id, resource_id, chunk_index, version, header_path, heading_depth,
            content_hash, embedding, is_current
        )
        SELECT
            cd.chunk_id, cd.resource_id, cd.chunk_index,
            COALESCE((SELECT MAX(version) FROM kb_chunks
                       WHERE resource_id = cd.resource_id
                         AND chunk_index = cd.chunk_index), 0) + 1,
            cd.header_path, cd.heading_depth,
            cd.content_hash, cd.embedding_str::vector, true
        FROM chunk_data cd
        RETURNING id, chunk_index
    ),
    inserted_content AS (
        INSERT INTO kb_chunk_content (chunk_id, content)
        SELECT cd.chunk_id, cd.content
        FROM chunk_data cd
        RETURNING chunk_id
    )
    SELECT COUNT(*) INTO v_count FROM inserted_chunks;

    PERFORM rebuild_resource_search_vector(p_resource_id);
    PERFORM set_config('temper.skip_search_rebuild', '', true);

    RETURN v_count;
END;
$$;
