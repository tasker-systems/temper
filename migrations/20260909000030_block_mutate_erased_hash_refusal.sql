-- The erased-content refusal on the projection write-back arm (erasure spec 2026-08-31, D4
-- arm 1; task 01a0577c Beat 3). `kb_erased_content` (20260909000010) is the materialized set
-- the write path refuses through: a revise whose incoming payload carries a hash in the set —
-- any chunk hash, or the verbatim `__blocks` bytes hash — RAISES, before anything is appended
-- or projected.
--
-- WHY BEFORE THE NO-OP SUPPRESSION, AND BESIDE IT RATHER THAN IN IT. D4's undo scenario runs
-- straight through suppression: erasure nulls the chunk embeddings, so check 3
-- (`AND NOT v_unembedded`) falls through — deliberately, it must never swallow an embed
-- repair — and `block_mutated` fires, letting the projector write the stale client's copy back
-- in: prose re-admitted under an erased hash. A refusal folded INTO the suppression block
-- would therefore miss exactly the write it exists for (the suppressed one), and a refusal
-- AFTER it would never run. It sits as a NEW check beside the five, before the IF: an erased
-- hash refuses even when the write would otherwise have been suppressed, because suppression
-- is not innocence — the client's payload carries redacted content either way, and the refusal
-- is what teaches the stale client to re-sync (erasure is a refusal, not an absence).
--
-- WHAT A LATER EDIT MUST NOT BREAK: the five suppression checks and their semantics —
-- including check 3's `AND NOT v_unembedded` — are UNTOUCHED from 20260726000030; the embed
-- repair for a genuinely unembedded non-erased block still fires, and the refusal must never
-- be merged into, reordered behind, or satisfied by any of them. The refusal is hash-keyed
-- (D2): the same hash refuses in every home, matching the set's global grain.
--
-- The sync-apply arm composes with this deliberately: reconcile's update path re-blocks
-- THROUGH `block_mutate`, and it drops erased-hash chunks BEFORE the merkle (db_backend's
-- resource phase) so a lawful sync never carries an erased hash into this refusal. A write
-- that still arrives with one is a stale client, and refusing it is the point.
--
-- ADDITIVE: `CREATE OR REPLACE`, signature unchanged (the 20260804000020 class argument: no
-- deployed binary can disagree with this schema across the apply).

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

    -- ── The erased-content refusal (D4 arm 1 — 20260909000030; see this file's header). ──
    -- Hash-keyed, both halves: the incoming chunk hashes and the verbatim bytes hash the caller
    -- supplied. Nothing in the erased set may be re-admitted, whatever the write would do.
    IF EXISTS (
           SELECT 1 FROM kb_erased_content ec
            WHERE ec.content_hash = ANY (
                      SELECT c.elem->>'content_hash'
                        FROM jsonb_array_elements(p_payload->'chunks') AS c(elem))
               OR ec.content_hash = p_content->'__blocks'->(v_block::text)->>'content_hash'
       ) THEN
        RAISE EXCEPTION 'block_mutate: block % carries ERASED content — a hash in its payload is in kb_erased_content; the write refuses (re-sync the emptied state, then write new text)', v_block;
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
    'Revise a block in place. Refuses any payload carrying a hash in kb_erased_content (the '
    'erasure act''s write-path refusal, 20260909000030) BEFORE no-op suppression, so a stale '
    'client cannot re-admit erased prose even as a would-be no-op. Otherwise suppresses a no-op '
    'revise at the write path — no event appended, so the tier-1 staleness cursor does not move '
    'and audited findings are not re-offered. Suppresses only when the projector would write '
    'nothing new: same merkle, same stored bytes, no chunk awaiting a vector, no new embedding '
    'model, nothing incorporated. See 20260726000030''s header for the two traps.';

SELECT declare_migration(
    20260909000030,
    'additive',
    'The erased-content refusal on the projection write-back arm (erasure spec 2026-08-31, D4 arm 1; task 01a0577c Beat 3): block_mutate now refuses any revise whose payload carries a content hash in kb_erased_content — chunk hashes or the verbatim __blocks bytes hash — with the check placed BEFORE the no-op suppression block, because D4''s undo scenario routes through check 3''s deliberate fall-through (erasure nulls the embeddings, the embed repair must not be swallowed, and the projector would write the stale client''s copy back in). The five suppression checks from 20260726000030 are untouched and byte-identical; the refusal is a new check beside them. Additive per the 20260804000020 class: CREATE OR REPLACE, signature unchanged, so no deployed binary can disagree with this schema across the apply.'
);
