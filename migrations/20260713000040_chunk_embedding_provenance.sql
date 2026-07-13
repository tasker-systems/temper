-- Embedding provenance: record WHICH MODEL produced each stored vector.
--
-- Nothing recorded this, and so nothing could tell two models apart. The CLI embedded with the fp32
-- model while the server embedded with the LFS-pinned quantized one, and the semantic index filled
-- with vectors from two different geometries — invisibly, for months. Measured against prod: ~95% of
-- stored vectors were fp32, and the only way to find that out was to re-embed sampled content and
-- compare cosines, because the database itself had no idea.
--
-- `embedded_with` is the sha256 of the ONNX model that produced the row's `embedding` — the SAME
-- identity the application pins at compile time (`temper_ingest::embed::EXPECTED_MODEL_SHA256`,
-- derived by build.rs from the model's git-lfs oid). So "is this vector current?" becomes a plain
-- equality test rather than an archaeology project.
--
-- It is also the **dirty flag for re-embedding**, which is why it earns a column:
--
--     dirty  ⇔  embedding IS NULL  OR  embedded_with IS DISTINCT FROM <current model sha>
--
-- Self-healing by construction. Every existing row is unstamped (NULL) ⇒ dirty ⇒ re-embedded, which
-- is correct: every one of them was produced by a model we can no longer vouch for. And any FUTURE
-- model change re-dirties the index automatically — no backfill script, no migration, ever again.
-- The drain (`/api/embed/dispatch`, already running every minute) does the work.
--
-- NOTE this is a *flag*, not a deletion. The stale vector stays in place and stays searchable until a
-- fresh one replaces it. Marking dirty by NULLing `embedding` would have made 31.8k chunks
-- unfindable for the duration of the drain, with no way back if the drain misbehaved.

ALTER TABLE kb_chunks
    ADD COLUMN embedded_with text;

COMMENT ON COLUMN kb_chunks.embedded_with IS
    'sha256 of the ONNX model that produced `embedding`. NULL = unknown provenance (pre-dates this column, or a client that did not declare its model) => treated as STALE and re-embedded. Compared against temper_ingest::embed::EXPECTED_MODEL_SHA256.';

-- Supports the drain's two questions: "which resources in <scope> hold a stale chunk?" and, per
-- claimed resource, "which of its chunks are stale?". Partial on is_current because superseded chunk
-- versions are never re-embedded — only live ones are readable.
CREATE INDEX idx_kb_chunks_embed_provenance
    ON kb_chunks (resource_id, embedded_with)
    WHERE is_current;

-- ---------------------------------------------------------------------------
-- Thread the model identity through the write path.
--
-- `_insert_chunk` is called ONLY from other SQL functions (_project_blocks, _project_block_mutated),
-- never from application code — so widening its signature causes NO deploy skew: the app-facing entry
-- points (block_append / block_mutated / resource_created) keep their signatures exactly. The new
-- parameter is defaulted regardless, so an un-updated caller still resolves.
--
-- The value rides in the content sidecar the callers already destructure for `content` /
-- `header_path` / `heading_depth` — one more key, no new plumbing.
-- ---------------------------------------------------------------------------

DROP FUNCTION IF EXISTS _insert_chunk(uuid, uuid, uuid, int, int, text, jsonb, boolean, text, text, smallint, timestamptz);

CREATE FUNCTION _insert_chunk(p_chunk uuid, p_block uuid, p_resource uuid, p_chunk_index int,
                              p_version int, p_content_hash text, p_emb jsonb, p_is_current boolean,
                              p_content text, p_header_path text, p_heading_depth smallint,
                              p_occurred timestamptz, p_embedded_with text DEFAULT NULL)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    -- header_path/heading_depth are production render metadata (§8 carry-as-is): persisted verbatim so
    -- a downstream read reconstructs headed markdown identically to production. They are NOT inputs to
    -- content_hash or the body_hash merkle (those stay over the chunk content hashes only). NULL on the
    -- scenario-authoring path (no production headings), exactly as before this carry existed.
    --
    -- p_embedded_with is the model sha256 that the PRODUCER OF THIS VECTOR declared. A producer that
    -- declares nothing gets NULL — which reads as "unknown provenance", hence STALE, hence re-embedded
    -- server-side rather than trusted. That is deliberate: an old CLI still POSTing fp32 vectors must
    -- not be able to pass them off as current simply by staying quiet.
    INSERT INTO kb_chunks (id, block_id, resource_id, chunk_index, version, content_hash,
                           embedding, is_current, header_path, heading_depth, created, embedded_with)
        VALUES (p_chunk, p_block, p_resource, p_chunk_index, p_version, p_content_hash,
                CASE
                    WHEN p_emb IS NULL OR jsonb_typeof(p_emb) = 'null' THEN NULL
                    WHEN jsonb_typeof(p_emb) = 'string' THEN (p_emb #>> '{}')::vector  -- replay: pgvector text
                    ELSE (p_emb::text)::vector                                          -- fire: JSON array
                END,
                p_is_current, p_header_path, p_heading_depth, p_occurred,
                -- A chunk with no vector has no provenance to record; keep the columns coherent so
                -- `embedding IS NULL` and `embedded_with IS NULL` can never disagree.
                CASE WHEN p_emb IS NULL OR jsonb_typeof(p_emb) = 'null' THEN NULL
                     ELSE p_embedded_with END);
    INSERT INTO kb_chunk_content (chunk_id, content) VALUES (p_chunk, p_content);
END;
$$;

-- ---------------------------------------------------------------------------
-- Re-point the two projectors at the widened writer.
--
-- Bodies below are copied VERBATIM from the live definitions (20260704000003_block_provenance_write_path.sql,
-- which itself carries the FTS beat from 20260626000001). The ONLY edit is the extra `embedded_with`
-- argument on each _insert_chunk call — the kb_block_revisions writes, the _rebuild_resource_search_vector
-- calls, and the provenance accretion are all preserved exactly. Re-deriving these bodies by hand is how
-- you silently delete the search index; they were machine-patched, then diffed.
-- ---------------------------------------------------------------------------

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
                                  NULLIF(v_side->>'heading_depth','')::smallint, v_occurred,
                                  v_side->>'embedded_with');
            v_chunk_hashes := v_chunk_hashes || (v_chunk_json->>'content_hash');
            v_chunk_count := v_chunk_count + 1;
        END LOOP;
        v_block_hash := encode(sha256(convert_to(v_chunk_hashes, 'UTF8')), 'hex');
        INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count, created)
            VALUES (v_block, v_block_hash, v_chunk_count, v_occurred);
        -- provenance: record the block's incorporation (empty ⇒ no-op) into kb_block_provenance.  ← NEW
        PERFORM _insert_block_provenance(v_block, p_event, v_block_json->'incorporated');
    END LOOP;
    PERFORM _recompute_resource_body_hash(p_resource, v_occurred);
    PERFORM _rebuild_resource_search_vector(p_resource);   -- ← Beat 1
