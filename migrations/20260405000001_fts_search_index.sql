-- =============================================================================
-- R10: Full-Text Search via tsvector — Schema, Triggers, Search Functions
-- =============================================================================
-- Adds: kb_resource_search_index table, GIN index, auto-update triggers,
--        rebuild_resource_search_vector(), fts_search(), unified_search(),
--        persist_resource_chunks(), replace_resource_chunks()
--
-- Enables MCP clients (and any non-embedding client) to search the knowledge
-- base using plain text queries. Vector search remains available when embeddings
-- are provided. Both result sets merge additively via unified_search().
--
-- Language: defaults to 'english'; per-resource override via search_config column.
--
-- TRIGGER GATING: All search triggers check the session variable
-- `temper.skip_search_rebuild`. When set to 'true' (via SET LOCAL within a
-- transaction), triggers become no-ops. This prevents O(n²) rebuilds during
-- batch chunk operations. The persist/replace_resource_chunks() functions
-- set this flag and call rebuild exactly once after all chunk writes complete.

-- ─── Search Index Table ──────────────────────────────────────────────────────

CREATE TABLE kb_resource_search_index (
    resource_id    UUID PRIMARY KEY REFERENCES kb_resources(id) ON DELETE CASCADE,
    search_vector  tsvector NOT NULL,
    search_config  VARCHAR(64) NOT NULL DEFAULT 'english',
    updated        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- GIN index with fastupdate=off for read-optimized performance.
-- Temper's write pattern is ingest-batch-then-search, so we trade slightly
-- slower writes for consistently fast reads with no pending-list merge cost.
CREATE INDEX idx_resource_search_vector
    ON kb_resource_search_index
    USING GIN (search_vector)
    WITH (fastupdate = off);

-- ─── Rebuild Function ────────────────────────────────────────────────────────
-- Rebuilds the search vector for a single resource by aggregating:
--   Weight A: title + slug (highest rank)
--   Weight B: all current chunk content (body text)
-- Uses the stored search_config for the resource, defaulting to 'english'.
-- Safe to call repeatedly (upsert via ON CONFLICT).

CREATE OR REPLACE FUNCTION rebuild_resource_search_vector(p_resource_id UUID)
RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    v_config VARCHAR(64);
    v_title TEXT;
    v_slug TEXT;
    v_body TEXT;
    v_vector tsvector;
BEGIN
    -- Get the current search config for this resource (or default)
    SELECT COALESCE(si.search_config, 'english')
      INTO v_config
      FROM kb_resource_search_index si
     WHERE si.resource_id = p_resource_id;

    v_config := COALESCE(v_config, 'english');

    -- Get resource metadata
    SELECT r.title, COALESCE(r.slug, '')
      INTO v_title, v_slug
      FROM kb_resources r
     WHERE r.id = p_resource_id;

    -- If resource doesn't exist, bail
    IF v_title IS NULL THEN
        RETURN;
    END IF;

    -- Aggregate all current chunk content for this resource
    SELECT COALESCE(string_agg(cc.content, ' '), '')
      INTO v_body
      FROM kb_chunks c
      JOIN kb_chunk_content cc ON cc.chunk_id = c.id
     WHERE c.resource_id = p_resource_id
       AND c.is_current = true;

    -- Build weighted tsvector
    v_vector :=
        setweight(to_tsvector(v_config::regconfig, COALESCE(v_title, '')), 'A') ||
        setweight(to_tsvector(v_config::regconfig, COALESCE(v_slug, '')), 'A') ||
        setweight(to_tsvector(v_config::regconfig, v_body), 'B');

    -- Upsert the search index row
    INSERT INTO kb_resource_search_index (resource_id, search_vector, search_config, updated)
    VALUES (p_resource_id, v_vector, v_config, now())
    ON CONFLICT (resource_id) DO UPDATE SET
        search_vector = EXCLUDED.search_vector,
        updated = now();
END;
$$;

-- ─── Trigger: kb_resources title/slug changes ────────────────────────────────
-- Gated: skips when temper.skip_search_rebuild = 'true' (batch mode).

CREATE OR REPLACE FUNCTION trg_resource_search_update()
RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    -- Skip during batch operations (persist/replace_resource_chunks sets this)
    IF current_setting('temper.skip_search_rebuild', true) = 'true' THEN
        RETURN NEW;
    END IF;

    IF TG_OP = 'INSERT'
       OR OLD.title IS DISTINCT FROM NEW.title
       OR OLD.slug IS DISTINCT FROM NEW.slug
    THEN
        PERFORM rebuild_resource_search_vector(NEW.id);
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER resource_search_update
    AFTER INSERT OR UPDATE OF title, slug ON kb_resources
    FOR EACH ROW
    EXECUTE FUNCTION trg_resource_search_update();

-- ─── Trigger: kb_chunk_content changes (body text) ───────────────────────────
-- Gated: skips when temper.skip_search_rebuild = 'true' (batch mode).

CREATE OR REPLACE FUNCTION trg_chunk_content_search_update()
RETURNS TRIGGER
LANGUAGE plpgsql AS $$
DECLARE
    v_resource_id UUID;
BEGIN
    -- Skip during batch operations
    IF current_setting('temper.skip_search_rebuild', true) = 'true' THEN
        IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
        RETURN NEW;
    END IF;

    IF TG_OP = 'DELETE' THEN
        SELECT c.resource_id INTO v_resource_id
          FROM kb_chunks c WHERE c.id = OLD.chunk_id;
    ELSE
        SELECT c.resource_id INTO v_resource_id
          FROM kb_chunks c WHERE c.id = NEW.chunk_id;
    END IF;

    IF v_resource_id IS NOT NULL THEN
        PERFORM rebuild_resource_search_vector(v_resource_id);
    END IF;

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER chunk_content_search_update
    AFTER INSERT OR UPDATE OR DELETE ON kb_chunk_content
    FOR EACH ROW
    EXECUTE FUNCTION trg_chunk_content_search_update();

-- ─── Trigger: kb_chunks.is_current changes (version rotation) ────────────────
-- Gated: skips when temper.skip_search_rebuild = 'true' (batch mode).

CREATE OR REPLACE FUNCTION trg_chunk_version_search_update()
RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    -- Skip during batch operations
    IF current_setting('temper.skip_search_rebuild', true) = 'true' THEN
        RETURN NEW;
    END IF;

    IF OLD.is_current IS DISTINCT FROM NEW.is_current THEN
        PERFORM rebuild_resource_search_vector(NEW.resource_id);
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER chunk_version_search_update
    AFTER UPDATE OF is_current ON kb_chunks
    FOR EACH ROW
    EXECUTE FUNCTION trg_chunk_version_search_update();

-- ─── Batch Chunk Persistence ─────────────────────────────────────────────────
-- persist_resource_chunks: Insert a full set of chunks for a resource in one
-- call. Accepts a JSONB array of chunk objects. Sets the trigger-gate flag,
-- does bulk INSERT via jsonb_array_elements, then rebuilds the search index
-- exactly once.
--
-- Each element in p_chunks must have:
--   chunk_index (int), header_path (text), content (text),
--   content_hash (text), embedding (text — pgvector literal "[0.1,0.2,...]")

CREATE OR REPLACE FUNCTION persist_resource_chunks(
    p_resource_id UUID,
    p_chunks      JSONB
) RETURNS INT
LANGUAGE plpgsql AS $$
DECLARE
    v_count INT;
BEGIN
    -- Gate triggers to prevent per-row search rebuilds
    PERFORM set_config('temper.skip_search_rebuild', 'true', true);

    -- Bulk insert into kb_chunks + kb_chunk_content
    WITH chunk_data AS (
        SELECT
            gen_random_uuid() AS chunk_id,
            p_resource_id AS resource_id,
            (elem->>'chunk_index')::INT AS chunk_index,
            COALESCE(elem->>'header_path', '') AS header_path,
            elem->>'content' AS content,
            elem->>'content_hash' AS content_hash,
            elem->>'embedding' AS embedding_str
        FROM jsonb_array_elements(p_chunks) AS elem
    ),
    inserted_chunks AS (
        INSERT INTO kb_chunks (
            id, resource_id, chunk_index, version, header_path,
            content_hash, embedding, is_current
        )
        SELECT
            cd.chunk_id, cd.resource_id, cd.chunk_index, 1, cd.header_path,
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

    -- Rebuild search index exactly once after all chunks are written
    PERFORM rebuild_resource_search_vector(p_resource_id);

    -- Reset the gate (SET LOCAL auto-resets on commit, but be explicit)
    PERFORM set_config('temper.skip_search_rebuild', '', true);

    RETURN v_count;
END;
$$;

-- replace_resource_chunks: Version-bump old chunks and insert new ones in a
-- single call. Used by the update path. Same trigger-gating pattern.
--
-- 1. Marks all current chunks as is_current = false (version bump)
-- 2. Inserts new chunks with version = max(existing version) + 1
-- 3. Rebuilds the search index once

CREATE OR REPLACE FUNCTION replace_resource_chunks(
    p_resource_id UUID,
    p_chunks      JSONB
) RETURNS INT
LANGUAGE plpgsql AS $$
DECLARE
    v_count INT;
BEGIN
    -- Gate triggers
    PERFORM set_config('temper.skip_search_rebuild', 'true', true);

    -- Version-bump all current chunks in one statement
    UPDATE kb_chunks
       SET is_current = false
     WHERE resource_id = p_resource_id
       AND is_current = true;

    -- Bulk insert new chunks with correct version numbers
    WITH chunk_data AS (
        SELECT
            gen_random_uuid() AS chunk_id,
            p_resource_id AS resource_id,
            (elem->>'chunk_index')::INT AS chunk_index,
            COALESCE(elem->>'header_path', '') AS header_path,
            elem->>'content' AS content,
            elem->>'content_hash' AS content_hash,
            elem->>'embedding' AS embedding_str
        FROM jsonb_array_elements(p_chunks) AS elem
    ),
    inserted_chunks AS (
        INSERT INTO kb_chunks (
            id, resource_id, chunk_index, version, header_path,
            content_hash, embedding, is_current
        )
        SELECT
            cd.chunk_id, cd.resource_id, cd.chunk_index,
            COALESCE((SELECT MAX(version) FROM kb_chunks
                       WHERE resource_id = cd.resource_id
                         AND chunk_index = cd.chunk_index), 0) + 1,
            cd.header_path,
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

    -- Rebuild search index exactly once
    PERFORM rebuild_resource_search_vector(p_resource_id);

    -- Reset gate
    PERFORM set_config('temper.skip_search_rebuild', '', true);

    RETURN v_count;
END;
$$;

-- ─── Full-Text Search Function ───────────────────────────────────────────────
-- Standalone FTS search, visibility-scoped via resources_visible_to().
-- Uses plainto_tsquery for natural language input (no special syntax needed).

CREATE FUNCTION fts_search(
    p_profile_id    UUID,
    p_query         TEXT,
    p_search_config VARCHAR DEFAULT 'english',
    p_context_name  VARCHAR DEFAULT NULL,
    p_doc_type      VARCHAR DEFAULT NULL,
    p_limit         INT DEFAULT 10,
    p_offset        INT DEFAULT 0
) RETURNS TABLE (
    resource_id  UUID,
    title        TEXT,
    slug         VARCHAR(256),
    kb_uri       TEXT,
    origin_uri   TEXT,
    context      VARCHAR(128),
    doc_type     VARCHAR(64),
    fts_rank     REAL
)
LANGUAGE SQL STABLE AS $$
    WITH
    visible AS (
        SELECT v.resource_id FROM resources_visible_to(p_profile_id) v
    ),
    query AS (
        SELECT plainto_tsquery(p_search_config::regconfig, p_query) AS q
    )
    SELECT
        r.id AS resource_id,
        r.title,
        r.slug,
        kb_resource_uri(r.id) AS kb_uri,
        r.origin_uri,
        ctx.name AS context,
        dt.name AS doc_type,
        ts_rank(si.search_vector, query.q)::REAL AS fts_rank
    FROM kb_resource_search_index si
    JOIN kb_resources r ON r.id = si.resource_id
    JOIN visible v ON v.resource_id = r.id
    LEFT JOIN kb_contexts ctx ON r.kb_context_id = ctx.id
    JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
    CROSS JOIN query
    WHERE r.is_active = true
      AND si.search_vector @@ query.q
      AND (p_context_name IS NULL OR ctx.name = p_context_name)
      AND (p_doc_type IS NULL OR dt.name = p_doc_type)
    ORDER BY fts_rank DESC
    LIMIT p_limit
    OFFSET p_offset
$$;

-- ─── Unified Search Function ─────────────────────────────────────────────────
-- Combines FTS and vector search. Either or both inputs can be provided.
-- Results are merged via FULL OUTER JOIN with configurable weights.
-- Designed for future extension: knowledge graph traversal (R7) will add
-- a third stage between vec_hits and combined.

CREATE FUNCTION unified_search(
    p_profile_id     UUID,
    p_query          TEXT DEFAULT '',
    p_embedding      vector(768) DEFAULT NULL,
    p_search_config  VARCHAR DEFAULT 'english',
    p_context_name   VARCHAR DEFAULT NULL,
    p_doc_type       VARCHAR DEFAULT NULL,
    p_fts_weight     FLOAT DEFAULT 0.5,
    p_vec_weight     FLOAT DEFAULT 0.5,
    p_limit          INT DEFAULT 10,
    p_offset         INT DEFAULT 0
) RETURNS TABLE (
    resource_id    UUID,
    title          TEXT,
    slug           VARCHAR(256),
    kb_uri         TEXT,
    origin_uri     TEXT,
    context        VARCHAR(128),
    doc_type       VARCHAR(64),
    fts_score      REAL,
    vector_score   REAL,
    combined_score REAL,
    origin         VARCHAR(16)
)
LANGUAGE SQL STABLE AS $$
    WITH
    visible AS (
        SELECT v.resource_id FROM resources_visible_to(p_profile_id) v
    ),

    -- Stage 1: Full-text search (runs when p_query is non-empty)
    fts_hits AS (
        SELECT
            r.id AS resource_id,
            ts_rank(si.search_vector, plainto_tsquery(p_search_config::regconfig, p_query))::REAL AS score
        FROM kb_resource_search_index si
        JOIN kb_resources r ON r.id = si.resource_id
        JOIN visible v ON v.resource_id = r.id
        LEFT JOIN kb_contexts ctx ON r.kb_context_id = ctx.id
        JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
        WHERE p_query != ''
          AND r.is_active = true
          AND si.search_vector @@ plainto_tsquery(p_search_config::regconfig, p_query)
          AND (p_context_name IS NULL OR ctx.name = p_context_name)
          AND (p_doc_type IS NULL OR dt.name = p_doc_type)
    ),

    -- Stage 2: Vector search (runs when p_embedding is provided)
    vec_hits AS (
        SELECT
            c.resource_id,
            MAX((1.0 - (c.embedding <=> p_embedding))::REAL) AS score
        FROM kb_current_chunks c
        JOIN visible v ON v.resource_id = c.resource_id
        JOIN kb_resources r ON r.id = c.resource_id
        LEFT JOIN kb_contexts ctx ON r.kb_context_id = ctx.id
        JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
        WHERE p_embedding IS NOT NULL
          AND r.is_active = true
          AND (p_context_name IS NULL OR ctx.name = p_context_name)
          AND (p_doc_type IS NULL OR dt.name = p_doc_type)
        GROUP BY c.resource_id
        ORDER BY MIN(c.embedding <=> p_embedding)
        LIMIT 50
    ),

    -- Stage 3: Combine (distinct union, additive scoring)
    combined AS (
        SELECT
            COALESCE(f.resource_id, ve.resource_id) AS resource_id,
            COALESCE(f.score, 0.0)::REAL AS fts_score,
            COALESCE(ve.score, 0.0)::REAL AS vector_score,
            (p_fts_weight * COALESCE(f.score, 0.0)
             + p_vec_weight * COALESCE(ve.score, 0.0))::REAL AS combined_score,
            CASE
                WHEN f.resource_id IS NOT NULL AND ve.resource_id IS NOT NULL THEN 'both'
                WHEN f.resource_id IS NOT NULL THEN 'fts'
                ELSE 'vector'
            END AS origin
        FROM fts_hits f
        FULL OUTER JOIN vec_hits ve ON ve.resource_id = f.resource_id
    )

    SELECT
        c.resource_id,
        r.title,
        r.slug,
        kb_resource_uri(r.id) AS kb_uri,
        r.origin_uri,
        ctx.name AS context,
        dt.name AS doc_type,
        c.fts_score,
        c.vector_score,
        c.combined_score,
        c.origin::VARCHAR(16)
    FROM combined c
    JOIN kb_resources r ON r.id = c.resource_id
    LEFT JOIN kb_contexts ctx ON r.kb_context_id = ctx.id
    JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
    ORDER BY c.combined_score DESC
    LIMIT p_limit
    OFFSET p_offset
$$;
