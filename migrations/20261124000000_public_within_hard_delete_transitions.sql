-- public_within: a hard-deleted team records what its removal did to every row it did not name.
--
-- 20261121000000's kb_teams_public_within_cascade records, as cascade_deleted, each row whose team or
-- anchor is the deleted team. A team between an anchor and its team names neither, yet deleting it
-- takes its kb_teams_parents rows with it (ON DELETE CASCADE), which can cut the anchor off from the
-- team (a structural lapse) or flip the row's effect: its forbid stops restricting, a pool it was
-- stops counting, a live path it carried is gone. No door hard-deletes a team; a manual or script
-- delete did all of that without a record.
--
--   kb_teams_public_within_hard_delete_begin   BEFORE DELETE, per statement: pushes this statement's
--                                              frame (a correlation id and an empty snapshot) onto a
--                                              transaction-local stack.
--   kb_teams_public_within_cascade             BEFORE DELETE, per row: takes the effects lock, records
--                                              the rows naming the team as before (now under the
--                                              frame's correlation id), and adds every other row the
--                                              delete can flip to the frame's snapshot.
--   kb_teams_public_within_hard_delete         AFTER DELETE, per statement, once the foreign-key
--                                              cascade has removed the links: pops its own frame and
--                                              runs _public_within_transitions over its snapshot, as a
--                                              soft delete's would be recorded.
--
-- A later edit must keep: the snapshot taken before any link is gone (BEFORE, per row) and the
-- transitions run after every link is gone (AFTER, per statement: the cascade's referential actions
-- fire before the statement's AFTER triggers); the effects lock taken before the trigger deletes any
-- public_within row (every act locks the key before it deletes one); and one frame per statement, so
-- a kb_teams statement nested inside another never consumes the outer statement's snapshot.

-- ── 1. A frame per statement ──────────────────────────────────────────────────────────────────────
CREATE FUNCTION kb_teams_public_within_hard_delete_begin() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    IF current_setting('temper.replaying', true) = 'on' THEN
        RETURN NULL;
    END IF;
    PERFORM set_config(
        'temper.public_within_hard_delete',
        (coalesce(nullif(current_setting('temper.public_within_hard_delete', true), '')::jsonb,
                  '[]'::jsonb)
         || jsonb_build_array(jsonb_build_object('correlation', uuid_generate_v7(),
                                                 'before', '{}'::jsonb)))::text,
        true);
    RETURN NULL;
END;
$$;

COMMENT ON FUNCTION kb_teams_public_within_hard_delete_begin() IS
$c$Before a DELETE statement on kb_teams runs: pushes one frame, {correlation, before}, onto the
transaction-local stack in temper.public_within_hard_delete. The statement's row triggers
(kb_teams_public_within_cascade) record under the frame's correlation id and add to its snapshot;
the statement's AFTER trigger (kb_teams_public_within_hard_delete_transitions) pops it. A stack, not
one slot, so a kb_teams statement nested inside another (from a trigger or a function) works on its
own frame and leaves the outer statement's snapshot whole. Skipped under replay (temper.replaying),
as both other triggers are.$c$;

CREATE TRIGGER kb_teams_public_within_hard_delete_begin
    BEFORE DELETE ON kb_teams
    FOR EACH STATEMENT EXECUTE FUNCTION kb_teams_public_within_hard_delete_begin();

-- ── 2. Per row: the lock, the rows naming the team, the snapshot of every other reachable row ────
CREATE OR REPLACE FUNCTION kb_teams_public_within_cascade() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_emitter     uuid;
    v_row         record;
    v_stack       jsonb;
    v_frame       jsonb;
    v_correlation uuid;
    v_before      jsonb;
BEGIN
    IF current_setting('temper.replaying', true) = 'on' THEN
        RETURN OLD;
    END IF;
    PERFORM _public_within_effects_lock();

    v_stack := coalesce(nullif(current_setting('temper.public_within_hard_delete', true), '')::jsonb,
                        '[]'::jsonb);
    v_frame := v_stack -> -1;
    IF v_frame IS NULL THEN
        v_frame := jsonb_build_object('correlation', uuid_generate_v7(), 'before', '{}'::jsonb);
        v_stack := jsonb_build_array(v_frame);
    END IF;
    v_correlation := (v_frame ->> 'correlation')::uuid;

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
                                       'target', jsonb_build_object('kind', 'kb_teams', 'id', v_row.anchor_id))),
                p_correlation => v_correlation);
        END LOOP;
    END IF;

    v_before := _public_within_effects_before(ARRAY[OLD.id], ARRAY[OLD.id]);
    IF v_before <> '{}'::jsonb THEN
        v_frame := jsonb_set(v_frame, '{before}', v_before || (v_frame -> 'before'));
        v_stack := jsonb_set(v_stack, '{-1}', v_frame);
        PERFORM set_config('temper.public_within_hard_delete', v_stack::text, true);
    END IF;
    RETURN OLD;
