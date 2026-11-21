-- Person erasure folds and erases the edges homed in the erased person's estate.
--
-- An edge homed in a context of the estate that no resource erasure reached keeps its label, its
-- facets and its ledger text after the person act. This migration adds the per-edge unit that
-- erases one such edge under the `edge` authority (20261201000050_edge_erasure_authority.sql).
--
--   1. _edge_erasure_apply_redaction(uuid, uuid): the projection body for one edge. Its label is
--      set to NULL and every property row it owns, live and folded, to erased-key-<n> / "erased"
--      and folded: step (9d) of _resource_erasure_apply_redaction
--      (20261127000010_block_content_hash_follows_redaction.sql) for this one edge.
--   2. edge_erasure_execute(uuid, uuid, uuid, uuid, uuid): the unit. It locks the edge, refuses an
--      edge not homed in _erasure_estate_contexts(estate_of), derives the ledger redaction, folds
--      the edge when it is live, appends one edge_erased record, projects its rows, runs the body,
--      then rewrites the edge's trail in one UPDATE.
--
-- A later edit must not break:
--   * derive before fold: _edge_erasure_payload_redaction runs before the fold event exists and
--     before the body rewrites the projection, as resource_erasure_execute's derivation does;
--   * the record follows the fold, and the trail UPDATE follows the body: the verifier admits an
--     `edge` rewrite only of a folded edge, homed in the estate the record names;
--   * the body reads no payload, so replay can call it at the record's position;
--   * a refusal raises before anything is appended.

-- ---------------------------------------------------------------------------
-- Section 1. The projection body for one edge.
-- ---------------------------------------------------------------------------
-- Every kb_edges-owned row is folded, the uq_kb_properties_active reason
-- _resource_erasure_apply_redaction gives: two live rows of one key would map to one
-- (erased-key-<n>, "erased") pair. Keys are numbered within the edge by
-- _resource_erasure_key_numbers, in ledger order of first appearance.
CREATE FUNCTION _edge_erasure_apply_redaction(p_edge uuid, p_event uuid)
RETURNS void
LANGUAGE plpgsql
SET search_path TO 'public', 'pg_temp'
AS $function$
BEGIN
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

COMMENT ON FUNCTION _edge_erasure_apply_redaction(uuid, uuid) IS
$c$The projection side of erasing one edge: its label set to NULL, and every kb_properties row it
owns, live and folded, set to erased-key-<n> (numbered within the edge by
_resource_erasure_key_numbers), value "erased", folded, last_event_id p_event. Step (9d) of
_resource_erasure_apply_redaction for one edge. Reads no payload, so edge_erasure_execute and replay
run the same body at the edge_erased record's position.$c$;

-- ---------------------------------------------------------------------------
-- Section 2. The unit.
-- ---------------------------------------------------------------------------
-- The edge row lock serializes two units on one edge: the second reads the first's committed
-- state and raises `already erased`. An edge already folded is erased without a new fold, and its
-- record says so (folded false). The record carries `folded` in both cases.
CREATE FUNCTION edge_erasure_execute(p_edge uuid, p_estate_of uuid, p_operator uuid,
                                     p_emitter uuid, p_request_ref uuid)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path TO 'public', 'pg_temp'
SET lock_timeout TO '0'
AS $function$
DECLARE
    v_found      boolean;
    v_was_folded boolean;
    v_label      text;
    v_home_table text;
    v_home_id    uuid;
    v_redact     jsonb;
    v_fields     jsonb;
    v_fold       jsonb;
    v_ev         uuid;
    v_payload    jsonb;
