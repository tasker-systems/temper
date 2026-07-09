-- Migration: set-based rewrite of edges_visible_to — kill the hidden N+1.
--
-- SQL function audit 2026-07-08 (docs/code-reviews/2026-07-08-sql-function-audit.md,
-- SQLA-1 finding: per-row-visibility-recompute). The previous body filtered kb_edges
-- through three per-row scalar gates — anchor_readable_by_profile plus
-- endpoint_readable_by_profile on BOTH endpoints — and endpoint_readable_by_profile's
-- kb_resources arm runs `IN (SELECT … FROM resources_visible_to(p))`. STABLE scalar
-- functions in a WHERE clause re-evaluate per row, so cost ≈ edges × cost(resources_
-- visible_to), feeding the graph_atlas degree laterals row-by-row.
--
-- Rewrite: materialize each readable/visible set ONCE per call and semi-join. The CTEs
-- mirror the scalar helpers branch-for-branch (the helpers stay — invocation readback
-- still gates through anchor_readable_by_profile, and they serve as the per-row spec in
-- edges_visible_to_equivalence_test, which asserts function == scalar-gate oracle across
-- every branch class):
--   vis               ↔ endpoint_readable_by_profile 'kb_resources' arm
--                       (resources_visible_to — is_active-gated since 20260708000007)
--   readable_cogmaps  ↔ cogmap_readable_by_profile (team-join on a reachable team, OR
--                       explicit cogmap read-grant via profile_explicit_grant)
--   readable_contexts ↔ anchor_readable_by_profile 'kb_contexts' arm (personal-owned;
--                       shared to a reachable team; owned by a DIRECT-membership team —
--                       flat by design, not ancestor-expanded; explicit context read-grant)
-- Unknown endpoint/anchor tables fall out of the OR arms — the CASE ELSE false, preserved.
-- Same signature; callers (graph_atlas_*, graph_subgraph_nodes, fetch_subgraph_edges)
-- untouched.

CREATE OR REPLACE FUNCTION edges_visible_to(p_profile uuid)
RETURNS TABLE(edge_id uuid)
LANGUAGE sql STABLE AS $$
    WITH reachable_teams AS (
        SELECT DISTINCT a.team_id
        FROM profile_effective_teams(p_profile) e
        CROSS JOIN LATERAL team_ancestors(e.team_id) a
    ),
    vis AS (
        SELECT resource_id FROM resources_visible_to(p_profile)
    ),
    readable_cogmaps AS (
        -- team-joined on a reachable (self-or-ancestor) team
        SELECT tc.cogmap_id AS id
        FROM kb_team_cogmaps tc
        JOIN reachable_teams rt ON rt.team_id = tc.team_id
        UNION
        -- explicit cogmap read-grant (profile direct, or team on a reachable team)
        SELECT g.subject_id
        FROM kb_access_grants g
        WHERE g.subject_table = 'kb_cogmaps' AND g.can_read
          AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
             OR (g.principal_table = 'kb_teams'
                   AND g.principal_id IN (SELECT team_id FROM reachable_teams)) )
    ),
    readable_contexts AS (
        -- personal context owned by the principal
        SELECT c.id
        FROM kb_contexts c
        WHERE c.owner_table = 'kb_profiles' AND c.owner_id = p_profile
        UNION
        -- context shared to a reachable (self-or-ancestor) team
        SELECT tc.context_id
        FROM kb_team_contexts tc
        JOIN reachable_teams rt ON rt.team_id = tc.team_id
        UNION
        -- context OWNED by a team the principal is a member of (flat — direct membership
        -- via profile_effective_teams, deliberately NOT ancestor-expanded)
        SELECT c.id
        FROM kb_contexts c
        JOIN profile_effective_teams(p_profile) pet ON pet.team_id = c.owner_id
        WHERE c.owner_table = 'kb_teams'
        UNION
        -- explicit context read-grant (profile direct, or team on a reachable team)
        SELECT g.subject_id
        FROM kb_access_grants g
        WHERE g.subject_table = 'kb_contexts' AND g.can_read
          AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
             OR (g.principal_table = 'kb_teams'
                   AND g.principal_id IN (SELECT team_id FROM reachable_teams)) )
    )
    SELECT e.id
    FROM kb_edges e
    WHERE NOT e.is_folded
      AND ( (e.home_anchor_table = 'kb_cogmaps'
               AND e.home_anchor_id IN (SELECT id FROM readable_cogmaps))
         OR (e.home_anchor_table = 'kb_contexts'
               AND e.home_anchor_id IN (SELECT id FROM readable_contexts)) )
      AND ( (e.source_table = 'kb_resources'
               AND e.source_id IN (SELECT resource_id FROM vis))
         OR (e.source_table = 'kb_cogmaps'
               AND e.source_id IN (SELECT id FROM readable_cogmaps)) )
      AND ( (e.target_table = 'kb_resources'
               AND e.target_id IN (SELECT resource_id FROM vis))
         OR (e.target_table = 'kb_cogmaps'
               AND e.target_id IN (SELECT id FROM readable_cogmaps)) );
$$;
