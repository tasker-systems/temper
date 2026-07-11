-- Soft-delete WRITE floor on can_modify_resource (I6 — a tombstone is invisible on EVERY axis).
--
-- Surfaced by the adversarial security review of the context-write axis (the same review that vetted
-- 20260712000010). The read side already has a soft-delete floor everywhere — resources_visible_to,
-- resources_accessible_to_cogmap, resources_readable_by all JOIN kb_resources … AND r.is_active. The
-- WRITE side never got one. can_modify_resource gates on ownership / grant / container-authorship but
-- never checks that the resource is live.
--
-- The consequence is a real leak, confirmed live and reproduced end-to-end:
--
--   * can_modify_resource(author, tombstone) returns TRUE while
--     resources_visible_to(author) excludes the tombstone — read says DENY, write says PERMIT on the
--     identical (profile, resource) pair.
--   * update_resource (db_backend.rs) gates ONLY on check_can_modify_next → can_modify_resource. The
--     visibility-gated readback prefetch sits behind an `if managed_meta.is_some() || title.is_some()`
--     guard, so a BODY-ONLY or OPEN_META-ONLY PATCH skips it entirely and the mutation commits onto
--     the tombstone. Not a harmless gate-says-yes: the write lands.
--
-- This predates 20260712000010 — the floor was missing in 20260706000003 / 20260701000003 — but it is
-- the same write-authz axis that migration reworks, and the review found it by probing that change, so
-- it bundles here.
--
-- CLASSIFICATION: NARROWING, negligible blast radius. It only ever denies writes to is_active=false
-- rows, which are already read-invisible and have no restore/undelete path — no legitimate write is
-- lost. Folds into the same alpha-window rollout thought as 20260712000010's context_authorable_by
-- narrowing.
--
-- Implementation: the four-arm body is reproduced VERBATIM from 20260706000003 (printed from the live
-- DB, not reconstructed); the ONLY change is the leading liveness conjunct wrapping the whole EXISTS.
-- Wrapping (rather than adding `AND is_active` to each arm) keeps the floor in ONE place — the future
-- fifth arm inherits it for free. derived_access_profile('write') delegates to this function and needs
-- no change; it inherits the floor.

CREATE OR REPLACE FUNCTION can_modify_resource(p_profile uuid, p_resource uuid)
RETURNS boolean
LANGUAGE sql STABLE AS $$
    -- Soft-delete WRITE floor: a tombstone is unmodifiable on every axis. ANDed over the whole
    -- predicate so every present and future authorization arm is gated by it.
    SELECT EXISTS (SELECT 1 FROM kb_resources r WHERE r.id = p_resource AND r.is_active)
       AND EXISTS (
        WITH reachable_teams AS (
            SELECT DISTINCT a.team_id
            FROM profile_effective_teams(p_profile) e
            CROSS JOIN LATERAL team_ancestors(e.team_id) a
        )
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
        -- context homes add an owner floor. See context_authorable_by_profile.
        SELECT 1 FROM kb_resource_homes h
         WHERE h.resource_id = p_resource
           AND CASE h.anchor_table
                 WHEN 'kb_cogmaps'  THEN cogmap_authorable_by_profile(p_profile, h.anchor_id)
                 WHEN 'kb_contexts' THEN context_authorable_by_profile(p_profile, h.anchor_id)
                 ELSE false
               END
    );
$$;

COMMENT ON FUNCTION can_modify_resource(uuid, uuid) IS
    'Resource mutation gate. A soft-deleted (is_active=false) resource is unmodifiable on every axis '
    '(the WRITE peer of the read-side soft-delete floor). Below that: owned/originated, direct or '
    'reachable-team write-grant, or container-authorship (context_authorable_by_profile / '
    'cogmap_authorable_by_profile).';
