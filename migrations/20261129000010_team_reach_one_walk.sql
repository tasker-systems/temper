-- Team reach in one walk: the up-walk is stated once, as team_ancestors_of(uuid[]), and both
-- team_ancestors(t) and profile_reachable_teams(p) become body-only wrappers over it.
--
-- profile_reachable_teams (20260804000010) re-ran the inlined team_ancestors walk once per
-- membership, through CROSS JOIN LATERAL. For a reader in 300 teams that cost about 24 ms, nearly
-- all of a single-resource visibility check (the planner already pushes the resource id into every
-- arm of resources_visible_to; the closure was the part that tracked the reader). One recursive walk seeded with every
-- membership returns the same set: the closure of a union of seeds is the union of the seeds'
-- closures, and UNION deduplicates. Every read set built on the closure gets the same rows.
--
-- What a later edit must not break:
--   * Both floors of team_ancestors (20260703000001) stay in team_ancestors_of, verbatim. The step
--     floor (pt.is_active) stops the walk at a dead team, so a dead umbrella confers nothing above
--     or through it. The seed floor (t.is_active) refuses a dead start team. It is redundant for the
--     profile path, whose seeds come from profile_effective_teams, but load-bearing for every caller
--     that seeds the walk with a team: public_within_row_effective relies on it to keep a dead T's
--     row dormant.
--   * One statement of the walk. Neither wrapper restates the recursion, and nothing else may;
--     reachable_teams_one_definition_test scans pg_proc for copies, inlined recursions included.
--   * Cycles are admitted states, and UNION (not UNION ALL) is what terminates the walk on one.
--   * The profile path is a function scan, not inlined: an argument holding a sub-select stops SQL
--     function inlining. ROWS 200 keeps its row estimate near what the inlined per-membership form
--     produced, instead of the 1000 default, so callers that join the closure plan as before.
--     team_ancestors(t) passes a plain array and still inlines.
--
-- Design: temper-artifacts specs/2026-11-15-single-resource-visibility-design.md (PR A).

CREATE FUNCTION team_ancestors_of(p_teams uuid[])
RETURNS TABLE(team_id uuid)
LANGUAGE sql
STABLE
ROWS 200
AS $$
    WITH RECURSIVE up AS (
        SELECT t.id AS team_id
        FROM kb_teams t
        WHERE t.id = ANY(p_teams) AND t.is_active
        UNION
        SELECT tp.parent_id
        FROM kb_teams_parents tp
        JOIN up ON tp.child_id = up.team_id
        JOIN kb_teams pt ON pt.id = tp.parent_id AND pt.is_active
    )
    SELECT team_id FROM up;
$$;

COMMENT ON FUNCTION team_ancestors_of(uuid[]) IS
$c$THE live up-walk of the team DAG, from a set of seed teams: each live seed, then every parent
reached through live teams only. A dead seed is not returned, and a dead team is neither returned
nor walked through. Duplicates and cycles are absorbed by UNION. team_ancestors(t) is this walk
seeded with one team; profile_reachable_teams(p) is this walk seeded with p's effective teams. Never
restate the recursion: call this. For a walk with no liveness floor (a forbid's reach) see
team_ancestors_any, which is not reach.$c$;

CREATE OR REPLACE FUNCTION team_ancestors(p_team uuid)
RETURNS TABLE(team_id uuid)
LANGUAGE sql
STABLE
AS $$
    SELECT a.team_id FROM team_ancestors_of(ARRAY[p_team]) a;
$$;

COMMENT ON FUNCTION team_ancestors(uuid) IS
$c$The live up-walk from one team: p_team itself if live, then every ancestor reached through live
teams. A body-only wrapper over team_ancestors_of. A dead p_team yields nothing, and a dead umbrella
severs the chain above it.$c$;

CREATE OR REPLACE FUNCTION profile_reachable_teams(p_profile uuid)
RETURNS TABLE(team_id uuid)
LANGUAGE sql
STABLE
AS $$
    SELECT a.team_id
    FROM team_ancestors_of(
             ARRAY(SELECT e.team_id FROM profile_effective_teams(p_profile) e)
         ) a;
$$;

COMMENT ON FUNCTION profile_reachable_teams(uuid) IS
    'The one definition of which teams a profile reaches: its effective (live) memberships, '
    'expanded up the enclosure chain through live teams, in one walk (team_ancestors_of). Join '
    'this; never restate it. Guarded by reachable_teams_one_definition_test.rs, which scans pg_proc '
    'for copies.';

SELECT declare_migration(
    20261129000010,
    'additive',
    'team reach in one walk: adds team_ancestors_of(uuid[]), and body-only CREATE OR REPLACE of '
    'team_ancestors(uuid) and profile_reachable_teams(uuid) with unchanged signatures, return types '
    'and volatility, delegating to it (team_ancestors_of declares ROWS 200). Every set they return is unchanged (the same two liveness '
    'floors, one walk instead of one per membership), so an older binary reads the same answers. '
    'No table, column, index or row changes.'
);
