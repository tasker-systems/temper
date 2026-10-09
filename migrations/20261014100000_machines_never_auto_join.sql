-- A machine principal is never enrolled into an auto-join team.
--
-- Ruled 2026-10-09 (team-roles-as-a-rank plan session, task 019f77a2-4860-7300-a04e-df0d750dc4c7;
-- machine-principal task 01a120d6-a184-7672-8334-e895762c54e5): a machine holds team membership only
-- where someone with authority over that team chose it. 20260923000010 enrolled approved machines
-- like any other profile ("D2 makes no human/machine split; eligibility is has_system_access"), so an
-- admin's choice of auto_join_role, made once for the "everyone" pool, widened every approved
-- machine's read reach with nobody choosing it for that machine. That split is now made.
--
-- A machine is a profile with a kb_machine_clients row, revoked or not: revocation ends the
-- credential, not the profile's being a machine. The three enrolling functions each gain that one
-- exclusion; each body is otherwise its latest definition verbatim (ensure_auto_join_memberships and
-- auto_join_reconcile from 20260923000010, backfill_auto_join_team from 20260722000010).
--
-- Enrollment only. Existing rows are left alone here; see the end of this file.

CREATE OR REPLACE FUNCTION ensure_auto_join_memberships(p_profile uuid)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    IF NOT has_system_access(p_profile) THEN
        RETURN;  -- not eligible (standing not approved); enroll nothing
    END IF;
    IF EXISTS (SELECT 1 FROM kb_machine_clients mc WHERE mc.profile_id = p_profile) THEN
        RETURN;  -- a machine: auto-join never enrolls one
    END IF;
    INSERT INTO kb_team_members (team_id, profile_id, role)
    SELECT t.id, p_profile, t.auto_join_role
      FROM kb_teams t
     WHERE t.auto_join_role IS NOT NULL
    ON CONFLICT (team_id, profile_id) DO NOTHING;
END;
$$;

COMMENT ON FUNCTION ensure_auto_join_memberships IS
  'Enroll an eligible profile (has_system_access) into EVERY auto-join team at '
  'the team''s auto_join_role. A machine principal (any kb_machine_clients row) '
  'is never enrolled. ON CONFLICT DO NOTHING: enrollment defers to explicit '
  'state — an existing row at any role is never rewritten. Called at the '
  'standing committer; not a decision point.';

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
       AND NOT EXISTS (SELECT 1 FROM kb_machine_clients mc WHERE mc.profile_id = p.id)
    ON CONFLICT (team_id, profile_id) DO NOTHING;
END;
$$;

CREATE OR REPLACE FUNCTION auto_join_reconcile()
RETURNS TABLE (team_slug text, profile_handle text)
LANGUAGE sql AS $$
    WITH ins AS (
        INSERT INTO kb_team_members (team_id, profile_id, role)
        SELECT t.id, p.id, t.auto_join_role
          FROM kb_teams t
         CROSS JOIN kb_profiles p
         WHERE t.auto_join_role IS NOT NULL
           AND has_system_access(p.id)
           AND NOT EXISTS (SELECT 1 FROM kb_machine_clients mc WHERE mc.profile_id = p.id)
           AND NOT EXISTS (
               SELECT 1 FROM kb_team_members m
                WHERE m.team_id = t.id AND m.profile_id = p.id)
        ON CONFLICT (team_id, profile_id) DO NOTHING
        RETURNING team_id, profile_id
    )
    SELECT t.slug, pr.handle
      FROM ins
      JOIN kb_teams t     ON t.id = ins.team_id
      JOIN kb_profiles pr ON pr.id = ins.profile_id
     ORDER BY t.slug, pr.handle;
$$;

COMMENT ON FUNCTION auto_join_reconcile IS
  'Converge every auto-join team to the standing-approved human population; '
  'returns each (team, profile) pair this call added. A machine principal is '
  'never added. Adds only: an existing row at any role, and IdP-authored rows, '
  'are never rewritten or removed.';

SELECT declare_migration(
    20261014100000,
    'additive',
    'Three functions replaced with signatures unchanged, each gaining one machine exclusion. A binary without this migration keeps working; it calls the same functions, which now enroll humans only.'
);
