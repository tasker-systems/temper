-- ============================================================================
-- Principal admission Phase 2 · PR-B — drop the retired columns (DESTRUCTIVE).
-- ----------------------------------------------------------------------------
-- The end of Phase 2. Phase 1 (D11) made `kb_principal_standing` +
-- `kb_principal_governance` authoritative for access and admin-ness; the three
-- columns below survived only as read-only projections. PR-A (A1–A5) removed the
-- last reader and writer of each — prod and tests — so nothing in the running
-- binary touches them. This migration removes them.
--
-- ORDER IS LOAD-BEARING:
--   1. Drop the trigger FIRST — its binding (`AFTER INSERT OR UPDATE OF
--      system_access`) names the column, so the column cannot drop while it
--      exists. Governance owns demotion now (standing_service Revoke/Deactivate),
--      so the trigger's `ELSE DELETE` is no longer the only automatic demotion
--      path; the auto-join membership it maintained is decorative under D18
--      (confers no access), so stale memberships left behind are harmless.
--   2. Drop the now-orphaned trigger function (nothing else calls it — verified
--      against pg_proc). Its body named `has_system_access` (a call, kept) and
--      `kb_team_members`, never the column, but it is dead without the trigger.
--   3. Drop `kb_profiles.system_access`, then the `system_access` enum TYPE
--      (only that column used it — verified against pg_attribute).
--   4. Drop `kb_profiles.is_active` — breaks no SQL (no function/view/index reads
--      it; the two Rust readers were repointed onto standing in PR-A · A1).
--   5. Drop `kb_system_settings.access_mode` — no live SQL reader (the only
--      `pg_proc` body match is `has_system_access`, in a comment; its body reads
--      standing). Retired as a control in D18.
--
-- NOT DROPPED — `kb_join_requests.status`, its indexes, and the
-- `join_request_status` PG enum (decision D-B). The status column is the live
-- request-outcome audit (pending → approved/rejected/withdrawn) with readers
-- (`get_own_request` → CLI, `vw_join_requests`) that `standing='requested'` does
-- NOT replace — it only replaces the duplicate-guard half. Spec §11's premise
-- ("drop it + `idx_join_requests_one_pending`, standing='requested' covers it")
-- was superseded by the shipped lifecycle: the admin-event sink that would
-- replace the audit is a future deliverable. Dropping the column is deferred with
-- it, out of Phase 2's scope.
-- ============================================================================

DROP TRIGGER trg_sync_system_membership ON kb_profiles;
DROP FUNCTION sync_system_membership();

ALTER TABLE kb_profiles DROP COLUMN system_access;
DROP TYPE system_access;

ALTER TABLE kb_profiles DROP COLUMN is_active;

ALTER TABLE kb_system_settings DROP COLUMN access_mode;
