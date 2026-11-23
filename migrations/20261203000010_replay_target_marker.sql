-- The replay-target marker, and the one predicate every guard asks before it stands down for
-- replay.
--
-- The guards that keep erasure and the ledger honest (the resource write guard, the ledger's order
-- guard, the redaction projector's trail check, the gating and public_within guards) stand down
-- while the replay walk projects, because replay restores rows as the ledger holds them. The walk
-- signals that with the custom setting `temper.replaying`, which any session can set. So a setting
-- alone must never lower a guard: _replay_mode() is true only when the setting is 'on' AND this
-- database carries the kb_replay_target row.
--
-- LOAD-BEARING:
--   * The row can be written only while kb_events is empty. The trigger
--     kb_replay_target_requires_an_empty_ledger refuses any INSERT or UPDATE otherwise, for every
--     writer, after taking SHARE on kb_events. Under READ COMMITTED (the default) no append can
--     commit between its check and the write; under REPEATABLE READ a snapshot taken before the
--     lock can miss a concurrent first event, and replay_posture() then reports the marker at the
--     next deploy. Getting past the trigger takes an owner-class act, each acceptable and none a
--     product path: DDL on the table, session_replication_role = replica (privileged; it skips
--     ordinary triggers), or TRUNCATE kb_events, which empties the ledger and so admits a marker.
--     A replay target is always a reset, empty namespace, so replay::replay marks it first
--     (_replay_target_mark) and a replay pointed at a database holding a ledger fails before it
--     marks anything. replay::replay deletes the row once its walk succeeds, so a rebuilt database
--     carries no marker onward. Deleting the row is open to any writer: it only raises the guards.
--   * Deployed databases stay unmarked by that rule and that clear, and replay_posture() names a
--     marked one: temper-migrate refuses the deploy (exit 5); the API logs it at startup.
--   * _replay_mode() reads the setting first and the table only when it is 'on', and pins no
--     search_path (every name in it is schema-qualified), so the live path of a guard that fires on
--     every append pays no table read and no setting save.
--   * Guards call public._replay_mode(); none reads `temper.replaying` itself. A test holds pg_proc
--     to that (replay_mode_guard_test).
--   * Test fixtures that need the marker on a seeded ledger bypass the trigger by DDL, on purpose
--     (temper_substrate::test_schema::mark_replay_target). .github/scripts/audit-erasure-callers.sh
--     pins every literal write it can see; its header names what it cannot.

CREATE TABLE kb_replay_target (
    id        boolean     PRIMARY KEY DEFAULT true CHECK (id),
    marked_at timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE kb_replay_target IS
$c$At most one row, present only in a replay walk's scratch database: with temper.replaying 'on',
it is what lets a guard stand down (_replay_mode). kb_replay_target_requires_an_empty_ledger
refuses any write of it while kb_events holds a row; the replay walk marks its empty target first
and deletes the row once the walk succeeds. A deployed database never carries it
(replay_posture).$c$;

CREATE FUNCTION _replay_target_requires_an_empty_ledger()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    LOCK TABLE kb_events IN SHARE MODE;
    IF EXISTS (SELECT 1 FROM kb_events) THEN
        RAISE EXCEPTION
            'kb_replay_target: the ledger holds events; a replay target is an empty namespace'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

COMMENT ON FUNCTION _replay_target_requires_an_empty_ledger() IS
$c$BEFORE INSERT OR UPDATE on kb_replay_target: takes SHARE on kb_events, so under READ COMMITTED
no append can commit between the check and the write, and refuses (check_violation) while
kb_events holds any row. Binds every writer. Only owner-class acts get past it: DDL on the table,
session_replication_role = replica, or TRUNCATE kb_events first.$c$;

CREATE TRIGGER kb_replay_target_requires_an_empty_ledger
    BEFORE INSERT OR UPDATE ON kb_replay_target
    FOR EACH ROW EXECUTE FUNCTION _replay_target_requires_an_empty_ledger();

CREATE FUNCTION _replay_mode()
RETURNS boolean
LANGUAGE plpgsql
STABLE
AS $$
BEGIN
    IF pg_catalog.current_setting('temper.replaying', true) IS DISTINCT FROM 'on' THEN
        RETURN false;
    END IF;
    RETURN EXISTS (SELECT 1 FROM public.kb_replay_target);
END;
$$;

COMMENT ON FUNCTION _replay_mode() IS
$c$True only while the replay walk projects: temper.replaying is 'on' for this session or
transaction AND the database carries the kb_replay_target row. Every guard that stands down for
replay asks this, never the setting. Reads the table only when the setting is 'on'; pins no
search_path, its names being schema-qualified, so the live path saves and restores no setting.$c$;

CREATE FUNCTION _replay_target_mark()
RETURNS void
LANGUAGE sql
SET search_path = public, pg_temp
AS $$
    INSERT INTO kb_replay_target (id) VALUES (true) ON CONFLICT (id) DO NOTHING;
$$;

COMMENT ON FUNCTION _replay_target_mark() IS
$c$Marks this database a replay target (the kb_replay_target row).
kb_replay_target_requires_an_empty_ledger refuses it (check_violation) while kb_events holds any row, so it marks only a reset, empty
namespace. Called first by the replay walk; idempotent.$c$;

CREATE FUNCTION replay_posture()
RETURNS text[]
LANGUAGE sql
STABLE
SET search_path = public, pg_temp
AS $$
    SELECT coalesce(array_agg(v ORDER BY v), '{}')
      FROM (
        SELECT 'kb_replay_target carries the replay-target marker' AS v
         WHERE EXISTS (SELECT 1 FROM kb_replay_target)
        UNION ALL
        SELECT 'a database or role default sets temper.replaying'
         WHERE EXISTS (
            SELECT 1
              FROM pg_db_role_setting s
             WHERE s.setdatabase IN (0, (SELECT oid FROM pg_database
                                          WHERE datname = current_database()))
               AND EXISTS (SELECT 1 FROM unnest(s.setconfig) c
                            WHERE lower(c) LIKE 'temper.replaying=%'))
        UNION ALL
        SELECT 'this session carries temper.replaying = on'
         WHERE current_setting('temper.replaying', true) = 'on'
      ) violations;
$$;

COMMENT ON FUNCTION replay_posture() IS
$c$The reasons this database is unsafe to serve, empty when it is safe: the kb_replay_target
marker, a default naming temper.replaying for this database, for any role in it, or for a role
cluster-wide (whichever role the caller runs as), or the calling session carrying it 'on'
(called on a fresh connection outside a transaction, that is a session-level setting).
temper-migrate refuses a deploy on any; the API logs them at startup.$c$;

SELECT declare_migration(
    20261203000010,
    'additive',
    'The replay-target marker. New table kb_replay_target (singleton boolean key, marked_at), empty '
    'in every deployed database, with a BEFORE INSERT OR UPDATE trigger refusing a write while '
    'kb_events holds a row. New functions _replay_mode(), _replay_target_mark(), '
    'replay_posture() and the trigger function; no guard calls them yet (20261203000020 does). No '
    'existing table, column, constraint or function is altered.'
);
