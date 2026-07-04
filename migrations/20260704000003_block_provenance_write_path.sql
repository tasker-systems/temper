-- T7a — un-stub the block-provenance write path.
--
-- `kb_block_provenance` has been schema-ready (table + read joins in resource_blocks /
-- cogmap_region_reference_standing) but nothing ever wrote it: neither projector read the payload's
-- `incorporated` list. This migration teaches BOTH projectors to record it, activating those already-
-- wired read + region-salience signals (which returned zero until now).
--
-- Additive (CREATE OR REPLACE of the two projector functions; a new helper + read fn) — never edits a
-- birth migration. `provenance_source_kind` stays ('event','resource'); the 'remote' value is T7c.
-- Replay-stable + idempotent: `_insert_block_provenance` is a pure function of (p_event, payload) and
-- ON CONFLICT-no-ops, so fire and replay produce identical kb_block_provenance rows.

-- ── shared source-INSERT helper (mirrors _insert_chunk's role) ────────────────
-- One row per Incorporation ({source:{kind,value}, seq}). accretion_seq = the caller-declared `seq`
-- (from the immutable payload, so replay-stable); contributed_by_event_id = the event that added it.
-- Idempotent on the DDL's UNIQUE (block_id, source_kind, source_id, contributed_by_event_id).
CREATE FUNCTION _insert_block_provenance(p_block uuid, p_event uuid, p_incorporated jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_inc jsonb;
BEGIN
    IF p_incorporated IS NULL OR jsonb_typeof(p_incorporated) <> 'array' THEN
        RETURN;
    END IF;
    FOR v_inc IN SELECT jsonb_array_elements(p_incorporated) LOOP
        INSERT INTO kb_block_provenance
            (block_id, source_kind, source_id, contributed_by_event_id, accretion_seq)
        VALUES (
            p_block,
            (v_inc #>> '{source,kind}')::provenance_source_kind,
            (v_inc #>> '{source,value}')::uuid,
            p_event,
            (v_inc ->> 'seq')::int
        )
        ON CONFLICT (block_id, source_kind, source_id, contributed_by_event_id) DO NOTHING;
    END LOOP;
END;
$$;

-- ── _project_blocks: create path — record per-block incorporation ─────────────
-- Body copied verbatim from the LIVE definition (the FTS override 20260626000001_fts_search_index.sql:42,
-- which adds the _rebuild_resource_search_vector beat the pre-FTS canonical lacks — copying the older
-- body here would silently REVERT create-path FTS), with ONE added call inside the block loop (after
-- the kb_block_revisions INSERT).
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
    -- provenance: accrete this revision's incorporation (empty ⇒ no-op) into kb_block_provenance.  ← NEW
    PERFORM _insert_block_provenance(v_block, p_event, p_payload->'incorporated');
    RETURN v_block;
END;
$$;

-- ── read surface: itemized per-block provenance for a resource ────────────────
-- resource_blocks(...) already exposes the AGGREGATE reinforce_count; this returns the individual
-- source rows the read tool surfaces. Access-gated (resources_readable_by), corrected rows excluded.
CREATE FUNCTION resource_block_provenance(
    p_resource uuid, p_principal_kind text, p_principal_id uuid
) RETURNS TABLE(block_id uuid, block_seq int, source_kind text, source_id uuid,
                accretion_seq int, contributed_by_event_id uuid, created timestamptz)
LANGUAGE sql STABLE AS $$
    SELECT b.id, b.seq, p.source_kind::text, p.source_id, p.accretion_seq,
           p.contributed_by_event_id, p.created
    FROM kb_content_blocks b
    JOIN kb_block_provenance p ON p.block_id = b.id AND NOT p.is_corrected
    WHERE b.resource_id = p_resource AND NOT b.is_folded
      AND p_resource IN (SELECT resource_id FROM resources_readable_by(p_principal_kind, p_principal_id))
    ORDER BY b.seq, p.accretion_seq;
$$;
