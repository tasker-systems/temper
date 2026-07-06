-- Container-write → node-write cascade (symmetric for cogmaps and contexts).
--
-- Decision + spec: docs/superpowers/specs/2026-07-06-container-write-cascade-and-authz-hardening-design.md
--
-- Unix directory semantics: whoever may write a container may modify (and supersede) any
-- node homed in it, regardless of the node's own owner/originator. A cogmap co-author or a
-- context writer could already create nodes and fold-then-recreate others' nodes, so gating
-- node-modify on node-ownership alone was illusory. This makes the coherent model explicit:
-- directory-write ⇒ file-rwx. Provenance is unaffected — the event ledger records the actual
-- actor on every fold/assert/facet.
--
-- Three additive CREATE OR REPLACE definitions (additive-only-on-main):
--   1. context_authorable_by_profile  — NEW (the cogmap predicate's context sibling)
--   2. can_modify_resource            — canonical body VERBATIM + one container-cascade arm
--   3. derived_access_profile         — adds the kb_contexts/'write' arm (was ELSE false)

-- ============================================================================
-- 1. context_authorable_by_profile — may `profile` author into this context?
--
-- Contexts (unlike cogmaps) have an owner, so this predicate carries an owner floor the
-- cogmap predicate lacks:
--   * personal-owned  → the owning profile authors it
--   * team-owned      → a reachable member of the OWNING team authors it
--   * explicit write grant (profile- or reachable-team-anchored)
--
-- The team-owned arm is DELIBERATE and is NOT the pre-Q-A "membership implies write" that
-- 20260701000001_cogmap_write_tightening removed. Q-A removed write for teams merely
-- JOINED-FOR-READ to a cogmap; OWNING a context is a strictly stronger relationship — a team
-- that owns a directory has directory-write. Do not "simplify" this arm to explicit-grant-only
-- on the belief it duplicates Q-A; it does not. Narrowing it is a separate deliberate flip.
-- ============================================================================
CREATE OR REPLACE FUNCTION context_authorable_by_profile(p_profile uuid, p_context uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        -- personal-owned: the owner authors their own context
        SELECT 1 FROM kb_contexts c
         WHERE c.id = p_context
           AND c.owner_table = 'kb_profiles' AND c.owner_id = p_profile
        UNION ALL
        -- team-owned: a reachable (self-or-ancestor) member of the owning team authors it
        SELECT 1 FROM kb_contexts c
         JOIN profile_effective_teams(p_profile) e ON TRUE
         CROSS JOIN LATERAL team_ancestors(e.team_id) a
         WHERE c.id = p_context
           AND c.owner_table = 'kb_teams' AND c.owner_id = a.team_id
    )
    -- explicit write grant (profile- or reachable-team-anchored)
    OR profile_explicit_grant(p_profile, 'write', 'kb_contexts', p_context);
$$;

-- ============================================================================
-- 2. can_modify_resource — canonical body (20260701000003:109-133) VERBATIM,
--    plus ONE new UNION ALL arm: the container-write cascade.
-- ============================================================================
CREATE OR REPLACE FUNCTION can_modify_resource(p_profile uuid, p_resource uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    WITH reachable_teams AS (
        SELECT DISTINCT a.team_id
        FROM profile_effective_teams(p_profile) e
        CROSS JOIN LATERAL team_ancestors(e.team_id) a
    )
    SELECT EXISTS (
        -- owned / originated (the home confers modify to its principals)
        SELECT 1 FROM kb_resource_homes h
         WHERE h.resource_id = p_resource
           AND (h.owner_profile_id = p_profile OR h.originator_profile_id = p_profile)
        UNION ALL
        -- direct profile-anchored WRITE grant. kb_access_grants store.
        SELECT 1 FROM kb_access_grants g
         WHERE g.subject_table = 'kb_resources' AND g.subject_id = p_resource
           AND g.principal_table = 'kb_profiles' AND g.principal_id = p_profile AND g.can_write
        UNION ALL
        -- team-anchored WRITE grant on a reachable (self-or-ancestor) team. kb_access_grants store.
        SELECT 1 FROM kb_access_grants g
         JOIN reachable_teams rt ON g.principal_id = rt.team_id
         WHERE g.subject_table = 'kb_resources' AND g.subject_id = p_resource
           AND g.principal_table = 'kb_teams' AND g.can_write
        UNION ALL
        -- container-write cascade: whoever may author the home container may modify its nodes
        -- (unix directory-write ⇒ file-rwx). Cogmap homes are ownerless (explicit grant);
        -- context homes add an owner floor. See context_authorable_by_profile above.
        SELECT 1 FROM kb_resource_homes h
         WHERE h.resource_id = p_resource
           AND CASE h.anchor_table
                 WHEN 'kb_cogmaps'  THEN cogmap_authorable_by_profile(p_profile, h.anchor_id)
                 WHEN 'kb_contexts' THEN context_authorable_by_profile(p_profile, h.anchor_id)
                 ELSE false
               END
    );
$$;

-- ============================================================================
-- 3. derived_access_profile — body reproduced VERBATIM from 20260704000001:11-31,
--    adding only the kb_contexts/'write' arm (was falling through to ELSE false).
-- ============================================================================
CREATE OR REPLACE FUNCTION derived_access_profile(
    p_profile uuid, p_action text, p_subject_table text, p_subject_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE
        WHEN p_subject_table = 'kb_resources' AND p_action = 'read'  THEN
            p_subject_id IN (SELECT resource_id FROM resources_visible_to(p_profile))
        WHEN p_subject_table = 'kb_resources' AND p_action = 'write' THEN
            can_modify_resource(p_profile, p_subject_id)
        WHEN p_subject_table = 'kb_resources' AND p_action = 'grant' THEN
            EXISTS (SELECT 1 FROM kb_resource_homes h
                    WHERE h.resource_id = p_subject_id
                      AND h.owner_profile_id = p_profile)
        WHEN p_subject_table = 'kb_cogmaps'  AND p_action = 'read'  THEN
            cogmap_readable_by_profile(p_profile, p_subject_id)
        WHEN p_subject_table = 'kb_cogmaps'  AND p_action = 'write' THEN
            cogmap_authorable_by_profile(p_profile, p_subject_id)
        WHEN p_subject_table = 'kb_contexts' AND p_action = 'read'  THEN
            context_visible_to(p_profile, p_subject_id)
        WHEN p_subject_table = 'kb_contexts' AND p_action = 'write' THEN
            context_authorable_by_profile(p_profile, p_subject_id)
        ELSE false
    END;
$$;
