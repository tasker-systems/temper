-- Every recorded team membership change writes its row and its ledger record in one statement
-- (team-role rank spec C8; plan `plans/2026-10-14-team-role-rank.md` Task B1).
--
-- Until now no writer of `kb_team_members` emitted an event: a member added, promoted, demoted or
-- removed left no record of who did it. This migration adds the admin event type
-- `team_membership_changed` and the one function that writes a membership row AND appends that
-- event, atomically, in the shape of `_admin_grant_created` / `_admin_grant_revoked`
-- (20260718000010_admin_grant_fns.sql).
--
-- WHAT THE FUNCTION DOES NOT DO: AUTHORIZE. Who may make which change is decided in Rust, by the
-- sealed `AuthorizedMembershipChange` (temper-services `authz/team_membership.rs`), whose
-- `membership_write` is this function's only caller. The function re-checks, under its locks,
-- only what must be atomic with the write: that the row is still what was authorized, and the
-- last-owner guard.
--
-- LOCKS. The caller locks the team row (`FOR NO KEY UPDATE`) and then the member's row
-- (`FOR UPDATE`) at the head of its transaction, before it re-resolves the caller's authority, so
-- every routed write to one team is serialized and the owner count read here is current. The
-- function takes the same two locks again, in the same order; within the caller's transaction
-- that is a no-op, and it keeps the order right for any caller that forgot. `FOR NO KEY UPDATE`
-- does not conflict with the `KEY SHARE` a member insert's foreign key takes on the team row
-- (the 20260712000110 precedent).
--
-- THE ACT IS TOLD, NEVER INFERRED (plan P13). `p_act` is `add`, `change` or `remove`. An `add`
-- over an existing row is `exists` (the service turns it into the 409 naming `change_role`), or a
-- silent `noop` for the two paths whose adds were always `ON CONFLICT DO NOTHING` (an accepted
-- invitation, an approval). A `change` or `remove` of a row that is gone is `absent`, and of a row
-- whose role or source is no longer the one authorized against is `stale`. None of these writes
-- or emits: a change that did not happen is not recorded (C8).
--
-- THE LAST-OWNER GUARD, AND ITS ONE EXCEPTION. With `p_guard_last_owner`, lowering or removing the
-- team's only `owner` returns `last_owner` and writes nothing (C6). SAML reconcile passes `false`:
-- the identity provider wins over the guard (R8), and when that leaves the team with no owner the
-- record says so with `left_ownerless`, for a system admin to repair (C9). A machine's personal
-- team never holds an owner (20261019100000_machines_never_govern.sql), so the guard cannot fire
-- there, which is the C6 carve-out holding by construction.
--
-- THE EMITTER. A person's act is emitted by that person's entity, which the caller resolves. A
-- system act (`p_actor` NULL; SAML reconcile only) is emitted by the canonical `system` entity,
-- deliberately never the member's: the admin ledger's actor axis (`list_by_actor`) would
-- otherwise show the member a record of something done to them (spec "The event", plan P4).
--
-- The derived writers stay outside this function and un-evented (spec C8, R13; plan P6):
-- personal-team genesis (`sync_personal_team`), auto-join materialization
-- (`ensure_auto_join_memberships`, `backfill_auto_join_team`, `auto_join_reconcile`) and the
-- machine-cap trigger (`machine_client_caps_profile`).

