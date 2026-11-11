-- The gating team stays live: once kb_system_settings.gating_team_slug is set, every committed state
-- has a live team holding it, whatever wrote the change.
--
-- Before this, the lifecycle delete arm refused only the root, so a system admin could soft-delete
-- the gating team; _admin_gating_team_slug_write checked that a team held the slug, not that it was
-- live (a soft-deleted team's slug stays reserved); and any writer outside the doors (an older
-- binary during a rolling deploy, an operator's SQL, a fixture) could soft- or hard-delete the team,
-- move its slug without the settings, or point or clear the settings slug at nothing.
--
--   _gating_team_dangling()               the one check: the set slug, if no live team holds it.
--   gating_team_stays_live()              deferred constraint triggers on kb_teams (a live team dies,
--                                         changes slug or is deleted) and kb_system_settings (the slug
--                                         is written, or the row is deleted): run the check at commit,
--                                         and on the settings side refuse a set slug becoming unset.
--   _admin_team_lifecycle_act,            body-only CREATE OR REPLACE: the delete arm answers
--   _admin_gating_team_slug_write         {outcome: gating} for the gating team, the slug write
--                                         {outcome: dead_team} for a soft-deleted one, so the doors
--                                         refuse by name before the backstop is reached.
--
-- Deferred, because legitimate writers pass through the refused state inside one transaction: the
-- root step (templates/system_initialization.sql) sets the slug before inserting its team, and a
-- rename changes the team's slug before the settings slug. NULL is the birth state (the seed row);
-- once set, the slug is never unset.
--
-- A later edit must keep: both sides reading under _gating_team_lock(false), which every slug writer
-- holds exclusively to its commit (kb_system_settings_gating_lock_first), so a team-side commit and a
-- slug write serialize and neither misses the other; the team side taking it after its row locks
-- (the order of every team act) and the settings side taking no team row lock, so no cycle forms.

-- A deployment already in the refused state fails here, naming the slug.
DO $$
DECLARE
    v_slug text;
BEGIN
    SELECT s.gating_team_slug INTO v_slug
      FROM kb_system_settings s
     WHERE s.gating_team_slug IS NOT NULL
       AND NOT EXISTS (SELECT 1 FROM kb_teams t
                        WHERE t.slug = s.gating_team_slug AND t.is_active
                          AND s.gating_team_slug <> '');
    IF FOUND THEN
        RAISE EXCEPTION 'the gating slug % names no live team: restore that team, or point '
                        'kb_system_settings.gating_team_slug at a live shared team, before this '
                        'migration', quote_literal(v_slug)
            USING ERRCODE = 'check_violation';
    END IF;
END;
$$;

-- ── 1. The check ──────────────────────────────────────────────────────────────────────────────────
CREATE FUNCTION _gating_team_dangling() RETURNS text
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT s.gating_team_slug
      FROM kb_system_settings s
     WHERE s.id = 1
       AND s.gating_team_slug IS NOT NULL
       AND NOT EXISTS (SELECT 1 FROM kb_teams t
                        WHERE t.slug = s.gating_team_slug AND t.is_active
                          AND s.gating_team_slug <> '');
$$;

COMMENT ON FUNCTION _gating_team_dangling() IS
$c$The instance's gating slug when it is set and no live team holds it (an empty slug holds none),
else NULL. gating_team_stays_live refuses a commit at which this answers. Read it under
_gating_team_lock, or a concurrent slug write can be missed.$c$;

-- ── 2. The backstop, at commit ────────────────────────────────────────────────────────────────────
CREATE FUNCTION gating_team_stays_live() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_slug text;
BEGIN
    -- No replay skip: replay restores kb_teams into an unseeded namespace with no settings row, which
    -- passes, and a skip would be a switch any writer could set.
    -- After the team row locks this transaction already holds (the order of every team act); a slug
    -- writer holds the exclusive side to its commit, so what is read below is committed.
    PERFORM _gating_team_lock(false);

    -- Nested: PL/pgSQL resolves OLD's fields whether or not an AND before them is false.
    IF TG_TABLE_NAME = 'kb_system_settings' AND TG_OP <> 'INSERT' THEN
        IF OLD.gating_team_slug IS NOT NULL
           AND NOT EXISTS (SELECT 1 FROM kb_system_settings s
                            WHERE s.id = 1 AND s.gating_team_slug IS NOT NULL) THEN
            RAISE EXCEPTION 'the gating slug cannot be cleared once set; point it at another live team instead'
                USING ERRCODE = 'check_violation', CONSTRAINT = 'gating_team_stays_live';
        END IF;
    END IF;

    v_slug := _gating_team_dangling();
    IF v_slug IS NULL THEN
        RETURN NULL;
    END IF;
    IF TG_TABLE_NAME = 'kb_teams' THEN
        RAISE EXCEPTION 'the gating team % cannot be deleted or lose its slug; point the gating slug at another live team first',
            v_slug
            USING ERRCODE = 'check_violation', CONSTRAINT = 'gating_team_stays_live';
    END IF;
    RAISE EXCEPTION 'the gating slug % names no live team; restore that team or choose another live team',
        quote_literal(v_slug)
        USING ERRCODE = 'check_violation', CONSTRAINT = 'gating_team_stays_live';
END;
$$;

COMMENT ON FUNCTION gating_team_stays_live() IS
$c$At commit (DEFERRABLE INITIALLY DEFERRED constraint triggers on kb_teams and kb_system_settings):
refuses the transaction if the gating slug is set and no live team holds it, and, on the settings
side, if a set slug was cleared (NULL is the seed row's birth state only) or the row deleted. Reads
under the shared _gating_team_lock, after the team row locks the transaction holds. Not skipped
under replay (temper.replaying): it binds every writer. The doors refuse by name before this is reached; this is the refusal for
every other writer.$c$;

CREATE CONSTRAINT TRIGGER trg_gating_team_stays_live_team
    AFTER UPDATE OF is_active, slug ON kb_teams
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    WHEN (OLD.is_active AND (NOT NEW.is_active OR NEW.slug IS DISTINCT FROM OLD.slug))
    EXECUTE FUNCTION gating_team_stays_live();

CREATE CONSTRAINT TRIGGER trg_gating_team_stays_live_team_delete
    AFTER DELETE ON kb_teams
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    WHEN (OLD.is_active)
    EXECUTE FUNCTION gating_team_stays_live();

CREATE CONSTRAINT TRIGGER trg_gating_team_stays_live_settings
    AFTER INSERT OR UPDATE OF gating_team_slug OR DELETE ON kb_system_settings
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION gating_team_stays_live();

-- ── 3. The doors refuse by name ───────────────────────────────────────────────────────────────────
-- Bodies as 20261124000010 but for the marked lines.
CREATE OR REPLACE FUNCTION _admin_gating_team_slug_write(p_slug text, p_correlation uuid DEFAULT NULL)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_from        text;
    v_live        boolean;
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
    -- 20261124000050: a soft-deleted team keeps its slug, so existence is not enough.
    SELECT t.is_active INTO v_live FROM kb_teams t WHERE t.slug = p_slug;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('outcome', 'no_team');
    END IF;
    IF NOT v_live THEN
        RETURN jsonb_build_object('outcome', 'dead_team');
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
correlation_id, lapsed, changes}, or, with nothing written, {outcome: no_team} when no team holds the
slug once the lock is held (a rename committed since the caller's pre-check took it, and a slug
naming no team would let any manager become the gating team by renaming onto it) and {outcome:
dead_team} when the team holding it is soft-deleted (the instance is never without a live gating
team: gating_team_stays_live).

The gating team counts as a pool under any anchor above it, and can never be a row's team, so moving
it flips rows anchored above the old or the new gating team, and rows whose team is either; the
snapshot covers every anchor at or above either team.

It takes the exclusive gating lock before the settings row (the order a rename takes them in), so
the shared request inside _public_within_effects_before is its own; a rename and the team side of
gating_team_stays_live take the same lock, so the liveness re-check cannot be raced. It marks its
own settings write (temper.gating_slug_recorded set to the transaction id, cleared straight after) so
the settings row triggers do not record the same transitions again. The settings trigger
(gating_team_is_never_personal) still refuses a personal team. There is no settings event type, so
each effect record carries act NULL and only the correlation id. IT NEVER AUTHORIZES: only a system
admin reaches it (access_service::update_system_settings).$c$;

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
        SELECT slug INTO v_from FROM kb_teams
          WHERE id = p_team AND is_active AND slug <> 'temper-system'
            FOR NO KEY UPDATE;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('outcome', 'absent');
        END IF;
        -- 20261124000050: the gating team is never deleted. Read under the shared gating lock, after
        -- the row lock (the order of every team act), so a concurrent repoint is committed or waits.
        PERFORM _gating_team_lock(false);
        IF EXISTS (SELECT 1 FROM kb_system_settings WHERE id = 1 AND gating_team_slug = v_from) THEN
            RETURN jsonb_build_object('outcome', 'gating');
        END IF;
        v_from := NULL;
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
lapsed, changes}, or {outcome: gating} (nothing written) for a delete of the instance's gating team,
which is never deleted (gating_team_stays_live): repoint the gating slug first. Only changed writes or
emits. created appends only and changes no row's effect.

