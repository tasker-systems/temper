-- The edge-erasure guards ask public._replay_mode() (20261203000010) instead of reading
-- `temper.replaying` themselves, as 20261203000020 made the other guards do, so the setting alone,
-- in a database without the kb_replay_target marker, lowers none of them.
--
-- Each function below is re-issued from its latest definition with two changes and no others: its
-- CREATE becomes CREATE OR REPLACE, and each read of the setting becomes a call to
-- public._replay_mode() (`IS DISTINCT FROM 'on'` becomes NOT public._replay_mode(); `IS NOT
-- DISTINCT FROM 'on'` becomes public._replay_mode()). Every other line is that definition's, byte
-- for byte. replay_mode_guard_test holds pg_proc to no inline read.
--
-- Latest definitions re-issued:
--   _resource_write_guard_owner, _project_relationship_folded (two reads),
--   _edge_erasure_apply_redaction
--     20261202000030_person_erasure_folds_estate_homed_edges
--   edge_erasure_requires_an_erased_owner
--     20261202000040_person_erasure_erases_estate_homed_edges
--
-- The COMMENTs that named the setting are re-issued to name _replay_mode, among them
-- _project_edge_erased_redactions, which reaches the trail check through _project_field_redactions.

-- _resource_write_guard_owner
CREATE OR REPLACE FUNCTION _resource_write_guard_owner(p_table text, p_id uuid)
RETURNS void LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_resource uuid;
    v_folded   boolean;
BEGIN
    -- The owner tables kb_properties admits (kb_properties_owner_table_check) and the edge
    -- endpoint tables (kb_edges_{source,target}_table_check). kb_cogmaps and kb_blobs write
    -- into no resource, so they fall through unguarded.
    IF p_table = 'kb_resources' THEN
        PERFORM _resource_write_guard(p_id);
    ELSIF p_table = 'kb_edges' THEN
        FOR v_resource IN
            SELECT e.source_id FROM kb_edges e
             WHERE e.id = p_id AND e.source_table = 'kb_resources'
            UNION
            SELECT e.target_id FROM kb_edges e
             WHERE e.id = p_id AND e.target_table = 'kb_resources'
            ORDER BY 1
        LOOP
            PERFORM _resource_write_guard(v_resource);
        END LOOP;
        -- Replay projects a lawful write whose event sorts after the edge's edge_erased record,
        -- as _resource_write_guard lets one through after a resource_erased.
        IF NOT public._replay_mode() THEN
            SELECT e.is_folded INTO v_folded FROM kb_edges e WHERE e.id = p_id FOR KEY SHARE;
            IF v_folded AND _edge_is_erased(p_id) THEN
                RAISE EXCEPTION 'edge % is erased; writes are refused', p_id;
            END IF;
        END IF;
    ELSIF p_table = 'kb_content_blocks' THEN
        PERFORM _resource_write_guard(
            (SELECT b.resource_id FROM kb_content_blocks b WHERE b.id = p_id));
    END IF;
END;
$$;

