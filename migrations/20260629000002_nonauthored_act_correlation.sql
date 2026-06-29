-- Non-authored act invocation-correlation (task 019f10c5 — follow-on to act-authorship PR #202).
--
-- The authored-4 mutation fns (resource_create/relationship_assert/relationship_fold/facet_set) were
-- BORN with `p_metadata`/`p_invocation` passthrough (20260624000002), so an authored act under a run
-- carries its `kb_events.invocation_id` (dedicated nullable column) + `kb_events.metadata` (authorship).
-- The NON-authored mutation fns were born WITHOUT those params, so update/delete/retype/reweight — and
-- every sub-event of an update fan-out + every reconcile mutation — fired with no correlation and never
-- appeared in `invocation_show`. This extends them to the SAME shape.
--
-- Append-only / byte-identical drift rule: this is a NEW forward migration; the born migrations
-- (20260624000002, 20260629000001) are never edited. Each body below is copied verbatim from its birth
-- definition with exactly two changes: (a) the signature gains the two defaulted params, and (b) the
-- single `_event_append(...)` call forwards them by name (`p_metadata =>`, `p_invocation =>`) — mirroring
-- `resource_create` (20260624000002:752-754). `_event_append` already accepts both and writes them to the
-- `metadata`/`invocation_id` columns, so no projection logic changes.
--
-- Mechanics: adding a parameter changes a function's identity, so CREATE OR REPLACE would create a second
-- overload (leaving the 2-arg form callable and ambiguous). DROP + CREATE truly replaces. None of these
-- fns are referenced by another SQL function/view/trigger (grep-verified — they are called only from the
-- Rust `fire` arms), so a plain DROP (no CASCADE) succeeds. The default values keep every existing
-- 2-/3-arg call site (the `fire()` default-context path) byte-identical.

-- ── resource_deleted ─────────────────────────────────────────────────────────
DROP FUNCTION resource_delete(jsonb, uuid);
CREATE FUNCTION resource_delete(p_payload jsonb, p_emitter uuid,
                                p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_resource uuid := (p_payload->>'resource_id')::uuid;
        v_anchor_tbl text; v_anchor uuid;
BEGIN
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table='kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN RAISE EXCEPTION 'resource_delete: resource % has no home', v_resource; END IF;
    v_ev := _event_append('resource_deleted', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_resource_deleted(v_ev, p_payload);
END;
$$;

-- ── resource_updated ─────────────────────────────────────────────────────────
DROP FUNCTION resource_update(jsonb, uuid);
CREATE FUNCTION resource_update(p_payload jsonb, p_emitter uuid,
                                p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_resource uuid := (p_payload->>'resource_id')::uuid;
        v_anchor_tbl text; v_anchor uuid;
BEGIN
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table='kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN RAISE EXCEPTION 'resource_update: resource % has no home', v_resource; END IF;
    v_ev := _event_append('resource_updated', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_resource_updated(v_ev, p_payload);
END;
$$;

-- ── resource_rehomed ─────────────────────────────────────────────────────────
DROP FUNCTION resource_rehome(jsonb, uuid);
CREATE FUNCTION resource_rehome(p_payload jsonb, p_emitter uuid,
                                p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('resource_rehomed', p_emitter,
                          p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_resource_rehomed(v_ev, p_payload);
END;
$$;

-- ── property_set ─────────────────────────────────────────────────────────────
DROP FUNCTION property_set(jsonb, uuid);
CREATE FUNCTION property_set(p_payload jsonb, p_emitter uuid,
                             p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
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
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_property_set(v_ev, p_payload);
END;
$$;

-- ── relationship_retyped ─────────────────────────────────────────────────────
DROP FUNCTION relationship_retype(jsonb, uuid);
CREATE FUNCTION relationship_retype(p_payload jsonb, p_emitter uuid,
                                    p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_edge uuid := (p_payload->>'edge_id')::uuid;
        v_home_tbl text; v_home uuid;
BEGIN
    SELECT home_anchor_table, home_anchor_id INTO v_home_tbl, v_home FROM kb_edges WHERE id = v_edge;
    IF v_home IS NULL THEN RAISE EXCEPTION 'relationship_retype: edge % not found', v_edge; END IF;
    v_ev := _event_append('relationship_retyped', p_emitter, v_home_tbl, v_home, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_relationship_retyped(v_ev, p_payload);
END;
$$;

-- ── relationship_reweighted ──────────────────────────────────────────────────
DROP FUNCTION relationship_reweight(jsonb, uuid);
CREATE FUNCTION relationship_reweight(p_payload jsonb, p_emitter uuid,
                                      p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_edge uuid := (p_payload->>'edge_id')::uuid;
        v_home_tbl text; v_home uuid;
BEGIN
    SELECT home_anchor_table, home_anchor_id INTO v_home_tbl, v_home FROM kb_edges WHERE id = v_edge;
    IF v_home IS NULL THEN RAISE EXCEPTION 'relationship_reweight: edge % not found', v_edge; END IF;
    v_ev := _event_append('relationship_reweighted', p_emitter, v_home_tbl, v_home, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_relationship_reweighted(v_ev, p_payload);
END;
$$;

-- ── block_mutated ────────────────────────────────────────────────────────────
DROP FUNCTION block_mutate(jsonb, jsonb, uuid);
CREATE FUNCTION block_mutate(p_payload jsonb, p_content jsonb, p_emitter uuid,
                             p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
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
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_block_mutated(v_ev, p_payload, p_content);
END;
$$;

-- ── charter_set ──────────────────────────────────────────────────────────────
DROP FUNCTION cogmap_charter_set(jsonb, jsonb, uuid);
CREATE FUNCTION cogmap_charter_set(p_payload jsonb, p_content jsonb, p_emitter uuid,
                                   p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
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
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_charter_set(v_ev, p_payload, p_content);
END;
$$;