-- The payload is the typed `temper_substrate::payloads::TeamMembershipChanged` wire contract;
-- `verify_ledger_roundtrip` deserializes every one of these events into it. The literal below is
-- `crates/temper-substrate/tests/fixtures/payloads/team_membership_changed.v1.schema.json`, and
-- `the_migration_literal_matches_the_committed_fixture` pins the pair.
INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('team_membership_changed', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "TeamMembershipChanged",
  "description": "`team_membership_changed` — one recorded write to `kb_team_members` (team-role rank spec C8,\n\"The event\"). Written only by `_admin_team_membership_write`, in the same statement as the row.\n\nONE TYPE FOR ADD, CHANGE AND REMOVE, discriminated by which roles are present:\n`previous_role` absent means the member was added, `role` absent means they were removed, both\npresent means the role changed. A change to the current role is not recorded at all.\n\nThe team is the event's `subject` reference and the member its `principal`. `actor` is absent\nwhen the system acted (SAML reconcile); the event is then emitted by the `system` entity, never\nby the member's, so the record does not reach the member through the actor axis.",
  "type": "object",
  "properties": {
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
    "invited_by": {
      "description": "Who made the invitation, on `via = invitation` only.",
      "anyOf": [
        {
          "$ref": "#/$defs/ProfileId"
        },
        {
          "type": "null"
        }
      ]
    },
    "left_ownerless": {
      "description": "True only on an unguarded (SAML reconcile) write that left the team with no owner (spec C6,\nR8). Absent otherwise.",
      "type": "boolean"
    },
    "previous_role": {
      "anyOf": [
        {
          "$ref": "#/$defs/TeamRole"
        },
        {
          "type": "null"
        }
      ]
    },
    "profile_id": {
      "$ref": "#/$defs/ProfileId"
    },
    "role": {
      "anyOf": [
        {
          "$ref": "#/$defs/TeamRole"
        },
        {
          "type": "null"
        }
      ]
    },
    "source": {
      "description": "The row's source: before the act for a change or remove, as written for an add.",
      "$ref": "#/$defs/TeamMemberSource"
    },
    "team_id": {
      "type": "string",
      "format": "uuid"
    },
    "via": {
      "$ref": "#/$defs/MembershipVia"
    }
  },
  "required": [
    "team_id",
    "profile_id",
    "source",
    "via"
  ],
  "$defs": {
    "MembershipVia": {
      "description": "The path a recorded team-membership change came through — the `via` of a\n`team_membership_changed` admin event (team-role rank spec, \"The event\").\n\nA closed set with a fixed SQL spelling: `_admin_team_membership_write` refuses any value not in\nit, and [`MembershipVia::as_str`] is the one Rust spelling of that set. The derived writers\n(personal-team genesis, auto-join materialization, the machine-cap trigger) have no `via`\nbecause they are not recorded (spec C8, R13).",
      "oneOf": [
        {
          "description": "An add, change or remove by a manager of the team or a system admin, including a system admin\nadding themselves or raising their own role.",
          "type": "string",
          "const": "member_act"
        },
        {
          "description": "A member acting on themselves: leaving, or lowering their own role.",
          "type": "string",
          "const": "self"
        },
        {
          "description": "An accepted invitation (the invitee is the actor; the payload names the inviter).",
          "type": "string",
          "const": "invitation"
        },
        {
          "description": "`create_team`'s creator row.",
          "type": "string",
          "const": "team_genesis"
        },
        {
          "description": "A system admin approving an access request (`access_service::review_request`).",
          "type": "string",
          "const": "access_approval"
        },
        {
          "description": "A machine registration's team reach (`machine_registration_service::apply_reach`).",
          "type": "string",
          "const": "machine_reach"
        },
        {
          "description": "SAML reconcile of identity-provider rows. The system is the actor.",
          "type": "string",
          "const": "idp_reconcile"
        }
      ]
    },
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    },
    "TeamMemberSource": {
      "description": "Provenance of a team membership row. Maps to the `team_member_source`\nPostgres enum (added by `20260702000001_saml_group_provisioning.sql`).\n`Idp` rows are owned by SAML reconcile and are not user-mutable.",
      "type": "string",
      "enum": [
        "native",
        "idp"
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

CREATE FUNCTION _admin_team_membership_write(
    p_emitter          uuid,
    p_actor            uuid,
    p_via              text,
    p_act              text,
    p_team             uuid,
    p_profile          uuid,
    p_role             team_role,
    p_expected_role    team_role,
    p_expected_source  team_member_source,
    p_new_source       team_member_source,
    p_guard_last_owner boolean,
    p_invited_by       uuid DEFAULT NULL,
    p_correlation      uuid DEFAULT NULL
) RETURNS text
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_prev_role    team_role;
    v_prev_source  team_member_source;
    v_found        boolean;
    v_lowers_owner boolean;
    v_left_owner   boolean := false;
    v_emitter      uuid;
    v_payload      jsonb;
    v_outcome      text;
BEGIN
    IF p_via IS NULL OR p_via NOT IN ('member_act', 'self', 'invitation', 'team_genesis',
                                      'access_approval', 'machine_reach', 'idp_reconcile') THEN
        RAISE EXCEPTION 'unknown team membership via %', p_via;
    END IF;
    IF p_act IS NULL OR p_act NOT IN ('add', 'change', 'remove') THEN
        RAISE EXCEPTION 'unknown team membership act %', p_act;
    END IF;
    IF p_act IN ('add', 'change') AND p_role IS NULL THEN
        RAISE EXCEPTION 'a team membership % names the role it writes', p_act;
    END IF;
    IF p_act = 'add' AND p_new_source IS NULL THEN
        RAISE EXCEPTION 'a team membership add names the source it writes';
    END IF;
    IF p_act IN ('change', 'remove') AND (p_expected_role IS NULL OR p_expected_source IS NULL) THEN
        RAISE EXCEPTION 'a team membership % names the role and source it was authorized against', p_act;
    END IF;
    IF p_guard_last_owner IS NULL THEN
        RAISE EXCEPTION 'a team membership write says whether the last-owner guard applies';
    END IF;

    -- The emitter is settled before anything is written, so a missing one fails the whole act.
    IF p_actor IS NULL THEN
        SELECT e.id INTO v_emitter
          FROM kb_entities e JOIN kb_profiles pr ON pr.id = e.profile_id
         WHERE pr.handle = 'system' AND e.name = 'system';
        IF v_emitter IS NULL THEN
            RAISE EXCEPTION 'no system entity to emit a system team membership change';
        END IF;
    ELSE
        IF p_emitter IS NULL THEN
            RAISE EXCEPTION 'a team membership change by a person names that person''s emitter';
        END IF;
        -- The record's two actor axes (the emitter and `payload.actor`) name one person.
        IF NOT EXISTS (SELECT 1 FROM kb_entities WHERE id = p_emitter AND profile_id = p_actor) THEN
            RAISE EXCEPTION 'team membership emitter % is not an entity of actor %', p_emitter, p_actor;
        END IF;
        v_emitter := p_emitter;
    END IF;

    -- The caller already holds both locks (see the header); taking them again is a no-op then.
    PERFORM 1 FROM kb_teams WHERE id = p_team FOR NO KEY UPDATE;
    SELECT role, source INTO v_prev_role, v_prev_source
      FROM kb_team_members
     WHERE team_id = p_team AND profile_id = p_profile
       FOR UPDATE;
    v_found := FOUND;

    IF p_act = 'add' THEN
        IF v_found THEN
            RETURN CASE WHEN p_via IN ('invitation', 'access_approval') THEN 'noop' ELSE 'exists' END;
        END IF;
    ELSE
        IF NOT v_found THEN
            RETURN 'absent';
        END IF;
        IF v_prev_role IS DISTINCT FROM p_expected_role
           OR v_prev_source IS DISTINCT FROM p_expected_source THEN
            RETURN 'stale';
        END IF;
        IF p_act = 'change' AND p_role = v_prev_role THEN
            RETURN 'noop';
        END IF;
    END IF;

    v_lowers_owner := COALESCE(v_prev_role = 'owner', false)
                      AND (p_act = 'remove' OR (p_act = 'change' AND p_role <> 'owner'));

    IF p_guard_last_owner AND v_lowers_owner AND NOT EXISTS (
        SELECT 1 FROM kb_team_members
         WHERE team_id = p_team AND role = 'owner' AND profile_id <> p_profile
    ) THEN
        RETURN 'last_owner';
    END IF;

    IF p_act = 'add' THEN
        INSERT INTO kb_team_members (team_id, profile_id, role, source)
        VALUES (p_team, p_profile, p_role, p_new_source);
        v_outcome := 'added';
    ELSIF p_act = 'change' THEN
        -- The role only. A row's source is never rewritten by a role change (C7).
        UPDATE kb_team_members SET role = p_role
         WHERE team_id = p_team AND profile_id = p_profile;
        v_outcome := 'changed';
    ELSE
        DELETE FROM kb_team_members WHERE team_id = p_team AND profile_id = p_profile;
        v_outcome := 'removed';
    END IF;

    IF NOT p_guard_last_owner AND v_lowers_owner AND NOT EXISTS (
        SELECT 1 FROM kb_team_members WHERE team_id = p_team AND role = 'owner'
    ) THEN
        v_left_owner := true;
    END IF;

    v_payload := jsonb_strip_nulls(jsonb_build_object(
        'team_id',       p_team,
        'profile_id',    p_profile,
        'previous_role', v_prev_role,
        'role',          CASE WHEN p_act = 'remove' THEN NULL ELSE p_role END,
        'source',        COALESCE(v_prev_source, p_new_source),
        'actor',         p_actor,
        'invited_by',    p_invited_by,
        'via',           p_via));
    IF v_left_owner THEN
        v_payload := v_payload || jsonb_build_object('left_ownerless', true);
    END IF;

    PERFORM _event_append(
        'team_membership_changed', v_emitter, NULL, NULL, v_payload,
        p_references => jsonb_build_array(
            jsonb_build_object('rel', 'subject',   'target', jsonb_build_object('kind', 'kb_teams',    'id', p_team)),
            jsonb_build_object('rel', 'principal', 'target', jsonb_build_object('kind', 'kb_profiles', 'id', p_profile))),
        p_correlation => p_correlation);

    RETURN v_outcome;
END;
$$;

COMMENT ON FUNCTION _admin_team_membership_write(uuid, uuid, text, text, uuid, uuid, team_role,
    team_role, team_member_source, team_member_source, boolean, uuid, uuid) IS
$c$Write one kb_team_members row (add, change or remove) and append its team_membership_changed admin
event, atomically. Returns added | changed | removed | noop | last_owner | exists | absent | stale;
only the first three write or emit.

It NEVER AUTHORIZES. Authorization is the Rust sealed value AuthorizedMembershipChange, and this
function's only caller is temper-services authz::team_membership::membership_write (pinned by a
tripwire). It relies on the caller having locked the team row, then the member row, before
resolving its authority; it re-takes both in that order.

p_guard_last_owner = false is for idp_reconcile ONLY: the identity provider wins over the
last-owner guard (team-role rank spec R8), and the record then carries left_ownerless. No other
path may pass false. A machine's personal team never holds an owner, so the guard cannot fire
there (the C6 carve-out holds by construction).$c$;

SELECT declare_migration(
    20261101200000,
    'additive',
    'Adds the team_membership_changed admin event type and _admin_team_membership_write, which '
    'writes one kb_team_members row and its ledger record in one statement. Nothing existing calls '
    'it, and no existing function, table or column changes, so a binary without this change keeps '
    'working; the new binary routes add_member, change_role and remove_member through it.'
);
