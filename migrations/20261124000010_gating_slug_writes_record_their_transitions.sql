-- The gating slug: every write of kb_system_settings.gating_team_slug takes the exclusive gating lock
-- before the settings row, and records what it did to public_within rows, whatever wrote it.
--
-- Two gaps, one place. (1) Lock order: the never-personal trigger (20261025100000) takes the
-- exclusive _gating_team_lock in a BEFORE ROW trigger, after UPDATE has locked the settings row,
-- while a gating-team rename takes the exclusive lock first and the settings row last; a plain
-- UPDATE racing a rename deadlocked (40P01). (2) Recording: 20261122000000 records a repoint's
-- public_within transitions only inside _admin_gating_team_slug_write, so any other writer (an older
-- binary's settings write during a rolling deploy, an operator's SQL) flipped effect unrecorded.
--
--   kb_system_settings_gating_lock_first     BEFORE UPDATE OF gating_team_slug, per statement: the
--                                            exclusive gating lock, before any row is locked.
--   kb_system_settings_gating_before /       BEFORE / AFTER, per row, when the slug changes and the
--   kb_system_settings_gating_after          writer has not recorded the change itself: snapshot,
--                                            then _public_within_transitions.
--   _admin_gating_team_slug_write,           body-only CREATE OR REPLACE: each marks its own settings
--   _admin_team_lifecycle_act                write (temper.gating_slug_recorded) so the row triggers
--                                            do not record it a second time, or, for a rename,
--                                            against the half-renamed state.
--
-- A later edit must keep the statement trigger BEFORE and per statement (a per-row trigger runs
-- after the row is locked, which is the inversion), and must keep every in-database writer that
-- records its own transitions marking its write.

-- ── 1. The lock, before the row ───────────────────────────────────────────────────────────────────
CREATE FUNCTION kb_system_settings_gating_lock_first() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    PERFORM _gating_team_lock(true);
    RETURN NULL;
END;
$$;

