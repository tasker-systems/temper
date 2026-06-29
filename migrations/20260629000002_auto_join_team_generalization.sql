-- Auto-join team generalization (org-provisioning roadmap, Chunk 1; 2026-06-28 spec §4).
-- Generalizes the temper-system-hardcoded `sync_system_membership` trigger so that ANY team
-- flagged with `kb_teams.auto_join_role` becomes an always-complete "everyone" pool — the
-- whole-Venn audience for org-wide cogmaps bound via `kb_team_cogmaps`. temper-system stops
-- being a special case (resolved Q-A): it is set as an ordinary auto-join team at `watcher`,
-- and the trigger loops over every flagged team instead of one hardcoded slug.
--
-- Additive — a NEW migration that ALTERs the table and CREATE-OR-REPLACEs the existing trigger
-- function (keeping the function name so the existing trigger binding survives); it never edits
-- the shipped `20260624000002` baseline. Namespace-free (no `SET search_path`).
--
-- Semantic invariants (do not deviate):
--   * Enrollment gates on `has_system_access(profile)` (computed), NOT the raw `system_access`
--     column. In `open` mode (default) that is true for everyone → every profile auto-joins
--     every auto-join team (decision #3's everyone-pool). In `invite_only` mode it is true only
--     for gating-team members.
--   * The `admin → owner` mapping is KEPT, applied uniformly across all auto-join teams:
--     `system_access = 'admin'` enrolls at `owner`, else the team's `auto_join_role`. This
--     preserves the test-harness admin-minting (`cogmap_authz_test.rs` mints an admin via
--     `UPDATE system_access='admin'` and relies on the trigger producing an `owner` row).
--   * On losing access (`has_system_access` false), the trigger removes the profile from ALL
--     auto-join teams (resolved Q-C).

-- ============================================================================
-- 1. The auto-join flag. NULL = not an auto-join team. temper-system follows the convention.
-- ============================================================================

ALTER TABLE kb_teams ADD COLUMN auto_join_role team_role;

-- temper-system becomes an ordinary auto-join team (no longer special-cased in the trigger).
UPDATE kb_teams SET auto_join_role = 'watcher' WHERE slug = 'temper-system';

-- ============================================================================
-- 2. Idempotent enrollment for ONE profile across ALL auto-join teams.
-- ----------------------------------------------------------------------------
-- Called by the generalized trigger and (Chunk 6) the invite_only access-grant site. Gated on
-- `has_system_access` (no-op when false). Role = admin→owner, else the team's auto_join_role.
-- `ON CONFLICT DO UPDATE SET role` so a status change (e.g. promotion to admin) is reflected.
-- ============================================================================

CREATE FUNCTION ensure_auto_join_memberships(p_profile uuid)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    IF NOT has_system_access(p_profile) THEN
        RETURN;  -- not eligible (invite_only non-member); enroll nothing
    END IF;
    INSERT INTO kb_team_members (team_id, profile_id, role)
    SELECT t.id, p_profile,
           CASE WHEN (SELECT system_access FROM kb_profiles WHERE id = p_profile) = 'admin'
                THEN 'owner'::team_role
                ELSE t.auto_join_role END
      FROM kb_teams t
     WHERE t.auto_join_role IS NOT NULL
    ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role;
END;
$$;

-- ============================================================================
-- 3. Backfill ONE team when its auto-join flag is newly enabled.
-- ----------------------------------------------------------------------------
-- Enrolls every `has_system_access` profile (the gap the per-profile trigger lacks — a team can
-- predate its members). `ON CONFLICT DO NOTHING` so it never clobbers a manually-set role.
-- No-op when the team's `auto_join_role IS NULL` (or the team is absent).
-- ============================================================================

CREATE FUNCTION backfill_auto_join_team(p_team uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_role team_role;
BEGIN
    SELECT auto_join_role INTO v_role FROM kb_teams WHERE id = p_team;
    IF v_role IS NULL THEN
        RETURN;  -- not an auto-join team; nothing to backfill
    END IF;
    INSERT INTO kb_team_members (team_id, profile_id, role)
    SELECT p_team, p.id,
           CASE WHEN p.system_access = 'admin' THEN 'owner'::team_role
                ELSE v_role END
      FROM kb_profiles p
     WHERE has_system_access(p.id)
    ON CONFLICT (team_id, profile_id) DO NOTHING;
END;
$$;

-- ============================================================================
-- 4. Generalize the trigger function (same name → existing trigger binding survives; the trigger
--    is NOT dropped/recreated). On a system_access change: if the profile has access, ensure its
--    auto-join memberships; otherwise revoke them from EVERY auto-join team (Q-C).
-- ============================================================================

CREATE OR REPLACE FUNCTION sync_system_membership()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF has_system_access(NEW.id) THEN
        PERFORM ensure_auto_join_memberships(NEW.id);
    ELSE
        DELETE FROM kb_team_members tm
         USING kb_teams t
         WHERE tm.team_id = t.id
           AND t.auto_join_role IS NOT NULL
           AND tm.profile_id = NEW.id;
    END IF;
    RETURN NEW;
END;
$$;

-- ============================================================================
-- 5. Backfill temper-system. The boot seed (`20260624000003`) runs BEFORE temper-system exists
--    (`20260625000001`), so the trigger no-oped at seed time and temper-system had zero members
--    on a fresh install. This enrolls the existing profiles now: the seed `system` admin → owner,
--    plus any others with `has_system_access`.
-- ============================================================================

SELECT backfill_auto_join_team((SELECT id FROM kb_teams WHERE slug = 'temper-system'));
