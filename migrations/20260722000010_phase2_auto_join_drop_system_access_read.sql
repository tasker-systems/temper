-- ============================================================================
-- Principal admission Phase 2 · A4 — stop the auto-join functions reading the
-- doomed `kb_profiles.system_access` column (additive, CREATE OR REPLACE).
-- ----------------------------------------------------------------------------
-- Phase 1 (D11) made `kb_principal_standing` authoritative for access and
-- repointed `has_system_access` onto it (`20260720000110`). `system_access`
-- survives only as a read-only projection that Phase 2's PR-B drops. Two
-- functions still READ the column — solely for the `admin → owner` team-role
-- coupling — so they must be repointed off it BEFORE the column can drop.
--
-- The rewrite drops that coupling entirely: every auto-join enrollment now uses
-- the team's `auto_join_role` uniformly. Admin-ness lives in
-- `kb_principal_governance` now (D10/D18), NOT in a team-membership row, so the
-- resulting cosmetic team-role churn for a newly-approved admin (`owner → the
-- team's auto_join_role`, e.g. `watcher`) is blessed by spec §11: auto-join
-- membership is decorative under D18 (confers no access). Existing owner rows
-- (e.g. the boot-seeded `system` actor, enrolled as `temper-system` owner by the
-- `20260629000002` migration-time backfill) are NOT rewritten — this is
-- CREATE OR REPLACE, not a re-backfill.
--
-- The `has_system_access(p_profile)` gate is unchanged: it already reads standing
-- (not the column) post-`20260720000110`, so eligibility semantics are identical.
-- Only the role-assignment CASE that named the column is removed.
--
-- The trigger `trg_sync_system_membership` and its body `sync_system_membership`
-- are untouched here: the body names no column (it calls `has_system_access` +
-- an `ELSE DELETE`), and the trigger *binding* (`OF system_access`) is dropped
-- with the column in PR-B.
-- ============================================================================

CREATE OR REPLACE FUNCTION ensure_auto_join_memberships(p_profile uuid)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    IF NOT has_system_access(p_profile) THEN
        RETURN;  -- not eligible (invite_only non-member); enroll nothing
    END IF;
    INSERT INTO kb_team_members (team_id, profile_id, role)
    SELECT t.id, p_profile, t.auto_join_role
      FROM kb_teams t
     WHERE t.auto_join_role IS NOT NULL
    ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role;
END;
$$;

CREATE OR REPLACE FUNCTION backfill_auto_join_team(p_team uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_role team_role;
BEGIN
    SELECT auto_join_role INTO v_role FROM kb_teams WHERE id = p_team;
    IF v_role IS NULL THEN
        RETURN;  -- not an auto-join team; nothing to backfill
    END IF;
    INSERT INTO kb_team_members (team_id, profile_id, role)
    SELECT p_team, p.id, v_role
      FROM kb_profiles p
     WHERE has_system_access(p.id)
    ON CONFLICT (team_id, profile_id) DO NOTHING;
END;
$$;