END;
$$;

-- ── _project_block_mutated: revise path — accrete incorporation ───────────────
-- Body copied verbatim from the LIVE definition (the FTS override 20260626000001_fts_search_index.sql:85,
-- which has the _rebuild_resource_search_vector beat the canonical one lacks), with ONE added call
-- before RETURN.
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
                              NULLIF(v_side->>'heading_depth','')::smallint, v_occurred,
                              v_side->>'embedded_with');
        v_chunk_hashes := v_chunk_hashes || (v_chunk_json->>'content_hash');
        v_chunk_count := v_chunk_count + 1;
    END LOOP;
    v_block_hash := encode(sha256(convert_to(v_chunk_hashes, 'UTF8')), 'hex');
    INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count, created)
        VALUES (v_block, v_block_hash, v_chunk_count, v_occurred);
    UPDATE kb_content_blocks SET last_event_id = p_event WHERE id = v_block;
    PERFORM _recompute_resource_body_hash(v_resource, v_occurred);
    PERFORM _rebuild_resource_search_vector(v_resource);   -- ← Beat 1
    -- provenance: accrete this revision's incorporation (empty ⇒ no-op) into kb_block_provenance.  ← NEW
    PERFORM _insert_block_provenance(v_block, p_event, p_payload->'incorporated');
    RETURN v_block;
END;
$$;
