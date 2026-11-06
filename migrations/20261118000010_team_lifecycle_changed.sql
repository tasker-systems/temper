-- A team's lifecycle acts are written, and recorded, in one statement.
--
-- Team DAG and lifecycle doors D8 (temper-artifacts
-- `specs/2026-11-06-team-dag-and-lifecycle-doors-design.md`, ruling R5 as extended): creating a
-- team, soft-deleting it, restoring it, renaming its slug and changing its `auto_join_role` each
-- change who reaches what. A restore revives every preserved row's reach; a rename re-addresses every
-- `+team/...` ref; an auto-join change enrols, or stops enrolling, every approved profile. Until now
-- none of them was recorded (only the creator's `team_genesis` membership row was). This migration
-- adds the record and the one writer for all five acts:
--
--   * the admin event type `team_lifecycle_changed` (payload: team, act, actor, and `from`/`to`
--     strings for a rename's slugs and an auto-join change's roles). The team is the `subject`
--     reference, so its managers read it on the subject axis, like its membership and link records.
--   * `_admin_team_lifecycle_write`, which performs the row change for each act and appends the
--     event, in the shape of `_admin_team_parent_link_write` (20261109000010):
--       created            appends only -- `create_team` inserted the row in this transaction
--       deleted            is_active -> false (never `temper-system`)
--       restored           is_active -> true
--       renamed            slug -> p_to; when the team is the gating team, `gating_team_slug`
--                          follows it under `_gating_team_lock`, so the instance is never without one
--       auto_join_changed  auto_join_role -> p_to (NULL clears it)
--     It returns `changed`, `noop` (a rename or auto-join change to the value already held; nothing
--     written or emitted) or `absent` (no team in the state the act needs; nothing written or
--     emitted). `from` is read from the row, never taken from the caller.
--
-- WHAT THE FUNCTION DOES NOT DO: AUTHORIZE. Who may delete, restore, rename or set auto-join is
-- decided in Rust (`team_service`), before calling it, as its sibling's comment says of links.
--
-- The payload schema literal below is
-- `crates/temper-substrate/tests/fixtures/payloads/team_lifecycle_changed.v1.schema.json`, and
-- `the_migration_literal_matches_the_committed_fixture` pins the pair.
INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('team_lifecycle_changed', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "TeamLifecycleChanged",
  "description": "`team_lifecycle_changed` — a team was created, soft-deleted, restored, had its slug renamed, or had\nits `auto_join_role` changed (team DAG and lifecycle doors D8). Written only by\n`_admin_team_lifecycle_write`, in the same statement as the `kb_teams` change (for `created`,\nin the transaction that inserted the row).\n\nEach act changes who reaches what: a restore revives every preserved row's reach, a rename\nre-addresses every `+team/...` ref, an auto-join change enrols (or stops enrolling) every approved\nprofile. So each is an authority record like a parent link. The team is the event's `subject`\nreference; its managers read it on the subject axis.",
  "type": "object",
  "properties": {
    "act": {
      "$ref": "#/$defs/TeamLifecycleAct"
    },
    "actor": {
      "anyOf": [
        {
          "$ref": "#/$defs/ProfileId"
        },
        {
          "type": "null"
        }
      ]
    },
    "from": {
      "description": "The value before the act: the old slug on `renamed`, the old `auto_join_role` on\n`auto_join_changed` (absent when it was unset). Absent on the other acts.",
      "type": [
        "string",
        "null"
      ]
    },
    "team_id": {
      "type": "string",
      "format": "uuid"
    },
    "to": {
      "description": "The value after the act: the new slug on `renamed`, the new `auto_join_role` on\n`auto_join_changed` (absent when cleared). Absent on the other acts.",
      "type": [
        "string",
        "null"
      ]
    }
  },
  "required": [
    "team_id",
    "act"
  ],
  "$defs": {
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    },
    "TeamLifecycleAct": {
      "description": "What happened to the team in a [`TeamLifecycleChanged`] record.",
      "oneOf": [
        {
          "description": "The team was created (by `create_team`, in its insert transaction).",
          "type": "string",
          "const": "created"
        },
        {
          "description": "The team was soft-deleted (`is_active = false`); every row is preserved.",
          "type": "string",
          "const": "deleted"
        },
        {
          "description": "A soft-deleted team was made live again; every preserved row confers again.",
          "type": "string",
          "const": "restored"
        },
        {
          "description": "The team's slug changed; `+old/...` refs stop resolving.",
          "type": "string",
          "const": "renamed"
        },
        {
          "description": "The team's `auto_join_role` was set, changed or cleared.",
          "type": "string",
          "const": "auto_join_changed"
        }
      ]
    }
  }
}
$JS$::jsonb, 1, 'admin');