COMMENT ON FUNCTION kb_system_settings_gating_lock_first() IS
$c$Before any statement that names gating_team_slug in its SET list runs, and so before it locks the
settings row: the exclusive _gating_team_lock, held to the end of the transaction. Every writer of
the slug then takes the exclusive lock, then the settings row, then (recording) the shared request
and the effects key, which are its own: the order a gating-team rename and
_admin_gating_team_slug_write take them in. Without it a plain UPDATE took the row first and the
exclusive lock in the never-personal row trigger, and a concurrent rename (lock first, row last)
deadlocked with it. It fires whenever the statement names gating_team_slug in its SET list, even
when the value does not change (the settings door's write of other fields names it); the lock is
then held to the end of that transaction, on an admin-only write.$c$;

CREATE TRIGGER kb_system_settings_gating_lock_first
    BEFORE UPDATE OF gating_team_slug ON kb_system_settings
    FOR EACH STATEMENT EXECUTE FUNCTION kb_system_settings_gating_lock_first();

-- ── 2. The record, for a writer that does not make its own ────────────────────────────────────────
CREATE FUNCTION kb_system_settings_gating_before() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    PERFORM set_config('temper.public_within_gating_slug', '', true);
    IF NEW.gating_team_slug IS NOT DISTINCT FROM OLD.gating_team_slug
       OR current_setting('temper.gating_slug_recorded', true) = pg_current_xact_id()::text
       OR current_setting('temper.replaying', true) = 'on' THEN
        RETURN NEW;
    END IF;
    PERFORM set_config(
        'temper.public_within_gating_slug',
        _public_within_effects_before(
            '{}'::uuid[],
            ARRAY(SELECT t.id FROM kb_teams t
                   WHERE t.slug = OLD.gating_team_slug OR t.slug = NEW.gating_team_slug))::text,
        true);
    RETURN NEW;
END;
$$;

COMMENT ON FUNCTION kb_system_settings_gating_before() IS
$c$Before a row write that changes gating_team_slug and was not marked by its writer
(temper.gating_slug_recorded holding the current transaction id, set only around the writes of
_admin_gating_team_slug_write and of a rename in _admin_team_lifecycle_act, which record their own
transitions; a value set for a session or a role names no transaction and marks nothing): first
clears any snapshot left in temper.public_within_gating_slug, so the AFTER trigger only ever consumes
the one this row write took, then snapshots every row
the change can flip with _public_within_effects_before, over the teams holding the old and the new
slug (the same scope _admin_gating_team_slug_write uses), into the transaction-local setting
temper.public_within_gating_slug, for kb_system_settings_gating_after. The effects lock it takes
follows the settings row and the exclusive gating lock (kb_system_settings_gating_lock_first), the
order of every slug writer. Skipped under replay.

Hand-written SQL that renames the gating team and then moves the slug, outside
_admin_team_lifecycle_act, is snapshotted in the half-renamed state (no team holds the old slug) and
records flips that did not happen; rename through the function, which marks its write. A DELETE or
re-INSERT of the settings row is not covered (nothing in the product does either).$c$;

CREATE FUNCTION kb_system_settings_gating_after() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_before jsonb;
BEGIN
    v_before := nullif(current_setting('temper.public_within_gating_slug', true), '')::jsonb;
    IF v_before IS NULL THEN
        RETURN NULL;
    END IF;
    PERFORM set_config('temper.public_within_gating_slug', '', true);
    PERFORM _public_within_transitions(v_before, NULL, uuid_generate_v7());
    RETURN NULL;
END;
$$;

COMMENT ON FUNCTION kb_system_settings_gating_after() IS
$c$After a row write that kb_system_settings_gating_before snapshotted: runs
_public_within_transitions over the snapshot and clears it. Each flipped row is recorded as
public_within_effect_changed with act NULL (there is no settings event type, as for
_admin_gating_team_slug_write) and a fresh correlation id; the gating team is never a row's team or
on a path, so a repoint never lapses a row. This is what keeps a writer that bypasses
_admin_gating_team_slug_write (an older binary during a rolling deploy, an operator's SQL) on the
record.$c$;

CREATE TRIGGER kb_system_settings_gating_before
    BEFORE UPDATE OF gating_team_slug ON kb_system_settings
    FOR EACH ROW EXECUTE FUNCTION kb_system_settings_gating_before();

CREATE TRIGGER kb_system_settings_gating_after
    AFTER UPDATE OF gating_team_slug ON kb_system_settings
    FOR EACH ROW EXECUTE FUNCTION kb_system_settings_gating_after();

-- ── 3. The writers that record their own mark their write ────────────────────────────────────────
CREATE OR REPLACE FUNCTION _admin_gating_team_slug_write(p_slug text, p_correlation uuid DEFAULT NULL)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_from        text;
    v_before      jsonb;
    v_correlation uuid;
BEGIN
    PERFORM _gating_team_lock(true);
    SELECT gating_team_slug INTO v_from FROM kb_system_settings WHERE id = 1 FOR UPDATE;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('outcome', 'absent');
    END IF;
    IF v_from IS NOT DISTINCT FROM p_slug THEN
        RETURN jsonb_build_object('outcome', 'noop');
    END IF;
    IF NOT EXISTS (SELECT 1 FROM kb_teams t WHERE t.slug = p_slug) THEN
        RETURN jsonb_build_object('outcome', 'no_team');
    END IF;
    v_before := _public_within_effects_before(
        '{}'::uuid[],
        ARRAY(SELECT t.id FROM kb_teams t WHERE t.slug = v_from OR t.slug = p_slug));
    PERFORM set_config('temper.gating_slug_recorded', pg_current_xact_id()::text, true);
    UPDATE kb_system_settings SET gating_team_slug = p_slug, updated = now() WHERE id = 1;
    PERFORM set_config('temper.gating_slug_recorded', '', true);
    v_correlation := coalesce(p_correlation, uuid_generate_v7());
    RETURN jsonb_build_object('outcome', 'changed', 'correlation_id', v_correlation)
        || _public_within_transitions(v_before, NULL, v_correlation);
END;
$$;

COMMENT ON FUNCTION _admin_gating_team_slug_write(text, uuid) IS
$c$Repoint the instance's gating team (kb_system_settings.gating_team_slug) and record what it did
to every public_within row, atomically. Answers jsonb {outcome: changed | noop | absent,
correlation_id, lapsed, changes}, or {outcome: no_team} (nothing written) when no team holds the
slug once the lock is held: a rename committed since the caller's pre-check took it, and a slug
naming no team would let any manager become the gating team by renaming onto it. The gating team
counts as a pool under any anchor above it, and can never be a row's team, so moving it flips rows
anchored above the old or the new gating team, and rows whose team is either; the snapshot covers
every anchor at or above either team.

It takes the exclusive gating lock before the settings row (the order a rename takes them in), so
the shared request inside _public_within_effects_before is its own; a rename takes the same exclusive
lock, so the existence re-check cannot be raced. It marks its own settings write
(temper.gating_slug_recorded set to the transaction id, cleared straight after) so the settings row
triggers do not record the
same transitions again. The settings trigger (gating_team_is_never_personal) still refuses a personal
team. There is no settings event type, so each effect record carries act NULL and only the
correlation id. IT NEVER AUTHORIZES: only a system admin reaches it
(access_service::update_system_settings).$c$;

CREATE OR REPLACE FUNCTION _admin_team_lifecycle_act(
    p_emitter     uuid,
    p_actor       uuid,
    p_team        uuid,
    p_act         text,
    p_to          text DEFAULT NULL,
    p_correlation uuid DEFAULT NULL
) RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_from        text;
    v_to          text;
    v_before      jsonb := '{}'::jsonb;
    v_correlation uuid;
BEGIN
    IF p_act = 'created' THEN
        IF NOT EXISTS (SELECT 1 FROM kb_teams WHERE id = p_team AND is_active) THEN
            RETURN jsonb_build_object('outcome', 'absent');
        END IF;
    ELSIF p_act = 'deleted' THEN
        PERFORM 1 FROM kb_teams
          WHERE id = p_team AND is_active AND slug <> 'temper-system'
            FOR NO KEY UPDATE;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('outcome', 'absent');
        END IF;
        v_before := _public_within_effects_before('{}'::uuid[], ARRAY[p_team]);
        UPDATE kb_teams SET is_active = false WHERE id = p_team;
    ELSIF p_act = 'restored' THEN
        PERFORM 1 FROM kb_teams WHERE id = p_team AND NOT is_active FOR NO KEY UPDATE;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('outcome', 'absent');
        END IF;
        v_before := _public_within_effects_before('{}'::uuid[], ARRAY[p_team]);
        UPDATE kb_teams SET is_active = true WHERE id = p_team;
    ELSIF p_act = 'renamed' THEN
        IF p_to IS NULL THEN
            RAISE EXCEPTION '_admin_team_lifecycle_act: renamed needs the new slug in p_to';
        END IF;
        PERFORM _gating_team_lock(true);
        SELECT slug INTO v_from FROM kb_teams WHERE id = p_team AND is_active FOR UPDATE;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('outcome', 'absent');
        END IF;
        IF v_from = p_to THEN
            RETURN jsonb_build_object('outcome', 'noop');
        END IF;
        v_before := _public_within_effects_before('{}'::uuid[], ARRAY[p_team]);
        UPDATE kb_teams SET slug = p_to WHERE id = p_team;
        PERFORM set_config('temper.gating_slug_recorded', pg_current_xact_id()::text, true);
        UPDATE kb_system_settings SET gating_team_slug = p_to WHERE gating_team_slug = v_from;
        PERFORM set_config('temper.gating_slug_recorded', '', true);
        v_to := p_to;
    ELSIF p_act = 'auto_join_changed' THEN
        SELECT auto_join_role::text INTO v_from FROM kb_teams
         WHERE id = p_team AND is_active FOR NO KEY UPDATE;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('outcome', 'absent');
        END IF;
        IF v_from IS NOT DISTINCT FROM p_to THEN
            RETURN jsonb_build_object('outcome', 'noop');
        END IF;
        v_before := _public_within_effects_before('{}'::uuid[], ARRAY[p_team]);
        UPDATE kb_teams SET auto_join_role = p_to::team_role WHERE id = p_team;
        v_to := p_to;
    ELSE
        RAISE EXCEPTION '_admin_team_lifecycle_act: p_act must be created, deleted, restored, '
                        'renamed or auto_join_changed, got %', p_act;
    END IF;

    v_correlation := coalesce(p_correlation, uuid_generate_v7());
    PERFORM _event_append(
        'team_lifecycle_changed', p_emitter, NULL, NULL,
        jsonb_strip_nulls(jsonb_build_object(
            'team_id', p_team,
            'act',     p_act,
            'actor',   p_actor,
            'from',    v_from,
            'to',      v_to)),
        p_references => jsonb_build_array(
            jsonb_build_object('rel', 'subject', 'target', jsonb_build_object('kind', 'kb_teams', 'id', p_team))),
        p_correlation => v_correlation);

    RETURN jsonb_build_object('outcome', 'changed', 'correlation_id', v_correlation)
        || _public_within_transitions(v_before, 'team_lifecycle_changed', v_correlation);
END;
$$;

COMMENT ON FUNCTION _admin_team_lifecycle_act(uuid, uuid, uuid, text, text, uuid) IS
$c$Perform one team lifecycle act (created | deleted | restored | renamed | auto_join_changed) on
kb_teams, append its team_lifecycle_changed admin event, and record what it did to every
public_within row, atomically. Answers jsonb {outcome: changed | noop | absent, correlation_id,
lapsed, changes}; only changed writes or emits. created appends only and changes no row's effect.

Each writing act locks its team row first (rename: the exclusive gating lock, then the row, as
before), decides absent or noop, then snapshots (_public_within_effects_before: anchors at or above
the team), writes, records, and runs _public_within_transitions. A delete or restore flips rows
through liveness; an auto-join change flips rows whose anchor's live subtree gains or loses a pool;
a rename moves the gating slug with it, so it flips nothing unless it changed which team is the root
or the gating team (the door refuses the root; renaming onto a gating slug that no team holds makes
the team the gating team, and those flips are recorded too). A rename marks its settings write
(temper.gating_slug_recorded set to the transaction id, cleared straight after): its own snapshot, taken before the team row
changed, is the record, and the settings row triggers would otherwise snapshot the half-renamed
state, in which no team holds the gating slug. The lifecycle record carries p_correlation, or, when
there is none, a fresh id that the transitions share, minted whether or not a row was in reach, so
the actor's own record says nothing about rows near the team. IT NEVER AUTHORIZES.$c$;

SELECT declare_migration(
    20261124000010,
    'additive',
    'gating slug writes: a BEFORE UPDATE OF gating_team_slug statement trigger on kb_system_settings '
    'that takes the exclusive gating lock before the row, BEFORE/AFTER row triggers that record the '
    'public_within transitions of a slug change its writer did not record, and body-only CREATE OR '
    'REPLACE (signatures and answers unchanged) of _admin_gating_team_slug_write and '
    '_admin_team_lifecycle_act, which mark their own settings write. An older binary''s settings '
    'write keeps its answer, now serializes with a rename instead of deadlocking, and is recorded.'
);