END;
$$;

COMMENT ON FUNCTION kb_teams_public_within_cascade() IS
$c$Before a kb_teams row is hard-deleted:
  * the effects lock (_public_within_effects_lock), first: every act takes it before it deletes a
    public_within row, so this trigger does too, or a hard delete holding a row an act is about to
    lapse would wait on the act's key while the act waits on the row;
  * each public_within row naming the team (as the team or the anchor) is deleted here and recorded
    as team_public_within_changed cascade_deleted, emitted by the system entity, with no actor, under
    the statement's correlation id: the foreign-key cascade would otherwise remove it unrecorded.
    Deleting the rows here records each one once even when one statement deletes both of its teams;
  * every other row the delete can flip is snapshotted with _public_within_effects_before (rows
    whose team is at or below the deleted team, or whose anchor is at or above it: a team's removal
    changes forbids above, pools below and paths through it, as a soft delete or an unlink does) and
    added to the statement's frame (kb_teams_public_within_hard_delete_begin). When one statement
    deletes several teams, the snapshot taken first for a row is kept: every row's snapshot is taken
    before any link is removed.
A statement deleting several teams locks each next team's row after the key; a concurrent act holding
that row and waiting on the key is then a deadlock that Postgres breaks by aborting one side, which
records nothing it did not do. No door hard-deletes a team today (teams are soft-deleted, and erasure
scrubs a personal team, which never holds a row); this keeps a manual or script delete on the record.
Skipped under replay (temper.replaying), which restores rows as they stand.$c$;

-- ── 3. After: the transitions, once the cascade has removed the links ────────────────────────────
CREATE FUNCTION kb_teams_public_within_hard_delete_transitions() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_stack jsonb;
    v_frame jsonb;
BEGIN
    IF current_setting('temper.replaying', true) = 'on' THEN
        RETURN NULL;
    END IF;
    v_stack := nullif(current_setting('temper.public_within_hard_delete', true), '')::jsonb;
    v_frame := v_stack -> -1;
    IF v_frame IS NULL THEN
        RETURN NULL;
    END IF;
    PERFORM set_config('temper.public_within_hard_delete', (v_stack - -1)::text, true);
    PERFORM _public_within_transitions(v_frame -> 'before', NULL,
                                       (v_frame ->> 'correlation')::uuid);
    RETURN NULL;
END;
$$;

COMMENT ON FUNCTION kb_teams_public_within_hard_delete_transitions() IS
$c$After a DELETE statement on kb_teams, and after the foreign-key cascade has removed the deleted
teams' kb_teams_parents links: pops this statement's frame from temper.public_within_hard_delete and
runs _public_within_transitions over its snapshot. A row whose anchor is no longer above its team is
deleted and recorded as lapsed; every other flipped row is recorded as public_within_effect_changed
(cause liveness or policy). There is no hard-delete event type, so each record carries act NULL and
the frame's correlation id, which the statement's cascade_deleted records share. Nothing to do when
the statement reached no row (an empty snapshot).

The setting is an ordinary custom setting: a session running raw SQL as the application role can
write it, as it can write kb_events itself. It carries no authority; it only says which rows to
recompute, and a row is deleted as lapsed only when its anchor is in fact no longer above its
team.$c$;

CREATE TRIGGER kb_teams_public_within_hard_delete
    AFTER DELETE ON kb_teams
    FOR EACH STATEMENT EXECUTE FUNCTION kb_teams_public_within_hard_delete_transitions();

SELECT declare_migration(
    20261124000000,
    'additive',
    'public_within hard delete: body-only CREATE OR REPLACE of the kb_teams BEFORE DELETE trigger '
    'function kb_teams_public_within_cascade (signature unchanged) so it takes the effects lock and '
    'snapshots the rows a hard delete can flip, and two new statement triggers on kb_teams (BEFORE '
    'DELETE pushes a frame, AFTER DELETE records the transitions). Only a hard delete of a team runs '
    'any of them, and no door hard-deletes a team, so an older binary is unaffected.'
);
