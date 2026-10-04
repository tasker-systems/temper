-- Resource erasure build order 2e: `cancelled` and `abandoned` are terminal ingest states.
--
-- `ck_kb_resources_ingest_state` (20260714000001) admitted only `in_progress` and `complete`. It now
-- also admits the two terminal states the `IngestState` type names: `cancelled` (an operator act —
-- the block history scrub — ended the ingest before it finalized) and `abandoned` (reserved for an
-- abandoned-ingest reaper; nothing sets it yet). No existing row is in either state, so re-adding the CHECK validates
-- every row as it stands. `idx_kb_resources_incomplete` stays `WHERE ingest_state = 'in_progress'`:
-- it enumerates RESUMABLE uploads, and a terminal ingest is not one.
--
-- A terminal ingest cannot be continued. `resource_finalize` (body from 20260715000030) and
-- `block_append` (body from 20260709000050) each gain ONE leading refusal, SQLSTATE TF004, and are
-- otherwise copied verbatim with the same signatures. In `block_append` the refusal precedes the
-- idempotency short-circuit, so a re-append of an already-landed seq is refused too.
--
-- WHY THE STATE READ TAKES A LOCK. Each refusal reads `ingest_state` with `SELECT … FOR KEY SHARE`.
-- The scrub takes `FOR UPDATE` on the resource row before it sets `cancelled`, and `FOR KEY SHARE`
-- conflicts with `FOR UPDATE`. So an append or finalize that arrives while the scrub holds the row
-- waits for the scrub's commit, then re-reads the committed row (the READ COMMITTED EvalPlanQual
-- re-check), sees `cancelled`, and refuses. An unlocked read would see the pre-scrub snapshot,
-- pass the refusal, and land a block on an ingest the scrub had just ended.

ALTER TABLE kb_resources DROP CONSTRAINT ck_kb_resources_ingest_state;
ALTER TABLE kb_resources
    ADD CONSTRAINT ck_kb_resources_ingest_state
        CHECK (ingest_state IN ('in_progress', 'complete', 'cancelled', 'abandoned'));

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
    v_ingest_state text;
    v_anchor_tbl text; v_anchor uuid;
    v_ev uuid;
BEGIN
    -- TF004: a terminal ingest cannot be finalized. FOR KEY SHARE waits out a scrub holding the row
    -- FOR UPDATE, then reads the state it committed (see this migration's header).
    SELECT ingest_state INTO v_ingest_state FROM kb_resources WHERE id = v_resource FOR KEY SHARE;
    IF v_ingest_state IN ('cancelled', 'abandoned') THEN
        RAISE EXCEPTION 'resource_finalize: resource % ingest is %; it cannot be continued',
            v_resource, v_ingest_state
            USING ERRCODE = 'TF004';
    END IF;
    SELECT count(*) INTO v_actual_blocks FROM kb_content_blocks
        WHERE resource_id = v_resource AND NOT is_folded;
    -- Custom SQLSTATEs so the transport can distinguish the failure modes and give the caller a
    -- reconciliation path (mapped in temper-services `From<sqlx::Error>`): TF001/TF002 are RESUMABLE
    -- (409 — re-list/append the gap and re-finalize), TF003 is NOT (422 — the committed bytes are wrong
    -- and `block_append` refuses to overwrite a seq, so the caller must discard + re-upload).
    IF v_actual_blocks <> v_expected_blocks THEN
        RAISE EXCEPTION 'resource_finalize: resource % has % live blocks, expected %',
            v_resource, v_actual_blocks, v_expected_blocks
            USING ERRCODE = 'TF001';
    END IF;
    SELECT body_hash INTO v_actual_hash FROM kb_resources WHERE id = v_resource;
    IF v_actual_hash IS DISTINCT FROM v_expected_hash THEN
        RAISE EXCEPTION 'resource_finalize: resource % body_hash % does not match expected %',
            v_resource, v_actual_hash, v_expected_hash
            USING ERRCODE = 'TF002';
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
                v_resource, v_actual_content_hash, p_payload->>'expected_content_hash'
                USING ERRCODE = 'TF003';
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

CREATE OR REPLACE FUNCTION block_append(p_payload jsonb, p_content jsonb, p_emitter uuid,
                                        p_metadata jsonb DEFAULT '{}', p_invocation uuid DEFAULT NULL,
                                        p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_resource uuid := (p_payload->>'resource_id')::uuid;
    v_block_json jsonb := p_payload->'block';
    v_block uuid := (v_block_json->>'block_id')::uuid;
    v_seq int := (v_block_json->>'seq')::int;
    v_incoming_hash text;
    v_existing_block uuid;
    v_existing_hash text;
    v_ingest_state text;
    v_anchor_tbl text; v_anchor uuid;
    v_ev uuid;
BEGIN
    -- TF004: a terminal ingest cannot be appended to — ahead of the idempotency short-circuit, so a
    -- re-append of a landed seq is refused too. FOR KEY SHARE waits out a scrub holding the row
    -- FOR UPDATE, then reads the state it committed (see this migration's header).
    SELECT ingest_state INTO v_ingest_state FROM kb_resources WHERE id = v_resource FOR KEY SHARE;
    IF v_ingest_state IN ('cancelled', 'abandoned') THEN
        RAISE EXCEPTION 'block_append: resource % ingest is %; it cannot be continued',
            v_resource, v_ingest_state
            USING ERRCODE = 'TF004';
    END IF;
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
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    PERFORM _project_block_created(v_ev, p_payload, p_content);
    RETURN v_block;
END;
$$;

SELECT declare_migration(
    20261003000110,
    'additive',
    'Terminal ingest states: ck_kb_resources_ingest_state admits cancelled and abandoned, and resource_finalize and block_append each refuse a terminal ingest with TF004, reading the state under FOR KEY SHARE. Additive: the widened CHECK refuses nothing a deployed binary writes; both functions keep their signatures; the new refusal fires only on states nothing set before this migration.'
);
