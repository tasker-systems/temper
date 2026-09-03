-- The capability seam floors can()'s explicit-grant branch on subject liveness.
--
-- can()'s profile arm unifies two branches: the derived floor (derived_access_profile), which
-- delegates to the concrete predicates -- every one semi-joins kb_resources.is_active or
-- kb_contexts.is_active -- and profile_explicit_grant, which is subject-polymorphic and reads
-- only kb_access_grants, so it cannot know a subject kind's liveness column. Both branches must
-- answer a tombstoned subject identically; the floor therefore lives at the seam, on the
-- explicit branch -- the same delegation shape context_authorable_by_profile (20260826000110)
-- applies to its own profile_explicit_grant arm. A grant row naming a subject id that no row
-- backs falls to the same EXISTS.
--
-- Subject kinds without a liveness column (kb_cogmaps, kb_connections) are unfloored by the
-- explicit ELSE true: a grant row on them stays answerable (20260714000020), and a grantable
-- kind joins this CASE as part of its own design, never by inheritance.

CREATE OR REPLACE FUNCTION can(
    p_principal_table text, p_principal_id uuid, p_action text,
    p_subject_table text, p_subject_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE p_principal_table
        WHEN 'kb_profiles' THEN
            (
                profile_explicit_grant(p_principal_id, p_action, p_subject_table, p_subject_id)
                -- Subject-liveness floor: the explicit arm answers a subject only while a live
                -- row backs it. Kinds with no liveness column are admitted (ELSE true).
                AND CASE p_subject_table
                        WHEN 'kb_resources' THEN EXISTS (
                            SELECT 1 FROM kb_resources r
                             WHERE r.id = p_subject_id AND r.is_active)
                        WHEN 'kb_contexts' THEN EXISTS (
                            SELECT 1 FROM kb_contexts c
                             WHERE c.id = p_subject_id AND c.is_active)
                        ELSE true
                    END
            )
            OR derived_access_profile(p_principal_id, p_action, p_subject_table, p_subject_id)
        WHEN 'kb_cogmaps' THEN
            p_subject_table = 'kb_resources' AND p_action = 'read'
            AND p_subject_id IN (SELECT resource_id FROM resources_accessible_to_cogmap(p_principal_id))
        ELSE false
    END;
$$;

COMMENT ON FUNCTION can(text, uuid, text, text, uuid) IS
    'Unified capability seam. Profile axis: explicit grant (floored on subject liveness: '
    'kb_resources.is_active / kb_contexts.is_active; kinds with no liveness column unfloored) '
    'OR the derived floor. Cogmap axis: the producer intersection, resource subjects, read only, '
    'no explicit grants. Both profile branches answer a tombstoned subject identically.';

SELECT declare_migration(
    20260902000010,
    'additive',
    'CREATE OR REPLACE on the STABLE can(text,uuid,text,text,uuid): the explicit-grant branch '
    'gains a subject-liveness floor (EXISTS on kb_resources.is_active / kb_contexts.is_active; '
    'ELSE true for kinds without a liveness column). Signature, return type and every live-subject '
    'answer are unchanged -- the floor fires only where no live subject row backs the grant row, '
    'where the derived branch already answered false. Design: temper task '
    '01a063ed-8fa2-7bb1-9690-89e710f05d96 (predicate-integrity harness + two floors).'
);
