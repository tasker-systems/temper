-- The guards that stand down for replay ask _replay_mode() (20261203000010) instead of reading
-- `temper.replaying` themselves, so the setting alone, in a database without the kb_replay_target
-- marker, lowers none of them.
--
-- Each function below is re-issued from its latest definition with two changes and no others: its
-- CREATE becomes CREATE OR REPLACE, and its read of the setting becomes a call to
-- public._replay_mode(), schema-qualified because two of them (_resource_write_guard,
-- kb_events_order_recorded) pin no search_path and must not resolve the helper through a caller's.
-- Every other line is that definition's, byte for byte. A later edit must keep both halves: the
-- body as it was, and no inline read of the setting
-- (replay_mode_guard_test holds pg_proc to the second).
--
-- Latest definitions re-issued:
--   _resource_write_guard                          20260929040730_resource_erasure_act
--   kb_events_order_recorded                       20261116000010_kb_events_record_order
--   _project_field_redactions                      20261202000020_edge_erasure_authority
--   kb_system_settings_gating_before
--     20261124000010_gating_slug_writes_record_their_transitions
--   kb_team_public_within_shape_guard              20261120000000_public_within_policy_and_helpers
--   kb_teams_public_within_cascade,
--   kb_teams_public_within_hard_delete_begin,
--   kb_teams_public_within_hard_delete_transitions
--     20261124000000_public_within_hard_delete_transitions
--
-- The COMMENTs that named the setting as the stand-down condition are re-issued to name
-- _replay_mode, among them two projectors that reach the trail check through
-- _project_field_redactions.

