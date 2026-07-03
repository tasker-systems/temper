-- Ownership reassignment: event-sourced owner change on kb_resource_homes.
-- Mirrors resource_rehomed (anchor move); this moves owner_profile_id in place.
-- Additive only: one event-type row + two functions. No table changes.

-- _event_append raises unless the event name is seeded. NULL payload_schema keeps
-- it out of the published-schema TYPED_EVENT_NAMES invariant (as resource_rehomed).
INSERT INTO kb_event_types (name, payload_schema, schema_version)
VALUES ('resource_reassigned', NULL, 1)
ON CONFLICT (name) DO NOTHING;

-- Projection half (replay-stable): set the resource's home owner to to_profile_id.
CREATE FUNCTION _project_resource_reassigned(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_resource uuid := (p_payload->>'resource_id')::uuid;
BEGIN
    UPDATE kb_resource_homes
       SET owner_profile_id = (p_payload->>'to_profile_id')::uuid
       WHERE resource_id = v_resource;
    IF NOT FOUND THEN RAISE EXCEPTION 'resource_reassign: resource % has no home', v_resource; END IF;
    RETURN v_resource;
END;
$$;

-- Mutation half: append the event at the resource's CURRENT home (it does not move),
-- then project. Act-correlation params mirror resource_rehome (20260629000003).
CREATE FUNCTION resource_reassign(p_payload jsonb, p_emitter uuid,
                                  p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL)
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
                          p_metadata => p_metadata, p_invocation => p_invocation);
    RETURN _project_resource_reassigned(v_ev, p_payload);
END;
$$;
