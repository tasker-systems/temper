-- public_within: an act's time under the instance-wide effects lock grows linearly with the rows it
-- flips. No answer changes: same signatures, same lists in the same order, same records.
--
-- Measured with scripts/visibility-cost/public_within_effects_cost.py at 5,000 rows, after
-- 20261124000020 and 20261124000030 had flattened the per-row predicate:
--   * JIT: the planner priced _public_within_effects_before's scan high enough to JIT-compile it,
--     about 300 ms per call, every act, whatever it reached;
--   * _public_within_transitions appended each record to a jsonb array (v := v || ...), which copies
--     the array every time: about 2 s of a root forbid's 3 s at 5,000 flips.
--
--   _public_within_effects_before   gains SET jit = off (ALTER FUNCTION; the body is untouched).
--   _public_within_transitions      body-only CREATE OR REPLACE: SET jit = off, and both lists built
--                                   as jsonb[] and converted once at the end.
--
-- A later edit must keep the lists' order (team_id, anchor_id, as the loops read them) and must not
-- go back to appending to a jsonb value in a loop.

ALTER FUNCTION _public_within_effects_before(uuid[], uuid[]) SET jit = off;

CREATE OR REPLACE FUNCTION _public_within_transitions(p_before jsonb, p_act text, p_correlation uuid)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_temp
SET jit = off
AS $$
DECLARE
    v_system  uuid;
    v_row     record;
    v_lapsed  jsonb[] := '{}';
    v_changes jsonb[] := '{}';
BEGIN
    IF p_before IS NULL OR p_before = '{}'::jsonb THEN
        RETURN jsonb_build_object('lapsed', '[]'::jsonb, 'changes', '[]'::jsonb);
    END IF;

    SELECT e.id INTO v_system
      FROM kb_entities e
      JOIN kb_profiles p ON p.id = e.profile_id
     WHERE p.handle = 'system' AND e.name = 'system';

    FOR v_row IN
        WITH gone AS (
            DELETE FROM kb_team_public_within pw
             WHERE p_before ? (pw.team_id::text || ':' || pw.anchor_id::text)
               AND NOT EXISTS (SELECT 1 FROM team_ancestors_any(pw.team_id) a
                                WHERE a.team_id = pw.anchor_id)
            RETURNING pw.team_id, pw.anchor_id, pw.admin_act, pw.instance_wide
        )
        SELECT * FROM gone ORDER BY team_id, anchor_id
    LOOP
        PERFORM _event_append(
            'team_public_within_changed', v_system, NULL, NULL,
            jsonb_build_object(
                'team_id',       v_row.team_id,
                'anchor_id',     v_row.anchor_id,
                'act',           'lapsed',
                'admin_act',     v_row.admin_act,
                'instance_wide', v_row.instance_wide),
            p_references => jsonb_build_array(
                jsonb_build_object('rel', 'subject',
                                   'target', jsonb_build_object('kind', 'kb_teams', 'id', v_row.team_id)),
                jsonb_build_object('rel', 'subject',
                                   'target', jsonb_build_object('kind', 'kb_teams', 'id', v_row.anchor_id))),
            p_correlation => p_correlation);
        v_lapsed := array_append(v_lapsed, jsonb_build_object(
            'team_id', v_row.team_id, 'anchor_id', v_row.anchor_id));
    END LOOP;

    FOR v_row IN
        SELECT s.team_id, s.anchor_id, s.was, s.was_live,
               public_within_row_effective(s.team_id, s.anchor_id, s.instance_wide) AS now,
               EXISTS (SELECT 1 FROM team_ancestors(s.team_id) a
                        WHERE a.team_id = s.anchor_id) AS now_live
          FROM (SELECT pw.team_id, pw.anchor_id, pw.instance_wide,
                       (p_before -> (pw.team_id::text || ':' || pw.anchor_id::text)
                                 ->> 'effective')::boolean AS was,
                       (p_before -> (pw.team_id::text || ':' || pw.anchor_id::text)
                                 ->> 'live')::boolean AS was_live
                  FROM kb_team_public_within pw
                 WHERE p_before ? (pw.team_id::text || ':' || pw.anchor_id::text)) s
         ORDER BY s.team_id, s.anchor_id
    LOOP
        CONTINUE WHEN v_row.was = v_row.now;
        PERFORM _event_append(
            'public_within_effect_changed', v_system, NULL, NULL,
            jsonb_strip_nulls(jsonb_build_object(
                'team_id',   v_row.team_id,
                'anchor_id', v_row.anchor_id,
                'from',      CASE WHEN v_row.was THEN 'effective' ELSE 'dormant' END,
                'to',        CASE WHEN v_row.now THEN 'effective' ELSE 'dormant' END,
                'cause',     CASE WHEN v_row.was_live IS DISTINCT FROM v_row.now_live
                                  THEN 'liveness' ELSE 'policy' END,
                'act',       p_act,
                'act_ref',   p_correlation)),
            p_references => jsonb_build_array(
                jsonb_build_object('rel', 'subject',
                                   'target', jsonb_build_object('kind', 'kb_teams', 'id', v_row.team_id)),
                jsonb_build_object('rel', 'subject',
                                   'target', jsonb_build_object('kind', 'kb_teams', 'id', v_row.anchor_id))),
            p_correlation => p_correlation);
        v_changes := array_append(v_changes, jsonb_build_object(
            'team_id',   v_row.team_id,
            'anchor_id', v_row.anchor_id,
            'effective', v_row.now));
    END LOOP;

    RETURN jsonb_build_object('lapsed', to_jsonb(v_lapsed), 'changes', to_jsonb(v_changes));
