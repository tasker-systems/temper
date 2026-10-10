-- A machine principal never governs: no team role above `member`, no governance grant.
--
-- A machine principal is a profile with any `kb_machine_clients` row, revoked or not — the same
-- definition the auto-join functions use (20261014100000_machines_never_auto_join.sql).
--
-- Until now the ceiling (`MAX_MACHINE_TEAM_ROLE = member`, D4b) was enforced in one Rust function,
-- `machine_authz::authorize_registration`, so it bounded only the reach a registration confers.
-- Every other way a role reaches a machine skipped it: `sync_personal_team` makes every profile
-- `owner` of its personal team, machines included; `team_service::add_member` / `change_role`
-- checked nothing about the target; `access_service::promote_admin` writes a gating-team `owner`
-- row and a governance grant for any profile. Ruled 2026-10-09: a machine holds its personal team
-- as `member` too, and never holds a role above `member` however the role came about.
--
-- Three rules, each in the database so no write path can route around it:
--
--   1. Becoming a machine caps a profile's roles. When a `kb_machine_clients` row lands, the
--      profile's `owner`/`maintainer` rows become `member`. This is what makes registration come
--      out right: the personal-team trigger fires on the profile insert, before the client row
--      exists, so at that moment the profile is not yet a machine. Capping here rather than
--      refusing keeps a binary that inserts in that order working. A governance grant on the
--      profile refuses the client row outright: registration always creates a fresh profile and
--      rebind adds a client row to a profile that is already a machine, so a governing profile
--      becoming a machine can only be a mistake.
--   2. A role above `member` is refused for a machine profile on any insert or update of
--      `kb_team_members`.
--   3. A governance grant is refused for a machine profile.
--
-- "Above member" is `role < 'member'`: Postgres orders an enum by declaration, and `team_role` is
-- declared highest-first (owner, maintainer, member, watcher, in
-- 20260624000001_canonical_schema.sql). A role added to the enum keeps these predicates right only
-- if it is declared in rank order, as `TeamRole::rank` must be extended in Rust.
--
-- The services refuse these cases first with a readable error; these triggers are the floor.

CREATE OR REPLACE FUNCTION is_machine_profile(p_profile uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (SELECT 1 FROM kb_machine_clients WHERE profile_id = p_profile);
$$;
COMMENT ON FUNCTION is_machine_profile(uuid) IS
  'Whether the profile is a machine principal: any kb_machine_clients row, revoked or not. A revoked '
  'credential ends the login, not what the profile is.';

CREATE OR REPLACE FUNCTION machine_team_role_ceiling()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.role < 'member'::team_role AND is_machine_profile(NEW.profile_id) THEN
        RAISE EXCEPTION 'a machine principal cannot hold the team role %', NEW.role
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_machine_team_role_ceiling
    BEFORE INSERT OR UPDATE OF role, profile_id ON kb_team_members
    FOR EACH ROW EXECUTE FUNCTION machine_team_role_ceiling();

CREATE OR REPLACE FUNCTION machine_never_governs()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF is_machine_profile(NEW.profile_id) THEN
        RAISE EXCEPTION 'a machine principal cannot hold a governance grant'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_machine_never_governs
    BEFORE INSERT OR UPDATE OF profile_id ON kb_principal_governance
    FOR EACH ROW EXECUTE FUNCTION machine_never_governs();

CREATE OR REPLACE FUNCTION machine_client_caps_profile()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM kb_principal_governance WHERE profile_id = NEW.profile_id) THEN
        RAISE EXCEPTION 'a profile holding a governance grant cannot become a machine principal'
            USING ERRCODE = 'check_violation';
    END IF;
    UPDATE kb_team_members
       SET role = 'member'
     WHERE profile_id = NEW.profile_id
       AND role < 'member'::team_role;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_machine_client_caps_profile
    AFTER INSERT OR UPDATE OF profile_id ON kb_machine_clients
    FOR EACH ROW EXECUTE FUNCTION machine_client_caps_profile();

-- Existing machines: personal-team `owner` rows (and anything else above `member`) become
-- `member`, and any governance grant is revoked through `principal_governance_set` so the
-- revocation is evented like every other. Before that, warn for each team (other than a machine's
-- own personal team) whose only owners are machines — lowering them leaves it ownerless, which no
-- API act can repair — and for a deployment whose only governance grants are held by machines.
DO $$
DECLARE r record;
BEGIN
    FOR r IN SELECT t.id, t.slug FROM kb_teams t
              WHERE (t.personal_of IS NULL OR NOT is_machine_profile(t.personal_of))
                AND EXISTS (SELECT 1 FROM kb_team_members tm
                             WHERE tm.team_id = t.id AND tm.role = 'owner')
                AND NOT EXISTS (SELECT 1 FROM kb_team_members tm
                                 WHERE tm.team_id = t.id AND tm.role = 'owner'
                                   AND NOT is_machine_profile(tm.profile_id))
    LOOP
        RAISE WARNING 'team % (%) is owned only by machine principals and is left without an owner',
            r.slug, r.id;
    END LOOP;
    IF EXISTS (SELECT 1 FROM kb_principal_governance)
       AND NOT EXISTS (SELECT 1 FROM kb_principal_governance g
                        WHERE NOT is_machine_profile(g.profile_id)) THEN
        RAISE WARNING 'every governance grant is held by a machine principal; revoking them leaves no system admin';
    END IF;
END $$;

UPDATE kb_team_members tm
   SET role = 'member'
 WHERE tm.role < 'member'::team_role
   AND is_machine_profile(tm.profile_id);

DO $$
DECLARE r record;
BEGIN
    FOR r IN SELECT g.profile_id FROM kb_principal_governance g
              WHERE is_machine_profile(g.profile_id)
    LOOP
        RAISE WARNING 'revoking a governance grant held by machine principal %', r.profile_id;
        PERFORM principal_governance_set(r.profile_id, false, NULL,
                                         'a machine principal never governs');
    END LOOP;
END $$;

SELECT declare_migration(
    20261017100000,
    'additive',
    'Adds is_machine_profile(uuid) and three triggers: kb_team_members refuses a role above member '
    'for a machine profile; kb_principal_governance refuses a machine profile; a kb_machine_clients '
    'insert caps the profile''s team roles at member (and refuses a profile holding governance). '
    'Lowers existing machine rows above member to member and revokes any machine governance grant. '
    'A binary without this change keeps working: registration inserts the profile, then the client '
    'row, and the cap applies on the second insert. The only writes newly refused are a role above '
    'member or a governance grant for a machine, which the new binary refuses first in Rust.'
);
