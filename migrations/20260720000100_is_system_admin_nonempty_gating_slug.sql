-- `is_system_admin` must not fall open on an empty gating-team slug.
--
-- NUMBERING. Deliberately `…000100`, not `…000040`. The principal-admission Phase 1 plan
-- (docs/superpowers/plans/2026-07-20-principal-admission-phase-1.md) reserves
-- `20260720000010`–`000060`, and its Task 7 is literally `20260720000040_repoint_predicates` —
-- an exact collision. This one is unapplied outside local dev, so it was cheap to move now and
-- would have been immutable after merge.
--
-- RELATIONSHIP TO THAT PLAN. Task 7 replaces this function's body outright, repointing it at
-- `kb_principal_governance`, at which point gating-team ownership carries no authorization
-- meaning and `gating_team_slug` is not consulted at all — so this fix is SUPERSEDED, not
-- conflicting. It is worth landing anyway: the predicate is live until that cutover, the
-- fall-open is real while it is, and a cutover is a bad moment to be standing on a predicate
-- that can silently admit an unintended owner.
--
-- THE DEFECT. The function (20260624000002_canonical_functions.sql) answers "is this profile an
-- OWNER of the gating team?" by matching `t.slug = settings.gating_team_slug`. Nothing constrains
-- `kb_system_settings.gating_team_slug` to be non-empty — there is no CHECK on the column — so if
-- that value is ever `''` (a bad UPDATE, an operator clearing the field, a bootstrap that writes an
-- empty string rather than leaving the default), the predicate stops naming the gating team and
-- starts naming "any team whose slug is the empty string". Every owner of such a team silently
-- becomes a system admin.
--
-- That is a FALL-OPEN, which is the direction that matters: the failure of a mis-set gating slug
-- should be that nobody is an admin (locked out, loudly, repairable by an operator), never that an
-- unrelated team's owners are. `is_system_admin` gates context reassignment
-- (20260715000010_context_reassign_fns.sql), the machine-registration path
-- (20260714000010_connections.sql) and the admin disconnect wrapper — so the blast radius is the
-- administrative surface, not one endpoint.
--
-- NULL needs no guard: `t.slug = NULL` is already NULL, hence not true. Only `''` is exploitable,
-- because `''` is a value that can genuinely equal a real column.
--
-- WHY `CREATE OR REPLACE` AND NOT `DROP` + `CREATE`. The signature and return type are unchanged,
-- so REPLACE is legal here and is the strictly safer form: a DROP leaves a window in which the
-- function does not exist at all, which breaks migrate-ahead-of-deploy (the running old code calls
-- `is_system_admin` throughout that window). This migration is therefore purely ADDITIVE and safe
-- to auto-deploy on `main`: old code calling the replaced function gets identical answers for every
-- non-empty gating slug, which is every real deployment. Precedent for same-signature REPLACE:
-- 20260715000040_demote_originator_from_access.sql, 20260712000110_recompute_body_hash_row_lock.sql.
--
-- SCOPE. `has_system_access` (same file, the `invite_only` branch) carries the same shape and is
-- deliberately NOT touched here — it is a different predicate with a different blast radius and it
-- deserves its own decision, not a drive-by.

CREATE OR REPLACE FUNCTION is_system_admin(p_profile_id UUID) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    WITH settings AS (
        SELECT gating_team_slug
          FROM kb_system_settings
         LIMIT 1
    )
    SELECT EXISTS (
        SELECT 1
          FROM kb_team_members tm
          JOIN kb_teams t ON t.id = tm.team_id
         WHERE tm.profile_id = p_profile_id
           AND t.slug = settings.gating_team_slug
           AND settings.gating_team_slug <> ''
           AND tm.role = 'owner'
    )
      FROM settings
$$;

COMMENT ON FUNCTION is_system_admin IS
  'Owner of the gating team named by kb_system_settings.gating_team_slug. The `<> ''''` guard is load-bearing: the column has no non-empty CHECK, and an empty slug would otherwise match any team slugged '''', making that team''s owners system admins. A mis-set gating slug must lock everyone out, never let someone in.';
