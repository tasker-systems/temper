-- Whole-body replace on the update path. The write-path blocking policy (20260905000010's
-- `resource_reblocked`, wired into the gated write path the same day) makes every multi-section
-- body land MULTI-block; a whole-body revise (`body` set, no `content_block` — the CLI/UI
-- default) must therefore REPLACE that partition, not address one block of it.
--
-- The Rust update arm resolves this to: mutate the FIRST live block with the full new chunk set
-- and `replaces_body = true`; this projector then folds every sibling live block BEFORE the
-- recomputes run. The folded rows keep their revisions and their `kb_block_provenance` rows —
-- ledger history is never lost; the new body's attribution is asserted fresh by this event alone
-- (carrying the old blocks' sources onto text that no longer contains them would fabricate).
-- Sibling chunk generations retire exactly as the target's own superseded generation always has.
--
-- TRUST BOUNDARY: `replaces_body` is set ONLY by the gated write path (temper-substrate
-- `update_resource_in_tx` derives it from the update's shape); no surface passes caller JSON to
-- this payload, and the reachability tripwire keeps the fold semantics fenced.
--
-- ADDITIVE: `replaces_body` is an optional payload key. Absent (every pre-existing event, and
-- every per-block revise) ⇒ coalesce false ⇒ byte-identical behavior to 20260714000002's
-- definition. The payload struct is serde-defaulted Rust-side; older events replay unchanged.
--
-- Version citations: body copied VERBATIM from the live definition (20260714000002
-- block_content_verbatim) with the ONE conditional block added — the same discipline
-- 20260715000030 used for `resource_finalize`. The tail ordering constraint
-- (`_recompute_body_storage` AFTER `_recompute_resource_body_hash`) is preserved:
-- 20260714000002:59-64.
CREATE OR REPLACE FUNCTION _project_block_mutated(p_event uuid, p_payload jsonb, p_content jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_block    uuid := (p_payload->>'block_id')::uuid;
        v_resource uuid;
        v_replaces boolean := coalesce((p_payload->>'replaces_body')::boolean, false);
        v_siblings uuid[];
        v_next_ver int;
        v_chunk_json jsonb; v_chunk uuid; v_side jsonb;
        v_chunk_hashes text := ''; v_chunk_count int := 0; v_block_hash text;
        v_revision uuid; v_blocks jsonb; v_raw jsonb;
BEGIN
    SELECT resource_id INTO v_resource FROM kb_content_blocks WHERE id = v_block;
    IF v_resource IS NULL THEN
        RAISE EXCEPTION '_project_block_mutated: block % not found', v_block;
    END IF;
    -- Whole-body replace: fold every sibling live block BEFORE the recomputes run, so the tail
    -- (_recompute_resource_body_hash → _recompute_body_storage → _rebuild_resource_search_vector)
    -- sees the final block set.
    IF v_replaces THEN
        SELECT coalesce(array_agg(id), '{}') INTO v_siblings
            FROM kb_content_blocks
           WHERE resource_id = v_resource AND id <> v_block AND NOT is_folded;
        UPDATE kb_content_blocks SET is_folded = true, last_event_id = p_event
         WHERE id = ANY(v_siblings);
        UPDATE kb_chunks SET is_current = false
         WHERE is_current AND block_id = ANY(v_siblings);
    END IF;
    -- Reserved key: the raw block bytes, keyed by block ID on the mutate path (the update path
    -- hardcodes seq 0 and addresses blocks by id). Absent ⇒ no bytes for this revision.
    v_blocks := coalesce(p_content->'__blocks', '{}'::jsonb);
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
        VALUES (v_block, v_block_hash, v_chunk_count, v_occurred)
        RETURNING id INTO v_revision;
    UPDATE kb_content_blocks SET last_event_id = p_event, current_revision_id = v_revision
        WHERE id = v_block;
    v_raw := v_blocks -> v_block::text;
    IF v_raw IS NOT NULL THEN
        INSERT INTO kb_block_content (block_revision_id, content, content_hash)
            VALUES (v_revision, v_raw->>'content', v_raw->>'content_hash');
    END IF;
    PERFORM _recompute_resource_body_hash(v_resource, v_occurred);
    PERFORM _recompute_body_storage(v_resource);
    PERFORM _rebuild_resource_search_vector(v_resource);
    -- provenance: accrete this revision's incorporation (empty ⇒ no-op) into kb_block_provenance.
    PERFORM _insert_block_provenance(v_block, p_event, p_payload->'incorporated');
    RETURN v_block;
END;
$$;

COMMENT ON FUNCTION _project_block_mutated(uuid, jsonb, jsonb) IS
    'Projects block_mutated: supersedes the target block''s chunk generation, writes the new '
    'revision (bytes via the __blocks sidecar keyed by block id), then recomputes body hash, '
    'body storage, and the search vector. With payload key replaces_body=true (the whole-body '
    'update arm) it first folds every sibling live block and retires their chunk generations — '
    'the revised text is the resource''s ENTIRE body. Folded rows keep revisions and provenance '
    'as history. Absent/false key: per-block revise, siblings untouched (pre-20260906000080 '
    'behavior). Amended by 20260906000080_block_mutate_whole_body.sql.';

SELECT declare_migration(
    20260906000080,
    'additive',
    'block_mutated gains the optional payload key replaces_body: the whole-body update arm folds '
    'sibling live blocks so a whole-body revise can REPLACE a policy-partitioned multi-block body. '
    'Absent key = pre-existing per-block behavior, byte-identical; no existing caller changes.'
);
