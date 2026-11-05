-- A re-assertion of an active edge records the id of the edge it updated.
--
-- `fire`'s RelationshipAssert arm mints a fresh `edge_id` for every assertion. When an active edge
-- with the same identity (uq_kb_edges_assertion) already exists, `_project_relationship_asserted`'s
-- ON CONFLICT reweights THAT edge and returns ITS id. So the event's payload named an id that never
-- became a row: replay, `element_trail_edge`, the change feed and every reader keyed on
-- `payload->>'edge_id'` saw a phantom edge. Production held 243 such events on 2026-10-08
-- (20261009100000 §6).
--
-- Section 1 fixes the write. Before the event is appended, `relationship_assert` resolves the
-- active edge of that identity and writes its id into the payload, so the event and the row agree.
--   * A transaction-scoped advisory lock keyed on the identity serialises two assertions of an
--     identity that has no active edge yet. Without it both read "none", both mint, and the loser's
--     ON CONFLICT answers with the winner's row while its event names its own minted id.
--   * The lookup takes `FOR UPDATE` in the same statement as `NOT is_folded`, as 20260930000070
--     does. A fold that commits first is re-checked out under READ COMMITTED, so the assertion
--     mints a fresh edge instead of naming a folded one.
--   * After projecting, the function raises if the projector returned any other id. A race the
--     lock does not cover (a retype that moves another edge into this identity concurrently) rolls
--     the assertion back. It does not record a phantom. This check lives here, on the fire path,
--     and NOT in the projector: replay runs the projector over the historical events, whose
--     payloads still name phantoms and must keep projecting as they always have.
-- The projector is unchanged, and so is replay. Rust still mints an id. The SQL replaces it only
-- when an active edge exists.
--
-- Section 2 lets the edge trail reach the historical events. Recorded events are never edited,
-- so the phantoms stay in the ledger. `element_trail_edge` matched only on `payload->>'edge_id'`,
-- so an edge's own trail left out every re-assertion that had reweighted it. A new arm reads those
-- events by identity (`_edge_phantom_reassertions`). Section 2's header gives the attribution rule.

