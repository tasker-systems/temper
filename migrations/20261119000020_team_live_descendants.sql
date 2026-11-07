-- team_live_descendants: the team DAG walked down, live teams only.
--
-- Bringing an auto-join or gating team under a team hands that team's material to the whole
-- instance, so it is a system admin's act. Attach, reparent and restore checked this only against
-- the child (or the restored team) itself. A subtree that CONTAINS such a team passed. team_service
-- now asks the whole live subtree, and needs a down-walk to do it: team_descendants was dropped in
-- 20260709000020, so this is a new primitive.
--
-- It mirrors team_ancestors (20260703000001) exactly, in the other direction: the seed is returned
-- only if it is live, and the recursion extends only through live children. A soft-deleted team is
-- neither returned nor walked through, because reach (team_ancestors) does not walk through one
-- either: a pool team below a dead team confers nothing upward, so it must not count here.

CREATE FUNCTION team_live_descendants(p_team uuid)
RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE down AS (
        SELECT t.id AS team_id
        FROM kb_teams t
        WHERE t.id = p_team AND t.is_active
        UNION
        SELECT tp.child_id
        FROM kb_teams_parents tp
        JOIN down ON tp.parent_id = down.team_id
        JOIN kb_teams ct ON ct.id = tp.child_id AND ct.is_active
    )
    SELECT team_id FROM down;
$$;

COMMENT ON FUNCTION team_live_descendants(uuid) IS
$c$The team DAG walked DOWN, live teams only: p_team itself if live, then every team reachable
through live child links. The exact mirror of team_ancestors' is_active floor, so the two agree on
what a dead team means: neither returned nor walked through. Used by team_service's subtree check
for attach, reparent and restore (is there an auto-join or gating team below?), and to lock that
subtree's rows before the check is repeated. A structural primitive, not a visibility arm: it never
decides what anyone reads.$c$;

SELECT declare_migration(
    20261119000020,
    'additive',
    'Adds team_live_descendants(p_team), the live-only DAG down-walk mirroring team_ancestors'' '
    'is_active floor. New function only; nothing existing changes, so an older binary is '
    'unaffected. The new binary''s team_service uses it to refuse a non-admin attach, reparent or '
    'restore whose live subtree contains an auto-join or gating team.'
);
