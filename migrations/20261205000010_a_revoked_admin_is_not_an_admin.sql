-- A principal whose standing is not live is never a system admin, whatever governance row remains.
--
-- Until now is_system_admin read kb_principal_governance and nothing else (20260720000110), and
-- "admin implies approved" (spec 2026-07-20 §9) was kept by transition only: Revoke and Deactivate
-- demote in the same transaction (standing_service::judge_and_apply_conn), and promotion approves
-- before it grants (access_service::promote_admin). No production door was found that leaves a
-- revoked principal holding governance, but nothing at read time refused one either. No trigger
-- forbids a direct UPDATE of kb_principal_standing, and that is the only known way to build the
-- state; a missed demotion or a future door that ends standing without the committer would do the
-- same. The read-time AND covers all of them.
--
-- THIS AMENDS 20260720000110 AND THE kb_principal_governance COMMENT (20260720000010), which said
-- the invariant is kept "never by an AND at read time" (D2/D10). Ruling 37 of decision
-- 01a21d95-1e3d-70bb-a01f-c3b2d3e9e658 chose the AND as defence in depth. D2's concern was
-- admission ANDing inputs from several tables; here governance reads admission's one answer, so
-- "may you act" stays a single-table question and only "may you govern" depends on it.
--
-- "LIVE" MEANS 'approved': exactly the set has_system_access admits.
--
-- THE BODY ASKS has_system_access, NOT A RESTATED STANDING TEST. "You cannot govern an instance
-- you may not use" is the invariant, and has_system_access is the one definition of "may use"
-- (standing = 'approved', 20260720000110). Every state outside it is excluded: revoked,
-- deactivated, denied, requested, and absence. If "may use" ever widens or narrows, admin-ness
-- follows it without another migration.
--
-- EXISTS AND EXISTS, never a scalar. Both halves are EXISTS, so the result is never NULL and the
-- function stays fail-closed both in a WHERE clause and under a plpgsql `IF NOT is_system_admin(...)`
-- (20260715000010:76, 20260731000040:106, 20260826000120:143,204, 20260826000130:74). The
-- 20260720000110 header has the measurement showing why.
--
-- The signature is unchanged, so every caller follows with no code change: the Rust passthrough
-- access_service::is_system_admin, auth::is_system_admin_locked_raw (which takes the governance row
-- FOR KEY SHARE and then calls this), and the in-database callers above and
-- cogmap_authorable_by_profile. The demotion on Revoke and Deactivate stays: it ends the grant,
-- and this only stops a stray one from taking effect.

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