CREATE FUNCTION _admin_team_lifecycle_write(
    p_emitter     uuid,
    p_actor       uuid,
    p_team        uuid,
    p_act         text,
    p_to          text DEFAULT NULL,
    p_correlation uuid DEFAULT NULL
) RETURNS text
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_rows integer;
    v_from text;
    v_to   text;
BEGIN
    IF p_act = 'created' THEN
        IF NOT EXISTS (SELECT 1 FROM kb_teams WHERE id = p_team AND is_active) THEN
            RETURN 'absent';
        END IF;
    ELSIF p_act = 'deleted' THEN
        UPDATE kb_teams SET is_active = false
         WHERE id = p_team AND is_active AND slug <> 'temper-system';
        GET DIAGNOSTICS v_rows = ROW_COUNT;
        IF v_rows = 0 THEN
            RETURN 'absent';
        END IF;
    ELSIF p_act = 'restored' THEN
        UPDATE kb_teams SET is_active = true WHERE id = p_team AND NOT is_active;
        GET DIAGNOSTICS v_rows = ROW_COUNT;
        IF v_rows = 0 THEN
            RETURN 'absent';
        END IF;
    ELSIF p_act = 'renamed' THEN
        IF p_to IS NULL THEN
            RAISE EXCEPTION '_admin_team_lifecycle_write: renamed needs the new slug in p_to';
        END IF;
        -- Exclusive, before the row lock: a gating slug change and a personal-team write cannot each
        -- miss the other (20261025100000), and this may be the gating slug change.
        PERFORM _gating_team_lock(true);
        SELECT slug INTO v_from FROM kb_teams WHERE id = p_team AND is_active FOR UPDATE;
        IF NOT FOUND THEN
            RETURN 'absent';
        END IF;
        IF v_from = p_to THEN
            RETURN 'noop';
        END IF;
        UPDATE kb_teams SET slug = p_to WHERE id = p_team;
        UPDATE kb_system_settings SET gating_team_slug = p_to WHERE gating_team_slug = v_from;
        v_to := p_to;
    ELSIF p_act = 'auto_join_changed' THEN
        SELECT auto_join_role::text INTO v_from FROM kb_teams
         WHERE id = p_team AND is_active FOR NO KEY UPDATE;
        IF NOT FOUND THEN
            RETURN 'absent';
        END IF;
        IF v_from IS NOT DISTINCT FROM p_to THEN
            RETURN 'noop';
        END IF;
        UPDATE kb_teams SET auto_join_role = p_to::team_role WHERE id = p_team;
        v_to := p_to;
    ELSE
        RAISE EXCEPTION '_admin_team_lifecycle_write: p_act must be created, deleted, restored, '
                        'renamed or auto_join_changed, got %', p_act;
    END IF;

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
        p_correlation => p_correlation);

    RETURN 'changed';
END;
$$;

COMMENT ON FUNCTION _admin_team_lifecycle_write(uuid, uuid, uuid, text, text, uuid) IS
$c$Perform one team lifecycle act (created | deleted | restored | renamed | auto_join_changed) on
kb_teams and append its team_lifecycle_changed admin event, atomically. created appends only.
Returns changed | noop | absent; only changed writes or emits. It NEVER AUTHORIZES: team_service
decides who may, before calling it.$c$;

SELECT declare_migration(
    20261118000010,
    'additive',
    'Adds the team_lifecycle_changed admin event type and _admin_team_lifecycle_write, which '
    'performs a team create/delete/restore/rename/auto-join act on kb_teams and appends its ledger '
    'record in one statement. No existing function, table or column changes, so a binary without '
    'this change keeps working; the new binary routes create_team and delete_team through it. '
    'Design: temper-artifacts specs/2026-11-06-team-dag-and-lifecycle-doors-design.md D8.'
);