-- _resource_write_guard
CREATE OR REPLACE FUNCTION _resource_write_guard(p_resource uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_erased timestamptz;
BEGIN
    IF public._replay_mode() THEN
        RETURN;
    END IF;
    -- One statement, so a wait on the act's FOR UPDATE re-reads the row version the act
    -- committed (READ COMMITTED re-check) and sees its erased_at. No row → nothing to guard;
    -- the caller's own write reports a missing resource on its own terms.
    SELECT r.erased_at INTO v_erased FROM kb_resources r WHERE r.id = p_resource FOR KEY SHARE;
    IF v_erased IS NOT NULL THEN
        RAISE EXCEPTION 'resource % is erased; writes are refused', p_resource;
    END IF;
END;
$$;

-- kb_events_order_recorded
CREATE OR REPLACE FUNCTION kb_events_order_recorded() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF public._replay_mode() THEN
        RETURN NEW;
    END IF;
    IF NEW.xact_id IS NULL OR NEW.seq IS NULL THEN
        RAISE EXCEPTION 'kb_events: a new row must record xact_id and seq (R5)'
            USING ERRCODE = 'check_violation';
    END IF;
    IF NEW.xact_id <> pg_current_xact_id() THEN
        RAISE EXCEPTION 'kb_events: xact_id % is not the inserting transaction''s own id (R5)', NEW.xact_id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

-- _project_field_redactions
CREATE OR REPLACE FUNCTION _project_field_redactions(p_event uuid, p_payload jsonb, p_authority text)
RETURNS void
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_name  text := CASE p_authority
                        WHEN 'erasure' THEN '_project_resource_erased_redactions'
                        WHEN 'scrub'   THEN '_project_resource_scrubbed_redactions'
                        WHEN 'principal' THEN '_project_principal_erased_redactions'
                        WHEN 'edge'    THEN '_project_edge_erased_redactions'
                    END;
    v_entry jsonb;
    v_path  text;
    v_type  text;
    v_trail jsonb;
BEGIN
    IF v_name IS NULL THEN
        RAISE EXCEPTION '_project_field_redactions: event % names no known authority %', p_event, p_authority;
    END IF;
    -- The subject's trail, once: event id → true. For the person act (`principal`), its scope:
    -- the context events of the estate it recorded, never a resource's trail. For an edge
    -- (`edge`), the edge's own trail.
    IF p_authority = 'principal' THEN
        SELECT coalesce(jsonb_object_agg(ce.event_id::text, true), '{}'::jsonb)
          INTO v_trail
          FROM _principal_erasure_context_events(
                   (p_payload ->> 'subject_id')::uuid,
                   ARRAY(SELECT jsonb_array_elements_text(
                             coalesce(p_payload -> 'estate_contexts', '[]'::jsonb))::uuid)) ce
         WHERE jsonb_array_length(coalesce(p_payload -> 'redacted_fields', '[]'::jsonb)) > 0;
    ELSIF p_authority = 'edge' THEN
        SELECT coalesce(jsonb_object_agg(ts.event_id::text, true), '{}'::jsonb)
          INTO v_trail
          FROM _edge_erasure_trail_scope((p_payload ->> 'subject_id')::uuid) ts
         WHERE jsonb_array_length(coalesce(p_payload -> 'redacted_fields', '[]'::jsonb)) > 0;
    ELSE
        SELECT coalesce(jsonb_object_agg(ts.event_id::text, true), '{}'::jsonb)
          INTO v_trail
          FROM _resource_erasure_trail_scope((p_payload ->> 'subject_id')::uuid) ts
         WHERE jsonb_array_length(coalesce(p_payload -> 'redacted_fields', '[]'::jsonb)) > 0;
    END IF;
    FOR v_entry IN
        SELECT e FROM jsonb_array_elements(coalesce(p_payload -> 'redacted_fields', '[]'::jsonb)) e
    LOOP
        SELECT et.name INTO v_type
          FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE ev.id = (v_entry ->> 'event')::uuid;
        IF v_type IS NULL THEN
            RAISE EXCEPTION '%: event % names a missing event %',
                v_name, p_event, v_entry ->> 'event';
        END IF;
        -- The replay walk projects the record as the ledger holds it, and a D14 order inversion
        -- can put a trail event's own block or edge after the act: checking the trail there would
        -- abort the whole replay. Live, kb_events_redaction_in_trail checks it again at the
        -- rewrite, so this check is the earlier of two.
        IF NOT v_trail ? (v_entry ->> 'event')
           AND NOT public._replay_mode() THEN
            RAISE EXCEPTION '%: event % names event %, which is not in its subject''s trail',
                v_name, p_event, v_entry ->> 'event';
        END IF;
        FOR v_path IN SELECT jsonb_array_elements_text(v_entry -> 'paths') LOOP
            -- Each authority on its own allowlist: the person act names only the context paths of
            -- _principal_erasure_redact_paths, and the resource and edge acts never name one.
            IF NOT EXISTS (
                SELECT 1 FROM _erasure_redact_paths() r
                 WHERE p_authority <> 'principal'
                   AND r.path = v_path AND r.class <> 'keep'
                   AND (r.event_type = v_type
                        OR (r.event_type IS NULL AND v_path LIKE 'metadata.%'))
                UNION ALL
                SELECT 1 FROM _principal_erasure_redact_paths() r
                 WHERE p_authority = 'principal'
                   AND r.path = v_path AND r.event_type = v_type) THEN
                RAISE EXCEPTION '%: event % names path % of a % event, which the allowlist does not hold',
                    v_name, p_event, v_path, v_type;
            END IF;
            INSERT INTO kb_event_field_redactions (event_id, path, redacted_by, authority)
            VALUES ((v_entry ->> 'event')::uuid, v_path, p_event, p_authority);
        END LOOP;
    END LOOP;
END;
$$;

-- kb_system_settings_gating_before
CREATE OR REPLACE FUNCTION kb_system_settings_gating_before() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    PERFORM set_config('temper.public_within_gating_slug', '', true);
    IF NEW.gating_team_slug IS NOT DISTINCT FROM OLD.gating_team_slug
       OR current_setting('temper.gating_slug_recorded', true) = pg_current_xact_id()::text
       OR public._replay_mode() THEN
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

-- kb_team_public_within_shape_guard
CREATE OR REPLACE FUNCTION kb_team_public_within_shape_guard() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    IF public._replay_mode() THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'INSERT' AND NEW.opened_by IS NULL THEN
        RAISE EXCEPTION 'public_within: a row records who opened it'
            USING ERRCODE = 'check_violation', CONSTRAINT = 'kb_team_public_within_opened_by';
    END IF;
    IF EXISTS (SELECT 1 FROM kb_teams t WHERE t.id = NEW.team_id AND t.personal_of IS NOT NULL) THEN
        RAISE EXCEPTION 'public_within: the team is a personal team'
            USING ERRCODE = 'check_violation', CONSTRAINT = 'kb_team_public_within_team_personal';
    END IF;
    IF team_is_root_or_gating(NEW.team_id) THEN
        RAISE EXCEPTION 'public_within: the team is the root or the gating team'
            USING ERRCODE = 'check_violation', CONSTRAINT = 'kb_team_public_within_team_root_or_gating';
    END IF;
    IF EXISTS (SELECT 1 FROM kb_teams t WHERE t.id = NEW.anchor_id AND t.personal_of IS NOT NULL) THEN
        RAISE EXCEPTION 'public_within: the anchor is a personal team'
            USING ERRCODE = 'check_violation', CONSTRAINT = 'kb_team_public_within_anchor_personal';
    END IF;
    RETURN NEW;
END;
$$;

-- kb_teams_public_within_cascade
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
    IF public._replay_mode() THEN
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

-- kb_teams_public_within_hard_delete_begin
CREATE OR REPLACE FUNCTION kb_teams_public_within_hard_delete_begin() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    IF public._replay_mode() THEN
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

-- kb_teams_public_within_hard_delete_transitions
CREATE OR REPLACE FUNCTION kb_teams_public_within_hard_delete_transitions() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_stack jsonb;
    v_frame jsonb;
BEGIN
    IF public._replay_mode() THEN
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

COMMENT ON FUNCTION _project_field_redactions(uuid,jsonb,text) IS
$c$The one body of every redaction projection: one kb_event_field_redactions row per (event, path)
of p_payload's redacted_fields, under p_authority (`erasure`, `scrub`, `principal` or `edge`; any
other raises). Every named event must exist and, outside replay (_replay_mode), be in the
subject's trail: a resource's (_resource_erasure_trail_scope) for `erasure` and `scrub`, the
recorded estate's context events (_principal_erasure_context_events) for `principal`, an edge's
(_edge_erasure_trail_scope) for `edge`. Every path must be on its authority's allowlist:
_principal_erasure_redact_paths for `principal`, _erasure_redact_paths (never a `keep` path) for
the others. Called through the per-kind wrappers.$c$;

