-- `block_mutate` appended a `block_mutated` event whenever an update CARRIED a body, not when the
-- body CHANGED. Since `20260726000010` made `kb_content_blocks.last_event_id` the auditor's staleness
-- cursor, saving a resource without editing it re-offered findings that were already audited.
--
-- Fix: return before `_event_append` when the write would change nothing. "Nothing" is five checks,
-- because the chunk merkle covers only one of the four things `_project_block_mutated` persists:
--   1. same chunk merkle
--   2. same `kb_block_content` bytes, when the caller supplies any
--   3. no current chunk missing an embedding
--   4. no incoming `embedded_with` the block lacks
--   5. nothing in `incorporated` (provenance is event-anchored)
-- Any column the projector later learns to persist must be added here.
--
-- TWO TRAPS. Both look like simplifications and are not:
--   * Check 2 is NOT redundant with the merkle. Chunk `content_hash` is `sha256(content.trim())`
--     (temper-ingest/src/chunk.rs) while the stored bytes are untrimmed, so "x" and "x\n\n\n" share
--     a merkle. Dropping check 2 silently discards whitespace-only edits.
--   * The merkle concatenates in chunk ARRAY order, matching the projectors that write
--     `block_body_hash`. `block_append` and `_recompute_resource_body_hash` sort by `chunk_index`
--     instead. Do not "fix" this to match them.
--
-- Suppressing at the write path (not the projector) leaves no event, and `replay.rs` calls
-- `_project_block_mutated` directly, so replay is unaffected. Blocks predating `20260714000002` have
-- a NULL `current_revision_id` and fall through to the old behaviour — safe, self-healing after one
-- revise. Rationale, alternatives, and the per-check test matrix: PR #551.
--
-- ADDITIVE: `CREATE OR REPLACE`, signature unchanged.

CREATE OR REPLACE FUNCTION block_mutate(p_payload jsonb, p_content jsonb, p_emitter uuid,
                                        p_metadata jsonb DEFAULT '{}', p_invocation uuid DEFAULT NULL,
                                        p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_block uuid := (p_payload->>'block_id')::uuid;
        v_resource uuid; v_anchor_tbl text; v_anchor uuid;
        v_incoming_hash text; v_existing_hash text;
        v_incoming_bytes text; v_existing_bytes text;
        v_live_chunks int; v_unembedded boolean; v_new_model boolean;
BEGIN
    SELECT resource_id INTO v_resource FROM kb_content_blocks WHERE id = v_block;
    IF v_resource IS NULL THEN
        RAISE EXCEPTION 'block_mutate: block % not found', v_block;
    END IF;
    -- An empty chunk set would supersede the block's current chunks and insert none, silently dropping
    -- the member from its region centroid and diverging body_hash from create-path semantics (which has
    -- no empty-body block). Reject before appending an event — a revise must carry content.
    IF p_payload->'chunks' IS NULL OR jsonb_array_length(p_payload->'chunks') = 0 THEN
        RAISE EXCEPTION 'block_mutate: empty chunk set for block % (a revise with no content would drop the block)', v_block;
    END IF;

    -- ── No-op suppression (see header) ── check 5: provenance is event-anchored, so a write
    -- carrying sources is never a no-op, whatever its bytes did.
    IF p_payload->'incorporated' IS NULL
       OR jsonb_typeof(p_payload->'incorporated') <> 'array'
       OR jsonb_array_length(p_payload->'incorporated') = 0 THEN

        -- Check 1. ARRAY order, matching the projector that writes the stored hash (trap 2).
        SELECT encode(sha256(convert_to(
                   coalesce(string_agg(c.elem->>'content_hash', '' ORDER BY c.ord), ''), 'UTF8')), 'hex')
          INTO v_incoming_hash
          FROM jsonb_array_elements(p_payload->'chunks') WITH ORDINALITY AS c(elem, ord);

        -- By pointer, not `ORDER BY created DESC LIMIT 1` — two revisions can share an `occurred_at`.
        SELECT rv.block_body_hash INTO v_existing_hash
          FROM kb_content_blocks b
          JOIN kb_block_revisions rv ON rv.id = b.current_revision_id
         WHERE b.id = v_block;

        -- Check 2 (trap 1). Only when the caller supplies bytes: if it does not, the projector would
        -- write none, so suppressing preserves what is stored rather than erasing it.
        v_incoming_bytes := p_content->'__blocks'->(v_block::text)->>'content_hash';
        IF v_incoming_bytes IS NOT NULL THEN
            SELECT bc.content_hash INTO v_existing_bytes
              FROM kb_content_blocks b
              JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id
             WHERE b.id = v_block;
        END IF;

        -- Checks 3 and 4. Never swallow an embed repair or a model rotation. A caller bringing no
        -- vectors is not blocked — the projector would only write NULLs over good ones.
        SELECT count(*), coalesce(bool_or(ch.embedding IS NULL), false)
          INTO v_live_chunks, v_unembedded
          FROM kb_chunks ch
         WHERE ch.block_id = v_block AND ch.is_current;

        SELECT coalesce(bool_or(
                   (p_content->(c->>'chunk_id')->>'embedded_with') IS NOT NULL
                   AND NOT EXISTS (
                       SELECT 1 FROM kb_chunks ch
                        WHERE ch.block_id = v_block AND ch.is_current
                          AND ch.embedded_with = (p_content->(c->>'chunk_id')->>'embedded_with'))
               ), false)
          INTO v_new_model
          FROM jsonb_array_elements(p_payload->'chunks') c;

        IF v_existing_hash IS NOT NULL
           AND v_existing_hash = v_incoming_hash
           AND (v_incoming_bytes IS NULL OR v_existing_bytes = v_incoming_bytes)
           AND v_live_chunks > 0
           AND NOT v_unembedded
           AND NOT v_new_model THEN
            RETURN v_block;  -- no-op: the projector would write nothing new
        END IF;
    END IF;

    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table = 'kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'block_mutate: resource % has no home to anchor the event', v_resource;
    END IF;
    v_ev := _event_append('block_mutated', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_block_mutated(v_ev, p_payload, p_content);
END;
$$;

COMMENT ON FUNCTION block_mutate(jsonb, jsonb, uuid, jsonb, uuid, uuid) IS
    'Revise a block in place. Suppresses a no-op revise at the write path — no event appended, so the '
    'tier-1 staleness cursor does not move and audited findings are not re-offered. Suppresses only '
    'when the projector would write nothing new: same merkle, same stored bytes, no chunk awaiting a '
    'vector, no new embedding model, nothing incorporated. See the migration header for the two traps.';
