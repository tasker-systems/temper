-- Backfill historical goal→task membership onto the canonical `advances` edge (task 019f468b).
--
-- Task→goal membership was written two divergent ways in prod. The current create/update projection
-- mints a `(leads_to, forward, 'advances')` edge task→goal (GOAL_EDGE_LABEL); the `--goal` list filter
-- and `--clear-goal` fold both key on exactly that shape. But historically the same fact was recorded
-- as a `(contains, forward, 'parent_of')` edge goal→task (EdgeType::ParentOf, graph.rs) — reversed
-- direction AND a different edge_kind. `parent_of` stopped being written on 2026-06-28; `advances` is
-- current. The filter only sees `advances`, so tasks linked the historical way are silently invisible
-- to `list --type task --goal <ref>`, and `--clear-goal` cannot retract them.
--
-- This converges the two onto ONE canonical representation: for every live `parent_of` edge whose
-- source is a `goal` and target is a `task`, assert the equivalent `advances` edge task→goal (homed on
-- the task's own anchor, mirroring assert_edge_from_source_home) and fold the `parent_of` edge. Scoped
-- to the goal→task doc-type pair ONLY — `parent_of` is also used goal→research and task→task, which are
-- unrelated to goal membership and left untouched.
--
-- Additive + data-only (no schema change). The mutation goes through the canonical event functions
-- (`relationship_assert`/`relationship_fold`), so the event log stays the source of truth and replay
-- reproduces the state. Re-runnable: the assert is idempotent on the active-edge invariant
-- (_project_relationship_asserted ON CONFLICT) and the fold self-excludes already-folded edges, so a
-- second invocation converts nothing. The conversion lives in a persisted function so it is invocable
-- again for verification/repair (and so tests can exercise the historical-edge path against a fresh DB).

CREATE OR REPLACE FUNCTION backfill_goal_parent_of_to_advances()
RETURNS integer LANGUAGE plpgsql AS $fn$
DECLARE
    v_emitter uuid := (SELECT e.id FROM kb_entities e
                         JOIN kb_profiles p ON p.id = e.profile_id
                        WHERE p.handle = 'system' AND e.name = 'system');
    r         RECORD;
    v_count   integer := 0;
BEGIN
    IF v_emitter IS NULL THEN
        RAISE EXCEPTION 'backfill_goal_parent_of_to_advances: system emitter entity not found';
    END IF;

    FOR r IN
        SELECT e.id            AS edge_id,
               e.source_id     AS goal_id,
               e.target_id     AS task_id,
               th.anchor_table AS home_table,
               th.anchor_id    AS home_id
          FROM kb_edges e
          -- source is a goal, target is a task (the ONLY pair this backfill touches)
          JOIN kb_properties sp
            ON sp.owner_table = 'kb_resources' AND sp.owner_id = e.source_id
           AND sp.property_key = 'doc_type' AND NOT sp.is_folded
           AND sp.property_value #>> '{}' = 'goal'
          JOIN kb_properties tp
            ON tp.owner_table = 'kb_resources' AND tp.owner_id = e.target_id
           AND tp.property_key = 'doc_type' AND NOT tp.is_folded
           AND tp.property_value #>> '{}' = 'task'
          -- the new advances edge homes on the TASK's own anchor (context or cogmap), as
          -- assert_edge_from_source_home does for the live create/update projection
          JOIN LATERAL (
                SELECT anchor_table, anchor_id
                  FROM kb_resource_homes
                 WHERE resource_id = e.target_id
                   AND anchor_table IN ('kb_contexts', 'kb_cogmaps')
                 ORDER BY (anchor_table = 'kb_cogmaps') DESC
                 LIMIT 1
              ) th ON true
         WHERE e.source_table = 'kb_resources'
           AND e.target_table = 'kb_resources'
           AND e.edge_kind    = 'contains'
           AND e.label        = 'parent_of'
           AND NOT e.is_folded
    LOOP
        -- Assert the canonical advances edge task→goal. Idempotent: if the task already carries an
        -- advances edge to this goal on the same home, ON CONFLICT returns the existing edge.
        PERFORM relationship_assert(
            jsonb_build_object(
                'edge_id',   uuid_generate_v7(),
                'source',    jsonb_build_object('table', 'kb_resources', 'id', r.task_id),
                'target',    jsonb_build_object('table', 'kb_resources', 'id', r.goal_id),
                'edge_kind', 'leads_to',
                'polarity',  'forward',
                'label',     'advances',
                'weight',    1.0,
                'home',      jsonb_build_object('table', r.home_table, 'id', r.home_id)
            ),
            v_emitter
        );

        -- Retire the historical parent_of edge now that the canonical one exists.
        PERFORM relationship_fold(
            jsonb_build_object('edge_id', r.edge_id,
                               'reason', 'goal membership canonicalized to advances (task 019f468b)'),
            v_emitter
        );

        v_count := v_count + 1;
    END LOOP;

    RETURN v_count;
END
$fn$;

-- Run the one-time convergence. Safe to re-run (idempotent, per above).
SELECT backfill_goal_parent_of_to_advances();
