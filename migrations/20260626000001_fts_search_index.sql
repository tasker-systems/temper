-- =============================================================================
-- Search Beat 1: stored full-text index (kb_resource_search_index) + GIN.
-- Maintained by the canonical _project_* projection functions (NOT triggers).
-- Behavior-preserving: title@A + body@B, body = string_agg current-chunk content,
-- config 'english' — the exact recipe readback::fts_search built inline.
-- Additive-only-on-main: new table/index/function + CREATE OR REPLACE + backfill.
-- =============================================================================

CREATE TABLE kb_resource_search_index (
    resource_id    UUID PRIMARY KEY REFERENCES kb_resources(id) ON DELETE CASCADE,
    search_vector  tsvector NOT NULL,
    search_config  VARCHAR(64) NOT NULL DEFAULT 'english',
    updated        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_resource_search_vector
    ON kb_resource_search_index USING GIN (search_vector) WITH (fastupdate = off);

-- Rebuild a resource's stored vector: title@A + body@B. Idempotent upsert.
CREATE FUNCTION _rebuild_resource_search_vector(p_resource uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_config varchar(64); v_title text; v_body text;
BEGIN
    SELECT COALESCE((SELECT search_config FROM kb_resource_search_index WHERE resource_id = p_resource),
                    'english') INTO v_config;
    SELECT title INTO v_title FROM kb_resources WHERE id = p_resource;
    IF v_title IS NULL THEN RETURN; END IF;
    SELECT COALESCE(string_agg(cc.content, ' '), '')
      INTO v_body
      FROM kb_chunks c JOIN kb_chunk_content cc ON cc.chunk_id = c.id
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

-- ── _project_blocks: + rebuild after body-hash recompute ─────────────────────
CREATE OR REPLACE FUNCTION _project_blocks(p_resource uuid, p_event uuid, p_manifests jsonb, p_content jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_block uuid; v_chunk uuid;
    v_block_json jsonb; v_chunk_json jsonb; v_side jsonb;
    v_block_hash text; v_chunk_hashes text; v_chunk_count int;
    v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    FOR v_block_json IN SELECT jsonb_array_elements(p_manifests) LOOP
        v_block := (v_block_json->>'block_id')::uuid;
        INSERT INTO kb_content_blocks (id, resource_id, seq, genesis_event_id, last_event_id, created)
            VALUES (v_block, p_resource, (v_block_json->>'seq')::int, p_event, p_event, v_occurred);
        IF v_block_json ? 'role' AND jsonb_typeof(v_block_json->'role') = 'string' THEN
            INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value,
                                       asserted_by_event_id, last_event_id, created)
            VALUES ('kb_content_blocks', v_block, 'block_role', v_block_json->'role',
                    p_event, p_event, v_occurred);
        END IF;
        v_chunk_hashes := '';
        v_chunk_count := 0;
        FOR v_chunk_json IN SELECT jsonb_array_elements(v_block_json->'chunks') LOOP
            v_chunk := (v_chunk_json->>'chunk_id')::uuid;
            v_side := p_content->(v_chunk_json->>'chunk_id');
            IF v_side IS NULL THEN
                RAISE EXCEPTION '_project_blocks: content sidecar missing chunk %', v_chunk;
            END IF;
            PERFORM _insert_chunk(v_chunk, v_block, p_resource, (v_chunk_json->>'chunk_index')::int,
                                  1, v_chunk_json->>'content_hash', v_side->'embedding', true,
                                  v_side->>'content', v_side->>'header_path',
                                  NULLIF(v_side->>'heading_depth','')::smallint, v_occurred);
            v_chunk_hashes := v_chunk_hashes || (v_chunk_json->>'content_hash');
            v_chunk_count := v_chunk_count + 1;
        END LOOP;
        v_block_hash := encode(sha256(convert_to(v_chunk_hashes, 'UTF8')), 'hex');
        INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count, created)
            VALUES (v_block, v_block_hash, v_chunk_count, v_occurred);
    END LOOP;
    PERFORM _recompute_resource_body_hash(p_resource, v_occurred);
    PERFORM _rebuild_resource_search_vector(p_resource);   -- ← Beat 1
END;
$$;

-- ── _project_block_mutated: + rebuild after body-hash recompute ──────────────
CREATE OR REPLACE FUNCTION _project_block_mutated(p_event uuid, p_payload jsonb, p_content jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_block    uuid := (p_payload->>'block_id')::uuid;
        v_resource uuid;
        v_next_ver int;
        v_chunk_json jsonb; v_chunk uuid; v_side jsonb;
        v_chunk_hashes text := ''; v_chunk_count int := 0; v_block_hash text;
BEGIN
    SELECT resource_id INTO v_resource FROM kb_content_blocks WHERE id = v_block;
    IF v_resource IS NULL THEN
        RAISE EXCEPTION '_project_block_mutated: block % not found', v_block;
    END IF;
    UPDATE kb_chunks SET is_current = false WHERE block_id = v_block AND is_current;
    SELECT coalesce(max(version), 0) + 1 INTO v_next_ver FROM kb_chunks WHERE block_id = v_block;
    FOR v_chunk_json IN SELECT jsonb_array_elements(p_payload->'chunks') LOOP
        v_chunk := (v_chunk_json->>'chunk_id')::uuid;
        v_side  := p_content->(v_chunk_json->>'chunk_id');
        IF v_side IS NULL THEN
            RAISE EXCEPTION '_project_block_mutated: content sidecar missing chunk %', v_chunk;
        END IF;
        PERFORM _insert_chunk(v_chunk, v_block, v_resource, (v_chunk_json->>'chunk_index')::int,
                              v_next_ver, v_chunk_json->>'content_hash', v_side->'embedding', true,
                              v_side->>'content', v_side->>'header_path',
                              NULLIF(v_side->>'heading_depth','')::smallint, v_occurred);
        v_chunk_hashes := v_chunk_hashes || (v_chunk_json->>'content_hash');
        v_chunk_count := v_chunk_count + 1;
    END LOOP;
    v_block_hash := encode(sha256(convert_to(v_chunk_hashes, 'UTF8')), 'hex');
    INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count, created)
        VALUES (v_block, v_block_hash, v_chunk_count, v_occurred);
    UPDATE kb_content_blocks SET last_event_id = p_event WHERE id = v_block;
    PERFORM _recompute_resource_body_hash(v_resource, v_occurred);
    PERFORM _rebuild_resource_search_vector(v_resource);   -- ← Beat 1
    RETURN v_block;
END;
$$;

-- ── _project_resource_updated: + rebuild when the title key is present ────────
CREATE OR REPLACE FUNCTION _project_resource_updated(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_resource uuid := (p_payload->>'resource_id')::uuid;
BEGIN
    UPDATE kb_resources SET
        title      = COALESCE(p_payload->>'title', title),
        origin_uri = COALESCE(p_payload->>'origin_uri', origin_uri),
        updated    = (SELECT occurred_at FROM kb_events WHERE id = p_event)
        WHERE id = v_resource;
    IF NOT FOUND THEN RAISE EXCEPTION 'resource_update: resource % not found', v_resource; END IF;
    IF p_payload ? 'title' THEN                            -- ← Beat 1 (origin_uri is not in the FTS vector)
        PERFORM _rebuild_resource_search_vector(v_resource);
    END IF;
    RETURN v_resource;
END;
$$;

-- ── Backfill every active resource (idempotent upsert) ───────────────────────
INSERT INTO kb_resource_search_index (resource_id, search_vector, search_config, updated)
SELECT r.id,
       setweight(to_tsvector('english', COALESCE(r.title,'')), 'A')
         || setweight(to_tsvector('english', COALESCE(b.body,'')), 'B'),
       'english', now()
FROM kb_resources r
LEFT JOIN LATERAL (
    SELECT string_agg(cc.content, ' ') AS body
      FROM kb_chunks c JOIN kb_chunk_content cc ON cc.chunk_id = c.id
     WHERE c.resource_id = r.id AND c.is_current
) b ON true
WHERE r.is_active
ON CONFLICT (resource_id) DO UPDATE
    SET search_vector = EXCLUDED.search_vector, updated = now();
