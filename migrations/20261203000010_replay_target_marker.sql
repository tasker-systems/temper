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
--   * kb_replay_target is empty in every freshly migrated database, so every deployed database is
--     unmarked by construction. Its only writer is _replay_target_mark(), which replay::replay
--     calls first, and which refuses unless kb_events is empty: a replay target is always a reset,
--     empty namespace, so a replay pointed at a database holding a ledger fails before it marks it.
--     (Test fixtures that set the flag on a populated database insert the row through
--     temper_substrate::test_schema::mark_replay_target; .github/scripts/audit-erasure-callers.sh
--     pins every writer.)
--   * _replay_mode() reads the setting first and the table only when it is 'on', so the live path
--     of a guard that fires on every append pays no table read.
--   * Guards call _replay_mode(); none reads `temper.replaying` itself. A test holds pg_proc to
--     that (replay_mode_guard_test).
--
-- replay_posture() names what makes a database unsafe to serve: the marker, a database or role
-- default for the setting, or the calling session carrying it. temper-migrate refuses a deploy on
-- any of them; the API logs them at startup.

CREATE TABLE kb_replay_target (
    id        boolean     PRIMARY KEY DEFAULT true CHECK (id),
    marked_at timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE kb_replay_target IS
$c$At most one row, present only in a replay walk's scratch database: with temper.replaying 'on', it
is what lets a guard stand down (_replay_mode). Written only by _replay_target_mark, which refuses a
database whose ledger holds events. A deployed database never carries it (replay_posture).$c$;

CREATE FUNCTION _replay_mode()
RETURNS boolean
LANGUAGE plpgsql
STABLE
SET search_path = public, pg_temp
AS $$
BEGIN
    IF current_setting('temper.replaying', true) IS DISTINCT FROM 'on' THEN
        RETURN false;
    END IF;
    RETURN EXISTS (SELECT 1 FROM kb_replay_target);
END;
$$;

COMMENT ON FUNCTION _replay_mode() IS
$c$True only while the replay walk projects: temper.replaying is 'on' in this transaction AND the
database carries the kb_replay_target row. Every guard that stands down for replay asks this, never
the setting. Reads the table only when the setting is 'on'.$c$;

CREATE FUNCTION _replay_target_mark()
RETURNS void
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM kb_events) THEN
        RAISE EXCEPTION
            '_replay_target_mark: the ledger holds events; a replay target is an empty namespace'
            USING ERRCODE = 'check_violation';
    END IF;
    INSERT INTO kb_replay_target (id) VALUES (true) ON CONFLICT (id) DO NOTHING;
END;
$$;

COMMENT ON FUNCTION _replay_target_mark() IS
$c$Marks this database a replay target (the kb_replay_target row). Refuses (check_violation) when
kb_events holds any row, so it marks only a reset, empty namespace. Called first by the replay
walk; idempotent.$c$;

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
               AND s.setrole IN (0, (SELECT oid FROM pg_roles WHERE rolname = current_user))
               AND EXISTS (SELECT 1 FROM unnest(s.setconfig) c
                            WHERE c LIKE 'temper.replaying=%'))
        UNION ALL
        SELECT 'this session carries temper.replaying = on'
         WHERE current_setting('temper.replaying', true) = 'on'
      ) violations;
$$;

COMMENT ON FUNCTION replay_posture() IS
$c$The reasons this database is unsafe to serve, empty when it is safe: the kb_replay_target
marker, a database or role default naming temper.replaying, or the calling session carrying it
'on' (called on a fresh connection outside a transaction, that is a session-level setting).
temper-migrate refuses a deploy on any; the API logs them at startup.$c$;

SELECT declare_migration(
    20261203000010,
    'additive',
    'The replay-target marker. New table kb_replay_target (singleton boolean key, marked_at), empty '
    'in every deployed database. New functions _replay_mode(), _replay_target_mark() and '
    'replay_posture(); no guard calls them yet (20261203000020 does). No existing table, column, '
    'constraint or function is altered.'
);
