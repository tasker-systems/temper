-- Verbatim block content: store the RAW source bytes of each block revision, so an ingest can be
-- read back byte-for-byte (`sha256(PUT) == sha256(GET)`) instead of reconstructed from the lossy
-- chunk transform.
--
-- W2 PR 3 of the honest-ingest arc. PR 2 made the segmenter byte-preserving; this stores those bytes.
-- PR 4 reads them back (a HAVING-verified coverage check); PR 5 adds a finalize integrity check.
--
-- ── The three shapes ──────────────────────────────────────────────────────────
--   kb_block_content            — one row per block REVISION holding its raw bytes + their sha256.
--                                 Keyed by block_revision_id so a superseded revision keeps its own
--                                 bytes (revise history is preserved, exactly like kb_chunks versions).
--   kb_content_blocks.current_revision_id — the block's live revision, so a reader joins content
--                                 without re-deriving "which revision is current" from kb_chunks.
--   kb_resources.body_storage    — a SURFACED SIGNAL: 'verbatim' iff every live block of the resource
--                                 carries bytes, else 'derived'. NOT a read-path branch (PR 4 verifies
--                                 coverage in the read query itself); this column exists so `show` can
--                                 answer "what guarantee does this body carry?" without that query.
--
-- ── Skew safety (additive both directions, no deploy split) ────────────────────
-- The raw bytes ride the transient content sidecar (`p_content`) under a reserved `__blocks` key,
-- EXTENDING the flat `{chunk_id: {...}}` map rather than reshaping it. The projectors read the sidecar
-- ONLY by keyed lookup (`p_content->(chunk_id)`); nothing iterates it (no jsonb_each/jsonb_object_keys
-- anywhere in migrations/). So:
--   • new app + old DB — old projector never looks up `__blocks` ⇒ no bytes ⇒ honestly 'derived'.
--   • old app + new DB — `p_content->'__blocks'` is NULL ⇒ no bytes ⇒ honestly 'derived'.
-- Key collision is impossible: every other sidecar key is a chunk UUID.
--
-- `ADD COLUMN … NOT NULL DEFAULT` is catalog-only on PG11+ — no table rewrite on PG17 (Neon prod) or
-- PG18 (local/CI). No backfill: every existing resource keeps `body_storage = 'derived'` (its blocks
-- have no bytes and NULL current_revision_id) and reads back through the unchanged reconstruction path.

CREATE TABLE kb_block_content (
    block_revision_id uuid PRIMARY KEY
        REFERENCES kb_block_revisions(id) ON DELETE CASCADE,
    content      text NOT NULL,
    content_hash text NOT NULL   -- bare sha256 hex of content's raw bytes (Rust `sha256_hex` twin)
);

ALTER TABLE kb_content_blocks
    ADD COLUMN current_revision_id uuid REFERENCES kb_block_revisions(id);

ALTER TABLE kb_resources
    ADD COLUMN body_storage text NOT NULL DEFAULT 'derived';
ALTER TABLE kb_resources
    ADD CONSTRAINT ck_kb_resources_body_storage
        CHECK (body_storage IN ('verbatim', 'derived'));

-- body_storage is DERIVED from coverage, never asserted. A resource is 'verbatim' iff EVERY live
-- (non-folded) block has a current revision carrying bytes. The mixed-coverage case (some blocks with
-- bytes, some without) is 'derived' — which is the whole point: a per-resource flag set to 'verbatim'
-- over partial coverage would let an INNER-JOIN readback skip the content-less blocks and return a
-- short body that looks complete. Recomputed at the tail of both projectors.
-- `count(*) > 0` is load-bearing: a resource with ZERO live blocks (an empty telos born by genesis,
-- before any charter_set) would otherwise satisfy `0 = 0` and be vacuously 'verbatim' — a body_storage
-- that claims a byte-exact guarantee over no bytes. It must be 'derived'. (An empty-manifest
-- `_project_blocks` recomputes this on one code path and not another, so the vacuous case also breaks
-- replay equivalence — the empty resource must land 'derived' on every path.)
CREATE FUNCTION _recompute_body_storage(p_resource uuid) RETURNS void
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE kb_resources r SET body_storage = CASE WHEN (
        SELECT count(*) > 0 AND count(*) = count(bc.block_revision_id)
          FROM kb_content_blocks b
          LEFT JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id
         WHERE b.resource_id = p_resource AND NOT b.is_folded
    ) THEN 'verbatim' ELSE 'derived' END
    WHERE r.id = p_resource;
END; $$;

-- ── Re-point the two content projectors at verbatim storage ────────────────────
-- Bodies re-derived VERBATIM from the LIVE definitions in
-- 20260713000040_chunk_embedding_provenance.sql (the embedding-provenance beat). The ONLY edits:
--   • capture the new revision id (RETURNING … INTO v_revision) and set it as the block's
--     current_revision_id,
--   • store the block's raw bytes from `p_content->'__blocks'` (absent ⇒ legacy caller ⇒ no bytes),
--   • recompute body_storage at the tail.
-- Every _insert_chunk call, the block-revision write, the search-vector rebuild, and the provenance
-- accretion are preserved EXACTLY. Re-deriving these by hand from an older migration is how you
-- silently revert the embedding-provenance carry; they were copied from the live body, then diffed.

CREATE OR REPLACE FUNCTION _project_blocks(p_resource uuid, p_event uuid, p_manifests jsonb, p_content jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_block uuid; v_chunk uuid;
    v_block_json jsonb; v_chunk_json jsonb; v_side jsonb;
    v_block_hash text; v_chunk_hashes text; v_chunk_count int;
    v_revision uuid; v_blocks jsonb; v_raw jsonb;
    v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    -- Reserved key: the raw block bytes, keyed by block SEQ on the create/append path. Absent for a
    -- legacy/derived caller ⇒ '{}' ⇒ no block gets bytes.
    v_blocks := coalesce(p_content->'__blocks', '{}'::jsonb);
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
            VALUES (v_block, v_block_hash, v_chunk_count, v_occurred)
            RETURNING id INTO v_revision;
        UPDATE kb_content_blocks SET current_revision_id = v_revision WHERE id = v_block;
        -- verbatim bytes for this block (keyed by seq), when the caller supplied them.
        v_raw := v_blocks -> (v_block_json->>'seq');
        IF v_raw IS NOT NULL THEN
            INSERT INTO kb_block_content (block_revision_id, content, content_hash)
                VALUES (v_revision, v_raw->>'content', v_raw->>'content_hash');
        END IF;
        -- provenance: record the block's incorporation (empty ⇒ no-op) into kb_block_provenance.
        PERFORM _insert_block_provenance(v_block, p_event, v_block_json->'incorporated');
    END LOOP;
    PERFORM _recompute_resource_body_hash(p_resource, v_occurred);
    PERFORM _recompute_body_storage(p_resource);
    PERFORM _rebuild_resource_search_vector(p_resource);
END;
$$;

CREATE OR REPLACE FUNCTION _project_block_mutated(p_event uuid, p_payload jsonb, p_content jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_block    uuid := (p_payload->>'block_id')::uuid;
        v_resource uuid;
        v_next_ver int;
        v_chunk_json jsonb; v_chunk uuid; v_side jsonb;
        v_chunk_hashes text := ''; v_chunk_count int := 0; v_block_hash text;
        v_revision uuid; v_blocks jsonb; v_raw jsonb;
BEGIN
    SELECT resource_id INTO v_resource FROM kb_content_blocks WHERE id = v_block;
    IF v_resource IS NULL THEN
        RAISE EXCEPTION '_project_block_mutated: block % not found', v_block;
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
