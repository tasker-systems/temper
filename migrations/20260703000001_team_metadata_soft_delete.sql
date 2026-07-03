-- Team metadata + soft-delete (teams-in-temper goal, scope task #5).
--
-- Unblocks the two schema-blocked team verbs: PATCH team metadata and DELETE
-- (soft-delete) a team. `kb_teams` was (id, slug, name, created, auto_join_role)
-- only — no `description`, no `is_active`.
--
-- ADDITIVE-ONLY (CLAUDE.md main invariant): two nullable/defaulted columns + a
-- set of CREATE OR REPLACE on STABLE read-path functions. No destructive change,
-- safe for auto-deploy.
--
-- Soft-delete semantics (cascade decision, documented):
--   • DELETE sets `is_active = false`. All rows (members, grants, context-shares,
--     cogmap-joins, child-parent links) are PRESERVED — soft-delete is reversible
--     (recovery is a `is_active = true` DB write).
--   • A soft-deleted team confers ZERO read-reach. Enforced at the two DAG
--     primitives below (`profile_effective_teams`, `team_ancestors`) so every
--     reachable-team branch inherits the exclusion, plus the three flat
--     direct-membership branches re-routed through `profile_effective_teams`.
--   • CHILD TEAMS STAY ACTIVE. Soft-deleting an umbrella does NOT recurse — only
--     the target flips. Children remain independently active but stop inheriting
--     the dead umbrella's grants, because `team_ancestors` halts at an inactive
--     node. Least-surprising, fully reversible, no recursive mutation.
--   • The globally-UNIQUE slug stays reserved while soft-deleted (so recovery is
--     unambiguous); a new team cannot reuse a soft-deleted team's slug until it
--     is hard-deleted.

-- ============================================================================
-- Additive columns.
-- ============================================================================
ALTER TABLE kb_teams ADD COLUMN description TEXT;
ALTER TABLE kb_teams ADD COLUMN is_active BOOLEAN NOT NULL DEFAULT true;

-- ============================================================================
-- Chokepoint 1 — profile_effective_teams: a soft-deleted team is not an
-- effective membership. This is the base of `reachable_teams` in every read
-- function, so membership in a soft-deleted team confers nothing anywhere.
-- ============================================================================
CREATE OR REPLACE FUNCTION profile_effective_teams(p_profile uuid)
RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS $$
    SELECT tm.team_id
    FROM kb_team_members tm
    JOIN kb_teams t ON t.id = tm.team_id
    WHERE tm.profile_id = p_profile AND t.is_active;
$$;

-- ============================================================================
-- Chokepoint 2 — team_ancestors: the DAG up-walk excludes inactive nodes. An
-- inactive umbrella is neither returned NOR walked through (the recursion only
-- extends via active parents), so a soft-deleted umbrella confers no grants to
-- its descendants and severs the reach-chain to grandparents. Also covers the
-- producer side (`vis_team` iterates `team_ancestors`) with no further edit.
-- ============================================================================
CREATE OR REPLACE FUNCTION team_ancestors(p_team uuid)
RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE up AS (
        SELECT t.id AS team_id
        FROM kb_teams t
        WHERE t.id = p_team AND t.is_active
        UNION
        SELECT tp.parent_id
        FROM kb_teams_parents tp
        JOIN up ON tp.child_id = up.team_id
        JOIN kb_teams pt ON pt.id = tp.parent_id AND pt.is_active
    )
    SELECT team_id FROM up;
$$;

-- ============================================================================
-- resources_visible_to: latest body (20260701000003) VERBATIM, except the flat
-- team-owned-context branch now routes its membership through
-- `profile_effective_teams` (which filters is_active) instead of joining
-- `kb_team_members` directly. Every other branch keys off `reachable_teams`
-- (built from the two primitives above) and is therefore already covered.
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
    -- direct profile-anchored grant (consumer-axis ONLY — never enters a vis(T)). kb_access_grants store.
    SELECT g.subject_id FROM kb_access_grants g
     WHERE g.subject_table = 'kb_resources' AND g.principal_table = 'kb_profiles'
       AND g.principal_id = p_profile AND g.can_read
    UNION
    -- team-anchored grant on a reachable (self-or-ancestor) team. kb_access_grants store.
    SELECT g.subject_id FROM kb_access_grants g
     JOIN reachable_teams rt ON g.principal_id = rt.team_id
     WHERE g.subject_table = 'kb_resources' AND g.principal_table = 'kb_teams' AND g.can_read
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
    -- do NOT ancestor-expand this branch. Membership routed through profile_effective_teams so a
    -- soft-deleted owning team confers nothing.
    SELECT h.resource_id
    FROM kb_contexts c
    JOIN profile_effective_teams(p_profile) pet
      ON pet.team_id = c.owner_id
    JOIN kb_resource_homes h
      ON h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id
    WHERE c.owner_table = 'kb_teams'
    UNION
    -- cogmap membership: resources homed in a cognitive map joined to a REACHABLE (self-or-ancestor)
    -- team — UP+union (D4) to match the team-grant/context-share reach above, so map-read and resource-read
    -- agree by construction at the up-expanded level.
    SELECT h.resource_id
    FROM kb_team_cogmaps tc
    JOIN reachable_teams rt ON rt.team_id = tc.team_id
    JOIN kb_resource_homes h
      ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = tc.cogmap_id
    UNION
    -- explicit subject-grant (D3a): resources homed in a cogmap or context the profile holds an explicit
    -- read-grant on (direct profile grant OR team grant on a reachable team). kb_access_grants only.
    SELECT h.resource_id
    FROM kb_resource_homes h
    JOIN kb_access_grants g
      ON g.subject_table = h.anchor_table AND g.subject_id = h.anchor_id
    WHERE h.anchor_table IN ('kb_cogmaps','kb_contexts') AND g.can_read
      AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
         OR (g.principal_table = 'kb_teams'    AND g.principal_id IN (SELECT team_id FROM reachable_teams)) );
