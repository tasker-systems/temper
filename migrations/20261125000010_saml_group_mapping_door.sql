-- A SAML group mapping is made and removed by a system admin, as a recorded act, at most at
-- `maintainer` unless the admin confirms more.
--
-- `kb_saml_group_mappings` (20260702000001) maps an IdP's asserted group to a (team, role). Every
-- login's reconcile turns the mappings into `source = 'idp'` memberships, so a mapping confers a role
-- on everyone the IdP puts in the group. Until now the only writer was SQL an operator applied with
-- psql: nothing capped the role, nothing set the root or gating team apart, nothing removed a
-- mapping, and nothing recorded either act. This migration adds:
--
--   * the admin event type `saml_group_mapping_changed` (payload: act, actor, idp_key, group_value,
--     team_id, role, confirmed). The team is the `subject` reference. No ledger arm reads it for a
--     team manager, so it is admin-only by the ledger's fail-closed default.
--   * `_admin_saml_group_mapping_write`, the one writer for both acts. It performs the row change and
--     appends the event in one statement, and answers jsonb `{outcome, ...}`:
--       created  an existing row is answered first (noop for the same role, conflict for another), so
--                an identical re-run never needs the confirmation.
--                needs_confirmation  role `owner`, or the root or gating team, without p_confirmed;
--                                    `needs` names which. Nothing written.
--                absent_idp / absent_team / personal_team   nothing written.
--                conflict            the key exists with another role (`role` names it). Nothing written.
--                noop                the key exists with this role. Nothing written or emitted.
--                changed             `confirmed` is true when the act needed, and had, confirmation.
--       removed  absent | changed (`role` as it was; `idp_memberships` counts the team's
--                `source = 'idp'` rows, which the removal leaves in place).
--
-- EXISTING ROWS ABOVE THE CEILING REFUSE THE MIGRATION (Pete, 2026-11-11). Every mapping the doors
-- did not write was written without confirmation, so the DO block below fails the migration, naming
-- each mapping that is `owner` or targets temper-system or the gating team; the operator removes the
-- row or lowers its role first. No CHECK and no trigger is added to the table: the ceiling binds this
-- writer, which the doors call, and a direct INSERT still bypasses it, as direct database access
-- bypasses every door. The reconcile reads the table as before.
--
-- A later edit must keep the create arm's lock order: `_gating_team_lock(false)` BEFORE the team row
-- (`FOR SHARE`). A rename takes the gating lock exclusively and then the team row, so taking the row
-- first would wait in a cycle with it. The row lock holds a soft delete off until commit. This is the
-- reverse of the delete arm's order (row, then the shared gating lock); shared against shared never
-- conflicts, so the two cannot deadlock, but with an exclusive gating request queued between them
-- the delete's shared request waits behind it and the deadlock detector reorders the queue after
-- deadlock_timeout: a stall of about a second, not an error.
--
-- WHAT THE FUNCTION DOES NOT DO: AUTHORIZE. `saml_mapping_service` asks for the system-admin proof
-- and re-asks it under the governance lock, before calling it. A later parameter goes on the end
-- with a default; adding one replaces the signature, so the old one is dropped in that migration.
--
-- The payload schema literal below is
-- `crates/temper-substrate/tests/fixtures/payloads/saml_group_mapping_changed.v1.schema.json`, and
-- `the_migration_literal_matches_the_committed_fixture` pins the pair.

-- Section 0. Refuse over a mapping above the ceiling (Pete, 2026-11-11).
DO $$
DECLARE
    v_found text;
BEGIN
    SELECT string_agg(format('%s/%s -> %s (%s)', m.idp_key, m.group_value, t.slug, m.role), ', '
                      ORDER BY m.idp_key, m.group_value, t.slug)
      INTO v_found
      FROM kb_saml_group_mappings m
      JOIN kb_teams t ON t.id = m.team_id
     WHERE m.role = 'owner'
        OR t.slug = 'temper-system'
        OR t.slug = (SELECT s.gating_team_slug FROM kb_system_settings s WHERE s.id = 1);
    IF v_found IS NOT NULL THEN
        RAISE EXCEPTION 'SAML group mappings above the maintainer ceiling or onto an instance team, '
                        'none of them confirmed (%). Before this migration: an `owner` row on an '
                        'ordinary team can be lowered to maintainer or below, or deleted; a row onto '
                        'temper-system or the gating team must be deleted (lowering its role does not '
                        'clear it). Once this migration has applied, re-create any that should stand '
                        'with `temper admin saml map-group --confirm-elevated`. In between, a login''s '
                        'IdP reconcile may revoke the idp-sourced memberships those rows conferred.', v_found
            USING ERRCODE = 'check_violation';
    END IF;
