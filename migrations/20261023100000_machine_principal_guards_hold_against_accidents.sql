-- The machine-principal floor holds against accidental database writes, not only against the app.
--
-- 20261019100000_machines_never_govern put "a machine never governs" in the database. Everything
-- that migration enforces hangs on one fact: a profile is a machine when it has a
-- `kb_machine_clients` row, revoked or not (`is_machine_profile`). The Rust classification
-- (`classify_caller`) reads the same fact. This migration guards that floor against mistakes made
-- by someone writing SQL by hand: a cleanup query, a fix-up script, a stray schema. Direct database
-- access already sits outside temper's governance boundary; these guards make the easy mistakes
-- loud instead of silent. Four changes:
--
--   1. A client row is never deleted, and never moved to another profile. Either one makes the
--      profile a person again everywhere: the triggers below, the refusals in Rust, and
--      `HumanPrincipal` minting. Revocation (`revoked_at`) is how a machine's credential ends.
--      TRUNCATE is not guarded: it is a whole-table wipe, used by the test suites that reset
--      every `kb_` table, and is not a quiet reclassification of one profile.
--   2. The three triggers that check each other's tables take one per-profile transaction lock
--      before they read. Without it, a client row and a governance grant (or a role above member)
--      for the same profile, written by two concurrent transactions, can each pass a check that
--      cannot see the other's uncommitted row. With it, the second writer waits, and under READ
--      COMMITTED (the default) its check then reads the first writer's committed row. Under
--      REPEATABLE READ the second writer's snapshot predates that commit, so the lock alone does
--      not close it there; under SERIALIZABLE the read/write pair is a conflict Postgres aborts.
--      The ceiling takes the lock on every write of a role outside `member`/`watcher`, and
--      `machine_never_governs` on every governance write, a person's included, so those writes
--      serialize per profile. The cap takes the lock before row-locking the rows it lowers, while
--      an UPDATE of an existing row's role holds that row before the ceiling takes the lock. So a
--      hand-written UPDATE racing a client-row insert for the same profile can deadlock; Postgres
--      detects it and aborts one side.
--   3. "Above member" is an allowlist, not enum order. The ceiling admits exactly `member` and
--      `watcher` for a machine and refuses anything else, so a role added to `team_role` later is
--      refused until someone decides where it ranks. The cap lowers exactly `owner` and
--      `maintainer` to `member`, and raises if any other role outside the allowlist remains,
--      rather than quietly raising an unknown lower role to `member`.
--   4. Each of these functions pins `search_path`, so a table created earlier in a caller's path
--      cannot stand in for `kb_machine_clients` and make every profile read as a person.

CREATE OR REPLACE FUNCTION is_machine_profile(p_profile uuid)
RETURNS boolean LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT EXISTS (SELECT 1 FROM kb_machine_clients WHERE profile_id = p_profile);
$$;

-- One lock key per profile, shared by every trigger below. The `machine_principal:` prefix keeps
-- it out of the key space of the repository's other `hashtextextended` advisory locks.
CREATE OR REPLACE FUNCTION _machine_principal_lock(p_profile uuid)
RETURNS void LANGUAGE sql VOLATILE
SET search_path = public, pg_temp
AS $$
    SELECT pg_advisory_xact_lock(hashtextextended('machine_principal:' || p_profile::text, 0));
$$;
COMMENT ON FUNCTION _machine_principal_lock(uuid) IS
  'Serializes the machine-principal triggers for one profile until the transaction ends, so a '
  'check on one table cannot miss a concurrent write to another.';

CREATE OR REPLACE FUNCTION machine_team_role_ceiling()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    IF NEW.role NOT IN ('member', 'watcher') THEN
        PERFORM _machine_principal_lock(NEW.profile_id);
        IF is_machine_profile(NEW.profile_id) THEN
            RAISE EXCEPTION 'a machine principal cannot hold the team role %', NEW.role
                USING ERRCODE = 'check_violation';
        END IF;
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION machine_never_governs()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    PERFORM _machine_principal_lock(NEW.profile_id);
    IF is_machine_profile(NEW.profile_id) THEN
        RAISE EXCEPTION 'a machine principal cannot hold a governance grant'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION machine_client_caps_profile()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    PERFORM _machine_principal_lock(NEW.profile_id);
    IF EXISTS (SELECT 1 FROM kb_principal_governance WHERE profile_id = NEW.profile_id) THEN
        RAISE EXCEPTION 'a profile holding a governance grant cannot become a machine principal'
            USING ERRCODE = 'check_violation';
    END IF;
    UPDATE kb_team_members
       SET role = 'member'
     WHERE profile_id = NEW.profile_id
       AND role IN ('owner', 'maintainer');
    IF EXISTS (SELECT 1 FROM kb_team_members
                WHERE profile_id = NEW.profile_id
                  AND role NOT IN ('member', 'watcher')) THEN
        RAISE EXCEPTION 'a profile becoming a machine principal holds a team role this function does not know how to cap'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION machine_client_row_is_permanent()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'a machine client row is never deleted: it is what makes profile % a machine principal; revoke it instead',
            OLD.profile_id
            USING ERRCODE = 'check_violation';
    END IF;
    IF NEW.profile_id IS DISTINCT FROM OLD.profile_id THEN
        RAISE EXCEPTION 'a machine client row never moves to another profile: moving it would make profile % a person again',
            OLD.profile_id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_machine_client_row_is_permanent
    BEFORE DELETE OR UPDATE OF profile_id ON kb_machine_clients
    FOR EACH ROW EXECUTE FUNCTION machine_client_row_is_permanent();

SELECT declare_migration(
    20261023100000,
    'additive',
    'Guards the machine-principal floor against accidental SQL. A new trigger on kb_machine_clients '
    'refuses DELETE and any UPDATE that moves profile_id. A new helper, _machine_principal_lock(uuid), '
    'takes a per-profile transaction advisory lock. CREATE OR REPLACE, every signature unchanged, of '
    'is_machine_profile, machine_team_role_ceiling, machine_never_governs and '
    'machine_client_caps_profile: each pins search_path; the three triggers take the lock; the ceiling '
    'admits exactly member and watcher for a machine; the cap lowers exactly owner and maintainer and '
    'raises on any other role. A binary without this migration keeps working: no code path deletes a '
    'client row or moves its profile, and the roles admitted and refused are unchanged for every role '
    'that exists today.'
);