$$;

-- ============================================================================
-- context_visible_to: latest body (20260630000002) VERBATIM, except the two
-- flat direct-membership branches (team-owned context, context-share) route
-- through `profile_effective_teams` so a soft-deleted team stops conferring
-- context read-reach.
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
                        SELECT 1 FROM profile_effective_teams(p_principal) pet
                        WHERE pet.team_id = c.owner_id))
              OR EXISTS (
                    SELECT 1 FROM kb_team_contexts tc
                    JOIN profile_effective_teams(p_principal) pet ON pet.team_id = tc.team_id
                    WHERE tc.context_id = c.id)
          )
    )
    OR profile_explicit_grant(p_principal, 'read', 'kb_contexts', p_context_id);
$$;

-- ============================================================================
-- anchor_readable_by_profile: latest body (20260701000004) VERBATIM, except the
-- flat team-owned-context branch routes through `profile_effective_teams`. The
-- context-share branch here already goes through the primitives (self-or-ancestor
-- expansion) and is covered.
-- ============================================================================
CREATE OR REPLACE FUNCTION anchor_readable_by_profile(p_profile uuid, p_anchor_table text, p_anchor_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE p_anchor_table
        WHEN 'kb_cogmaps'  THEN cogmap_readable_by_profile(p_profile, p_anchor_id)
        WHEN 'kb_contexts' THEN (
            -- context owned by the principal themselves (mirrors
            -- `context_visible_to` clause 1 — personal context)
            EXISTS (
                SELECT 1
                FROM kb_contexts c
                WHERE c.id = p_anchor_id
                  AND c.owner_table = 'kb_profiles' AND c.owner_id = p_profile
            )
            -- context shared to a reachable (self-or-ancestor) team
            OR EXISTS (
                SELECT 1
                FROM profile_effective_teams(p_profile) e
                CROSS JOIN LATERAL team_ancestors(e.team_id) a
                JOIN kb_team_contexts tc ON tc.team_id = a.team_id
                WHERE tc.context_id = p_anchor_id
            )
            -- context OWNED by a team the principal is a member of (mirrors
            -- `context_visible_to` clause 2 — direct membership; soft-delete
            -- filtered via profile_effective_teams)
            OR EXISTS (
                SELECT 1
                FROM kb_contexts c
                JOIN profile_effective_teams(p_profile) pet
                  ON pet.team_id = c.owner_id
                WHERE c.id = p_anchor_id AND c.owner_table = 'kb_teams'
            )
            -- explicit context read-grant (NEW, additive — mirrors `context_visible_to` clause 4):
            -- a profile granted read on the context sees the edges homed in it, not just its resources.
            OR profile_explicit_grant(p_profile, 'read', 'kb_contexts', p_anchor_id)
        )
        ELSE false
    END;
$$;
