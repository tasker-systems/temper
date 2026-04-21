-- UUID v7 portability: make `uuid_generate_v7()` callable on both
-- PostgreSQL 17 (Neon prod, via `pg_uuidv7` extension) and PostgreSQL 18
-- (local dev, native `uuidv7()`). Then redefine chunk-dedup functions to
-- call `uuid_generate_v7()` instead of the PG18-only `uuidv7()`.
--
-- Incident: 2026-04-21 prod `/api/ingest` failed with
--   `function uuidv7() does not exist`
-- because `20260420000006_chunk_dedup_functions.sql` hardcoded the native
-- PG18 name. Neon tops out at PG17 and exposes UUID v7 through the
-- `pg_uuidv7` extension, which provides `uuid_generate_v7()` but not
-- `uuidv7()`.
--
-- Pattern mirrors tasker-core's
-- `20250810140001_pgmq_extensions_and_headers.sql` compat shim.

DO $uuid_compat$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = 'pg_uuidv7') THEN
    CREATE EXTENSION IF NOT EXISTS pg_uuidv7 CASCADE;
  ELSE
    -- PG18+: native uuidv7() exists; create an alias so call sites can use
    -- the single portable name.
    IF NOT EXISTS (
      SELECT 1 FROM pg_proc
       WHERE proname = 'uuid_generate_v7'
         AND pronamespace = 'public'::regnamespace
    ) THEN
      CREATE OR REPLACE FUNCTION public.uuid_generate_v7() RETURNS uuid
      AS $fn$ SELECT uuidv7(); $fn$ LANGUAGE SQL VOLATILE PARALLEL SAFE;
    END IF;
  END IF;
END $uuid_compat$;

-- Redefine persist_resource_chunks to use the portable name.
-- Changes from 20260420000006:
--   * `uuidv7()` → `uuid_generate_v7()` (portability)
--   * chunk id: `gen_random_uuid()` → `uuid_generate_v7()` (btree locality —
--     chunks written in one batch land adjacent on the PK index)
CREATE OR REPLACE FUNCTION persist_resource_chunks(
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

    v_revision_id := uuid_generate_v7();
    INSERT INTO kb_resource_revisions (id, resource_id, audit_id, body_hash, chunk_count)
    VALUES (v_revision_id, p_resource_id, p_audit_id, p_body_hash, jsonb_array_length(p_chunks));

    WITH chunk_data AS (
        SELECT
            uuid_generate_v7() AS chunk_id,
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

-- Redefine replace_resource_chunks to use the portable name.
-- Changes from 20260420000006:
--   * `uuidv7()` → `uuid_generate_v7()` (portability)
--   * new chunk id: `gen_random_uuid()` → `uuid_generate_v7()` (btree locality)
CREATE OR REPLACE FUNCTION replace_resource_chunks(
    p_resource_id UUID,
    p_audit_id    UUID,
    p_body_hash   TEXT,
    p_chunks      JSONB
) RETURNS UUID
LANGUAGE plpgsql AS $$
DECLARE
    v_revision_id     UUID;
    v_latest_rev_id   UUID;
    v_latest_body_hash TEXT;
BEGIN
    SELECT id, body_hash
      INTO v_latest_rev_id, v_latest_body_hash
      FROM kb_resource_revisions
     WHERE resource_id = p_resource_id
     ORDER BY created DESC
     LIMIT 1;

    IF FOUND AND v_latest_body_hash = p_body_hash THEN
        RETURN v_latest_rev_id;
    END IF;

    PERFORM set_config('temper.skip_search_rebuild', 'true', true);

    v_revision_id := uuid_generate_v7();
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
            uuid_generate_v7(), p_resource_id, ti.chunk_index,
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
