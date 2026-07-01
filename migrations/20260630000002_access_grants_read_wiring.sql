-- Generalized access-capability arc — Deliverable 3a: explicit context/cogmap READ grants confer read.
-- Design: docs/superpowers/specs/2026-06-30-generalized-access-capability-model-design.md §4 step 2 (read half).
--
-- ADDITIVE, widens-never-narrows: each read predicate gains a PARALLEL explicit-grant branch beside its
-- existing membership/share branches, which are re-emitted VERBATIM. A profile with no kb_access_grants
-- row reads exactly as before (no behavior change); an explicit read grant on a cogmap or context now
-- confers read WITHOUT team membership — the "grant profile X read on cogmap Y" case that was
-- inexpressible in the resources-only kb_resource_access. Grants reach via the union-up Profile axis
-- (direct profile grant OR team grant on a reachable self-or-ancestor team), exactly as `profile_explicit_grant`
-- (20260630000001) defines it; the Cogmap producer axis is untouched (Q-B).
--
-- The WRITE half of step 2 (the `cogmap_authorable_by_profile` Q-A tightening) is deliberately deferred to
-- its own deliverable — it needs creator-seeding + co-author granting + a prod-data backfill, none of which
-- belong in this additive read change.
--
-- Scope note: this wires shape-read, homed-resource read, context-read, and wayfind admission. Edge-home
-- visibility follows for free on the COGMAP side (`anchor_readable_by_profile`/`endpoint_readable_by_profile`
-- delegate to `cogmap_readable_by_profile`, replaced below). The CONTEXT-homed-edge arm of
-- `anchor_readable_by_profile` is the one place a context read-grant does not yet reach; left as a noted
-- follow-on (it needs an edge fixture to test) — a false-negative, never a leak.
--
-- Namespace-free (no SET search_path): names resolve against the connection's search_path (public).

-- ============================================================================
-- (1) cogmap_readable_by_profile — shape-read. Existing flat membership branch + explicit read grant.
--     anchor_readable_by_profile / endpoint_readable_by_profile / cogmap_scope_ids delegate here, so
--     cogmap-homed edges and single-map search scope follow the grant automatically.
-- ============================================================================
CREATE OR REPLACE FUNCTION cogmap_readable_by_profile(p_profile uuid, p_cogmap uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
        FROM kb_team_cogmaps tc
        JOIN profile_effective_teams(p_profile) e ON e.team_id = tc.team_id
        WHERE tc.cogmap_id = p_cogmap
    )
    OR profile_explicit_grant(p_profile, 'read', 'kb_cogmaps', p_cogmap);
$$;

-- ============================================================================
-- (2) cogmap_visible_maps — wayfind admission. Flat-membership maps UNION explicit-read-grant maps.
--     "map-read = resource-read agree by construction" is preserved: both this and
--     cogmap_readable_by_profile resolve to (flat membership ∪ explicit read grant).
-- ============================================================================
CREATE OR REPLACE FUNCTION cogmap_visible_maps(p_principal uuid)
RETURNS SETOF uuid LANGUAGE sql STABLE AS $$
    WITH reachable_teams AS (
        SELECT DISTINCT a.team_id
        FROM profile_effective_teams(p_principal) e
        CROSS JOIN LATERAL team_ancestors(e.team_id) a
    )
    SELECT DISTINCT tc.cogmap_id
    FROM kb_team_cogmaps tc
    JOIN profile_effective_teams(p_principal) e ON e.team_id = tc.team_id
    UNION
    -- explicit cogmap read-grant (direct profile, or team grant on a reachable team) admits the map
    SELECT g.subject_id
    FROM kb_access_grants g
    WHERE g.subject_table = 'kb_cogmaps' AND g.can_read
      AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_principal)
         OR (g.principal_table = 'kb_teams'    AND g.principal_id IN (SELECT team_id FROM reachable_teams)) );
$$;

