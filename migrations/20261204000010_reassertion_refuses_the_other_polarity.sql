-- A re-assertion of an active edge with the other polarity is refused, naming the live edge.
--
-- An edge's live identity (uq_kb_edges_assertion) is source, target, kind, label and home. Polarity
-- is not in it, by ruling: "A contains B" asserted forward and then inverse is a contradiction, not
-- a second fact. Until now `_project_relationship_asserted`'s ON CONFLICT reweighted the live edge
-- and left its polarity as it was, so the caller asked for one direction and was answered with the
-- other, and the event recorded a polarity the row never took.
--
-- `relationship_assert` already resolves the live edge of the identity before it appends the event
-- (20261117000030, under the identity's advisory lock and FOR UPDATE on the row). It now also reads
-- that edge's polarity and, when the payload asks for the other one, raises SQLSTATE TF005 BEFORE
-- the append: nothing is recorded and nothing is projected. The message and DETAIL name the live
-- edge's id, which is the handle a caller retypes or folds to change the direction.
--
-- What a later edit must not break:
--   1. THE REFUSAL IS IN THE ACT, NEVER IN THE PROJECTOR. Replay runs `_project_relationship_asserted`
--      over the recorded events, and the ledger already holds re-assertions with the other polarity
--      that collapsed into one row. They must keep projecting exactly as they did: the projector is
--      not touched here.
--   2. NO ROW IS READ OR WRITTEN BY THIS MIGRATION. It replaces one function, so it applies on any
--      install whatever its edges hold, contradictory ones included.
--   3. The payload's polarity defaults to 'forward' when absent, as the projector reads it.
--   4. DETAIL is exactly the live edge's id: temper-substrate decodes TF005 by reading it.
--   5. The rest of the body is 20261117000030's, line for line.

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
        v_asked    edge_polarity := COALESCE(p_payload->>'polarity', 'forward')::edge_polarity;
        v_active   uuid;
        v_live     edge_polarity;
        v_edge     uuid;
BEGIN
    PERFORM pg_advisory_xact_lock(hashtextextended(
        concat_ws(E'\x1f', 'kb_edges_assertion', v_src_tbl, v_src::text, v_tgt_tbl, v_tgt::text,
                  v_kind, v_label, v_home_tbl, v_home::text), 0));

    SELECT e.id, e.polarity INTO v_active, v_live
      FROM kb_edges e
     WHERE e.source_table = v_src_tbl AND e.source_id = v_src
       AND e.target_table = v_tgt_tbl AND e.target_id = v_tgt
       AND e.edge_kind = v_kind::edge_kind
       AND COALESCE(e.label, '') = v_label
       AND e.home_anchor_table = v_home_tbl AND e.home_anchor_id = v_home
       AND NOT e.is_folded
       FOR UPDATE;

    IF v_active IS NOT NULL AND v_live IS DISTINCT FROM v_asked THEN
        RAISE EXCEPTION 'relationship_assert: edge % already asserts this relationship as %; an assertion as % is refused (retype or fold edge % to change its direction)',
            v_active, v_live, v_asked, v_active
            USING ERRCODE = 'TF005', DETAIL = v_active::text;
    END IF;

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
(20261117000030). If that edge holds the other polarity, nothing is appended and SQLSTATE TF005 is
raised, its DETAIL the live edge''s id (20261204000010); the projector does not check polarity, so
replay of earlier contradictory re-assertions is unchanged. Raises serialization_failure (40001,
safe to retry; nothing retries it automatically) rather than record an event whose edge_id the
projection did not answer with.';

SELECT declare_migration(
    20261204000010,
    'additive',
    'CREATE OR REPLACE of relationship_assert with its signature unchanged; no table, column, index or row is touched, so it applies whatever an install''s edges hold. An older binary calls it with the same arguments and reads the same returned id, except that a re-assertion of an active edge with the other polarity now raises SQLSTATE TF005 and records nothing, where it used to record the event and leave the row''s polarity unchanged. An older binary renders that raise as an internal error.'
);
