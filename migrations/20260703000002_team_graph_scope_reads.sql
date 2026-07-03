-- Graph Atlas — Chunk A / R1: team-graph-scope read functions.
-- Design: docs/superpowers/specs/2026-07-03-temper-ui-graph-visualization-atlas-design.md (read model R1).
--
-- Additive over the existing access substrate — reuses team_ancestors / resources_visible_to.
-- The view opens at a team position in the DAG and needs two navigation primitives the
-- existing (upward) visibility functions do not provide:
--   * team_child_zones  — enterable direct children (downward, membership-gated).
--   * resources_in_team_scope — a scope's OWN bindings (team + ancestors), no descendant leak.
-- team_descendants is the DAG-down mirror of team_ancestors, used by team_child_zones.
--
-- All LANGUAGE sql STABLE so runtime sqlx callers stay stable-checkable.
-- Namespace-free (no SET search_path): names resolve against the connection's search_path (public).

-- ============================================================================
-- team_descendants: {self} ∪ all descendants (walk DOWN kb_teams_parents).
-- ============================================================================
CREATE FUNCTION team_descendants(p_team uuid)
RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE down AS (
        SELECT p_team AS team_id
        UNION
        SELECT tp.child_id
        FROM kb_teams_parents tp
        JOIN down ON tp.parent_id = down.team_id
    )
    SELECT team_id FROM down;
$$;

-- ============================================================================
-- team_child_zones: direct children of p_scope the profile can ENTER — it is a
-- member of that child OR of any of the child's descendants (the door leads
-- somewhere the profile can go). Membership-gated, one level of children.
-- ============================================================================
CREATE FUNCTION team_child_zones(p_profile uuid, p_scope uuid)
RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS $$
    SELECT c.child_id AS team_id
    FROM kb_teams_parents c
    WHERE c.parent_id = p_scope
      AND EXISTS (
          SELECT 1
          FROM team_descendants(c.child_id) d
          JOIN kb_team_members tm
            ON tm.team_id = d.team_id AND tm.profile_id = p_profile
      );
$$;

-- ============================================================================
-- resources_in_team_scope: resources VISIBLE to p_profile that are bound at
-- p_team's own scope — p_team and its ANCESTORS (upward inheritance), never a
-- descendant's private bindings. Intersected with resources_visible_to so it
-- can never exceed what the profile may already see (defense in depth).
-- ============================================================================
CREATE FUNCTION resources_in_team_scope(p_profile uuid, p_team uuid)
RETURNS TABLE(resource_id uuid) LANGUAGE sql STABLE AS $$
    WITH scope_teams AS (
        SELECT a.team_id FROM team_ancestors(p_team) a
    ),
    scoped AS (
        -- team-anchored resource read-grant on a scope team (kb_access_grants store)
        SELECT g.subject_id AS resource_id
        FROM kb_access_grants g
        JOIN scope_teams st ON g.principal_id = st.team_id
        WHERE g.subject_table = 'kb_resources' AND g.principal_table = 'kb_teams' AND g.can_read
        UNION
        -- resources homed in a context SHARED to a scope team
        SELECT h.resource_id
        FROM kb_team_contexts tc
        JOIN scope_teams st ON tc.team_id = st.team_id
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_contexts' AND h.anchor_id = tc.context_id
        UNION
        -- resources homed in a context OWNED by a scope team.
        -- Team-owned context is FLAT in the visibility model (only DIRECT members of the
        -- owning team see it — never ancestor-expanded), so this branch self-gates on
        -- membership rather than relying on the trailing intersection. It stays scope-bounded
        -- (owner ∈ scope_teams) so an owned context outside T's scope is not counted.
        SELECT h.resource_id
        FROM kb_contexts c
        JOIN scope_teams st ON c.owner_table = 'kb_teams' AND c.owner_id = st.team_id
        JOIN kb_team_members tm ON tm.team_id = c.owner_id AND tm.profile_id = p_profile
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id
        UNION
        -- resources homed in a cogmap JOINED to a scope team
        SELECT h.resource_id
        FROM kb_team_cogmaps tc
        JOIN scope_teams st ON tc.team_id = st.team_id
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = tc.cogmap_id
        UNION
        -- explicit container-grant (D3a): resources homed in a context/cogmap the scope
        -- team holds a read-grant on (kb_access_grants subject = the home anchor)
        SELECT h.resource_id
        FROM kb_resource_homes h
        JOIN kb_access_grants g
          ON g.subject_table = h.anchor_table AND g.subject_id = h.anchor_id
        JOIN scope_teams st ON g.principal_id = st.team_id
        WHERE h.anchor_table IN ('kb_cogmaps','kb_contexts')
          AND g.principal_table = 'kb_teams' AND g.can_read
    )
    SELECT s.resource_id
    FROM scoped s
    JOIN resources_visible_to(p_profile) v ON v.resource_id = s.resource_id;
$$;
