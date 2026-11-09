-- public_within: indexes that keep the per-row cost of an effect computation flat as an instance
-- grows. No function body changes here, so no answer changes.
--
-- Every act that can change a row's effect evaluates public_within_row_effective once before and
-- once after for every row it can flip, under the instance-wide effects lock
-- (_public_within_effects_lock). Two of that predicate's reads scanned all of kb_teams per row:
--
--   kb_teams_parents_parent_id         the downward walks (team_live_descendants,
--                                      _public_within_effects_before's `down`, every_team_below)
--                                      join kb_teams_parents on parent_id, which only the primary
--                                      key (child_id, parent_id) covered as its second column.
--   kb_teams_public_within_forbidden   public_within_allowed_over looks for a forbidding team; the
--                                      planner began from a scan of every team for the flag. The
--                                      partial index holds only forbidding teams, a handful at most.
--
-- The pool half of the predicate is 20261124000030 (it reads auto_join_role, which one test baseline
-- omits). Measured with scripts/visibility-cost/public_within_effects_cost.py.

CREATE INDEX kb_teams_parents_parent_id ON kb_teams_parents (parent_id);

COMMENT ON INDEX kb_teams_parents_parent_id IS
    'The downward team walks (children of a team). The primary key leads with child_id.';

CREATE INDEX kb_teams_public_within_forbidden ON kb_teams (id) WHERE public_within_forbidden;

COMMENT ON INDEX kb_teams_public_within_forbidden IS
    'The forbidding teams, for public_within_allowed_over: a handful, never a scan of every team.';

SELECT declare_migration(
    20261124000020,
    'additive',
    'public_within effect cost: two new indexes, kb_teams_parents (parent_id) and a partial index '
    'on kb_teams (id) WHERE public_within_forbidden. No function, column or row changes, so no '
    'answer changes and an older binary is unaffected.'
);
