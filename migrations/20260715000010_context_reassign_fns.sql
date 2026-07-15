-- Context ownership transfer: event-sourced owner change on kb_contexts.
-- Mirrors resource_reassign; this moves (owner_table, owner_id) in place.
-- Additive only: one event-type row + two functions. No table changes.
--
-- kb_contexts is a replay INPUT table (restored verbatim), not a projection, so this
-- projector is an idempotent re-apply on replay (see the context-transfer plan's
-- replay-roundtrip test). This is why an evented context mutation is safe even though
-- context create/share/unshare are un-evented.

-- _event_append raises unless the event name is seeded. NULL payload_schema keeps it
-- out of the published-schema TYPED_EVENT_NAMES invariant (as resource_reassigned).
INSERT INTO kb_event_types (name, payload_schema, schema_version)
VALUES ('context_reassigned', NULL, 1)
ON CONFLICT (name) DO NOTHING;

-- Projection half: set the context's owner to (to_owner_table, to_owner_id).
-- The UNIQUE(owner_table, owner_id, slug) constraint is the backstop for a slug
-- collision under the new owner (the service pre-checks and returns 409 first).
CREATE FUNCTION _project_context_reassigned(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_context uuid := (p_payload->>'context_id')::uuid;
BEGIN
    UPDATE kb_contexts
       SET owner_table = p_payload->>'to_owner_table',
           owner_id    = (p_payload->>'to_owner_id')::uuid
     WHERE id = v_context;
    IF NOT FOUND THEN RAISE EXCEPTION 'context_reassign: context % not found', v_context; END IF;
    RETURN v_context;
END;
$$;

-- Mutation half: append the event anchored to the context itself, then project.
-- Full 5-param act-context signature (matches every mutation fn post-20260709000050).
CREATE FUNCTION context_reassign(p_payload jsonb, p_emitter uuid,
                                 p_metadata jsonb DEFAULT '{}'::jsonb,
                                 p_invocation uuid DEFAULT NULL,
                                 p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_context uuid := (p_payload->>'context_id')::uuid;
BEGIN
    IF NOT EXISTS (SELECT 1 FROM kb_contexts WHERE id = v_context) THEN
        RAISE EXCEPTION 'context_reassign: context % not found', v_context;
    END IF;
    v_ev := _event_append('context_reassigned', p_emitter, 'kb_contexts', v_context, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_context_reassigned(v_ev, p_payload);
END;
$$;