COMMENT ON FUNCTION _project_principal_erased_redactions(uuid,jsonb) IS
$c$The principal_erased redaction projection (R7): one kb_event_field_redactions row per (event, path)
of redacted_fields, authority `principal`. The same body as the resource acts'
(_project_field_redactions): every named event is a context event of the estate the record names
(skipped under replay, _replay_mode), and every path is on _principal_erasure_redact_paths for its
event's type. The act calls it after appending its record; replay calls it at the event's position.$c$;

COMMENT ON FUNCTION _project_resource_scrubbed_redactions(uuid,jsonb) IS
$c$The resource_scrubbed projection (field-grain scrub spec S6.4, S8): one kb_event_field_redactions
row per (event, path) of redacted_fields, authority `scrub`. The same checks as the erasure's
projector, by the same body (_project_field_redactions): every named event is in the subject's
trail (skipped under replay, _replay_mode), and every path is on the allowlist for its event's
type. Replay calls it at the event's position and applies nothing else.$c$;

COMMENT ON FUNCTION _resource_write_guard(uuid) IS
$c$the write guard (spec 2026-09-28 D13): FOR KEY SHARE on the kb_resources row, then RAISE
'resource % is erased; writes are refused' when erased_at IS NOT NULL. Conflicts only with the
erasure act's FOR UPDATE. Reads erased_at, never is_active, so replay of historical writes to a
soft-deleted resource never trips it. Returns at once under replay (_replay_mode:
temper.replaying 'on' in a database carrying the kb_replay_target row), as the replay walk projects: a lawful write
whose event committed before the act can sort after the act's events in walk order (event ids are
uuidv7, unordered across transactions within a millisecond on PG17), and replay must then diverge
(D14) rather than abort. The bypass is not an authorization boundary: the guard is a state floor
for the write paths, and a raw SQL session can write the tables directly with or without it.
The setting alone never bypasses it: without the kb_replay_target row the guard stays up, and
replay_posture names the row or a session or role default in a deployed database.$c$;

COMMENT ON FUNCTION kb_team_public_within_shape_guard() IS
$c$Refuses a public_within row whose team is personal, the root (temper-system) or the gating team,
or whose anchor is personal, and an insert that names no opener. The
guard runs on an insert and on a change of team or anchor, never on any other update (so the
opened_by SET NULL cascade and a later flag change pass), and not under replay (_replay_mode),
which restores rows as they stand: a row whose opener was since deleted, or whose team has since
become the gating team, is restored, not re-judged (the kb_events precedent). The constraint name on each refusal is
what Rust renders the reason from. The anchor may be the root or the gating team: that is an
admin's open, decided by the door, not here.$c$;

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
Skipped under replay (_replay_mode), which restores rows as they stand.$c$;

COMMENT ON FUNCTION kb_teams_public_within_hard_delete_begin() IS
$c$Before a DELETE statement on kb_teams runs: pushes one frame, {correlation, before}, onto the
transaction-local stack in temper.public_within_hard_delete. The statement's row triggers
(kb_teams_public_within_cascade) record under the frame's correlation id and add to its snapshot;
the statement's AFTER trigger (kb_teams_public_within_hard_delete_transitions) pops it. A stack, not
one slot, so a kb_teams statement nested inside another (from a trigger or a function) works on its
own frame and leaves the outer statement's snapshot whole. Skipped under replay (_replay_mode),
as both other triggers are.$c$;

SELECT declare_migration(
    20261203000020,
    'additive',
    'Re-issues eight trigger and guard functions (_resource_write_guard, kb_events_order_recorded, '
    '_project_field_redactions, kb_system_settings_gating_before, kb_team_public_within_shape_guard, '
    'kb_teams_public_within_cascade, kb_teams_public_within_hard_delete_begin, '
    'kb_teams_public_within_hard_delete_transitions) with signatures unchanged; each reads '
    '_replay_mode() where it read temper.replaying. Outside replay the guards behave exactly as '
    'before. Seven function COMMENTs re-issued. No table, column or constraint is altered.'
);
