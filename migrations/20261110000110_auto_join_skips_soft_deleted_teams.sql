-- Auto-join enrols no one onto a soft-deleted team (soft-delete design D4: nothing is written onto a
-- dead team). auto_join_reconcile, backfill_auto_join_team and ensure_auto_join_memberships each
-- gain `is_active` on the team they read; nothing else in their bodies changes.
--
-- Split from 20261110000100 only because these bodies read kb_teams.auto_join_role, which
-- temper-substrate's pre-auto-join test baseline (tests/common/mod.rs) omits, so that harness skips
-- this file as it skips the other auto-join migrations.
--
-- Design: temper-artifacts specs/2026-10-31-soft-deleted-team-confers-nothing-design.md D4.

CREATE OR REPLACE FUNCTION public.auto_join_reconcile()
 RETURNS TABLE(team_slug text, profile_handle text)
 LANGUAGE sql
AS $function$
    WITH ins AS (
        INSERT INTO kb_team_members (team_id, profile_id, role)
        SELECT t.id, p.id, t.auto_join_role
          FROM kb_teams t
         CROSS JOIN kb_profiles p
         WHERE t.auto_join_role IS NOT NULL AND t.is_active
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
$function$;

CREATE OR REPLACE FUNCTION public.backfill_auto_join_team(p_team uuid)
 RETURNS void
 LANGUAGE plpgsql
AS $function$
DECLARE
    v_role team_role;
BEGIN
    SELECT auto_join_role INTO v_role FROM kb_teams WHERE id = p_team AND is_active;
    IF v_role IS NULL THEN
        RETURN;  -- not a live auto-join team; nothing to backfill
    END IF;
    INSERT INTO kb_team_members (team_id, profile_id, role)
    SELECT p_team, p.id, v_role
      FROM kb_profiles p
     WHERE has_system_access(p.id)
       AND NOT EXISTS (SELECT 1 FROM kb_machine_clients mc WHERE mc.profile_id = p.id)
    ON CONFLICT (team_id, profile_id) DO NOTHING;
END;
$function$;

CREATE OR REPLACE FUNCTION public.ensure_auto_join_memberships(p_profile uuid)
 RETURNS void
 LANGUAGE plpgsql
AS $function$
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
     WHERE t.auto_join_role IS NOT NULL AND t.is_active
    ON CONFLICT (team_id, profile_id) DO NOTHING;
END;
$function$;

SELECT declare_migration(
    20261110000110,
    'additive',
    'Body-only CREATE OR REPLACE, signatures and return types unchanged, of auto_join_reconcile, '
    'backfill_auto_join_team and ensure_auto_join_memberships: each skips a soft-deleted team. No row '
    'is rewritten. Design: temper-artifacts '
    'specs/2026-10-31-soft-deleted-team-confers-nothing-design.md D4.'
);
