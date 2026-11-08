-- public_within: anchor_subtree_holds_pool walks up from each pool team instead of down through the
-- anchor's whole subtree. Same answer for every anchor; the cost is the number of pool teams times
-- their depth, not the size of the anchor's subtree and not the size of kb_teams.
--
-- 20261120000005's body filtered every kb_teams row by `id = anchor OR id IN (the anchor's live
-- subtree)`: a scan of every team, per row, for every row an act can flip, under the instance-wide
-- effects lock. A pool team P (the root, the gating team, an auto-join team) is in the anchor's
-- live subtree exactly when P is the anchor, or the anchor is in team_ancestors(P): both walks admit
-- a path only if every team on it, the anchor and P included, is live. Pool teams are few, and found
-- by index (the slug key; the partial auto-join index added here).
--
-- A later edit must keep the two arms: `t.id = p_anchor` regardless of liveness (as before), and the
-- live upward walk, which is the mirror of team_live_descendants. Split from 20261124000020 because
-- it reads kb_teams.auto_join_role, which temper-substrate's pre-auto-join test baseline omits; that
-- harness skips this file by name.

CREATE INDEX kb_teams_auto_join ON kb_teams (id) WHERE auto_join_role IS NOT NULL;

COMMENT ON INDEX kb_teams_auto_join IS
    'The auto-join teams, for anchor_subtree_holds_pool: found by index, never a scan of every team.';

CREATE OR REPLACE FUNCTION anchor_subtree_holds_pool(p_anchor uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
          FROM kb_teams t
         WHERE ( t.auto_join_role IS NOT NULL
              OR t.slug = 'temper-system'
              OR t.slug = (SELECT s.gating_team_slug FROM kb_system_settings s WHERE s.id = 1))
           AND ( t.id = p_anchor
              OR EXISTS (SELECT 1 FROM team_ancestors(t.id) a WHERE a.team_id = p_anchor)));
$$;

COMMENT ON FUNCTION anchor_subtree_holds_pool(uuid) IS
$c$True when the anchor is the root (temper-system), or the anchor itself or a live team beneath it
is an auto-join team or the gating team. Each brings the whole instance into the anchor's subtree
(every personal team sits under the root), so a public_within row over it would read instance-wide;
only an admin's instance_wide row survives it (public_within_row_effective). A pool team below a
dead team is not counted, because it confers nothing upward.

Evaluated from the pool side: for each pool team, is it the anchor (whatever the anchor's liveness),
or is the anchor in its team_ancestors (the live walk up, which reaches the anchor only through live
teams, the anchor and the pool team included)? That is the same set as the anchor's live subtree
(team_live_descendants), read from the few pool teams rather than the anchor's whole subtree.$c$;

SELECT declare_migration(
    20261124000030,
    'additive',
    'public_within pool check: a partial index on kb_teams (id) WHERE auto_join_role IS NOT NULL, '
    'and a body-only CREATE OR REPLACE of anchor_subtree_holds_pool (signature, return type and '
    'volatility unchanged) that answers the same for every anchor by walking up from each pool '
    'team. An older binary calls the same function and gets the same answers.'
);
