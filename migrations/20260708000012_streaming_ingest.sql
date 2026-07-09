-- Streaming/segmented ingestion: activate the dormant `block_created` event per
-- appended segment, add a `resource_finalized` completion event, and provide the
-- idempotent block_append + validating resource_finalize functions. Additive:
-- block 0 still lands via resource_created; segments 1..N append here.

-- `block_created` is already seeded (canonical_seed.sql) with its minimal
-- {block_id, resource_id, seq} contract. Seed only the new completion type,
-- permissive (NULL schema) like resource_updated/deleted/invocation_closed.
INSERT INTO kb_event_types (name, payload_schema, schema_version)
VALUES ('resource_finalized', NULL, 1)
ON CONFLICT (name) DO NOTHING;

-- Replay-symmetric projection half: reproject one appended block from the
-- block_created ledger payload {resource_id, block:{block_id, seq, chunks:[…]}} +
-- content sidecar. Mirrors _project_resource_created → _project_blocks. Called by
-- both block_append (live) and ledger replay, so an appended block reconstructs from
-- the ledger exactly like a create-path block (chunk structure in the payload, prose
-- in the transient sidecar — CAS rule; never prose in the ledger).
CREATE FUNCTION _project_block_created(p_event uuid, p_payload jsonb, p_content jsonb)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM _project_blocks((p_payload->>'resource_id')::uuid, p_event,
                            jsonb_build_array(p_payload->'block'), p_content);
END;
$$;

-- Append one new block at seq=N into an existing resource, firing block_created.
-- Idempotent on (resource_id, seq, block merkle): a re-append of the same segment
-- is a no-op returning the existing block id; a same-seq different-content append
-- raises. Anchor resolution mirrors block_mutate.
CREATE FUNCTION block_append(p_payload jsonb, p_content jsonb, p_emitter uuid,
                             p_metadata jsonb DEFAULT '{}', p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_resource uuid := (p_payload->>'resource_id')::uuid;
    v_block_json jsonb := p_payload->'block';
    v_block uuid := (v_block_json->>'block_id')::uuid;
    v_seq int := (v_block_json->>'seq')::int;
    v_incoming_hash text;
    v_existing_block uuid;
    v_existing_hash text;
    v_anchor_tbl text; v_anchor uuid;
    v_ev uuid;
BEGIN
    IF v_resource IS NULL OR v_block IS NULL THEN
        RAISE EXCEPTION 'block_append: payload missing resource_id or block.block_id';
    END IF;
    IF v_block_json->'chunks' IS NULL OR jsonb_array_length(v_block_json->'chunks') = 0 THEN
        RAISE EXCEPTION 'block_append: empty chunk set for resource % seq %', v_resource, v_seq;
    END IF;
    -- Incoming block merkle = sha256 over the ordered chunk content_hashes (same
    -- rule _project_blocks uses to derive block_body_hash).
    SELECT encode(sha256(convert_to(string_agg(c->>'content_hash', '' ORDER BY (c->>'chunk_index')::int), 'UTF8')), 'hex')
      INTO v_incoming_hash
      FROM jsonb_array_elements(v_block_json->'chunks') c;

    -- Idempotency: an already-landed non-folded block at this seq.
    SELECT b.id INTO v_existing_block
      FROM kb_content_blocks b
     WHERE b.resource_id = v_resource AND b.seq = v_seq AND NOT b.is_folded;
    IF v_existing_block IS NOT NULL THEN
        SELECT block_body_hash INTO v_existing_hash
          FROM kb_block_revisions WHERE block_id = v_existing_block
         ORDER BY created DESC LIMIT 1;
        IF v_existing_hash IS DISTINCT FROM v_incoming_hash THEN
            RAISE EXCEPTION 'block_append: seq % already present for resource % with different content (source changed?)', v_seq, v_resource;
        END IF;
        RETURN v_existing_block;  -- no-op: same segment re-appended
    END IF;

    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table = 'kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'block_append: resource % has no home to anchor the event', v_resource;
    END IF;

    v_ev := _event_append('block_created', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    PERFORM _project_block_created(v_ev, p_payload, p_content);
    RETURN v_block;
END;
$$;

-- Declare a segmented ingest complete. Validates the landed set against the
-- caller's expectation, then records a projection-less resource_finalized event.
CREATE FUNCTION resource_finalize(p_payload jsonb, p_emitter uuid,
                                  p_metadata jsonb DEFAULT '{}', p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_resource uuid := (p_payload->>'resource_id')::uuid;
    v_expected_blocks int := (p_payload->>'expected_blocks')::int;
    v_expected_hash text := p_payload->>'expected_body_hash';
    v_actual_blocks int;
    v_actual_hash text;
    v_anchor_tbl text; v_anchor uuid;
BEGIN
    SELECT count(*) INTO v_actual_blocks FROM kb_content_blocks
        WHERE resource_id = v_resource AND NOT is_folded;
    IF v_actual_blocks <> v_expected_blocks THEN
        RAISE EXCEPTION 'resource_finalize: resource % has % live blocks, expected %',
            v_resource, v_actual_blocks, v_expected_blocks;
    END IF;
    SELECT body_hash INTO v_actual_hash FROM kb_resources WHERE id = v_resource;
    IF v_actual_hash IS DISTINCT FROM v_expected_hash THEN
        RAISE EXCEPTION 'resource_finalize: resource % body_hash % does not match expected %',
            v_resource, v_actual_hash, v_expected_hash;
    END IF;
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table = 'kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'resource_finalize: resource % has no home', v_resource;
    END IF;
    RETURN _event_append('resource_finalized', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                         p_metadata => p_metadata, p_invocation => p_invocation);
END;
$$;
