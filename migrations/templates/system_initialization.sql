-- migrations/templates/system_initialization.sql
--
-- The irreducible SQL root step for a self-hosted instance: configure the gating team, then admit
-- and promote the first system admin. Nothing can authenticate an admin-gated command until the
-- first admin exists, so this one step runs directly against the database. Everything after it is
-- a surfaced `temper` command (see docs/playbooks/bootstrap-an-org.md).
--
-- Run it with psql, filling in the two variables. The first admin must have signed in once, so its
-- kb_profiles row exists:
--
--   psql "$DATABASE_URL" -X --set=ON_ERROR_STOP=1 \
--     --set=gating=temper-system --set=admin=<first-admin-profile-uuid> \
--     -f migrations/templates/system_initialization.sql
--
-- `scripts/bootstrap/system-bootstrap.sh --run-root` runs this file as-is, and
-- `crates/temper-services/tests/system_initialization_template_test.rs` runs it against the
-- migrated schema, so a schema change that breaks it fails CI rather than a self-hoster. The e2e
-- suites' bootstrap helpers are separate copies that this test does not cover.
--
-- Admission and governance are separate, and each reads exactly one table:
--   has_system_access  -> kb_principal_standing   (may this principal use the instance?)
--   is_system_admin    -> kb_principal_governance (may this principal change the rules?)
-- Neither reads team membership. Owning the gating team does not make anyone an admin.
--
-- Further admins are promoted by an existing admin, never by hand-written SQL:
--   temper admin promote <profile-uuid>

-- One transaction: admission and governance land together or not at all. Without it, a failure
-- in the second call would leave the profile admitted but not an admin.
BEGIN;

-- The slug before the team: setting it takes the gating-team lock (20261025100000) before the team's
-- slug exists, so a concurrent sign-up whose `personal-<handle>` is this slug waits and then skips it,
-- rather than deadlocking on the uncommitted team.
UPDATE kb_system_settings SET gating_team_slug = :'gating' WHERE id = 1;

INSERT INTO kb_teams (slug, name) VALUES (:'gating', :'gating')
  ON CONFLICT (slug) DO NOTHING;

-- The functions, not raw INSERTs: each records the transition and emits its event.
SELECT principal_standing_apply(:'admin'::uuid, 'approve', 'approved', NULL, 'root bootstrap');
SELECT principal_governance_set(:'admin'::uuid, true, NULL, 'root bootstrap');

COMMIT;

-- Both must read true. They are independent, and one without the other is a silent half-state.
SELECT has_system_access(:'admin'::uuid) AS admitted,
       is_system_admin(:'admin'::uuid)   AS governs;
