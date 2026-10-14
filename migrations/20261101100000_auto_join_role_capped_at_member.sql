-- No team auto-enrolls anyone above `member` (team-role rank spec C15).
--
-- `kb_teams.auto_join_role` is a plain `team_role` (20260629000002), with no CHECK. The only gate
-- on it was the system-admin check in `team_service::create_team`, so an auto-join team could be
-- configured at `owner` or `maintainer`, and `ensure_auto_join_memberships`,
-- `backfill_auto_join_team` and `auto_join_reconcile` would then make every approved person a
-- governor of that team. A role that governs a team is granted by a person, never by enrollment.
--
-- Two steps, in this order:
--
--   1. Data. Any team whose `auto_join_role` is above `member` is lowered to `member`, with a
--      warning naming the team and the value it held (the 20261019100000_machines_never_govern.sql
--      pattern), so an operator reading the migration log can see what changed.
--   2. Constraint. `auto_join_role` is NULL, `member` or `watcher` from now on. The service
--      refuses a higher value first with a 400; this CHECK is the floor.
--
-- MEMBERSHIPS THAT AUTO-JOIN ALREADY WROTE ABOVE `member` ARE NOT TOUCHED. Such a row is
-- indistinguishable from one a person granted deliberately, so rewriting it would be an automated
-- guess about intent. Any that exist are an operator review, not a migration (spec C15).

DO $$
DECLARE r record;
BEGIN
    FOR r IN SELECT t.id, t.slug, t.auto_join_role FROM kb_teams t
              WHERE t.auto_join_role IN ('owner', 'maintainer')
    LOOP
        RAISE WARNING 'team % (%) auto-joined at %; lowered to member', r.slug, r.id, r.auto_join_role;
    END LOOP;
END $$;

UPDATE kb_teams
   SET auto_join_role = 'member'
 WHERE auto_join_role IN ('owner', 'maintainer');

ALTER TABLE kb_teams
    ADD CONSTRAINT kb_teams_auto_join_role_at_most_member
    CHECK (auto_join_role IS NULL OR auto_join_role IN ('member', 'watcher'));

COMMENT ON CONSTRAINT kb_teams_auto_join_role_at_most_member ON kb_teams IS
  'No team auto-enrolls anyone above member (team-role rank spec C15): a role that governs a team '
  'is granted by a person. team_service::create_team refuses a higher value with a 400 first.';

SELECT declare_migration(
    20261101100000,
    'additive',
    'Adds a CHECK holding kb_teams.auto_join_role to NULL, member or watcher, after lowering any '
    'existing owner or maintainer value to member with a warning. Memberships already written are '
    'untouched. A binary without this change keeps working: the only write newly refused is an '
    'auto_join_role above member, which only a system admin could set and which the new binary '
    'refuses first with a 400.'
);
