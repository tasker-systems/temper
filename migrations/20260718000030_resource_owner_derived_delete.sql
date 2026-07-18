-- A derived `delete` arm for resource-home owners (plan Task 5b.3).
--
-- WHY THIS EXISTS: 5b.3 makes grant-administration ATTENUATING -- a delegated administrator may
-- confer only capabilities they themselves hold on the subject. "Hold" resolves through `can(...)`,
-- and `derived_access_profile` had NO `delete` arm for ANY subject type, so `can(..., 'delete', ...)`
-- was true only via an explicit grant row. Prod carries zero such rows (5 grants total, all
-- write-only). Attenuation would therefore have made `can_delete` a BOOTSTRAP DEADLOCK: nobody holds
-- it, so no non-admin could ever confer it, so nobody ever comes to hold it -- delete becoming
-- permanently admin-only as an artifact of the rule rather than as anyone's design decision.
--
-- The fix is to make the capability derivable by the one principal who evidently should have it: the
-- owner of the resource's home. An owner who may delete their own resource may now also confer that.
--
-- SCOPED TO kb_resources DELIBERATELY. Cogmaps and contexts get no delete arm here: neither has an
-- ownership floor comparable to `kb_resource_homes.owner_profile_id`, and granting one would be a
-- design conversation about those subject types, not a consequence of the attenuation rule. The
-- `ELSE false` arm keeps them fail-closed, unchanged.
--
-- Shape mirrors the existing `grant` arm exactly -- same table, same ownership predicate -- because
-- that arm already encodes "this profile is the resource's owner" and a second spelling of the same
-- idea is a second thing to drift.
--
-- Additive and idempotent: CREATE OR REPLACE of a pure `sql`/STABLE function, no data touched, no
-- dependent object dropped. Version-portable across PG17 (Neon prod) and PG18 (local/CI).

CREATE OR REPLACE FUNCTION derived_access_profile(
    p_profile       uuid,
    p_action        text,
    p_subject_table text,
    p_subject_id    uuid
) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT CASE
        WHEN p_subject_table = 'kb_resources' AND p_action = 'read'  THEN
            p_subject_id IN (SELECT resource_id FROM resources_visible_to(p_profile))
        WHEN p_subject_table = 'kb_resources' AND p_action = 'write' THEN
            can_modify_resource(p_profile, p_subject_id)
        WHEN p_subject_table = 'kb_resources' AND p_action = 'grant' THEN
            EXISTS (SELECT 1 FROM kb_resource_homes h
                    WHERE h.resource_id = p_subject_id
                      AND h.owner_profile_id = p_profile)
        -- NEW (5b.3): the owner of a resource's home derives `delete` on it.
        WHEN p_subject_table = 'kb_resources' AND p_action = 'delete' THEN
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

COMMENT ON FUNCTION derived_access_profile IS
  'Access derivable from structure rather than an explicit kb_access_grants row.

WHAT: the kb_resources `delete` arm (added 2026-07-18) lets the owner of a resource''s home derive `delete` on it, alongside the `grant` arm that already keys on the same ownership predicate.

WHY: grant-administration attenuates -- a delegated administrator may confer only capabilities it itself holds. "Holds" resolves through can(), so with no derivable `delete` holder anywhere, and zero kb_access_grants rows carrying can_delete, can_delete would have been a bootstrap deadlock: nobody holds it, so no delegated administrator could ever confer it, so nobody ever comes to hold it. Delete would have become admin-only as an artifact of the attenuation rule rather than as a decision anyone made.

WHY ONLY kb_resources: cogmaps and contexts deliberately get NO delete arm. Neither has an ownership floor comparable to kb_resource_homes.owner_profile_id -- a cogmap has no owner column at all, and context ownership is a different relation -- so a delete arm for either is a design question about THAT subject type, not a consequence of the attenuation rule that motivated this one. They stay fail-closed on the ELSE arm; adding them later is additive and should be argued on its own merits.';
