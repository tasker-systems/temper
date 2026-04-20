-- Add dedup-aware + revision-linked overloads of persist_resource_chunks
-- and replace_resource_chunks. The new 4-arg signatures coexist with the
-- pre-existing 2-arg forms (PostgreSQL allows function overloading by
-- argument list). This lets Rust `sqlx::query!` macros compile against
-- cached 2-arg metadata while Task 4/5 migrate callers to the new form;
-- Task 5 drops the legacy 2-arg variants as its final step.
--
-- New signatures: (resource_id, audit_id, body_hash, chunks). Return
-- changes from INT (chunk count) to UUID (the new revision id).
--
-- Spec: docs/superpowers/specs/2026-04-20-chunk-dedup-and-revisions-design.md

-- First-create path. No existing chunks; just insert all at version 1.
CREATE FUNCTION persist_resource_chunks(
    p_resource_id UUID,
    p_audit_id    UUID,
    p_body_hash   TEXT,
    p_chunks      JSONB
) RETURNS UUID
LANGUAGE plpgsql AS $$
DECLARE
    v_revision_id UUID;
BEGIN
    PERFORM set_config('temper.skip_search_rebuild', 'true', true);

    v_revision_id := uuidv7();
    INSERT INTO kb_resource_revisions (id, resource_id, audit_id, body_hash, chunk_count)
    VALUES (v_revision_id, p_resource_id, p_audit_id, p_body_hash, jsonb_array_length(p_chunks));

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
            content_hash, embedding, is_current, first_revision_id
        )
        SELECT
            cd.chunk_id, cd.resource_id, cd.chunk_index, 1, cd.header_path, cd.heading_depth,
            cd.content_hash, cd.embedding_str::vector, true, v_revision_id
        FROM chunk_data cd
        RETURNING id
    )
    INSERT INTO kb_chunk_content (chunk_id, content)
    SELECT cd.chunk_id, cd.content FROM chunk_data cd;

    PERFORM rebuild_resource_search_vector(p_resource_id);
    PERFORM set_config('temper.skip_search_rebuild', '', true);

    RETURN v_revision_id;
END;
$$;

-- Update path. Dedup on (chunk_index, content_hash):
--   * Preserve rows that match input exactly — no write, first_revision_id stays.
--   * Supersede rows whose content_hash differs at same chunk_index, or whose
--     chunk_index is no longer present in input.
--   * Insert new rows for (chunk_index, content_hash) pairs not in current set.
CREATE FUNCTION replace_resource_chunks(
    p_resource_id UUID,
    p_audit_id    UUID,
    p_body_hash   TEXT,
    p_chunks      JSONB
) RETURNS UUID
LANGUAGE plpgsql AS $$
DECLARE
    v_revision_id UUID;
BEGIN
    PERFORM set_config('temper.skip_search_rebuild', 'true', true);

    v_revision_id := uuidv7();
    INSERT INTO kb_resource_revisions (id, resource_id, audit_id, body_hash, chunk_count)
    VALUES (v_revision_id, p_resource_id, p_audit_id, p_body_hash, jsonb_array_length(p_chunks));

    WITH incoming AS (
        SELECT
            (elem->>'chunk_index')::INT AS chunk_index,
            COALESCE(elem->>'header_path', '') AS header_path,
            COALESCE((elem->>'heading_depth')::SMALLINT, 0) AS heading_depth,
            elem->>'content' AS content,
            elem->>'content_hash' AS content_hash,
            elem->>'embedding' AS embedding_str
        FROM jsonb_array_elements(p_chunks) AS elem
    ),
    existing AS (
        SELECT id, chunk_index, content_hash
          FROM kb_chunks
         WHERE resource_id = p_resource_id
           AND is_current = true
    ),
    preserved AS (
        SELECT e.id
          FROM existing e
          JOIN incoming i
            ON i.chunk_index = e.chunk_index
           AND i.content_hash = e.content_hash
    ),
    to_supersede AS (
        SELECT e.id
          FROM existing e
         WHERE e.id NOT IN (SELECT id FROM preserved)
    ),
    superseded AS (
        UPDATE kb_chunks
           SET is_current = false,
               superseded_revision_id = v_revision_id
         WHERE id IN (SELECT id FROM to_supersede)
        RETURNING id
    ),
    to_insert AS (
        SELECT i.*
          FROM incoming i
         WHERE NOT EXISTS (
             SELECT 1 FROM existing e
              WHERE e.chunk_index = i.chunk_index
                AND e.content_hash = i.content_hash
         )
    ),
    inserted_chunks AS (
        INSERT INTO kb_chunks (
            id, resource_id, chunk_index, version, header_path, heading_depth,
            content_hash, embedding, is_current, first_revision_id
        )
        SELECT
            gen_random_uuid(), p_resource_id, ti.chunk_index,
            COALESCE((SELECT MAX(version) FROM kb_chunks
                       WHERE resource_id = p_resource_id
                         AND chunk_index = ti.chunk_index), 0) + 1,
            ti.header_path, ti.heading_depth,
            ti.content_hash, ti.embedding_str::vector, true, v_revision_id
          FROM to_insert ti
        RETURNING id, chunk_index
    )
    INSERT INTO kb_chunk_content (chunk_id, content)
    SELECT ic.id, ti.content
      FROM inserted_chunks ic
      JOIN to_insert ti ON ti.chunk_index = ic.chunk_index;

    PERFORM rebuild_resource_search_vector(p_resource_id);
    PERFORM set_config('temper.skip_search_rebuild', '', true);

    RETURN v_revision_id;
END;
$$;