END;
$$;

-- Section 1. The record and its writer.
INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('saml_group_mapping_changed', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "SamlGroupMappingChanged",
  "description": "`saml_group_mapping_changed` — a SAML group mapping (`kb_saml_group_mappings`: an IdP's group\nvalue conferring a role on a team) was created or removed by a system admin. Written only by\n`_admin_saml_group_mapping_write`, in the same statement as the row.\n\nA mapping confers membership on every person the IdP asserts the group for, at each login's\nreconcile, so who made one, and who removed one, is an authority record like a membership\nchange. The team is the event's `subject` reference. Removing a mapping deletes no membership:\nthe next reconcile of each member decides those.",
  "type": "object",
  "properties": {
    "act": {
      "$ref": "#/$defs/SamlGroupMappingAct"
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
    "confirmed": {
      "description": "True on a `created` that needed the admin's explicit confirmation (an `owner` role, or the\nroot or gating team as the target) and had it. Absent otherwise.",
      "type": "boolean"
    },
    "group_value": {
      "description": "The asserted group value the mapping matches.",
      "type": "string"
    },
    "idp_key": {
      "description": "The IdP whose assertions the mapping reads (`kb_saml_idp.idp_key`).",
      "type": "string"
    },
    "role": {
      "description": "The role the mapping confers: as written on `created`, as it was on `removed`.",
      "$ref": "#/$defs/TeamRole"
    },
    "team_id": {
      "type": "string",
      "format": "uuid"
    }
  },
  "required": [
    "act",
    "idp_key",
    "group_value",
    "team_id",
    "role"
  ],
  "$defs": {
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    },
    "SamlGroupMappingAct": {
      "description": "What happened to the mapping in a [`SamlGroupMappingChanged`] record.",
      "oneOf": [
        {
          "description": "The mapping was created.",
          "type": "string",
          "const": "created"
        },
        {
          "description": "The mapping was removed; the memberships it conferred were left in place.",
          "type": "string",
          "const": "removed"
        }
      ]
    },
    "TeamRole": {
      "description": "Team role — strict hierarchy: Owner > Maintainer > Member > Watcher.\n\nMaps directly to the `team_role` Postgres enum. Four roles is small enough\nthat explicit matching in SQL functions and Rust logic is clearer than a\njoin-table permission model.",
      "type": "string",
      "enum": [
        "owner",
        "maintainer",
        "member",
        "watcher"
      ]
    }
  }
}
$JS$::jsonb, 1, 'admin');

CREATE FUNCTION _admin_saml_group_mapping_write(
    p_emitter     uuid,
    p_actor       uuid,
    p_act         text,
    p_idp_key     text,
    p_group       text,
    p_team        uuid,
    p_role        team_role DEFAULT NULL,
    p_confirmed   boolean   DEFAULT false,
    p_correlation uuid      DEFAULT NULL
) RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_slug     text;
    v_personal uuid;
    v_held     team_role;
    v_needs    text[] := '{}';
    v_members  bigint;
    v_try      integer;
