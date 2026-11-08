-- public_within: a hard-deleted team records what its removal did to every row it did not name.
--
-- 20261121000000's kb_teams_public_within_cascade records, as cascade_deleted, each row whose team or
-- anchor is the deleted team. A team between an anchor and its team names neither, yet deleting it
-- takes its kb_teams_parents rows with it (ON DELETE CASCADE), which can cut the anchor off from the
-- team (a structural lapse) or flip the row's effect: its forbid stops restricting, a pool it was
-- stops counting, a live path it carried is gone. No door hard-deletes a team; a manual or script
-- delete did all of that without a record.
--
--   kb_teams_public_within_cascade        BEFORE DELETE, per row: unchanged for the rows naming the
--                                         team; now also snapshots every other row the delete can
--                                         flip (_public_within_effects_before) into a
--                                         transaction-local setting.
--   kb_teams_public_within_hard_delete    AFTER DELETE, per statement, once the foreign-key cascade
--                                         has removed the links: runs _public_within_transitions over
--                                         that snapshot, so a lapse is deleted and recorded and every
--                                         flip is recorded, exactly as a soft delete's would be.
--
-- A later edit must keep the snapshot taken before any link is gone (BEFORE, per row) and the
-- transitions run after every link is gone (AFTER, per statement: the cascade's own triggers fire at
-- the end of each row's referential action, before the statement's AFTER triggers).

-- ── 1. Before: the rows naming the team, then the snapshot of every other reachable row ──────────
CREATE OR REPLACE FUNCTION kb_teams_public_within_cascade() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_emitter uuid;
    v_row     record;
    v_before  jsonb;
    v_pending jsonb;
BEGIN
    IF current_setting('temper.replaying', true) = 'on' THEN
        RETURN OLD;
    END IF;
    IF EXISTS (SELECT 1 FROM kb_team_public_within pw
                WHERE pw.team_id = OLD.id OR pw.anchor_id = OLD.id) THEN
        SELECT e.id INTO v_emitter
          FROM kb_entities e
          JOIN kb_profiles p ON p.id = e.profile_id
         WHERE p.handle = 'system' AND e.name = 'system';
        FOR v_row IN
            DELETE FROM kb_team_public_within pw
             WHERE pw.team_id = OLD.id OR pw.anchor_id = OLD.id
            RETURNING pw.team_id, pw.anchor_id, pw.admin_act, pw.instance_wide
        LOOP
            PERFORM _event_append(
                'team_public_within_changed', v_emitter, NULL, NULL,
                jsonb_build_object(
                    'team_id',       v_row.team_id,
                    'anchor_id',     v_row.anchor_id,
                    'act',           'cascade_deleted',
                    'admin_act',     v_row.admin_act,
                    'instance_wide', v_row.instance_wide),
                p_references => jsonb_build_array(
                    jsonb_build_object('rel', 'subject',
                                       'target', jsonb_build_object('kind', 'kb_teams', 'id', v_row.team_id)),
                    jsonb_build_object('rel', 'subject',
                                       'target', jsonb_build_object('kind', 'kb_teams', 'id', v_row.anchor_id))));
        END LOOP;
    END IF;

    v_before := _public_within_effects_before(ARRAY[OLD.id], ARRAY[OLD.id]);
    IF v_before <> '{}'::jsonb THEN
        v_pending := nullif(current_setting('temper.public_within_hard_delete', true), '')::jsonb;
        PERFORM set_config('temper.public_within_hard_delete',
                           (v_before || coalesce(v_pending, '{}'::jsonb))::text, true);
    END IF;
    RETURN OLD;
END;
$$;

COMMENT ON FUNCTION kb_teams_public_within_cascade() IS
$c$Before a kb_teams row is hard-deleted:
  * each public_within row naming it (as the team or the anchor) is deleted here and recorded as
    team_public_within_changed cascade_deleted, emitted by the system entity, with no actor: the
    foreign-key cascade would otherwise remove it unrecorded. Deleting the rows here records each
    one once even when one statement deletes both of its teams;
  * every other row the delete can flip is snapshotted with _public_within_effects_before (rows
    whose team is at or below the deleted team, or whose anchor is at or above it: a team's removal
    changes forbids above, pools below and paths through it, as a soft delete or an unlink does), and
    the snapshot is added to the transaction-local setting temper.public_within_hard_delete. The
    statement's AFTER trigger (kb_teams_public_within_hard_delete_transitions) computes the
    transitions once the links are gone. When one statement deletes several teams, the snapshot
    taken first for a row is kept: every row's snapshot is taken before any link is removed.
This takes the effects lock (the shared gating lock, then the effects key) after the deleted row's
own lock, as every act does. A statement deleting several teams locks each next team's row after
the key; a concurrent act holding that row and waiting on the key is then a deadlock that Postgres
breaks by aborting one side, which records nothing it did not do. No door hard-deletes a team today
(teams are soft-deleted, and erasure scrubs a personal team, which never holds a row); this keeps a
manual or script delete on the record. Skipped under replay (temper.replaying), which restores rows
as they stand.$c$;

-- ── 2. After: the transitions, once the cascade has removed the links ────────────────────────────
CREATE FUNCTION kb_teams_public_within_hard_delete_transitions() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_before jsonb;
BEGIN
    v_before := nullif(current_setting('temper.public_within_hard_delete', true), '')::jsonb;
    IF v_before IS NULL THEN
        RETURN NULL;
    END IF;
    PERFORM set_config('temper.public_within_hard_delete', '', true);
    PERFORM _public_within_transitions(v_before, NULL, uuid_generate_v7());
    RETURN NULL;
END;
$$;

COMMENT ON FUNCTION kb_teams_public_within_hard_delete_transitions() IS
$c$After a statement hard-deletes kb_teams rows, and after the foreign-key cascade has removed their
kb_teams_parents links: runs _public_within_transitions over the snapshot kb_teams_public_within_cascade
left in temper.public_within_hard_delete, then clears it. A row whose anchor is no longer above its
team by any path is deleted and recorded as lapsed; every other flipped row is recorded as
public_within_effect_changed (cause liveness or policy). There is no hard-delete event type, so each
record carries act NULL and a fresh correlation id that the records of this one statement share.
Nothing to do when no row was in reach (the setting is unset or empty).$c$;

CREATE TRIGGER kb_teams_public_within_hard_delete
    AFTER DELETE ON kb_teams
    FOR EACH STATEMENT EXECUTE FUNCTION kb_teams_public_within_hard_delete_transitions();

SELECT declare_migration(
    20261124000000,
    'additive',
    'public_within hard delete: body-only CREATE OR REPLACE of the kb_teams BEFORE DELETE trigger '
    'function kb_teams_public_within_cascade (signature unchanged) so it also snapshots the rows a '
    'hard delete can flip, and a new AFTER DELETE statement trigger on kb_teams that records their '
    'transitions. Only a hard delete of a team runs either, and no door hard-deletes a team, so an '
    'older binary is unaffected.'
);
