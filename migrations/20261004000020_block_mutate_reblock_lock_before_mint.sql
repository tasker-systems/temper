-- Resource erasure: block_mutate and resource_reblock take R's row before they mint their event.
--
-- Both functions call _event_append before anything locks kb_resources. The first lock either
-- takes on R is the write guard's FOR KEY SHARE inside the projector, after the event id
-- (uuidv7) is minted and after the projector has started superseding chunks. A block history
-- scrub that takes R FOR UPDATE inside that gap does not wait: it reads the pre-mutate present,
-- keeps it, and commits an event that sorts AFTER the mutate's. When the mutate then commits,
-- the revision the scrub kept as current is history that still holds its prose. Replay walks the
-- mutate first, so at the scrub's position that revision is already history and gets emptied.
-- Live and replay diverge. A replaces_body mutate folds sibling blocks in the same gap.
--
-- The fix is the shape 20261003000110 gave block_append and resource_finalize: SELECT … FOR KEY
-- SHARE on R as the first lock. FOR KEY SHARE conflicts only with FOR UPDATE, which the scrub and
-- the erasure act take, so ordinary writers never wait on each other. A mutate that arrives while
-- the scrub holds R waits, then mints its id after the scrub commits. A scrub that arrives after
-- the mutate has the lock waits for the mutate's commit and reads the present the mutate left.
-- Either way, the second act mints its event only after the first has committed, so it never acts
-- on a state the first act's event will later change. In block_mutate the lock also precedes the
-- no-op suppression's reads, so those reads see the state the scrub committed.
--
-- Both bodies are their live definitions (block_mutate from 20260911000000, resource_reblock from
-- 20260908000010), verbatim except for the lock and its comment. Signatures, return types and
-- COMMENTs are unchanged.
--
-- Witnesses: block_history_scrub.rs, a_mutate_racing_the_scrub_serializes_on_the_resource (16i)
-- and a_reblock_racing_the_scrub_serializes_on_the_resource (16j).

CREATE OR REPLACE FUNCTION public.block_mutate(p_payload jsonb, p_content jsonb, p_emitter uuid, p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL::uuid, p_correlation uuid DEFAULT NULL::uuid)
 RETURNS uuid
 LANGUAGE plpgsql
AS $function$
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
    -- R's row, FOR KEY SHARE, before the no-op reads and before _event_append mints this event's
    -- id (see this migration's header).
    PERFORM 1 FROM kb_resources WHERE id = v_resource FOR KEY SHARE;
    -- An empty chunk set would supersede the block's current chunks and insert none, silently dropping
    -- the member from its region centroid and diverging body_hash from create-path semantics (which has
    -- no empty-body block). Reject before appending an event — a revise must carry content.
    IF p_payload->'chunks' IS NULL OR jsonb_array_length(p_payload->'chunks') = 0 THEN
        RAISE EXCEPTION 'block_mutate: empty chunk set for block % (a revise with no content would drop the block)', v_block;
    END IF;

    -- ── No-op suppression (20260726000030; the erased-content refusal that once sat here
    -- was retired by 20260911000000) ── check 5: provenance is event-anchored, so a write
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
$function$;

CREATE OR REPLACE FUNCTION public.resource_reblock(p_payload jsonb, p_content jsonb, p_emitter uuid, p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL::uuid, p_correlation uuid DEFAULT NULL::uuid)
 RETURNS uuid
 LANGUAGE plpgsql
AS $function$
DECLARE v_ev uuid; v_resource uuid := (p_payload->>'resource_id')::uuid;
        v_anchor_tbl text; v_anchor uuid;
BEGIN
    IF v_resource IS NULL OR NOT EXISTS (SELECT 1 FROM kb_resources WHERE id = v_resource) THEN
        RAISE EXCEPTION 'resource_reblock: resource % not found', v_resource;
    END IF;
    -- R's row, FOR KEY SHARE, before _event_append mints this event's id (see this migration's
    -- header).
    PERFORM 1 FROM kb_resources WHERE id = v_resource FOR KEY SHARE;
    IF coalesce(jsonb_array_length(coalesce(p_payload->'created', '[]'::jsonb)), 0) = 0
       AND coalesce(jsonb_array_length(coalesce(p_payload->'folded', '[]'::jsonb)), 0) = 0
       AND NOT EXISTS (
            SELECT 1 FROM jsonb_array_elements(coalesce(p_payload->'kept', '[]'::jsonb)) k
            WHERE coalesce(jsonb_array_length(k->'attribution'), 0) > 0
               OR EXISTS (SELECT 1 FROM kb_content_blocks b
                           WHERE b.id = (k->>'block_id')::uuid
                             AND b.seq IS DISTINCT FROM (k->>'seq')::int)) THEN
        RAISE EXCEPTION 'resource_reblock: manifest changes nothing for resource % (no created, no folded, no new assertions, no kept seq moves)', v_resource;
    END IF;
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table = 'kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'resource_reblock: resource % has no home to anchor the event', v_resource;
    END IF;
    v_ev := _event_append('resource_reblocked', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_resource_reblocked(v_ev, p_payload, p_content);
END;
$function$;

SELECT declare_migration(
    20261004000020,
    'additive',
    'CREATE OR REPLACE of block_mutate and resource_reblock with the same signatures and return types; each body is its live definition plus one FOR KEY SHARE on the kb_resources row before _event_append. A writer racing a block history scrub or the erasure act now waits on their FOR UPDATE instead of minting its event first. No table, column, constraint, grant or COMMENT changes.'
);
