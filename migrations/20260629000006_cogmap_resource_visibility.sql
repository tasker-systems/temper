-- Cogmap-membership RESOURCE visibility (multi-author read RBAC, Surface B Half 2 Beat A0).
--
-- Carried-forward boundary from Half 1: `resources_visible_to` had no cogmap-membership
-- clause, so a `--cogmap` search returned only the searcher's OWN cogmap-homed
-- resources — a co-member who can READ the map (via `cogmap_readable_by_profile`) still
-- did NOT see a peer's resource homed in it. Multi-author maps were therefore pointless.
--
-- This closes that gap with the resource-grain mirror of `cogmap_readable_by_profile`:
-- a team joined to a cogmap confers read access to the resources homed in it, exactly as
-- it already confers read access to the *map*. Resolution is membership-flat
-- (`kb_team_cogmaps ∩ profile_effective_teams`), NOT the ancestor-expanded
-- `reachable_teams` the context clauses use — so "a map you can read" and "the resources
-- homed in it" agree by construction (spec §7).
--
-- This is a false-negative fix, NOT a leak fix: non-members still see nothing. It is the
-- exact resource-grain mirror of the team-owned-context clause already in
-- `20260627000002_team_owned_context_resource_visibility.sql` (lines 58-67).
--
-- `resources_readable_by` (canonical_functions.sql:244) delegates its PROFILE arm directly
-- to `resources_visible_to`, so this fix propagates to cogmap-principal read paths with no
-- parallel branch there. (The cogmap *principal* arm — a cogmap reading its own members —
-- is a separate concern and out of scope.)
--
-- Ripple: the L0 kernel telos (`…0005-000000000002`) is homed in the L0 cogmap, bound to
-- the auto-join `temper-system` team, so every approved profile now correctly sees the L0
-- telos at the resource grain (it is the public kernel; this also closes a latent gap).
--
-- Stays STABLE + LANGUAGE sql so `sqlx::query!` callers remain compile-checked.

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
    -- team-owned context: resources homed in a context OWNED by a team the
    -- principal is a member of (mirrors `context_visible_to` clause 2 — direct
    -- membership, so addressing and resource-visibility agree by construction)
    SELECT h.resource_id
    FROM kb_contexts c
    JOIN kb_team_members tm
      ON tm.team_id = c.owner_id AND tm.profile_id = p_profile
    JOIN kb_resource_homes h
      ON h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id
    WHERE c.owner_table = 'kb_teams'
    UNION
    -- cogmap membership: resources homed in a cognitive map joined to a team the
    -- principal is a member of (resource-grain mirror of cogmap_readable_by_profile —
    -- membership-flat, so map-read and resource-read agree by construction).
    -- Additive false-negative fix: non-members still see nothing.
    SELECT h.resource_id
    FROM kb_team_cogmaps tc
    JOIN profile_effective_teams(p_profile) e ON e.team_id = tc.team_id
    JOIN kb_resource_homes h
      ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = tc.cogmap_id;
$$;
