-- A resource that leaves a home says so in that home: the departure.
--
-- Ruled by the owner 2026-11-03 for task 01a1a1a5-773a-7599-a157-6128705ef71e: a context's change
-- feed (20261116000010) has an entry for every change to what lives there, arrivals and departures
-- included, and never for what a thing contains. A move left a context silently: resource_rehome
-- anchors its one event on the DESTINATION, and its payload names no prior home, so the home it
-- left recorded nothing. A deletion already says so: resource_deleted is anchored on the home.
--
-- An erasure writes no departure, by the owner's ruling the same day: an admin act is not a change
-- a context's readers may see as an act, and the feed withholds every admin act's events
-- (_event_is_admin_act, 20261116000010). Its silence there is intended.
--
-- resource_departed is one event anchored on the home being left, carrying only the resource id.
-- It names no destination, so a reader of the old home learns only that the resource left. It
-- projects nothing: the act it belongs to does the work, and replay walks it as a no-op.
-- _resource_departure_append holds the rule in one place. It writes a departure only for a live
-- resource leaving a home it still has, so a resource deleted earlier does not depart twice, and a
-- move to the home it is already in is no departure.
--
-- resource_rehome is replaced body-only, signature unchanged.

INSERT INTO kb_event_types (name, payload_schema, schema_version, category)
VALUES ('resource_departed', NULL, 1, 'domain');

-- The departure of p_resource from its current home, unless it is not live, has no home, or the
-- destination is that same home. p_to_table/p_to_id name the destination of a move. Returns the event id, or NULL when there is no departure.
--
-- The resource row is locked first, so two acts on one resource serialize here: two concurrent
-- moves each read the home the other left, never the same one twice, and a move that overlaps an
-- erasure or a deletion waits for it and reads whether the resource is still live.
-- The read that follows is its own statement, so it sees what the lock waited for. The departure
-- carries no metadata: it is a pointer, and a move's reasoning stays on the move.
CREATE FUNCTION _resource_departure_append(
    p_resource uuid, p_emitter uuid,
    p_to_table text, p_to_id uuid,
    p_correlation uuid, p_invocation uuid DEFAULT NULL
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_table text; v_id uuid;
BEGIN
    PERFORM 1 FROM kb_resources WHERE id = p_resource FOR NO KEY UPDATE;
    SELECT h.anchor_table, h.anchor_id INTO v_table, v_id
      FROM kb_resource_homes h JOIN kb_resources r ON r.id = h.resource_id
     WHERE h.resource_id = p_resource AND r.is_active;
    IF v_id IS NULL OR (v_table = p_to_table AND v_id = p_to_id) THEN
        RETURN NULL;
    END IF;
    RETURN _event_append('resource_departed', p_emitter, v_table, v_id,
                         jsonb_build_object('resource_id', p_resource),
                         p_invocation => p_invocation, p_correlation => p_correlation);
END;
$$;

-- The move: its record on the destination as before, then the departure on the home it leaves,
-- under the same correlation id, read from the record so a move fired without one still pairs.
-- Both before the projection re-points the home.
CREATE OR REPLACE FUNCTION resource_rehome(p_payload jsonb, p_emitter uuid,
                                           p_metadata jsonb DEFAULT '{}'::jsonb,
                                           p_invocation uuid DEFAULT NULL,
                                           p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
BEGIN
    v_ev := _event_append('resource_rehomed', p_emitter,
                          p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    PERFORM _resource_departure_append(
        (p_payload->>'resource_id')::uuid, p_emitter,
        p_payload#>>'{home,table}', (p_payload#>>'{home,id}')::uuid,
        (SELECT correlation_id FROM kb_events WHERE id = v_ev),
        p_invocation);
    RETURN _project_resource_rehomed(v_ev, p_payload);
END;
$$;


SELECT declare_migration(
    20261116000020,
    'additive',
    'Registers the domain event type resource_departed (permissive, NULL payload_schema), adds _resource_departure_append, and replaces resource_rehome body-only with its signature unchanged: a move now also appends a resource_departed anchored on the home it leaves. Nothing is dropped, renamed or reshaped. A binary from before this migration calls resource_rehome unchanged and gets the departure written for it; the one thing it cannot do is replay a ledger holding resource_departed rows, because its EventKind has no arm for the type. Replay is an operator and test tool, not a serving path.'
);
