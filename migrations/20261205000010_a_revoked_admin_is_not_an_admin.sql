-- A principal whose standing is not live is never a system admin, whatever governance row remains.
--
-- Until now is_system_admin read kb_principal_governance alone (20260720000110), and "admin implies
-- approved" (spec 2026-07-20 §9) held by transition only: Revoke and Deactivate demote in the same
-- transaction (standing_service), and promote_admin approves before it grants. Nothing refused the
-- drift at read. Two unguarded routes produce it: a direct UPDATE of kb_principal_standing (no
-- trigger forbids one), and principal_governance_set(p, true, ...) on a principal not approved
-- (it has no standing guard; its one production caller approves first). The read-time AND covers
-- both, and any future door that ends standing without the committer.
--
-- THIS AMENDS the "never an AND at read time" claims in 20260720000110 and in the
-- kb_principal_governance comment, per ruling 37 of decision 01a21d95-1e3d-70bb-a01f-c3b2d3e9e658
-- (defence in depth). Governance reads admission's one answer, so "may you act" stays one table.
--
-- "LIVE" MEANS 'approved', and the body asks has_system_access rather than restating it, so
-- admin-ness follows the one definition of "may use" if that ever changes.
--
-- NEVER NULL. Both halves are EXISTS, so every `IF NOT is_system_admin(...)` site and every WHERE
-- use stays fail-closed (why that matters: the 20260720000110 header). The signature is unchanged,
-- so every caller follows, including auth::is_system_admin_locked_raw, which locks the governance
-- row FOR KEY SHARE first. The DO block below reports existing drift by count; it changes no row.

CREATE OR REPLACE FUNCTION is_system_admin(p_profile_id UUID) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM kb_principal_governance g WHERE g.profile_id = p_profile_id
    )
    AND has_system_access(p_profile_id)
$$;

COMMENT ON FUNCTION is_system_admin IS
  'May this principal change the rules? True only for a kb_principal_governance row AND live '
  'standing (has_system_access). A revoked, deactivated or unadmitted principal never governs, '
  'whatever rows remain. Total: never NULL (20261205000010).';

COMMENT ON TABLE kb_principal_governance IS
  'Who may change the rules (spec 2026-07-20 D10). A row grants admin-ness only while the '
  'principal''s standing is approved: is_system_admin also asks has_system_access '
  '(20261205000010). Revoke and Deactivate still delete the row (standing_service), and promotion '
  'approves before it grants, so a row with standing not approved is drift that is refused at read.';

-- Report drift instead of hiding it: an install holding governance rows whose standing is not
-- approved learns how many on apply (those principals stop being admins now). Count only, no ids.
DO $$
DECLARE v_n bigint;
BEGIN
    SELECT count(*) INTO v_n
      FROM kb_principal_governance g
     WHERE NOT has_system_access(g.profile_id);
    IF v_n > 0 THEN
        RAISE WARNING '% governance row(s) belong to a principal whose standing is not approved; they no longer confer system admin',
            v_n;
    END IF;
END $$;

SELECT declare_migration(
    20261205000010,
    'additive',
    'Body-only CREATE OR REPLACE of is_system_admin(uuid), signature and return type unchanged: a '
    'governance row confers admin-ness only while has_system_access holds. Behaviour change, '
    'deliberate and narrowing: a principal holding governance with standing not approved stops '
    'being an admin on apply. No door is known to produce that state, so no live admin is expected '
    'to lose access. No row is rewritten. A running binary calls the function by name and follows '
    'the new body at once.'
);
