-- A team-anchored grant confers mutation only through an authoring role; a watcher never mutates,
-- and re-sharing never inherits. Reading still inherits down the team DAG at any role.
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
--   flag                     honoured for
--   can_read                 unchanged: profile_reachable_teams(p) -- any role, ancestors included
--   can_write, can_delete    a DIRECT owner|maintainer|member row on the granted team or on any team
--                            beneath it (the delegation still reaches down the tree; a watcher's row
--                            reaches nothing)
--   can_grant                a DIRECT owner|maintainer row on the granted team itself; no inheritance
--
-- WHY WRITE STILL INHERITS. A write grant to an umbrella team is a deliberate delegation to everyone
-- authoring beneath it (`context_read_predicate_test::an_explicit_write_grant_still_reaches_through_
-- the_chain`, ruled 2026-10-30 to stand). The hole was never the reach; it was that the reach ignored
-- role, so a watcher, or anyone a child team's owner added as one, held write, delete and grant. The
-- parent's managers can now see a child's roster and detach it (team-authority fixes D3), so the
-- reach is no longer invisible.
--
-- WHY can_grant DOES NOT INHERIT. Re-sharing is conferral, and the team-role rank rule already says
-- members do not confer. Letting a child team's managers re-share a parent's grant would hand the
-- parent's audience to whoever runs the child.
--
-- No row is rewritten; every existing row keeps conferring read exactly as before.
--
-- =================================================================================================
-- SHAPE
-- =================================================================================================
--
-- One new principal-agnostic helper, `profile_authoring_teams(p_profile, p_action)`, and two
-- body-only CREATE OR REPLACEs with unchanged signatures: `profile_explicit_grant` (and through it
-- `can()`, `cogmap_authorable_by_profile`, `context_authorable_by_profile`'s grant arm and grant
-- attenuation) and `can_modify_resource`'s team-anchored WRITE arm. The helper walks
-- `team_ancestors` from a ROLE-FILTERED direct membership, which is a different relation from
-- `profile_reachable_teams` (that one is role-blind), and is named as such in
-- `reachable_teams_one_definition_test.rs`.
--
-- Design: temper-artifacts `specs/2026-10-30-team-authority-fixes-design.md` D1.

CREATE OR REPLACE FUNCTION profile_authoring_teams(p_profile uuid, p_action text)
RETURNS TABLE(team_id uuid)
LANGUAGE sql
STABLE
AS $$
    -- write / delete: the teams a DIRECT authoring membership reaches, up the enclosure chain.
    SELECT DISTINCT a.team_id
    FROM kb_team_members tm
    JOIN kb_teams t ON t.id = tm.team_id AND t.is_active
    CROSS JOIN LATERAL team_ancestors(tm.team_id) a
    WHERE tm.profile_id = p_profile
      AND p_action IN ('write', 'delete')
      AND tm.role IN ('owner', 'maintainer', 'member')
    UNION
    -- grant: a DIRECT managing membership of the granted team itself. Never inherited.
    SELECT tm.team_id
    FROM kb_team_members tm
    JOIN kb_teams t ON t.id = tm.team_id AND t.is_active
    WHERE tm.profile_id = p_profile
      AND p_action = 'grant'
      AND tm.role IN ('owner', 'maintainer');
$$;

COMMENT ON FUNCTION profile_authoring_teams(uuid, text) IS
    'The teams whose team-anchored grants let a profile perform a MUTATING action. write/delete: every '
    'team reached up the enclosure chain from a direct, active owner|maintainer|member row (a watcher '
    'row reaches nothing). grant: a direct, active owner|maintainer row on the team itself, never '
    'inherited. Any other action reaches nothing. Reading uses profile_reachable_teams instead.';

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
                       -- Reading inherits down the DAG at any role; mutating needs an authoring role.
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
        -- team-anchored WRITE grant on a team the profile authors in (a direct authoring row on it or
        -- on a team beneath it), never through a watcher row (see this migration's header).
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
    'team-anchored grant now confers write/delete only through a direct owner|maintainer|member row '
    'on the granted team or a team beneath it (never a watcher), and grant only on a direct '
    'owner|maintainer of the granted team; read is unchanged (any role, inherited down the DAG). No '
    'row is rewritten. Design: temper-artifacts '
    'specs/2026-10-30-team-authority-fixes-design.md D1.'
);
