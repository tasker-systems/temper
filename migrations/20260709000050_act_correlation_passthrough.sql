-- Caller-supplied act-correlation passthrough (task 019f4912 — P3 of the temper-rb goal).
--
-- `_event_append` was BORN accepting `p_correlation uuid DEFAULT NULL` and applying the root-event
-- convention `COALESCE(p_correlation, v_ev)` (20260624000002:765-786). But no caller has ever passed
-- it: every mutation function calls the sink with `p_metadata =>`/`p_invocation =>` only, so every
-- event in the ledger to date self-roots. This migration opens the last hop — each mutation fn gains
-- a `p_correlation` passthrough so a client can stitch two writes into one act-grain thread.
--
-- Why act-grain and not `invocation_id`: an invocation is run-grain and agent-shaped
-- (trigger_kind, originating_cogmap_id, delegated_launch, scoped_entity_id) — it models an agent
-- working-session envelope. A Rails request is not an agent run. But "publish this postmortem"
-- spanning a Puma request and the Sidekiq job it enqueued IS one act, and a bare correlation UUID
-- serialized into the job arguments outlives any credential. That is what this carries.
--
-- Correlation is a correlation aid, NEVER authorization. No gate keys off it; it is provenance only.
-- An unsupplied correlation still self-roots, byte-identical to today.
--
-- Append-only / byte-identical drift rule: this is a NEW forward migration; the born migrations are
-- never edited. Each body below is copied verbatim from its CURRENT definition (see the table) with
-- exactly two changes: (a) the signature gains `p_correlation uuid DEFAULT NULL` appended LAST, and
-- (b) the single `_event_append(...)` call forwards it by name (`p_correlation =>`). Appending last
-- keeps every existing positional call site (`facet_set($1,$2)`, the `fire()` default-context path)
-- resolving unchanged.
--
--   resource_create        20260624000002:747     relationship_retype    20260629000003:92
--   relationship_assert    20260624000002:823     relationship_reweight  20260629000003:108
--   relationship_fold      20260624000002:852     block_mutate           20260629000003:124
--   facet_set              20260624000002:889     cogmap_charter_set     20260629000003:153
--   resource_delete        20260629000003:25      resource_reassign      20260703140000:26
--   resource_update        20260629000003:42      block_append           20260708000012:31
--   resource_rehome        20260629000003:59
--   property_set           20260629000003:73
--
-- Mechanics: adding a parameter changes a function's identity, so CREATE OR REPLACE would leave a
-- second, ambiguous overload callable. DROP + CREATE truly replaces. No view, trigger, or SQL
-- function calls any of these 14 at runtime — the only in-SQL callers (relationship_assert,
-- relationship_fold from 20260709000005_backfill_goal_parent_of_to_advances.sql) live inside a
-- one-shot backfill that already ran and is ordered before this migration — so a plain DROP
-- (no CASCADE) succeeds.