END;
$$;

COMMENT ON FUNCTION _public_within_transitions(jsonb, text, uuid) IS
$c$The second half of the one definition of a public_within transition, run after an act wrote,
over the rows _public_within_effects_before snapshotted. Answers jsonb
{lapsed: [{team_id, anchor_id}], changes: [{team_id, anchor_id, effective}]}.

  * Structural lapse: a row whose anchor is no longer in team_ancestors_any(team) (no path, live or
    dead, runs from the anchor down to the team) is DELETED and recorded as
    team_public_within_changed lapsed. It never revives: re-linking the teams does not bring it back,
    and a reopen needs both sides again. Liveness and policy never delete a row.
  * Every other row whose effect flipped is recorded as public_within_effect_changed, from and to,
    with cause liveness when its live-chain clause flipped and policy otherwise (a forbid, the
    gating team, or a pool at the anchor), act = p_act (the causing act's event type, or NULL) and
    act_ref = p_correlation.

Every record names both teams as subject references, so the managers of either side read it, and
carries p_correlation, the causing act's correlation id. Every record is emitted by the system
entity, never the actor: an act's answer names a row only to a manager of its team or anchor or an
admin, and a record emitted by the actor would reach the actor's own history in the ledger, which is
not gated by subject, and name every row the answer withheld. Who caused it is the act's own record,
joined by the correlation id.

A row deleted or closed by another transaction since the snapshot is skipped. It never authorizes.

It runs with jit off (as does _public_within_effects_before): the planner prices the per-row
effect predicate high enough to compile every snapshot and every scan, which costs more than the
scan. Each list is built as an array and converted once, so an act that flips n rows does O(n)
work, not the O(n^2) of appending to a jsonb array row by row.$c$;

SELECT declare_migration(
    20261124000040,
    'additive',
    'public_within transitions cost: ALTER FUNCTION _public_within_effects_before SET jit = off, and '
    'a body-only CREATE OR REPLACE of _public_within_transitions (signature and return type '
    'unchanged; SET jit = off; its two answer lists built as arrays). Same answers, same records, '
    'same order, so an older binary is unaffected.'
);
