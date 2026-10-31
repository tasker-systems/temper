-- A child team's link to its parent is written, and recorded, in one statement.
--
-- A `kb_teams_parents` row is read reach: every member of the child reaches everything attached to
-- the parent and every ancestor above it. Until now the row was written by `create_team` with no
-- record of who made it, and no API removed one. Team-authority fixes D3
-- (temper-artifacts `specs/2026-10-30-team-authority-fixes-design.md`) keeps rank per team (a parent
-- role confers no membership authority on a child) and gives the parent's managers the two things
-- adding a member to the parent directly always had: they can see the child's roster, and they can
-- cut the link. This migration adds the record and the one writer for both acts:
--
--   * the admin event type `team_parent_link_changed` (payload: parent, child, `linked` or
--     `unlinked`, actor). The parent is the `subject` reference, so the parent's managers read it on
--     the subject axis; the child is a `child` reference.
--   * `_admin_team_parent_link_write`, which inserts or deletes the row and appends the event, in the
--     shape of `_admin_team_membership_write` (20261101200000).
--
-- WHAT THE FUNCTION DOES NOT DO: AUTHORIZE. Who may link (create a child) and who may unlink is
-- decided in Rust (`team_service::create_team`, `team_service::detach_child`). A link that already
-- exists answers `exists`, and an unlink of a link that is not there answers `absent`; neither writes
-- or emits.
--
-- The personal-team trigger's structural link under `temper-system` stays a derived, unrecorded
-- writer, like the membership row it writes beside it (spec C8, R13).
--
-- The payload schema literal below is
-- `crates/temper-substrate/tests/fixtures/payloads/team_parent_link_changed.v1.schema.json`, and
-- `the_migration_literal_matches_the_committed_fixture` pins the pair.
INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('team_parent_link_changed', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "TeamParentLinkChanged",
  "description": "`team_parent_link_changed` — a child team linked under, or detached from, a parent team\n(team-authority fixes D3). Written only by `_admin_team_parent_link_write`, in the same statement\nas the `kb_teams_parents` row.\n\nA link is read reach: every member of the child reaches everything attached to the parent and\nits ancestors. So who made a link, and who cut one, is an authority record like a membership\nchange. The parent is the event's `subject` reference and the child its `child` reference; the\nparent's managers read it on the subject axis. The personal-team trigger's structural link under\n`temper-system` is a derived writer and is not recorded.",
  "type": "object",
  "properties": {
    "act": {
      "$ref": "#/$defs/TeamParentLinkAct"
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
    "child_team_id": {
      "type": "string",
      "format": "uuid"
    },
    "parent_team_id": {
      "type": "string",
      "format": "uuid"
    }
  },
  "required": [
    "parent_team_id",
    "child_team_id",
    "act"
  ],
  "$defs": {
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    },
    "TeamParentLinkAct": {
      "description": "What happened to a [`TeamParentLinkChanged`] link.",
      "oneOf": [
        {
          "description": "The child was created under the parent.",
          "type": "string",
          "const": "linked"
        },
        {
          "description": "The parent's manager (or a system admin) detached the child.",
          "type": "string",
          "const": "unlinked"
        }
      ]
    }
  }
}
$JS$::jsonb, 1, 'admin');

CREATE FUNCTION _admin_team_parent_link_write(
    p_emitter     uuid,
    p_actor       uuid,
    p_act         text,
    p_parent      uuid,
    p_child       uuid,
    p_correlation uuid DEFAULT NULL
) RETURNS text
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_rows integer;
BEGIN
    IF p_act = 'link' THEN
        INSERT INTO kb_teams_parents (child_id, parent_id)
        VALUES (p_child, p_parent)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS v_rows = ROW_COUNT;
        IF v_rows = 0 THEN
            RETURN 'exists';
        END IF;
    ELSIF p_act = 'unlink' THEN
        DELETE FROM kb_teams_parents WHERE child_id = p_child AND parent_id = p_parent;
        GET DIAGNOSTICS v_rows = ROW_COUNT;
        IF v_rows = 0 THEN
            RETURN 'absent';
        END IF;
    ELSE
        RAISE EXCEPTION '_admin_team_parent_link_write: p_act must be link or unlink, got %', p_act;
    END IF;

    PERFORM _event_append(
        'team_parent_link_changed', p_emitter, NULL, NULL,
        jsonb_strip_nulls(jsonb_build_object(
            'parent_team_id', p_parent,
            'child_team_id',  p_child,
            'act',            CASE p_act WHEN 'link' THEN 'linked' ELSE 'unlinked' END,
            'actor',          p_actor)),
        p_references => jsonb_build_array(
            jsonb_build_object('rel', 'subject', 'target', jsonb_build_object('kind', 'kb_teams', 'id', p_parent)),
            jsonb_build_object('rel', 'child',   'target', jsonb_build_object('kind', 'kb_teams', 'id', p_child))),
        p_correlation => p_correlation);

    RETURN CASE p_act WHEN 'link' THEN 'linked' ELSE 'unlinked' END;
END;
$$;

COMMENT ON FUNCTION _admin_team_parent_link_write(uuid, uuid, text, uuid, uuid, uuid) IS
$c$Insert (p_act = 'link') or delete (p_act = 'unlink') one kb_teams_parents row and append its
team_parent_link_changed admin event, atomically. Returns linked | unlinked | exists | absent; only
the first two write or emit. It NEVER AUTHORIZES: team_service::create_team and
team_service::detach_child decide who may, before calling it.$c$;

SELECT declare_migration(
    20261109000010,
    'additive',
    'Adds the team_parent_link_changed admin event type and _admin_team_parent_link_write, which '
    'writes or deletes one kb_teams_parents row and its ledger record in one statement. Nothing '
    'existing calls it and no existing function, table or column changes, so a binary without this '
    'change keeps working; the new binary routes create_team''s parent link and the new detach '
    'through it. Design: temper-artifacts specs/2026-10-30-team-authority-fixes-design.md D3.'
);