-- ---------------------------------------------------------------------------
-- Section 1. The write records the edge it updated.
-- ---------------------------------------------------------------------------
-- The body is 20260709000050's, with the resolve step before the append and the check after the
-- projection. The signature is unchanged, so CREATE OR REPLACE applies.
CREATE OR REPLACE FUNCTION relationship_assert(p_payload jsonb, p_emitter uuid,
                                               p_metadata jsonb DEFAULT '{}'::jsonb,
                                               p_invocation uuid DEFAULT NULL,
                                               p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev       uuid;
        v_payload  jsonb := p_payload;
        v_src_tbl  text  := p_payload#>>'{source,table}';
        v_src      uuid  := (p_payload#>>'{source,id}')::uuid;
        v_tgt_tbl  text  := p_payload#>>'{target,table}';
        v_tgt      uuid  := (p_payload#>>'{target,id}')::uuid;
        v_kind     text  := p_payload->>'edge_kind';
        v_label    text  := COALESCE(p_payload->>'label', '');
        v_home_tbl text  := p_payload#>>'{home,table}';
        v_home     uuid  := (p_payload#>>'{home,id}')::uuid;
        v_active   uuid;
        v_edge     uuid;
BEGIN
    PERFORM pg_advisory_xact_lock(hashtextextended(
        concat_ws(E'\x1f', 'kb_edges_assertion', v_src_tbl, v_src::text, v_tgt_tbl, v_tgt::text,
                  v_kind, v_label, v_home_tbl, v_home::text), 0));

    SELECT e.id INTO v_active
      FROM kb_edges e
     WHERE e.source_table = v_src_tbl AND e.source_id = v_src
       AND e.target_table = v_tgt_tbl AND e.target_id = v_tgt
       AND e.edge_kind = v_kind::edge_kind
       AND COALESCE(e.label, '') = v_label
       AND e.home_anchor_table = v_home_tbl AND e.home_anchor_id = v_home
       AND NOT e.is_folded
       FOR UPDATE;

    IF v_active IS NOT NULL THEN
        v_payload := jsonb_set(v_payload, '{edge_id}', to_jsonb(v_active::text));
    END IF;

    v_ev := _event_append('relationship_asserted', p_emitter, v_home_tbl, v_home, v_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    v_edge := _project_relationship_asserted(v_ev, v_payload);

    IF v_edge IS DISTINCT FROM (v_payload->>'edge_id')::uuid THEN
        RAISE EXCEPTION 'relationship_assert: event % names edge % but the projection answered %',
            v_ev, v_payload->>'edge_id', v_edge
            USING ERRCODE = 'serialization_failure';
    END IF;
    RETURN v_edge;
END;
$$;

COMMENT ON FUNCTION relationship_assert(jsonb, uuid, jsonb, uuid, uuid) IS
'Assert a typed edge, homed per the payload: append relationship_asserted and project it, in one
transaction. When an active edge of the same identity (uq_kb_edges_assertion) exists, the
payload''s edge_id is replaced with that edge''s id BEFORE the append, under an identity-keyed
advisory lock and FOR UPDATE on the row, so the recorded event names the row it updated
(20261117000030). Raises serialization_failure (40001, safe to retry; nothing retries it automatically) rather than record an event whose
edge_id the projection did not answer with. Events from before 20261117000030 may still name an id
with no row; _edge_phantom_reassertions reads them.';

-- ---------------------------------------------------------------------------
-- Section 2. The edge trail reaches the historical re-assertions.
-- ---------------------------------------------------------------------------
-- A historical re-assertion belongs to edge E when all of the following hold:
--   * it is a relationship_asserted event whose payload edge_id has no kb_edges row;
--   * its source, target, home and label equal those of E's own asserting event (both sides are
--     ledger payloads, so an erasure that numbers a label numbers both the same way);
--   * its edge_kind equals E's kind at that moment: the kind of the latest event at or before it
--     among E's asserting event and E's relationship_retyped events;
--   * it falls inside E's active window: after E's asserting event and before E's first
--     relationship_folded event, if E has one. An identity folded and later re-asserted yields a
--     new edge with its own window, so a re-assertion is attributed to one edge only.
-- The window is by event id. Event ids are uuid_generate_v7, the order every trail sorts by.
-- The candidates come from idx_kb_events_payload_source_id (20261009100000), which is text-typed,
-- so the comparison is on text.
CREATE OR REPLACE FUNCTION _edge_phantom_reassertions(p_edge uuid)
RETURNS TABLE(event_id uuid) LANGUAGE sql STABLE AS $$
    WITH edge AS (
        SELECT a.id AS asserted_ev, a.payload AS ap
          FROM kb_edges e
          JOIN kb_events a ON a.id = e.asserted_by_event_id
         WHERE e.id = p_edge
    ), fold AS (
        -- A scalar subquery, so an edge with no fold still yields one row (folded_ev NULL).
        SELECT (SELECT f.id
                  FROM kb_events f
                  JOIN kb_event_types ft ON ft.id = f.event_type_id
                 WHERE (f.payload ->> 'edge_id')::uuid = p_edge
                   AND ft.name = 'relationship_folded'
                 ORDER BY f.id
                 LIMIT 1) AS folded_ev
    )
    SELECT p.id
      FROM edge
      CROSS JOIN fold
      JOIN kb_events p
        ON (p.payload -> 'source') ->> 'id' = (edge.ap -> 'source') ->> 'id'
      JOIN kb_event_types pt ON pt.id = p.event_type_id
     WHERE pt.name = 'relationship_asserted'
       AND p.producing_anchor_table IS NOT NULL
       AND p.id > edge.asserted_ev
       AND (fold.folded_ev IS NULL OR p.id < fold.folded_ev)
       AND p.payload #>> '{source,table}' = edge.ap #>> '{source,table}'
       AND p.payload #>> '{target,table}' = edge.ap #>> '{target,table}'
       AND p.payload #>> '{target,id}'    = edge.ap #>> '{target,id}'
       AND p.payload #>> '{home,table}'   = edge.ap #>> '{home,table}'
       AND p.payload #>> '{home,id}'      = edge.ap #>> '{home,id}'
       AND COALESCE(p.payload ->> 'label', '') = COALESCE(edge.ap ->> 'label', '')
       AND NOT EXISTS (SELECT 1 FROM kb_edges x WHERE x.id = (p.payload ->> 'edge_id')::uuid)
       AND p.payload ->> 'edge_kind' = (
            SELECT k.payload ->> 'edge_kind'
              FROM kb_events k
              JOIN kb_event_types kt ON kt.id = k.event_type_id
             WHERE k.id <= p.id
               AND (k.id = edge.asserted_ev
                    OR (kt.name = 'relationship_retyped'
                        AND (k.payload ->> 'edge_id')::uuid = p_edge))
             ORDER BY k.id DESC
             LIMIT 1);
$$;

COMMENT ON FUNCTION _edge_phantom_reassertions(uuid) IS
'The relationship_asserted events from before 20261117000030 that re-asserted edge p_edge while
it was active, but whose payload names a minted edge_id that never became a row. Matched by the
identity of p_edge''s asserting event, the kind p_edge held at the time (by its retypes), and
p_edge''s active window (its asserting event up to its first fold). Read by element_trail_edge.
After 20261117000030 a re-assertion names its edge, so this set does not grow.';

-- element_trail_edge: 20261007100000's body verbatim, plus the third ev_ids arm.
CREATE OR REPLACE FUNCTION public.element_trail_edge(p_profile uuid, p_edge uuid)
 RETURNS TABLE(event_id uuid, kind text, actor_entity_id uuid, occurred_at timestamp with time zone, metadata jsonb, payload jsonb, actor_name text, correlation_id uuid)
 LANGUAGE sql
 STABLE
AS $function$
    WITH ev_ids AS (
        SELECT ev.id FROM kb_events ev
         WHERE (ev.payload ->> 'edge_id')::uuid = p_edge
           AND ev.producing_anchor_table IS NOT NULL
        UNION
        SELECT ev.id FROM kb_events ev
         WHERE ev.payload -> 'owner' ->> 'table' = 'kb_edges'
           AND (ev.payload -> 'owner' ->> 'id')::uuid = p_edge
           AND ev.producing_anchor_table IS NOT NULL
        UNION
        SELECT ph.event_id FROM _edge_phantom_reassertions(p_edge) ph
    )
    SELECT ev.id, et.name, ev.emitter_entity_id, ev.occurred_at, ev.metadata, ev.payload, en.name,
           ev.correlation_id
    FROM kb_edges edg
    JOIN ev_ids ON TRUE
    JOIN kb_events ev ON ev.id = ev_ids.id
    JOIN kb_event_types et ON et.id = ev.event_type_id
    JOIN kb_entities en ON en.id = ev.emitter_entity_id
    WHERE edg.id = p_edge
      AND et.category = 'domain'
      AND et.name <> 'webhook_received'
      AND anchor_readable_by_profile(p_profile, edg.home_anchor_table, edg.home_anchor_id)
      AND endpoint_readable_by_profile(p_profile, edg.source_table, edg.source_id)
      AND endpoint_readable_by_profile(p_profile, edg.target_table, edg.target_id)
    ORDER BY ev.id;
$function$
;

SELECT declare_migration(
    20261117000030,
    'additive',
    'CREATE OR REPLACE of relationship_assert and element_trail_edge with unchanged signatures, plus a new STABLE helper _edge_phantom_reassertions; no table, column or index changes. An older binary calls relationship_assert with the same arguments and reads the same returned id; the only difference it can see is a re-assertion event naming the existing edge instead of a minted id, and a rare serialization_failure (SQLSTATE 40001) on a lost race, which surfaces as an error; nothing retries it automatically. An older binary reading element_trail_edge gets the same columns, with the re-assertions it used to miss.'
);