-- _project_relationship_folded
CREATE OR REPLACE FUNCTION _project_relationship_folded(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_edge   uuid := (p_payload->>'edge_id')::uuid;
    v_folded boolean;
BEGIN
    IF NOT public._replay_mode() THEN
        SELECT e.is_folded INTO v_folded FROM kb_edges e WHERE e.id = v_edge FOR KEY SHARE;
    ELSE
        SELECT e.is_folded INTO v_folded FROM kb_edges e WHERE e.id = v_edge;
    END IF;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'relationship_fold: edge % not found', v_edge;
    END IF;
    IF NOT public._replay_mode()
       AND v_folded AND _edge_is_erased(v_edge) THEN
        RAISE EXCEPTION 'edge % is erased; writes are refused', v_edge;
    END IF;
    PERFORM _edge_fold_apply(p_event, v_edge);
    RETURN v_edge;
END;
$$;

-- _edge_erasure_apply_redaction
CREATE OR REPLACE FUNCTION _edge_erasure_apply_redaction(p_edge uuid, p_event uuid)
RETURNS void
LANGUAGE plpgsql
SET search_path TO 'public', 'pg_temp'
AS $function$
BEGIN
    IF NOT public._replay_mode()
       AND NOT EXISTS (
           SELECT 1
             FROM kb_events e
             JOIN kb_event_types t ON t.id = e.event_type_id
            WHERE e.id = p_event
              AND t.name = 'edge_erased'
              AND e.payload ->> 'subject_table' = 'kb_edges'
              AND e.payload ->> 'subject_id' = p_edge::text
              AND e.created = now()) THEN
        RAISE EXCEPTION '_edge_erasure_apply_redaction: edge % has no edge_erased record of this transaction',
            p_edge;
    END IF;

    UPDATE kb_edges SET label = NULL WHERE id = p_edge;

    UPDATE kb_properties p
       SET property_key   = 'erased-key-' || k.n::text,
           property_value = '"erased"'::jsonb,
           is_folded      = true,
           last_event_id  = COALESCE(p_event, p.last_event_id)
      FROM kb_edges e
     CROSS JOIN LATERAL _resource_erasure_key_numbers('kb_edges', e.id) k
     WHERE e.id = p_edge
       AND p.owner_table = 'kb_edges' AND p.owner_id = e.id
       AND p.property_key = k.property_key;
END;
$function$;

-- edge_erasure_requires_an_erased_owner
CREATE OR REPLACE FUNCTION edge_erasure_requires_an_erased_owner()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_record    uuid;
    v_is_edge   boolean;
    v_estate_of uuid;
    v_subject   uuid;
BEGIN
    IF public._replay_mode() THEN
        RETURN NULL;
    END IF;
    -- Separate statements: PL/pgSQL resolves a NEW field whenever its expression runs.
    IF TG_TABLE_NAME = 'kb_events' THEN
        v_record := NEW.id;
    ELSE
        v_record := NEW.redacted_by;
    END IF;
    SELECT t.name = 'edge_erased'
      INTO v_is_edge
      FROM kb_events e
      JOIN kb_event_types t ON t.id = e.event_type_id
     WHERE e.id = v_record;
    v_is_edge := coalesce(v_is_edge, false);
    IF TG_TABLE_NAME = 'kb_events' AND NOT v_is_edge THEN
        RETURN NULL;
    END IF;
    -- estate_of and subject_id are read as uuids only from an edge_erased record, whose payload
    -- carries both.
    IF v_is_edge THEN
        SELECT (e.payload ->> 'estate_of')::uuid,
               CASE WHEN e.payload ->> 'subject_table' = 'kb_edges'
                    THEN (e.payload ->> 'subject_id')::uuid END
          INTO v_estate_of, v_subject
          FROM kb_events e
         WHERE e.id = v_record;
        IF NOT EXISTS (SELECT 1 FROM kb_profiles p
                        WHERE p.id = v_estate_of AND p.tombstoned_at IS NOT NULL) THEN
            RAISE EXCEPTION 'edge erasure: the estate''s owner % is not erased', v_estate_of
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'edge_erasure_requires_an_erased_owner';
        END IF;
        -- The record's subject as the verifier admits an `edge` rewrite of it: a record alone, with
        -- no row and no rewrite for the verifier to read, makes _edge_is_erased refuse every write
        -- to the edge it names.
        IF NOT EXISTS (SELECT 1 FROM kb_edges s WHERE s.id = v_subject AND s.is_folded)
           OR NOT _edge_homed_in_estate(v_subject, v_estate_of) THEN
            RAISE EXCEPTION 'edge erasure: edge % is not a folded edge homed in the estate of %',
                v_subject, v_estate_of
                USING ERRCODE = 'check_violation',
                      CONSTRAINT = 'edge_erasure_requires_an_erased_owner';
        END IF;
        RETURN NULL;
    END IF;
    RAISE EXCEPTION 'edge erasure: the estate''s owner % is not erased', v_estate_of
        USING ERRCODE = 'check_violation', CONSTRAINT = 'edge_erasure_requires_an_erased_owner';
END;
$$;

COMMENT ON FUNCTION _edge_erasure_apply_redaction(uuid,uuid) IS
$c$The projection side of erasing one edge: its label set to NULL, and every kb_properties row it
owns, live and folded, set to erased-key-<n> (numbered within the edge by
_resource_erasure_key_numbers), value "erased", folded, last_event_id p_event. Step (9d) of
_resource_erasure_apply_redaction for one edge. Outside replay (_replay_mode false) raises
unless p_event is an edge_erased record naming p_edge (subject_table kb_edges, subject_id) appended
in this transaction. Reads no other payload, so edge_erasure_execute and replay run the same body
at the edge_erased record's position.$c$;

COMMENT ON FUNCTION _project_edge_erased_redactions(uuid,jsonb) IS
$c$The edge_erased redaction projection: one kb_event_field_redactions row per (event, path) of
redacted_fields, authority `edge`, through _project_field_redactions. Every named event is on the
subject edge's own trail (_edge_erasure_trail_scope; skipped under replay, _replay_mode), and every
path is on _erasure_redact_paths for its event's type. The act calls it after appending its record;
replay calls it at the event's position.$c$;

COMMENT ON FUNCTION _project_relationship_folded(uuid,jsonb) IS
$c$The relationship_folded projection: reads the edge row once, raising when it is missing; outside
replay (_replay_mode false) that read takes FOR KEY SHARE, which waits out
edge_erasure_execute's FOR UPDATE, and raises 'edge % is erased; writes are refused' when the row
is folded and _edge_is_erased answers true for the edge (a live edge is never looked up on the
ledger); then folds it through _edge_fold_apply. Guarded, unlike the fold 20260929040730 leaves
unguarded, because a fold's reason is text on the erased edge's own trail.$c$;

COMMENT ON FUNCTION _resource_write_guard_owner(text,uuid) IS
$c$the write guard by owner (spec 2026-09-28 D13): resolves an owner to the resource or resources it
writes into and guards each — kb_resources itself; kb_edges each endpoint whose table is
kb_resources; kb_content_blocks the block's resource_id; kb_cogmaps (and a kb_blobs endpoint)
nothing. For kb_edges it then takes FOR KEY SHARE on the edge row, which waits out
edge_erasure_execute's FOR UPDATE, and RAISEs 'edge % is erased; writes are refused' when the
row it locked is folded and _edge_is_erased answers true for the edge; a live edge is never looked
up on the ledger. Neither the edge lock nor that refusal runs under replay
(_replay_mode). A writer of two edges in descending id order can deadlock against an
act that locks them ascending; the writer has no retry, so that request fails once with a 500.$c$;

COMMENT ON FUNCTION edge_erasure_requires_an_erased_owner() IS
$c$At commit (DEFERRABLE INITIALLY DEFERRED constraint triggers
trg_edge_erased_requires_an_erased_owner on kb_events and
trg_edge_redaction_requires_an_erased_owner on kb_event_field_redactions): refuses the transaction
unless the edge_erased record (the inserted event, or an `edge` row's redacted_by) names in
estate_of a profile whose tombstoned_at is set, and names a kb_edges subject that is folded and
homed in a context of that estate (_edge_homed_in_estate). Confirms the type by name before it
reads estate_of. An `edge` row whose redacted_by is no edge_erased record is refused too; another
kind of event on kb_events passes. Raises 'edge erasure: the estate''s owner % is not erased' or
'edge erasure: edge % is not a folded edge homed in the estate of %' (check_violation). Stands down
under replay (_replay_mode).$c$;

SELECT declare_migration(
    20261203000030,
    'additive',
    'Re-issues four edge-erasure guard functions (_resource_write_guard_owner, '
    '_project_relationship_folded, _edge_erasure_apply_redaction, '
    'edge_erasure_requires_an_erased_owner) with signatures unchanged; each reads _replay_mode() '
    'where it read temper.replaying. Outside replay they behave exactly as before. Five function '
    'COMMENTs re-issued. No table, column or constraint is altered.'
);