-- ============================================================================
-- (3) resources_visible_to — the six existing branches (verbatim) + one explicit-subject-grant branch:
--     resources homed in a cogmap or context the profile holds an explicit read-grant on. Reuses the
--     function's existing reachable_teams CTE for the team-anchored grant reach.
-- ============================================================================
CREATE OR REPLACE FUNCTION resources_visible_to(p_profile uuid)
RETURNS TABLE(resource_id uuid) LANGUAGE sql STABLE AS $$
    WITH reachable_teams AS (
        SELECT DISTINCT a.team_id
        FROM profile_effective_teams(p_profile) e
        CROSS JOIN LATERAL team_ancestors(e.team_id) a
    )
    -- owned / originated (the home confers access to its principals)
    SELECT h.resource_id FROM kb_resource_homes h
     WHERE h.owner_profile_id = p_profile OR h.originator_profile_id = p_profile
    UNION
    -- direct profile-anchored grant (consumer-axis ONLY — never enters a vis(T))
    SELECT ra.resource_id FROM kb_resource_access ra
     WHERE ra.anchor_table = 'kb_profiles' AND ra.anchor_id = p_profile AND ra.can_read
    UNION
    -- team-anchored grant on a reachable (self-or-ancestor) team
    SELECT ra.resource_id FROM kb_resource_access ra
     JOIN reachable_teams rt ON ra.anchor_id = rt.team_id
     WHERE ra.anchor_table = 'kb_teams' AND ra.can_read
    UNION
    -- context-share: resources homed in a context shared to a reachable team (WS6 §2)
    SELECT h.resource_id
    FROM kb_team_contexts tc
    JOIN reachable_teams rt ON tc.team_id = rt.team_id
    JOIN kb_resource_homes h
      ON h.anchor_table = 'kb_contexts' AND h.anchor_id = tc.context_id
    UNION
    -- team-owned context: resources homed in a context OWNED by a team the principal is a member of
    SELECT h.resource_id
    FROM kb_contexts c
    JOIN kb_team_members tm
      ON tm.team_id = c.owner_id AND tm.profile_id = p_profile
    JOIN kb_resource_homes h
      ON h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id
    WHERE c.owner_table = 'kb_teams'
    UNION
    -- cogmap membership: resources homed in a cognitive map joined to a team the principal is a member of
    SELECT h.resource_id
    FROM kb_team_cogmaps tc
    JOIN profile_effective_teams(p_profile) e ON e.team_id = tc.team_id
    JOIN kb_resource_homes h
      ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = tc.cogmap_id
    UNION
    -- explicit subject-grant (NEW, additive): resources homed in a cogmap or context the profile holds an
    -- explicit read-grant on (direct profile grant OR team grant on a reachable team). kb_access_grants only.
    SELECT h.resource_id
    FROM kb_resource_homes h
    JOIN kb_access_grants g
      ON g.subject_table = h.anchor_table AND g.subject_id = h.anchor_id
    WHERE h.anchor_table IN ('kb_cogmaps','kb_contexts') AND g.can_read
      AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
         OR (g.principal_table = 'kb_teams'    AND g.principal_id IN (SELECT team_id FROM reachable_teams)) );
$$;

-- ============================================================================
-- (4) context_visible_to — the three existing ownership/share clauses (verbatim) + explicit read grant.
-- ============================================================================
CREATE OR REPLACE FUNCTION context_visible_to(p_principal uuid, p_context_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM kb_contexts c
        WHERE c.id = p_context_id
          AND (
              (c.owner_table = 'kb_profiles' AND c.owner_id = p_principal)
              OR (c.owner_table = 'kb_teams'
                    AND EXISTS (
                        SELECT 1 FROM kb_team_members tm
                        WHERE tm.team_id = c.owner_id AND tm.profile_id = p_principal))
              OR EXISTS (
                    SELECT 1 FROM kb_team_contexts tc
                    JOIN kb_team_members tm ON tm.team_id = tc.team_id
                    WHERE tc.context_id = c.id AND tm.profile_id = p_principal)
          )
    )
    OR profile_explicit_grant(p_principal, 'read', 'kb_contexts', p_context_id);
$$;
