-- `reachable_teams` gets one authoritative home -- the mutation gates.
--
-- The second half of `20260804000010`, which created `profile_reachable_teams(p_profile)` and
-- routed the seven READ-path hand-copies through it. Read that migration's header first; it carries
-- the full statement of what this is and what it does not claim.
--
-- =================================================================================================
-- WHY THESE TWO ARE SEPARATE
-- =================================================================================================
--
-- Nine copies, two risk classes. These are the two that gate MUTATION:
--
--   * `can_modify_resource`    -- the write gate itself. An equivalence defect here is a mutation
--                                 that should have been refused and was not.
--   * `profile_explicit_grant` -- action-parameterized (read/write/delete/grant). Called by `can`,
--                                 `cogmap_authorable_by_profile`, `context_authorable_by_profile`,
--                                 and `cogmap_readable_by_profile`.
--
-- `profile_explicit_grant` serves read callers too, and is routed here anyway. Applying the risk
-- rule to a function's WORST caller is the point of having the rule; a helper that can decide a
-- write is a write-path function whatever else it also decides.
--
-- Separate so the two classes are independently revertible. A read-side defect and a write-side
-- defect are found by different people, at different times, and want different responses.
--
-- Note what the split is NOT. Both halves are declared `additive`, so the deploy applies them in one
-- run and no operator stands between them. The split buys a clean revert boundary and a legible
-- history, not a staged rollout. If a staged rollout is ever wanted here, that is a class decision,
-- and it is the one 20260804000010's header records being made the other way.
--
-- With this applied, clause 1 of goal 019fb881 holds: nothing outside `profile_reachable_teams`
-- restates the expression. The clause-1 witness in `reachable_teams_one_definition_test.rs` --
-- a `pg_proc` scan, not a fixed list, so it also catches the tenth copy nobody has written yet --
-- goes green here and not before.
--
-- =================================================================================================
-- ONE THING WORTH NOT MISSING
-- =================================================================================================
--
-- `can_modify_resource`'s soft-delete WRITE floor and its container-write cascade are untouched.
-- The ONLY change is the CTE that feeds the team-anchored write-grant arm. The floor
-- (`EXISTS (... r.is_active)`) still short-circuits the whole predicate, and the cascade still
-- delegates to `cogmap_authorable_by_profile` / `context_authorable_by_profile` -- both of which
-- reach `profile_reachable_teams` through `profile_explicit_grant` below, so after this migration
-- every arm of the write gate consults one definition.

-- -------------------------------------------------------------------------------------------------
-- 8/9 -- profile_explicit_grant.
-- -------------------------------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION profile_explicit_grant(p_profile uuid, p_action text, p_subject_table text, p_subject_id uuid)
RETURNS boolean
LANGUAGE sql
STABLE
AS $$
    WITH reachable_teams AS (
        SELECT team_id FROM profile_reachable_teams(p_profile)
    )
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
                   AND g.principal_id IN (SELECT team_id FROM reachable_teams)) )
    );
$$;

-- -------------------------------------------------------------------------------------------------
-- 9/9 -- can_modify_resource.
-- -------------------------------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION can_modify_resource(p_profile uuid, p_resource uuid)
RETURNS boolean
LANGUAGE sql
STABLE
AS $$
    -- Soft-delete WRITE floor: a tombstone is unmodifiable on every axis.
    SELECT EXISTS (SELECT 1 FROM kb_resources r WHERE r.id = p_resource AND r.is_active)
       AND EXISTS (
        WITH reachable_teams AS (
            SELECT team_id FROM profile_reachable_teams(p_profile)
        )
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
        -- team-anchored WRITE grant on a reachable (self-or-ancestor) team.
        SELECT 1 FROM kb_access_grants g
         JOIN reachable_teams rt ON g.principal_id = rt.team_id
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
    20260804000020,
    'additive',
    'reachable_teams gets one authoritative home -- the mutation gates. Routes the two remaining hand-copies (can_modify_resource, profile_explicit_grant) through profile_reachable_teams, completing goal 019fb881 clause 1. Split from 20260804000010 on risk class: these two gate writes, so a defect here is an unrefused mutation rather than mis-scoped visibility, and the two halves must be independently revertible. Additive for the same reason as its sibling: signatures are unchanged, so no deployed binary can disagree with this schema across the apply. See 20260804000010''s header for the full reasoning and for what it decides against in goal 019fb881.'
);
