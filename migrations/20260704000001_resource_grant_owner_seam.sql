-- Owner-grant seam for per-resource capability sharing.
--
-- `derived_access_profile` (the non-explicit-grant reach behind `can()`) had no 'grant'
-- arm for resources, so a bare resource OWNER failed `can_administer_grant` and could not
-- share their own resource. Add owner => grant, symmetric with `can_modify_resource`'s
-- "the home confers modify to its principals". Scoped to owner_profile_id ONLY --
-- originator is provenance, not access. Non-destructive CREATE OR REPLACE (additive-only-on-main).
--
-- The rest of the body is reproduced verbatim from 20260630000001_access_grants_seam.sql:75-91
-- (the only prior definition; never redefined since). Only the resource 'grant' arm is new.
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
        ELSE false
    END;
$$;
