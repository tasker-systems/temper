-- A SAML group mapping onto a team whose subtree holds a pool needs the admin's confirmation; a
-- structural team act that puts a pool beneath an already-mapped team answers and records each
-- mapping it elevated (Pete, 2026-11-11: B on all three; goal clause C6 widens). And an IdP group
-- confers at most `watcher` on temper-system or the gating team (Pete, 2026-11-12).
--
-- WATCHER ONLY ON AN INSTANCE TEAM (Pete, 2026-11-12). A SAML assertion is an access-promotion path:
-- one spoofed or intercepted carries every role its groups map to. So a mapping onto temper-system or
-- the gating team is `watcher` only, refused above it with or without confirm_elevated (outcome
-- instance_team_watcher_only), and the IdP reconcile clamps any such row to `watcher` whatever it
-- says, so a row written before this rule, by #1350's door with confirmation, or by SQL, is bound
-- too. A higher role on either team is a system admin's native act (the gating roster is admin-only,
-- R11). A gating repoint records each authoring mapping onto the new gating team as `clamped`
-- (`_saml_mappings_clamped_record`), and Section 4 records every existing one.
--
-- `confirmed` (20261125000010) is a key of the event payload, not a column: an admin's
-- acknowledgement of a mapping's reach, read by no door and no reconcile, and saying nothing about any
-- principal's standing. No row is backfilled as confirmed here (B′ extended to pool ancestors was
-- withdrawn, Pete, 2026-11-12: it would record an acknowledgement nobody gave).
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
--   * `_saml_mappings_over_pool(seeds)`: the authoring-role mappings whose team is in that set AND
--     is one of `seeds` or above one of them (dead teams included), as a jsonb array of {idp_key,
--     group, team_id, role} ordered by idp_key, group, team_id. The seeds are the teams the act
--     writes at: the parent it links under, the team it flips, the new gating team. Only a team at
--     or above a seed can gain a pool from the act, so the scope keeps every other team's mappings,
--     and every concurrent create of them, out of the act's answer and its records.
--   * `_saml_mapping_elevations_record(emitter, actor, seeds, before, correlation)`: reads that set
--     again, and for every mapping in it that is not in `before` appends one
--     `saml_group_mapping_changed` event, act `elevated`, naming the act's actor; answers the array
--     of them. A structural act takes `_public_within_effects_lock()` (so elevating acts serialize
--     and never record one mapping twice), reads `_saml_mappings_over_pool(seeds)` on its own
--     transaction after its row locks and before its write, and calls this after the write. A
--     create onto a team at or above a seed locks the seed row FOR SHARE, which the act holds FOR NO
--     KEY UPDATE, so no such create commits between the two reads. The acts: the gating repoint, attach, reparent,
--     set_auto_join, restore, and creating an auto-join team under a parent. Restore never elevates:
--     it flips is_active only, and dead teams already count. No act asks for confirmation; all of
--     them that can put a pool beneath a team are already a system admin's.
--   * the payload schema re-registered with the acts `elevated` and `clamped` (the committed fixture
--     saml_group_mapping_changed.v1.schema.json; payload_schema.rs pins the pair).
--   * `_saml_mappings_clamped_record(emitter, actor, teams, migration, correlation)`: one `clamped`
--     event per mapping above watcher onto one of `teams`.
--   * `_admin_saml_group_mapping_write`, body-only CREATE OR REPLACE (signature as before; outcomes
--     gain `instance_team_watcher_only` and `subtree_changed`, needs gains `pool_beneath`, and
--     `instance_team` is no longer answered). An authoring role onto temper-system or the gating
--     team is refused. A create needs p_confirmed for an `owner` role, or for an authoring role when
--     the target's subtree holds a pool (`pool_beneath`). A WATCHER create never needs it, onto the
--     root or the gating team included.
--
-- LOCKS. An authoring create takes FOR SHARE on the target and every team below it, dead ones
-- included, in ascending id order in one statement, THEN the shared `_gating_team_lock`, then walks
-- the subtree again; if it grew while the create waited, the create answers `subtree_changed` and
-- writes nothing (the door's retryable 409). Every act that can put a pool at or below the target
-- takes FOR NO KEY UPDATE on a row in that set (attach, reparent and create_team the parent it
-- links under; set_auto_join the team), which FOR SHARE conflicts with; the gating repoint takes the
-- gating lock exclusively. So whichever commits first, the other reads it, on a fresh READ
-- COMMITTED statement after its lock wait: the create's predicate sees the pool, or the act's
-- elevation scan sees the mapping. A watcher create locks the target row only, and so does a create
-- onto temper-system or the gating team: each is a pool by itself, which only a repoint (the
-- exclusive gating lock) can change, so its subtree cannot move the answer. temper-system's subtree
-- is every team, which this keeps from being locked; a gating target that a repoint moved before the
-- gating lock was held answers subtree_changed.
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
-- #1350's writer (20261125000010) took the gating lock before the team row, which deadlocks with a
-- rename of the target; this migration reverses it (Section 3).
--
-- No existing row is changed or adopted. An authoring mapping already over a pool that is not the
-- root or the gating team is left as it is and confers as before; one onto the root or the gating
-- team is recorded as clamped (Section 4) and confers watcher from the member's next login.

-- Section 1. One definition of "the subtree holds a pool", and the mappings over one.
-- PL/pgSQL, not SQL, so the body is not resolved at CREATE time: temper-substrate's test baseline
-- omits kb_teams.auto_join_role (crates/temper-substrate/tests/common/mod.rs), and this migration
-- must still apply there for the payload schema it re-registers.
CREATE FUNCTION _teams_over_pool() RETURNS SETOF uuid
LANGUAGE plpgsql STABLE
SET search_path = public, pg_temp
AS $$
BEGIN
    RETURN QUERY
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
    SELECT up.id FROM up;
END;
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

CREATE FUNCTION _saml_mappings_over_pool(p_seeds uuid[]) RETURNS jsonb
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    WITH RECURSIVE up AS (
        SELECT unnest(p_seeds) AS id
        UNION
        SELECT tp.parent_id FROM kb_teams_parents tp JOIN up ON tp.child_id = up.id
    )
    SELECT coalesce(jsonb_agg(jsonb_build_object(
               'idp_key', m.idp_key, 'group', m.group_value, 'team_id', m.team_id, 'role', m.role)
               ORDER BY m.idp_key, m.group_value, m.team_id), '[]'::jsonb)
      FROM kb_saml_group_mappings m
     WHERE m.role <> 'watcher'
       AND m.team_id IN (SELECT id FROM up)
       AND m.team_id IN (SELECT o FROM _teams_over_pool() o)
       -- A mapping onto temper-system or the gating team confers watcher only (the reconcile's
       -- clamp), so it is clamped, never elevated.
       AND m.team_id NOT IN (SELECT t.id FROM kb_teams t
                              WHERE t.slug = 'temper-system'
                                 OR t.slug = (SELECT s.gating_team_slug FROM kb_system_settings s
                                               WHERE s.id = 1));
$$;

COMMENT ON FUNCTION _saml_mappings_over_pool(uuid[]) IS
$c$The authoring-role (owner, maintainer, member) SAML group mappings whose team is one of p_seeds or
above one (dead teams included) and whose subtree holds a pool, as a jsonb array of {idp_key, group,
team_id, role}, ordered. A structural team act reads it before its write and hands it, with the same
seeds, to _saml_mapping_elevations_record after.$c$;

CREATE FUNCTION _saml_mapping_elevations_record(
    p_emitter     uuid,
    p_actor       uuid,
    p_seeds       uuid[],
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
        SELECT a.m FROM jsonb_array_elements(_saml_mappings_over_pool(p_seeds)) AS a(m)
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

COMMENT ON FUNCTION _saml_mapping_elevations_record(uuid, uuid, uuid[], jsonb, uuid) IS
$c$For each authoring-role SAML mapping at or above p_seeds now over a pool (_saml_mappings_over_pool)
that p_before (the same read, with the same seeds, taken by the act before its write) did not hold, append one
saml_group_mapping_changed event, act elevated, naming p_actor; answer the array of them. Call it on
the structural act's own transaction, after the write. It changes no row and NEVER AUTHORIZES.$c$;

CREATE FUNCTION _saml_mappings_clamped_record(
    p_emitter     uuid,
    p_actor       uuid,
    p_teams       uuid[],
    p_migration   bigint DEFAULT NULL,
    p_correlation uuid   DEFAULT NULL
) RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_out jsonb := '[]'::jsonb;
    r     record;
BEGIN
    FOR r IN
        SELECT m.idp_key, m.group_value, m.team_id, m.role
          FROM kb_saml_group_mappings m
         WHERE m.team_id = ANY(p_teams) AND m.role <> 'watcher'
         ORDER BY m.idp_key, m.group_value, m.team_id
    LOOP
        PERFORM _event_append(
            'saml_group_mapping_changed', p_emitter, NULL, NULL,
            jsonb_strip_nulls(jsonb_build_object(
                'act',         'clamped',
                'actor',       p_actor,
                'idp_key',     r.idp_key,
                'group_value', r.group_value,
                'team_id',     r.team_id,
                'role',        r.role,
                'migration',   p_migration)),
            p_references => jsonb_build_array(
                jsonb_build_object('rel', 'subject',
                                   'target', jsonb_build_object('kind', 'kb_teams', 'id', r.team_id))),
            p_correlation => p_correlation);
        v_out := v_out || jsonb_build_array(jsonb_build_object(
            'idp_key', r.idp_key, 'group', r.group_value, 'team_id', r.team_id, 'role', r.role));
    END LOOP;
    RETURN v_out;
END;
$$;

COMMENT ON FUNCTION _saml_mappings_clamped_record(uuid, uuid, uuid[], bigint, uuid) IS
$c$For each SAML mapping onto one of p_teams (temper-system or the gating team) with a role above
watcher, append one saml_group_mapping_changed event, act clamped: the IdP reconcile confers watcher
there, whatever the row says. Answers the array of them. Called by a gating repoint for the new
gating team, and by 20261126000010 for every such row. It changes no row and NEVER AUTHORIZES.$c$;

-- Section 2. The payload schema, re-registered with the act `elevated`. The literal is the committed
-- fixture crates/temper-substrate/tests/fixtures/payloads/saml_group_mapping_changed.v1.schema.json;
-- payload_schema.rs pins the two together. Every payload already written still validates: the
-- change adds an enum value and rewords descriptions.
UPDATE kb_event_types
   SET payload_schema = $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "SamlGroupMappingChanged",
  "description": "`saml_group_mapping_changed` — a SAML group mapping (`kb_saml_group_mappings`: an IdP's group\nvalue conferring a role on a team) was created or removed by a system admin, or was elevated or\nclamped by a structural team act or a migration. `created` and `removed` are written only by\n`_admin_saml_group_mapping_write`, in the same statement as the row; `elevated` only by\n`_saml_mapping_elevations_record` and `clamped` only by `_saml_mappings_clamped_record`.\n\nA mapping confers membership on every person the IdP asserts the group for, at each login's\nreconcile, so who made one, and who removed one, is an authority record like a membership\nchange. The team is the event's `subject` reference. Removing a mapping deletes no membership:\nthe next reconcile of each member decides those.",
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
      "description": "True on a `created` that needed the admin's explicit confirmation (an `owner` role, or an\nauthoring role onto a team whose subtree, dead teams included, holds an auto-join team) and\nhad it, and on every `adopted`. Absent otherwise. It records an admin's acknowledgement of a\nmapping's reach; it says nothing about any principal's standing.",
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
      "description": "On `adopted`, the version of the migration that recorded the row as confirmed; on a\n`clamped` written by a migration, the version that recorded the clamp. Such a record has no\nperson actor.",
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
        },
        {
          "description": "A mapping that asks for more than `watcher` on temper-system or the gating team, which an\nIdP group never confers there: the reconcile confers `watcher` instead, and a higher role on\neither team is a system admin's native act. Written by the migration that introduced the\nrule for each such row (`migration` names it, no person actor), and by a gating repoint for\neach such mapping onto the new gating team (`actor` is the repoint's). No row changed.",
          "type": "string",
          "const": "clamped"
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
    v_instance boolean;
    v_now_instance boolean;
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
        -- The target itself a pool that only the exclusive gating lock (held off below) can unmake:
        -- its subtree cannot change the answer, so the target row alone. temper-system's subtree is
        -- every team, which this keeps an authoring create onto the root from locking.
        SELECT slug INTO v_slug FROM kb_teams WHERE id = p_team;
        v_instance := coalesce(v_slug = 'temper-system'
                               OR EXISTS (SELECT 1 FROM kb_system_settings
                                           WHERE id = 1 AND gating_team_slug = v_slug), false);
        IF p_role = 'watcher' OR v_instance THEN
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
        -- Watcher only on an instance team (Pete, 2026-11-12), with or without confirmation: an IdP
        -- group never confers an authoring role on temper-system or the gating team. Read now,
        -- under the gating lock, so a repoint committed while this create waited is seen.
        v_now_instance := v_slug = 'temper-system'
                          OR EXISTS (SELECT 1 FROM kb_system_settings
                                      WHERE id = 1 AND gating_team_slug = v_slug);
        IF p_role <> 'watcher' AND v_now_instance THEN
            RETURN jsonb_build_object('outcome', 'instance_team_watcher_only');
        END IF;
        -- An instance target locked alone that a repoint has since made an ordinary team: its
        -- subtree was not locked, so refuse rather than decide on it.
        IF p_role <> 'watcher' AND v_instance THEN
            RETURN jsonb_build_object('outcome', 'subtree_changed');
        END IF;
        -- A team linked under the subtree while this create waited is not locked: refuse rather than
        -- decide on a subtree held only in part.
        IF p_role <> 'watcher' AND NOT v_instance AND NOT (ARRAY(
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
        IF p_role <> 'watcher' AND _team_subtree_holds_pool(p_team) THEN
            v_needs := v_needs || 'pool_beneath'::text;
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
event, atomically. A create onto temper-system or the gating team is watcher only (outcome
instance_team_watcher_only above it, confirmed or not). A create needs p_confirmed for role owner, or
for an authoring role (owner, maintainer, member) onto a team whose subtree, dead teams included,
holds a pool (needs: pool_beneath); a watcher create never does. An authoring create locks the target's subtree FOR SHARE, ascending, then the shared
gating lock, and answers subtree_changed (nothing written) if the subtree grew meanwhile. A removal
leaves every membership in place. Answers jsonb {outcome, ...}; only outcome changed writes or
emits. It NEVER AUTHORIZES: saml_mapping_service decides who may.$c$;

-- Section 4. Record the existing rows the clamp binds (Pete, 2026-11-12). A mapping onto
-- temper-system or the gating team with a role above watcher stays as it is, and the reconcile
-- confers watcher for it from now on. Each gets one `clamped` record naming this migration, emitted
-- by the bare `system` entity with no person actor, and a NOTICE lists them. No row is changed. An
-- IdP membership such a mapping already conferred keeps its role until the member's next login,
-- when the reconcile clamps it.
DO $$
DECLARE
    v_emitter uuid;
    v_teams   uuid[];
    v_out     jsonb;
BEGIN
    v_teams := ARRAY(SELECT t.id FROM kb_teams t
                      WHERE t.slug = 'temper-system'
                         OR t.slug = (SELECT s.gating_team_slug FROM kb_system_settings s
                                       WHERE s.id = 1));
    IF NOT EXISTS (SELECT 1 FROM kb_saml_group_mappings
                    WHERE team_id = ANY(v_teams) AND role <> 'watcher') THEN
        RETURN;
    END IF;
    SELECT id INTO v_emitter FROM kb_entities WHERE name = 'system';
    IF v_emitter IS NULL THEN
        RAISE EXCEPTION 'the system entity is missing, so the SAML mappings onto an instance team '
                        'cannot be recorded as clamped';
    END IF;
    v_out := _saml_mappings_clamped_record(v_emitter, NULL, v_teams, 20261126000010);
    RAISE NOTICE 'SAML group mappings onto temper-system or the gating team above watcher, which the '
                 'IdP reconcile now confers as watcher, recorded as clamped on the admin ledger: %', v_out;
END;
$$;

SELECT declare_migration(
    20261126000010,
    'additive',
    'Adds _teams_over_pool, _team_subtree_holds_pool, _saml_mappings_over_pool, '
    '_saml_mapping_elevations_record and _saml_mappings_clamped_record; re-registers the '
    'saml_group_mapping_changed payload schema with the acts elevated and clamped; records each '
    'existing mapping above watcher onto temper-system or the gating team as clamped (no row '
    'changes); and body-only CREATE OR REPLACE (signature unchanged) of '
    '_admin_saml_group_mapping_write: an authoring role onto an instance team is refused, an '
    'authoring-role create onto a team whose subtree holds a pool needs confirmation, a watcher '
    'create never does, and an authoring create locks the target subtree before the shared gating '
    'lock. No table, column or constraint changes. A binary without this change still calls the '
    'writer by the same signature, renders the new outcomes instance_team_watcher_only and '
    'subtree_changed as its internal error, and its reconcile does not clamp.'
);