BEGIN
    IF p_act = 'created' THEN
        IF p_role IS NULL THEN
            RAISE EXCEPTION '_admin_saml_group_mapping_write: created needs p_role';
        END IF;
        -- An inactive IdP is accepted: an operator may stage mappings before activating it. KEY
        -- SHARE holds its row against a concurrent delete, which would otherwise surface as an FK
        -- violation.
        PERFORM 1 FROM kb_saml_idp WHERE idp_key = p_idp_key FOR KEY SHARE;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('outcome', 'absent_idp');
        END IF;
        PERFORM _gating_team_lock(false);
        SELECT slug, personal_of INTO v_slug, v_personal
          FROM kb_teams WHERE id = p_team AND is_active FOR SHARE;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('outcome', 'absent_team');
        END IF;
        IF v_personal IS NOT NULL THEN
            RETURN jsonb_build_object('outcome', 'personal_team');
        END IF;
        IF p_role = 'owner' THEN
            v_needs := v_needs || 'owner_role'::text;
        END IF;
        IF v_slug = 'temper-system'
           OR EXISTS (SELECT 1 FROM kb_system_settings WHERE id = 1 AND gating_team_slug = v_slug) THEN
            v_needs := v_needs || 'instance_team'::text;
        END IF;
        -- An existing row is answered before the confirmation is asked, so re-running an identical
        -- mapping is a no-op even without it (the bootstrap script re-runs every mapping).
        SELECT role INTO v_held FROM kb_saml_group_mappings
         WHERE idp_key = p_idp_key AND group_value = p_group AND team_id = p_team;
        IF v_held = p_role THEN
            RETURN jsonb_build_object('outcome', 'noop', 'role', v_held);
        ELSIF v_held IS NOT NULL THEN
            RETURN jsonb_build_object('outcome', 'conflict', 'role', v_held);
        END IF;
        IF cardinality(v_needs) > 0 AND NOT p_confirmed THEN
            RETURN jsonb_build_object('outcome', 'needs_confirmation', 'needs', to_jsonb(v_needs));
        END IF;

        -- A conflicting row a concurrent removal deletes before it is read is retried once, so a
        -- conflict always names the role actually held.
        FOR v_try IN 1..2 LOOP
            INSERT INTO kb_saml_group_mappings (idp_key, group_value, team_id, role)
            VALUES (p_idp_key, p_group, p_team, p_role)
            ON CONFLICT (idp_key, group_value, team_id) DO NOTHING;
            EXIT WHEN FOUND;
            SELECT role INTO v_held FROM kb_saml_group_mappings
             WHERE idp_key = p_idp_key AND group_value = p_group AND team_id = p_team;
            IF v_held = p_role THEN
                RETURN jsonb_build_object('outcome', 'noop', 'role', v_held);
            ELSIF v_held IS NOT NULL THEN
                RETURN jsonb_build_object('outcome', 'conflict', 'role', v_held);
            ELSIF v_try = 2 THEN
                RAISE EXCEPTION '_admin_saml_group_mapping_write: the mapping kept vanishing under a create'
                    USING ERRCODE = 'serialization_failure';
            END IF;
        END LOOP;
        v_held := p_role;
    ELSIF p_act = 'removed' THEN
        DELETE FROM kb_saml_group_mappings
         WHERE idp_key = p_idp_key AND group_value = p_group AND team_id = p_team
        RETURNING role INTO v_held;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('outcome', 'absent');
        END IF;
        SELECT count(*) INTO v_members FROM kb_team_members
         WHERE team_id = p_team AND source = 'idp';
    ELSE
        RAISE EXCEPTION '_admin_saml_group_mapping_write: p_act must be created or removed, got %', p_act;
    END IF;

    PERFORM _event_append(
        'saml_group_mapping_changed', p_emitter, NULL, NULL,
        jsonb_strip_nulls(jsonb_build_object(
            'act',         p_act,
            'actor',       p_actor,
            'idp_key',     p_idp_key,
            'group_value', p_group,
            'team_id',     p_team,
            'role',        v_held,
            'confirmed',   CASE WHEN p_act = 'created' AND cardinality(v_needs) > 0 THEN true END)),
        p_references => jsonb_build_array(
            jsonb_build_object('rel', 'subject', 'target', jsonb_build_object('kind', 'kb_teams', 'id', p_team))),
        p_correlation => p_correlation);

    RETURN jsonb_strip_nulls(jsonb_build_object(
        'outcome',         'changed',
        'role',            v_held,
        'confirmed',       CASE WHEN p_act = 'created' AND cardinality(v_needs) > 0 THEN true END,
        'idp_memberships', v_members));
END;
$$;

COMMENT ON FUNCTION _admin_saml_group_mapping_write(uuid, uuid, text, text, text, uuid, team_role, boolean, uuid) IS
$c$Create or remove one kb_saml_group_mappings row and append its saml_group_mapping_changed admin
event, atomically. A create of role owner, or onto temper-system or the gating team, needs
p_confirmed. A removal leaves every membership in place. Answers jsonb {outcome, ...}; only
outcome changed writes or emits. It NEVER AUTHORIZES: saml_mapping_service decides who may.$c$;

SELECT declare_migration(
    20261125000010,
    'additive',
    'Adds the saml_group_mapping_changed admin event type and _admin_saml_group_mapping_write, '
    'which creates or removes a kb_saml_group_mappings row and appends its ledger record in one '
    'statement. No table, column, constraint or existing function changes, so a binary without this '
    'change keeps working and the IdP reconcile reads the table as before. It refuses to apply over '
    'an existing mapping that is owner or targets temper-system or the gating team (Pete, '
    '2026-11-11), naming each.'
);
