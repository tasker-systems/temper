-- Team-owned-context RESOURCE visibility (follow-up to the I2 context-ref fix).
--
-- The I2 fix (`context_visible_to`, migration `20260627000001`) made a TEAM-OWNED
-- context (`owner_table='kb_teams'`) with NO `kb_team_contexts` self-share row
-- addressable and listable by a member of the owning team. But the RESOURCE-side
-- predicates still gated team access on a `kb_team_contexts` share row only:
--
--   * `resources_visible_to` admitted context-homed resources via the
--     "context shared to a reachable team" clause, but had NO clause for
--     resources homed in a context OWNED by a team the principal belongs to.
--   * `anchor_readable_by_profile` (the `kb_contexts` arm) gated a context anchor
--     on a `kb_team_contexts` share alone, with the same gap.
--
-- So a member could address/list a team-owned context yet not see the resources
-- inside it. This migration closes that by adding a membership-first clause to
-- both functions, mirroring `context_visible_to` clause 2 so that "a context you
-- can address" and "the resources inside it" agree by construction. Direct
-- `kb_team_members` membership (not ancestor closure) is used deliberately, to
-- match `context_visible_to` exactly — neither broader nor narrower than the
-- addressing predicate.
--
-- This is a false-negative fix, NOT a leak fix: non-members still see nothing.
-- It is deliberately scoped to the TEAM-owned case; the separate PROFILE-owned
-- context anchor gap (an owner cannot read edges homed in their own context) is
-- tracked to land with the graph-neighbor surface wiring (see
-- temper-substrate readback `neighbors`), not here.
--
-- Both functions stay STABLE + LANGUAGE sql so `sqlx::query!` callers remain
-- compile-checked.

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
    WHERE c.owner_table = 'kb_teams';
$$;

CREATE OR REPLACE FUNCTION anchor_readable_by_profile(p_profile uuid, p_anchor_table text, p_anchor_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE p_anchor_table
        WHEN 'kb_cogmaps'  THEN cogmap_readable_by_profile(p_profile, p_anchor_id)
        WHEN 'kb_contexts' THEN (
            -- context shared to a reachable (self-or-ancestor) team
            EXISTS (
                SELECT 1
                FROM profile_effective_teams(p_profile) e
                CROSS JOIN LATERAL team_ancestors(e.team_id) a
                JOIN kb_team_contexts tc ON tc.team_id = a.team_id
                WHERE tc.context_id = p_anchor_id
            )
            -- context OWNED by a team the principal is a member of (mirrors
            -- `context_visible_to` clause 2 — direct membership)
            OR EXISTS (
                SELECT 1
                FROM kb_contexts c
                JOIN kb_team_members tm
                  ON tm.team_id = c.owner_id AND tm.profile_id = p_profile
                WHERE c.id = p_anchor_id AND c.owner_table = 'kb_teams'
            )
        )
        ELSE false
    END;
$$;
