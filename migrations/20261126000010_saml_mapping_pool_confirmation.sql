-- A SAML group mapping onto a team whose subtree holds a pool needs the admin's confirmation, as an
-- instance-team target always did; and a structural team act that puts a pool beneath an
-- already-mapped team answers and records each mapping it elevated (Pete, 2026-11-11: B on all
-- three; goal clause C6 widens).
--
-- Reads flow DOWN the team DAG: a pool's members read every team above it. A pool is the root
-- (temper-system), the gating team or an auto-join team; each one's roster is every approved
-- profile. So an IdP group mapped with an AUTHORING role (owner, maintainer, member) onto a pool's
-- ancestor writes material the whole instance reads. A watcher reads only.
--
-- This migration adds:
--
--   * `_teams_over_pool()`: every team, live or dead, that is a pool or has one anywhere beneath
--     it, walking up from the pool teams through kb_teams_parents. Dead teams count: a pool below a
--     dead team is latent reach, which a restore revives (the R-z reasoning). It is the ONE
--     definition of "the subtree holds a pool"; `_team_subtree_holds_pool(uuid)` reads it, and so
--     does `team_service::subtree_holds_pool_on`.
--   * `_saml_mappings_over_pool()`: the authoring-role mappings whose team is in that set, as a
--     jsonb array of {idp_key, group, team_id, role} ordered by idp_key, group, team_id.
--   * `_saml_mapping_elevations_record(emitter, actor, before, correlation)`: reads that set again,
--     and for every mapping in it that is not in `before` appends one `saml_group_mapping_changed`
--     event, act `elevated`, naming the act's actor; answers the array of them. A structural act
--     reads `_saml_mappings_over_pool()` on its own transaction after its locks and before its
--     write, and calls this after the write. The acts: the gating repoint, attach, reparent,
--     set_auto_join, restore, and creating an auto-join team under a parent. Restore never elevates:
--     it flips is_active only, and dead teams already count. No act asks for confirmation; all of
--     them that can put a pool beneath a team are already a system admin's.
--   * the payload schema re-registered with the act `elevated` (the committed fixture
--     saml_group_mapping_changed.v1.schema.json; payload_schema.rs pins the pair).
--   * `_admin_saml_group_mapping_write`, body-only CREATE OR REPLACE (signature and answers as
--     before, plus `needs: pool_beneath` and the outcome `subtree_changed`). A create needs
--     p_confirmed for an `owner` role, or for an authoring role when the target's subtree holds a
--     pool (`instance_team` when the target is itself the root or the gating team, `pool_beneath`
--     otherwise). A WATCHER create never needs it, onto the root or the gating team included: this
--     is the one relaxation of 20261125000010's rule.
--
-- LOCKS. An authoring create takes FOR SHARE on the target and every team below it, dead ones
-- included, in ascending id order in one statement, THEN the shared `_gating_team_lock`, then walks
-- the subtree again; if it grew while the create waited, the create answers `subtree_changed` and
-- writes nothing (the door's retryable 409). Every act that can put a pool at or below the target
-- takes FOR NO KEY UPDATE on a row in that set (attach, reparent and create_team the parent it
-- links under; set_auto_join the team), which FOR SHARE conflicts with; the gating repoint takes the
-- gating lock exclusively. So whichever commits first, the other reads it, on a fresh READ
-- COMMITTED statement after its lock wait: the create's predicate sees the pool, or the act's
-- elevation scan sees the mapping. A watcher create locks the target row only.
--
-- The order is rows, then the shared gating lock: the order of every team act (lock_teams_and_subtree,
-- a delete's gating check, a rename's row then exclusive lock). 20261125000010 took the gating lock
-- FIRST, on the belief that a rename takes the gating lock before its row; rename_slug in fact locks
-- its row first and takes the exclusive gating lock inside the lifecycle writer, so that order waited
-- in a cycle with a rename of the target (the create holding the shared lock and waiting on the row,
-- the rename holding the row and waiting on the exclusive lock). Rows ascending against rows
-- ascending cannot cycle, and the repoint holds no team row while it waits on the exclusive lock.
-- A later edit must keep this order.
--
-- No existing row is changed or adopted: an authoring mapping already over a pool that is not the
-- root or the gating team is left as it is, unrecorded.

-- Section 1. One definition of "the subtree holds a pool", and the mappings over one.
CREATE FUNCTION _teams_over_pool() RETURNS SETOF uuid
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    WITH RECURSIVE up AS (
        SELECT t.id FROM kb_teams t WHERE t.auto_join_role IS NOT NULL
        UNION
        SELECT t.id FROM kb_teams t WHERE t.slug = 'temper-system'
        UNION
        SELECT t.id FROM kb_teams t
         WHERE t.slug = (SELECT s.gating_team_slug FROM kb_system_settings s WHERE s.id = 1)
        UNION
        SELECT tp.parent_id FROM kb_teams_parents tp JOIN up ON tp.child_id = up.id
    )
    SELECT id FROM up;
$$;

COMMENT ON FUNCTION _teams_over_pool() IS
$c$Every team, live or dead, that is a pool (temper-system, the gating team, an auto-join team) or
has one anywhere beneath it through kb_teams_parents, dead teams included. The one definition of
"the subtree holds a pool": _team_subtree_holds_pool, _saml_mappings_over_pool and
team_service::subtree_holds_pool_on all read it.$c$;

CREATE FUNCTION _team_subtree_holds_pool(p_team uuid) RETURNS boolean
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT EXISTS (SELECT 1 FROM _teams_over_pool() o WHERE o = p_team);
$$;

COMMENT ON FUNCTION _team_subtree_holds_pool(uuid) IS
$c$True when the team itself, or any team below it, live or dead, is a pool (_teams_over_pool).$c$;

CREATE FUNCTION _saml_mappings_over_pool() RETURNS jsonb
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT coalesce(jsonb_agg(jsonb_build_object(
               'idp_key', m.idp_key, 'group', m.group_value, 'team_id', m.team_id, 'role', m.role)
               ORDER BY m.idp_key, m.group_value, m.team_id), '[]'::jsonb)
      FROM kb_saml_group_mappings m
     WHERE m.role <> 'watcher'
       AND m.team_id IN (SELECT o FROM _teams_over_pool() o);
$$;

COMMENT ON FUNCTION _saml_mappings_over_pool() IS
$c$The authoring-role (owner, maintainer, member) SAML group mappings whose team's subtree holds a
pool, as a jsonb array of {idp_key, group, team_id, role}, ordered. A structural team act reads it
before its write and hands it to _saml_mapping_elevations_record after.$c$;

CREATE FUNCTION _saml_mapping_elevations_record(
    p_emitter     uuid,
    p_actor       uuid,
    p_before      jsonb,
    p_correlation uuid DEFAULT NULL
) RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_out jsonb := '[]'::jsonb;
    r     record;
BEGIN
    FOR r IN
        SELECT a.m FROM jsonb_array_elements(_saml_mappings_over_pool()) AS a(m)
         WHERE NOT (coalesce(p_before, '[]'::jsonb) @> jsonb_build_array(a.m))
    LOOP
        PERFORM _event_append(
            'saml_group_mapping_changed', p_emitter, NULL, NULL,
            jsonb_strip_nulls(jsonb_build_object(
                'act',         'elevated',
                'actor',       p_actor,
                'idp_key',     r.m->>'idp_key',
                'group_value', r.m->>'group',
                'team_id',     (r.m->>'team_id')::uuid,
                'role',        r.m->>'role')),
            p_references => jsonb_build_array(
                jsonb_build_object('rel', 'subject',
                                   'target', jsonb_build_object('kind', 'kb_teams',
                                                                'id', (r.m->>'team_id')::uuid))),
            p_correlation => p_correlation);
        v_out := v_out || jsonb_build_array(r.m);
    END LOOP;
    RETURN v_out;
END;
$$;

COMMENT ON FUNCTION _saml_mapping_elevations_record(uuid, uuid, jsonb, uuid) IS
$c$For each authoring-role SAML mapping now over a pool (_saml_mappings_over_pool) that p_before
(the same read, taken by the act before its write) did not hold, append one
saml_group_mapping_changed event, act elevated, naming p_actor; answer the array of them. Call it on
the structural act's own transaction, after the write. It changes no row and NEVER AUTHORIZES.$c$;

-- Section 2. The payload schema, re-registered with the act `elevated`. The literal is the committed
-- fixture crates/temper-substrate/tests/fixtures/payloads/saml_group_mapping_changed.v1.schema.json;
-- payload_schema.rs pins the two together. Every payload already written still validates: the
-- change adds an enum value and rewords descriptions.
UPDATE kb_event_types
   SET payload_schema = $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "SamlGroupMappingChanged",
  "description": "`saml_group_mapping_changed` — a SAML group mapping (`kb_saml_group_mappings`: an IdP's group\nvalue conferring a role on a team) was created or removed by a system admin, or was elevated by a\nstructural team act. `created` and `removed` are written only by `_admin_saml_group_mapping_write`,\nin the same statement as the row; `elevated` only by `_saml_mapping_elevations_record`, in the\nstructural act's own transaction.\n\nA mapping confers membership on every person the IdP asserts the group for, at each login's\nreconcile, so who made one, and who removed one, is an authority record like a membership\nchange. The team is the event's `subject` reference. Removing a mapping deletes no membership:\nthe next reconcile of each member decides those.",
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
      "description": "True on a `created` that needed the admin's explicit confirmation (an `owner` role, or an\nauthoring role onto a team whose subtree, dead teams included, holds a pool: the root, the\ngating team or an auto-join team) and had it, and on every `adopted`. Absent otherwise.",
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
    "migration": {
      "description": "On `adopted` only: the version of the migration that recorded the row as confirmed. Such a\nrecord has no person actor.",
      "type": [
        "integer",
        "null"
      ],
      "format": "int64"
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
        },
        {
          "description": "A mapping that already existed above the ceiling when the doors arrived, recorded as\nconfirmed by the migration that introduced them (`migration` names it). No row changed.",
          "type": "string",
          "const": "adopted"
        },
        {
          "description": "A structural team act (a gating repoint, an attach, a reparent, setting auto-join, creating\nan auto-join team under a parent) put a pool beneath the team of an authoring-role mapping\nwhose team held none, so the mapping now confers a role that the whole pool reads the\nmaterial of. `actor` is the act's. No row changed and nothing was asked to confirm.",
          "type": "string",
          "const": "elevated"
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
$JS$::jsonb
 WHERE name = 'saml_group_mapping_changed';

-- Section 3. The writer: the pool predicate, the watcher relaxation, and the lock order above.
CREATE OR REPLACE FUNCTION _admin_saml_group_mapping_write(
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
    v_set      uuid[];
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
        -- The rows, then the shared gating lock (the header's LOCKS). An authoring role locks the
        -- whole subtree, dead teams included, in one ascending pass; a watcher the target only.
        IF p_role = 'watcher' THEN
            v_set := ARRAY[p_team];
        ELSE
            v_set := ARRAY(
                WITH RECURSIVE s AS (
                    SELECT p_team AS id
                    UNION
                    SELECT tp.child_id FROM kb_teams_parents tp JOIN s ON tp.parent_id = s.id)
                SELECT s.id FROM s);
        END IF;
        PERFORM 1 FROM kb_teams WHERE id = ANY(v_set) ORDER BY id FOR SHARE;
        PERFORM _gating_team_lock(false);
        SELECT slug, personal_of INTO v_slug, v_personal
          FROM kb_teams WHERE id = p_team AND is_active;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('outcome', 'absent_team');
        END IF;
        IF v_personal IS NOT NULL THEN
            RETURN jsonb_build_object('outcome', 'personal_team');
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
        -- A team linked under the subtree while this create waited is not locked: refuse rather than
        -- decide on a subtree held only in part.
        IF p_role <> 'watcher' AND NOT (ARRAY(
                WITH RECURSIVE s AS (
                    SELECT p_team AS id
                    UNION
                    SELECT tp.child_id FROM kb_teams_parents tp JOIN s ON tp.parent_id = s.id)
                SELECT s.id FROM s) <@ v_set) THEN
            RETURN jsonb_build_object('outcome', 'subtree_changed');
        END IF;
        IF p_role = 'owner' THEN
            v_needs := v_needs || 'owner_role'::text;
        END IF;
        IF p_role <> 'watcher' THEN
            IF v_slug = 'temper-system'
               OR EXISTS (SELECT 1 FROM kb_system_settings WHERE id = 1 AND gating_team_slug = v_slug) THEN
                v_needs := v_needs || 'instance_team'::text;
            ELSIF _team_subtree_holds_pool(p_team) THEN
                v_needs := v_needs || 'pool_beneath'::text;
            END IF;
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
event, atomically. A create needs p_confirmed for role owner, or for an authoring role (owner,
maintainer, member) onto a team whose subtree, dead teams included, holds a pool (needs:
instance_team for temper-system or the gating team itself, pool_beneath otherwise); a watcher create
never does. An authoring create locks the target's subtree FOR SHARE, ascending, then the shared
gating lock, and answers subtree_changed (nothing written) if the subtree grew meanwhile. A removal
leaves every membership in place. Answers jsonb {outcome, ...}; only outcome changed writes or
emits. It NEVER AUTHORIZES: saml_mapping_service decides who may.$c$;

SELECT declare_migration(
    20261126000010,
    'additive',
    'Adds _teams_over_pool, _team_subtree_holds_pool, _saml_mappings_over_pool and '
    '_saml_mapping_elevations_record; re-registers the saml_group_mapping_changed payload schema '
    'with the act elevated; and body-only CREATE OR REPLACE (signature unchanged) of '
    '_admin_saml_group_mapping_write: an authoring-role create onto a team whose subtree holds a '
    'pool now needs confirmation, a watcher create never does, and an authoring create locks the '
    'target subtree before the shared gating lock. No table, column or constraint changes. A binary '
    'without this change still calls the writer by the same signature; it renders the new needs '
    'value pool_beneath without a reason and the new outcome subtree_changed as its internal error.'
);
