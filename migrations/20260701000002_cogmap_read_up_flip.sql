-- Generalized access-capability arc — Deliverable 4 (D4): cogmap-read direction lockstep flip.
-- Design: docs/superpowers/specs/2026-06-30-generalized-access-capability-model-design.md §3.4 + §4 step 3.
--
-- The three flat cogmap-read functions move their MEMBERSHIP branch from the flat
-- `profile_effective_teams(p)` join to the UP+union expansion `profile_effective_teams(p) ⋈
-- team_ancestors(·)` — the identical `reachable_teams` CTE `resources_visible_to`'s team-grant and
-- context-share branches already use (…06:32-36). After the flip all three key on the same up-expanded
-- team set over `kb_team_cogmaps`, so **map-read = resource-read holds by construction** — now at the
-- up-expanded level instead of the flat level.
--
-- Leak direction: reads get *broader* — a member of a CHILD team now reads a map joined to an ANCESTOR
-- (parent) team, because `team_ancestors(child) = {child} ∪ parents` (grants/shares inherit DOWN the
-- teams DAG, §2 member-vantage upward model). This is the intended visibility EXPANSION, not a leak.
-- The reverse never happens: a parent-team member does NOT reach a child-only map (ancestors go up, not
-- down). Case 2 (`resources_accessible_to_cogmap`, `vis_team`) calls NONE of these three and is
-- untouched (Q-B: the producer/least-privilege axis is preserved).
--
-- ADDITIVE re-emit: this is a `CREATE OR REPLACE` that carries forward D3a's explicit-grant branches
-- (20260630000002) VERBATIM — only the membership join changes. The team-owned-CONTEXT branch of
-- `resources_visible_to` stays FLAT deliberately (§3 item C: context-ownership is home-like, single-owned,
-- and paired with flat addressability — it does NOT flip).
--
-- Two more functions follow the flip for free (no edit — they DELEGATE to (1)):
-- `anchor_readable_by_profile`'s `kb_cogmaps` arm (20260627000003:29) and `endpoint_readable_by_profile`'s
-- `kb_cogmaps` arm (…02:296), plus `cogmap_scope_ids` (…05:12) — edge-home, endpoint, and single-map
-- search scope all inherit the up-flip automatically. The lockstep is genuinely THREE `CREATE OR REPLACE`s.
--
-- Namespace-free (no SET search_path): names resolve against the connection's search_path (public).

-- ============================================================================
-- (1) cogmap_readable_by_profile — shape-read. Membership branch UP+union (was flat …30:34), plus the
--     D3a explicit read-grant branch (verbatim). anchor_readable_by_profile / endpoint_readable_by_profile
--     / cogmap_scope_ids delegate here, so cogmap-homed edges and single-map search scope follow the flip.
-- ============================================================================
CREATE OR REPLACE FUNCTION cogmap_readable_by_profile(p_profile uuid, p_cogmap uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
        FROM kb_team_cogmaps tc
        JOIN (SELECT DISTINCT a.team_id
              FROM profile_effective_teams(p_profile) e
              CROSS JOIN LATERAL team_ancestors(e.team_id) a) rt ON rt.team_id = tc.team_id
        WHERE tc.cogmap_id = p_cogmap
    )
    OR profile_explicit_grant(p_profile, 'read', 'kb_cogmaps', p_cogmap);
$$;

-- ============================================================================
-- (2) cogmap_visible_maps — wayfind admission. Up-expanded-membership maps UNION explicit-read-grant maps.
--     "map-read = resource-read agree by construction" is preserved: both this and
--     cogmap_readable_by_profile now resolve to (up-expanded membership ∪ explicit read grant).
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
    JOIN reachable_teams rt ON rt.team_id = tc.team_id
    UNION
    -- explicit cogmap read-grant (direct profile, or team grant on a reachable team) admits the map
    SELECT g.subject_id
    FROM kb_access_grants g
    WHERE g.subject_table = 'kb_cogmaps' AND g.can_read
      AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_principal)
         OR (g.principal_table = 'kb_teams'    AND g.principal_id IN (SELECT team_id FROM reachable_teams)) );
$$;

-- ============================================================================
-- (3) resources_visible_to — the six existing branches + the D3a explicit-subject-grant branch, ALL
--     verbatim EXCEPT the cogmap-membership branch, whose join flips flat → reachable_teams (UP+union).
--     The team-owned-CONTEXT branch stays FLAT (kb_team_members, §3 item C — do NOT flip it).
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
    -- team-owned context: resources homed in a context OWNED by a team the principal is a member of.
    -- FLAT by design (§3 item C: context-ownership is home-like + paired with flat addressability) —
    -- do NOT ancestor-expand this branch.
    SELECT h.resource_id
    FROM kb_contexts c
    JOIN kb_team_members tm
      ON tm.team_id = c.owner_id AND tm.profile_id = p_profile
    JOIN kb_resource_homes h
      ON h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id
    WHERE c.owner_table = 'kb_teams'
    UNION
    -- cogmap membership: resources homed in a cognitive map joined to a REACHABLE (self-or-ancestor)
    -- team — UP+union (was flat …30:108) to match the team-grant/context-share reach above, so map-read
    -- and resource-read agree by construction at the up-expanded level.
    SELECT h.resource_id
    FROM kb_team_cogmaps tc
    JOIN reachable_teams rt ON rt.team_id = tc.team_id
    JOIN kb_resource_homes h
      ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = tc.cogmap_id
    UNION
    -- explicit subject-grant (D3a, additive): resources homed in a cogmap or context the profile holds an
    -- explicit read-grant on (direct profile grant OR team grant on a reachable team). kb_access_grants only.
    SELECT h.resource_id
    FROM kb_resource_homes h
    JOIN kb_access_grants g
      ON g.subject_table = h.anchor_table AND g.subject_id = h.anchor_id
    WHERE h.anchor_table IN ('kb_cogmaps','kb_contexts') AND g.can_read
      AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
         OR (g.principal_table = 'kb_teams'    AND g.principal_id IN (SELECT team_id FROM reachable_teams)) );
$$;