BEGIN
    IF p_edge IS NULL THEN
        RAISE EXCEPTION 'edge_erasure_execute: p_edge is required';
    END IF;
    -- The request reference is the act's correlation id: every event the unit appends carries it.
    IF p_request_ref IS NULL THEN
        RAISE EXCEPTION 'edge_erasure_execute: p_request_ref is required';
    END IF;

    SELECT true, e.is_folded, e.label, e.home_anchor_table, e.home_anchor_id
      INTO v_found, v_was_folded, v_label, v_home_table, v_home_id
      FROM kb_edges e
     WHERE e.id = p_edge
       FOR UPDATE;
    IF v_found IS NULL THEN
        RAISE EXCEPTION 'edge_erasure_execute: edge % not found', p_edge;
    END IF;
    IF v_home_table IS DISTINCT FROM 'kb_contexts'
       OR v_home_id NOT IN (SELECT _erasure_estate_contexts(p_estate_of)) THEN
        RAISE EXCEPTION 'edge_erasure_execute: edge % is not homed in the estate of %',
            p_edge, p_estate_of;
    END IF;

    -- The ledger redaction, derived before the fold event exists and before the body rewrites
    -- the projection.
    SELECT coalesce(jsonb_agg(jsonb_build_object('event', d.event_id, 'payload', d.new_payload,
                                                 'metadata', d.new_metadata, 'paths', d.paths)
                               ORDER BY d.event_id), '[]'::jsonb)
      INTO v_redact
      FROM _edge_erasure_payload_redaction(p_edge) d;
    SELECT coalesce(jsonb_agg(jsonb_build_object('event', r->'event', 'paths', r->'paths')
                               ORDER BY r->>'event'), '[]'::jsonb)
      INTO v_fields
      FROM jsonb_array_elements(v_redact) r;

    IF v_was_folded
       AND v_label IS NULL
       AND jsonb_array_length(v_redact) = 0
       AND NOT EXISTS (
               SELECT 1 FROM kb_properties p
                WHERE p.owner_table = 'kb_edges' AND p.owner_id = p_edge
                  AND p.property_key !~ '^erased-key-[1-9][0-9]*$') THEN
        RAISE EXCEPTION 'edge_erasure_execute: already erased';
    END IF;

    -- A live edge is folded by the incumbent verb, at its home, reason the fixed literal
    -- 'estate_erased'.
    IF NOT v_was_folded THEN
        v_fold := jsonb_build_object('edge_id', p_edge, 'reason', 'estate_erased');
        v_ev := _event_append('relationship_folded', p_emitter, v_home_table, v_home_id, v_fold,
                              p_correlation => p_request_ref);
        PERFORM _project_relationship_folded(v_ev, v_fold);
    END IF;

    v_payload := jsonb_build_object(
            'subject_table', 'kb_edges',
            'subject_id', p_edge,
            'actor', p_operator,
            'estate_of', p_estate_of,
            'folded', NOT v_was_folded,
            'redacted_fields', v_fields);
    v_ev := _event_append(
        'edge_erased', p_emitter, NULL, NULL, v_payload,
        p_references => jsonb_build_array(
            jsonb_build_object('rel','subject',
                'target', jsonb_build_object('kind','kb_edges','id', p_edge)),
            jsonb_build_object('rel','request',
                'target', jsonb_build_object('kind','kb_events','id', p_request_ref))),
        p_correlation => p_request_ref);

    PERFORM _project_edge_erased_redactions(v_ev, v_payload);

    PERFORM _edge_erasure_apply_redaction(p_edge, v_ev);

    -- kb_events_append_only admits each row against the record's redaction rows, the folded edge's
    -- home in the named estate and the class sentinels; kb_events_redaction_in_trail checks the
    -- edge's trail once for the statement.
    UPDATE kb_events e
       SET payload  = r->'payload',
           metadata = r->'metadata'
      FROM jsonb_array_elements(v_redact) r
     WHERE e.id = (r->>'event')::uuid;

    RETURN jsonb_build_object(
        'event_id',        v_ev,
        'folded',          NOT v_was_folded,
        'redacted_fields', v_fields);
END;
$function$;

COMMENT ON FUNCTION edge_erasure_execute(uuid, uuid, uuid, uuid, uuid) IS
$c$Erase one edge homed in a context of _erasure_estate_contexts(p_estate_of), under the `edge`
authority. Locks the edge row; raises when it is missing, not homed in that estate, or already
erased (folded, no label, nothing left to rewrite on its trail, every owned property key at its
sentinel). Derives _edge_erasure_payload_redaction, folds a live edge (relationship_folded, reason
estate_erased), appends one edge_erased record (subject, actor, estate_of, folded, redacted_fields;
references subject and request), projects its rows, runs _edge_erasure_apply_redaction, then
rewrites the derived events in one UPDATE. Every event carries p_request_ref as its correlation.
Returns {event_id, folded, redacted_fields}.$c$;

SELECT declare_migration(
    20261201000060,
    'additive',
    'Adds the functions _edge_erasure_apply_redaction(uuid, uuid) and '
    'edge_erasure_execute(uuid, uuid, uuid, uuid, uuid), each with a COMMENT. Nothing existing '
    'calls them, so every existing answer is unchanged. No table, column, constraint, event type '
    'or grant changes.'
);
