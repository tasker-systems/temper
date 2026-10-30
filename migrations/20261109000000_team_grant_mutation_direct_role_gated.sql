-- A team-anchored grant confers mutation only on direct members of the granted team who hold an
-- authoring role. Reading still inherits down the team DAG.
--
-- =================================================================================================
-- WHAT CHANGES
-- =================================================================================================
--
-- A `kb_access_grants` row whose principal is a team (`principal_table = 'kb_teams'`) used to be
-- honoured, for every flag on it, for anyone in `profile_reachable_teams(p)`: every direct
-- membership at ANY role, expanded up through every active ancestor. So a `watcher` -- on the
-- granted team or on any team beneath it -- held write, delete and grant.
--
-- That contradicts the invariant the narrowed arm of `context_authorable_by_profile`
-- (20260826000110) already states: "Read inherits up the enclosure chain; mutation does not.
-- `watcher` is read-only." This migration makes the grant arms agree with it:
--
--   flag                     honoured for
--   can_read                 unchanged: profile_reachable_teams(p) -- any role, ancestors included
--   can_write, can_delete    DIRECT, active membership of the granted team, role owner|maintainer|member
--   can_grant                DIRECT, active membership of the granted team, role owner|maintainer
--
-- WHY DIRECT. A child team's roster is set by the child's owner, not the parent's. If a role on a
-- child unlocked a parent's write grant, the parent's mutation rights would belong to whoever runs
-- the child.
--
-- WHY can_grant NEEDS A MANAGING ROLE. Re-sharing is conferral, and the team-role rank rule already
-- says members do not confer.
--
-- BEHAVIOUR CHANGE, stated: a write grant on an umbrella team no longer lets members of its
-- descendant teams write. Grant the descendant directly for that. No row is rewritten; every
-- existing row keeps conferring read exactly as before.
--
-- =================================================================================================
-- SHAPE
-- =================================================================================================
--
-- One new principal-agnostic helper, `profile_authoring_teams(p_profile, p_action)`, and two
-- body-only CREATE OR REPLACEs with unchanged signatures: `profile_explicit_grant` (and through it
-- `can()`, `cogmap_authorable_by_profile`, `context_authorable_by_profile`'s grant arm and grant
-- attenuation) and `can_modify_resource`'s team-anchored WRITE arm. The helper walks no ancestors,
-- so `profile_reachable_teams` stays the one definition of reach
-- (`reachable_teams_one_definition_test.rs`).
--
-- Design: temper-artifacts `specs/2026-10-30-team-authority-fixes-design.md` D1.

CREATE OR REPLACE FUNCTION profile_authoring_teams(p_profile uuid, p_action text)
RETURNS TABLE(team_id uuid)
LANGUAGE sql
STABLE
AS $$
    -- The teams through which a team-anchored grant lets this profile MUTATE: direct, active
    -- memberships whose role may perform `p_action`. No ancestor walk -- mutation does not inherit.
    SELECT tm.team_id
    FROM kb_team_members tm
    JOIN kb_teams t ON t.id = tm.team_id AND t.is_active
    WHERE tm.profile_id = p_profile
      AND CASE p_action
            WHEN 'write'  THEN tm.role IN ('owner', 'maintainer', 'member')
            WHEN 'delete' THEN tm.role IN ('owner', 'maintainer', 'member')
            WHEN 'grant'  THEN tm.role IN ('owner', 'maintainer')
            ELSE false
          END;
$$;

COMMENT ON FUNCTION profile_authoring_teams(uuid, text) IS
    'The teams whose team-anchored grants let a profile perform a MUTATING action: direct, active '
    'memberships only (no ancestor walk), role owner|maintainer|member for write and delete, '
    'owner|maintainer for grant; any other action reaches nothing. Reading uses '
    'profile_reachable_teams instead. Read inherits down the team DAG; mutation does not.';

CREATE OR REPLACE FUNCTION profile_explicit_grant(p_profile uuid, p_action text, p_subject_table text, p_subject_id uuid)
RETURNS boolean
LANGUAGE sql
STABLE
AS $$
    SELECT EXISTS (
        SELECT 1 FROM kb_access_grants g
        WHERE g.subject_table = p_subject_table AND g.subject_id = p_subject_id
          AND CASE p_action WHEN 'read'   THEN g.can_read
                            WHEN 'write'  THEN g.can_write
                            WHEN 'delete' THEN g.can_delete
                            WHEN 'grant'  THEN g.can_grant
                            ELSE false END
          AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
             OR (g.principal_table = 'kb_teams'
                   AND g.principal_id IN (
                       -- Reading inherits down the DAG, at any role; mutating does not.
                       SELECT team_id FROM profile_reachable_teams(p_profile) WHERE p_action = 'read'
                       UNION ALL
                       SELECT team_id FROM profile_authoring_teams(p_profile, p_action)
                        WHERE p_action <> 'read')) )
    );
$$;

CREATE OR REPLACE FUNCTION can_modify_resource(p_profile uuid, p_resource uuid)
RETURNS boolean
LANGUAGE sql
STABLE
AS $$
    -- Soft-delete WRITE floor: a tombstone is unmodifiable on every axis.
    SELECT EXISTS (SELECT 1 FROM kb_resources r WHERE r.id = p_resource AND r.is_active)
       AND EXISTS (
        -- owned (the home confers modify to its OWNER; originator is provenance only, not access)
        SELECT 1 FROM kb_resource_homes h
         WHERE h.resource_id = p_resource
           AND h.owner_profile_id = p_profile
        UNION ALL
        -- direct profile-anchored WRITE grant.
        SELECT 1 FROM kb_access_grants g
         WHERE g.subject_table = 'kb_resources' AND g.subject_id = p_resource
           AND g.principal_table = 'kb_profiles' AND g.principal_id = p_profile AND g.can_write
        UNION ALL
        -- team-anchored WRITE grant on a team the profile DIRECTLY authors in. Not ancestor-expanded,
        -- and never for a watcher: mutation does not inherit (see this migration's header).
        SELECT 1 FROM kb_access_grants g
         JOIN profile_authoring_teams(p_profile, 'write') at ON g.principal_id = at.team_id
         WHERE g.subject_table = 'kb_resources' AND g.subject_id = p_resource
           AND g.principal_table = 'kb_teams' AND g.can_write
        UNION ALL
        -- container-write cascade: whoever may author the home container may modify its nodes.
        SELECT 1 FROM kb_resource_homes h
         WHERE h.resource_id = p_resource
           AND CASE h.anchor_table
                 WHEN 'kb_cogmaps'  THEN cogmap_authorable_by_profile(p_profile, h.anchor_id)
                 WHEN 'kb_contexts' THEN context_authorable_by_profile(p_profile, h.anchor_id)
                 ELSE false
               END
    );
$$;

SELECT declare_migration(
    20261109000000,
    'additive',
    'New STABLE function profile_authoring_teams(uuid,text) and body-only CREATE OR REPLACE of '
    'profile_explicit_grant and can_modify_resource with unchanged signatures and return types, so a '
    'binary deployed before or after reads them identically. Behaviour change, deliberate: a '
    'team-anchored grant now confers write/delete only on direct owner|maintainer|member of the '
    'granted team and grant only on direct owner|maintainer; read is unchanged (any role, inherited '
    'down the DAG). No row is rewritten. Design: temper-artifacts '
    'specs/2026-10-30-team-authority-fixes-design.md D1.'
);
