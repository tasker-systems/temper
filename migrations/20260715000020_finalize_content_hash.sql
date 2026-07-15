-- W2 PR 5 (#420 set 3): the raw-bytes INTEGRITY check at finalize.
--
-- `expected_body_hash` is NOT an integrity check — it is the two-level chunk MERKLE, handed to the
-- client as a CONCURRENCY token ("nothing changed between my last append and now") precisely because a
-- non-chunking caller (MCP) cannot derive it. Repurposing it would break MCP. So this adds a SEPARATE,
-- optional field: `expected_content_hash`, the bare-hex sha256 of the FULL raw body the caller uploaded.
--
-- Body copied VERBATIM from the live definition (20260714000001_ingest_state.sql), with ONE added
-- block. The 4-arg signature is UNCHANGED — CREATE OR REPLACE cannot add a parameter (it would mint a
-- second overload and make old-arity calls ambiguous), and a signature change is a write outage across
-- deploy skew. The new field rides in the `jsonb` payload we already own on both ends.
--
-- Skew-safe both directions: an OLD app's payload has no `expected_content_hash` key ⇒ the `?` guard is
-- false ⇒ the check is skipped, identical to today. A NEW app against an OLD (pre-this-migration)
-- function ⇒ the extra key is simply ignored ⇒ no check, no failure. MCP omits the key (honestly exempt:
-- its finalize never sees the whole body).
--
-- On mismatch the RAISE rolls the transaction back, `_project_resource_finalized` never runs, and the
-- resource stays `in_progress` — still resumable, never silently done. Same fail-loud contract the
-- block-count and body_hash checks already carry.
CREATE OR REPLACE FUNCTION resource_finalize(p_payload jsonb, p_emitter uuid,
                                             p_metadata jsonb DEFAULT '{}', p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_resource uuid := (p_payload->>'resource_id')::uuid;
    v_expected_blocks int := (p_payload->>'expected_blocks')::int;
    v_expected_hash text := p_payload->>'expected_body_hash';
    v_actual_blocks int;
    v_actual_hash text;
    v_actual_content_hash text;
    v_anchor_tbl text; v_anchor uuid;
    v_ev uuid;
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
    -- W2 PR 5: raw-bytes integrity, only when the caller supplied it. `string_agg('' ORDER BY seq)`
    -- reconstructs the exact bytes the verbatim readback (PR 4) returns — the stored block content
    -- already carries its own terminators — so this is `sha256(the very bytes we would read back)`,
    -- compared against the caller's declared sha256 of what it uploaded. `convert_to(...,'UTF8')` gives
    -- the same UTF-8 bytes the client's `sha256_hex(body.as_bytes())` hashes; the DB stores BARE hex.
    IF p_payload ? 'expected_content_hash' THEN
        SELECT encode(sha256(convert_to(
                 coalesce(string_agg(bc.content, '' ORDER BY b.seq), ''), 'UTF8')), 'hex')
          INTO v_actual_content_hash
          FROM kb_content_blocks b
          JOIN kb_block_content  bc ON bc.block_revision_id = b.current_revision_id
         WHERE b.resource_id = v_resource AND NOT b.is_folded;
        IF v_actual_content_hash IS DISTINCT FROM (p_payload->>'expected_content_hash') THEN
            RAISE EXCEPTION 'resource_finalize: resource % stored bytes hash %, expected %',
                v_resource, v_actual_content_hash, p_payload->>'expected_content_hash';
        END IF;
    END IF;
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table = 'kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'resource_finalize: resource % has no home', v_resource;
    END IF;
    v_ev := _event_append('resource_finalized', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    PERFORM _project_resource_finalized(v_ev, p_payload);
    RETURN v_ev;
END;
$$;