Each writing act locks its team row first (rename: the exclusive gating lock, then the row, as
before; delete: the row, then the shared gating lock for the gating check), decides absent, noop or
gating, then snapshots (_public_within_effects_before: anchors at or above the team), writes,
records, and runs _public_within_transitions. A delete or restore flips rows through liveness; an
auto-join change flips rows whose anchor's live subtree gains or loses a pool; a rename moves the
gating slug with it, so it flips nothing unless it changed which team is the root or the gating team
(the door refuses the root; renaming onto a gating slug that no team holds makes the team the gating
team, and those flips are recorded too). A rename marks its settings write
(temper.gating_slug_recorded set to the transaction id, cleared straight after): its own snapshot,
taken before the team row changed, is the record, and the settings row triggers would otherwise
snapshot the half-renamed state, in which no team holds the gating slug. The lifecycle record carries
p_correlation, or, when there is none, a fresh id that the transitions share, minted whether or not a
row was in reach, so the actor's own record says nothing about rows near the team. IT NEVER
AUTHORIZES.$c$;

SELECT declare_migration(
    20261124000050,
    'additive',
    'the gating team stays live: a new function _gating_team_dangling() and three DEFERRABLE INITIALLY '
    'DEFERRED constraint triggers (kb_teams update of is_active or slug on a live team, kb_teams delete, '
    'kb_system_settings insert, update of gating_team_slug or delete) that refuse at commit a state in '
    'which a set gating slug names no live team, or a set slug was cleared; body-only CREATE OR REPLACE '
    '(signatures unchanged) of _admin_gating_team_slug_write (answers dead_team for a soft-deleted team) '
    'and _admin_team_lifecycle_act (the delete arm answers gating for the gating team). Every other '
    'answer is unchanged. An older binary that deletes the gating team or points the slug at a dead team '
    'is refused (it renders the new answer or the commit error as an error), which is the point. Refuses '
    'to apply if the deployment is already in the refused state.'
);