-- ── resource_created ─────────────────────────────────────────────────────────
DROP FUNCTION resource_create(jsonb, jsonb, uuid, jsonb, uuid);
CREATE FUNCTION resource_create(p_payload jsonb, p_content jsonb, p_emitter uuid,
                                p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL,
                                p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('resource_created', p_emitter,
                          p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_resource_created(v_ev, p_payload, p_content);
END;
$$;

-- ── relationship_asserted ────────────────────────────────────────────────────
DROP FUNCTION relationship_assert(jsonb, uuid, jsonb, uuid);
CREATE FUNCTION relationship_assert(p_payload jsonb, p_emitter uuid,
                                    p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL,
                                    p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('relationship_asserted', p_emitter,
                          p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_relationship_asserted(v_ev, p_payload);
END;
$$;

-- ── relationship_folded ──────────────────────────────────────────────────────
DROP FUNCTION relationship_fold(jsonb, uuid, jsonb, uuid);
CREATE FUNCTION relationship_fold(p_payload jsonb, p_emitter uuid,
                                  p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL,
                                  p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_edge uuid := (p_payload->>'edge_id')::uuid;
        v_home_tbl text; v_home uuid;
BEGIN
    SELECT home_anchor_table, home_anchor_id INTO v_home_tbl, v_home
        FROM kb_edges WHERE id = v_edge;
    IF v_home IS NULL THEN
        RAISE EXCEPTION 'relationship_fold: edge % not found', v_edge;
    END IF;
    v_ev := _event_append('relationship_folded', p_emitter, v_home_tbl, v_home, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_relationship_folded(v_ev, p_payload);
END;
$$;

-- ── property_asserted (facet_set) ────────────────────────────────────────────
DROP FUNCTION facet_set(jsonb, uuid, jsonb, uuid);
CREATE FUNCTION facet_set(p_payload jsonb, p_emitter uuid,
                          p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL,
                          p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_anchor_tbl text; v_anchor uuid;
        v_owner uuid := (p_payload#>>'{owner,id}')::uuid;
BEGIN
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_owner ORDER BY (anchor_table='kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'facet_set: resource % has no home to anchor the property event', v_owner;
    END IF;
    v_ev := _event_append('property_asserted', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_property_asserted(v_ev, p_payload);
END;
$$;

-- ── resource_deleted ─────────────────────────────────────────────────────────
DROP FUNCTION resource_delete(jsonb, uuid, jsonb, uuid);
CREATE FUNCTION resource_delete(p_payload jsonb, p_emitter uuid,
                                p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL,
                                p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_resource uuid := (p_payload->>'resource_id')::uuid;
        v_anchor_tbl text; v_anchor uuid;
BEGIN
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table='kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN RAISE EXCEPTION 'resource_delete: resource % has no home', v_resource; END IF;
    v_ev := _event_append('resource_deleted', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_resource_deleted(v_ev, p_payload);
END;
$$;

-- ── resource_updated ─────────────────────────────────────────────────────────
DROP FUNCTION resource_update(jsonb, uuid, jsonb, uuid);
CREATE FUNCTION resource_update(p_payload jsonb, p_emitter uuid,
                                p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL,
                                p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_resource uuid := (p_payload->>'resource_id')::uuid;
        v_anchor_tbl text; v_anchor uuid;
BEGIN
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table='kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN RAISE EXCEPTION 'resource_update: resource % has no home', v_resource; END IF;
    v_ev := _event_append('resource_updated', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_resource_updated(v_ev, p_payload);
END;
$$;

-- ── resource_rehomed ─────────────────────────────────────────────────────────
DROP FUNCTION resource_rehome(jsonb, uuid, jsonb, uuid);
CREATE FUNCTION resource_rehome(p_payload jsonb, p_emitter uuid,
                                p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL,
                                p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('resource_rehomed', p_emitter,
                          p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_resource_rehomed(v_ev, p_payload);
END;
$$;

-- ── property_set ─────────────────────────────────────────────────────────────
DROP FUNCTION property_set(jsonb, uuid, jsonb, uuid);
CREATE FUNCTION property_set(p_payload jsonb, p_emitter uuid,
                             p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL,
                             p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_anchor_tbl text; v_anchor uuid;
        v_owner uuid := (p_payload#>>'{owner,id}')::uuid;
BEGIN
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_owner ORDER BY (anchor_table='kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'property_set: resource % has no home to anchor the property event', v_owner;
    END IF;
    v_ev := _event_append('property_set', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_property_set(v_ev, p_payload);
END;
$$;

-- ── relationship_retyped ─────────────────────────────────────────────────────
DROP FUNCTION relationship_retype(jsonb, uuid, jsonb, uuid);
CREATE FUNCTION relationship_retype(p_payload jsonb, p_emitter uuid,
                                    p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL,
                                    p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_edge uuid := (p_payload->>'edge_id')::uuid;
        v_home_tbl text; v_home uuid;
BEGIN
    SELECT home_anchor_table, home_anchor_id INTO v_home_tbl, v_home FROM kb_edges WHERE id = v_edge;
    IF v_home IS NULL THEN RAISE EXCEPTION 'relationship_retype: edge % not found', v_edge; END IF;
    v_ev := _event_append('relationship_retyped', p_emitter, v_home_tbl, v_home, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_relationship_retyped(v_ev, p_payload);
END;
$$;

-- ── relationship_reweighted ──────────────────────────────────────────────────
DROP FUNCTION relationship_reweight(jsonb, uuid, jsonb, uuid);
CREATE FUNCTION relationship_reweight(p_payload jsonb, p_emitter uuid,
                                      p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL,
                                      p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_edge uuid := (p_payload->>'edge_id')::uuid;
        v_home_tbl text; v_home uuid;
BEGIN
    SELECT home_anchor_table, home_anchor_id INTO v_home_tbl, v_home FROM kb_edges WHERE id = v_edge;
    IF v_home IS NULL THEN RAISE EXCEPTION 'relationship_reweight: edge % not found', v_edge; END IF;
    v_ev := _event_append('relationship_reweighted', p_emitter, v_home_tbl, v_home, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_relationship_reweighted(v_ev, p_payload);
END;
$$;

-- ── block_mutated ────────────────────────────────────────────────────────────
DROP FUNCTION block_mutate(jsonb, jsonb, uuid, jsonb, uuid);
CREATE FUNCTION block_mutate(p_payload jsonb, p_content jsonb, p_emitter uuid,
                             p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL,
                             p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_block uuid := (p_payload->>'block_id')::uuid;
        v_resource uuid; v_anchor_tbl text; v_anchor uuid;
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

-- ── charter_set ──────────────────────────────────────────────────────────────
DROP FUNCTION cogmap_charter_set(jsonb, jsonb, uuid, jsonb, uuid);
CREATE FUNCTION cogmap_charter_set(p_payload jsonb, p_content jsonb, p_emitter uuid,
                                   p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL,
                                   p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
        v_cogmap uuid := (p_payload->>'cogmap_id')::uuid;
        v_telos  uuid := cogmap_telos(v_cogmap);
BEGIN
    IF v_telos IS NULL THEN
        RAISE EXCEPTION 'cogmap_charter_set: cogmap % has no telos', v_cogmap;
    END IF;
    IF p_payload->'blocks' IS NULL OR jsonb_array_length(p_payload->'blocks') = 0 THEN
        RAISE EXCEPTION 'cogmap_charter_set: empty charter for cogmap % (would blank the telos)', v_cogmap;
    END IF;
    v_ev := _event_append('charter_set', p_emitter, 'kb_cogmaps', v_cogmap, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_charter_set(v_ev, p_payload, p_content);
END;
$$;

-- ── resource_reassigned ──────────────────────────────────────────────────────
DROP FUNCTION resource_reassign(jsonb, uuid, jsonb, uuid);
CREATE FUNCTION resource_reassign(p_payload jsonb, p_emitter uuid,
                                  p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL,
                                  p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_resource uuid := (p_payload->>'resource_id')::uuid;
        v_anchor_tbl text; v_anchor uuid;
BEGIN
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor
      FROM kb_resource_homes WHERE resource_id = v_resource;
    IF v_anchor IS NULL THEN RAISE EXCEPTION 'resource_reassign: resource % has no home', v_resource; END IF;
    -- Backstop: only context-homed resources are reassignable. A cogmap interior is
    -- team-resource-derived, not personally owned (spec non-goal) — refuse at the write
    -- primitive so the invariant holds even if a future surface bypasses the service.
    IF v_anchor_tbl <> 'kb_contexts' THEN
        RAISE EXCEPTION 'resource_reassign: resource % is not context-homed (cogmap interiors are not reassignable)', v_resource;
    END IF;
    v_ev := _event_append('resource_reassigned', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_resource_reassigned(v_ev, p_payload);
END;
$$;

-- ── block_created (block_append) ─────────────────────────────────────────────
DROP FUNCTION block_append(jsonb, jsonb, uuid, jsonb, uuid);
CREATE FUNCTION block_append(p_payload jsonb, p_content jsonb, p_emitter uuid,
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
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    PERFORM _project_block_created(v_ev, p_payload, p_content);
    RETURN v_block;
END;
$$;
