-- public_within, Phase 1: the schema, the policy and the helpers. Nothing reads through them yet.
--
-- Design: temper-artifacts specs/2026-11-06-public-within-design.md (P1, P4, P5, P6, P9; rulings
-- R-a .. R-o). Plan: plans/2026-11-06-public-within.md Phase 1.
--
-- A `public_within(T, A)` row lets the people under the anchor team A read the contexts T owns. This
-- migration adds the row, the two policy switches over it, and the predicates that say whether a row
-- confers. The read arm that consults them is the next migration (Phase 2), so on its own this one
-- changes no answer anywhere.
--
--   kb_system_settings.public_within_enabled   the instance switch, false by default everywhere (R-e).
--                                              There is no backfill to true, here or in any seed.
--   kb_teams.public_within_forbidden           an owner's forbid over everything beneath it (P4).
--   kb_team_public_within                      the rows; a trigger refuses the shapes P1 bars.
--   team_is_root_or_gating                     the root or the current gating team (both checks).
--   team_ancestors_any                         the live-or-dead up-walk, for the forbid ONLY.
--   public_within_allowed_over                 the switch, then no forbid on any ancestor of T.
--   anchor_subtree_holds_pool                  the gating team at or under A (PW-1); 20261120000005
--                                              adds auto-join teams.
--   public_within_row_effective                one row's read-time conditions (P5, N9).
--   contexts_public_within                     the contexts effective rows open to a team closure.
--   public_within_reading                      the machine fence: is this profile a person? (R-n)
--   _admin_public_within_switch_write          the instance switch and its record, one statement.
--   three admin event types                    team_public_within_changed,
--                                              public_within_effect_changed,
--                                              public_within_policy_changed (P9).
--
-- `team_live_descendants` already exists (20261119000020, Phase 0) and is reused, not redefined.
--
-- Every rationale lives in COMMENT ON FUNCTION, never in a body: the guard scans read
-- pg_get_functiondef (PW-9).

-- ── 1. The switch and the forbid ──────────────────────────────────────────────────────────────────
ALTER TABLE kb_system_settings
    ADD COLUMN public_within_enabled boolean NOT NULL DEFAULT false;

COMMENT ON COLUMN kb_system_settings.public_within_enabled IS
$c$The instance switch for public_within (spec P4, R-e). False by default on every deployment. While
it is false no kb_team_public_within row confers anything and every open is refused; close and
list still work. Set by a system admin through _admin_public_within_switch_write, which records
public_within_policy_changed.$c$;

ALTER TABLE kb_teams
    ADD COLUMN public_within_forbidden boolean NOT NULL DEFAULT false;

COMMENT ON COLUMN kb_teams.public_within_forbidden IS
$c$An owner's forbid (spec P4, R-f): no public_within row confers for any team beneath this one, by
any path, live or dead (public_within_allowed_over walks team_ancestors_any). A lower owner cannot
lift it; only leaving the subtree does. Not available on the root (its policy is the instance
switch) or on personal teams; that is enforced by the door that sets it.$c$;

-- ── 2. The rows ───────────────────────────────────────────────────────────────────────────────────
CREATE TABLE kb_team_public_within (
    team_id       uuid        NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    anchor_id     uuid        NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    opened_by     uuid            NULL REFERENCES kb_profiles(id) ON DELETE SET NULL,
    opened_at     timestamptz NOT NULL DEFAULT now(),
    admin_act     boolean     NOT NULL DEFAULT false,
    instance_wide boolean     NOT NULL DEFAULT false,
    PRIMARY KEY (team_id, anchor_id),
    CONSTRAINT kb_team_public_within_not_self CHECK (team_id <> anchor_id)
);

CREATE INDEX idx_kb_team_public_within_anchor ON kb_team_public_within (anchor_id);

COMMENT ON TABLE kb_team_public_within IS
$c$public_within(T, A): the people under anchor team A read the contexts team T owns, while the row
is effective (public_within_row_effective). Opened by both sides (P2), closed by either (P3). A
structural lapse deletes the row (P5); liveness and policy causes leave it dormant. Owned contexts
only: a share to T, a grant held by T and a map bound to T are never exposed (N8).$c$;
COMMENT ON COLUMN kb_team_public_within.team_id IS 'T: the team whose owned contexts are opened.';
COMMENT ON COLUMN kb_team_public_within.anchor_id IS
    'A: a strict ancestor of T whose subtree reads them.';
COMMENT ON COLUMN kb_team_public_within.opened_by IS
    'Who opened the row. Required on insert (the shape guard); NULL only once that profile is '
    'deleted, when the ledger record keeps the act.';
COMMENT ON COLUMN kb_team_public_within.admin_act IS
    'A system admin stood in for the anchor side (P2 rule 3). Never for the T side (R-k).';
COMMENT ON COLUMN kb_team_public_within.instance_wide IS
$c$Opened by an admin over an anchor whose subtree holds an auto-join or gating team (P2 rule 5).
Only such a row survives a pool team under A at read time (PW-1).$c$;

CREATE FUNCTION team_is_root_or_gating(p_team uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM kb_teams t
         WHERE t.id = p_team
           AND ( t.slug = 'temper-system'
              OR t.slug = (SELECT s.gating_team_slug FROM kb_system_settings s WHERE s.id = 1)));
$$;

COMMENT ON FUNCTION team_is_root_or_gating(uuid) IS
$c$The root (temper-system) or the instance's current gating team: the SQL twin of
access_service::is_root_or_gating_team. public_within's shape guard refuses such a team as T at
write time, and public_within_row_effective refuses it at read time, through this one predicate.$c$;

CREATE FUNCTION kb_team_public_within_shape_guard() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    IF current_setting('temper.replaying', true) = 'on' THEN
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

COMMENT ON FUNCTION kb_team_public_within_shape_guard() IS
$c$Refuses a public_within row whose team is personal, the root (temper-system) or the gating team,
or whose anchor is personal (spec P1, P2 rule 8, PW-13), and an insert that names no opener. The
guard runs on an insert and on a change of team or anchor, never on any other update (so the
opened_by SET NULL cascade and a later flag change pass), and not under replay (temper.replaying),
which restores rows as they stand: a row whose opener was since deleted, or whose team has since
become the gating team, is restored, not re-judged (the kb_events precedent). The constraint name on each refusal is
what Rust renders the reason from. The anchor may be the root or the gating team: that is an
admin's open (P2 rule 4), decided by the door, not here.$c$;

CREATE TRIGGER kb_team_public_within_shape
    BEFORE INSERT OR UPDATE OF team_id, anchor_id ON kb_team_public_within
    FOR EACH ROW EXECUTE FUNCTION kb_team_public_within_shape_guard();

-- ── 3. The helpers ────────────────────────────────────────────────────────────────────────────────
CREATE FUNCTION team_ancestors_any(p_team uuid)
RETURNS TABLE(team_id uuid) LANGUAGE sql STABLE AS $$
    WITH RECURSIVE up AS (
        SELECT t.id AS team_id
        FROM kb_teams t
        WHERE t.id = p_team
        UNION
        SELECT tp.parent_id
        FROM kb_teams_parents tp
        JOIN up ON tp.child_id = up.team_id
    )
    SELECT team_id FROM up;
$$;

COMMENT ON FUNCTION team_ancestors_any(uuid) IS
$c$The team DAG walked UP with NO liveness floor: p_team itself, then every parent through every
link, live or soft-deleted. It exists for exactly one caller, public_within_allowed_over, because
a forbid is a restriction and a restriction fails closed: soft-deleting the forbidding team, or a
team between it and T, must not lift it (spec P4, PW-2/F5).

NEVER USE IT FOR REACH. A dead team confers nothing (team_ancestors' is_active floor); a walk that
passes through dead teams would let a soft-delete widen what anyone reads.$c$;

CREATE FUNCTION public_within_allowed_over(p_team uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE
        WHEN COALESCE((SELECT s.public_within_enabled FROM kb_system_settings s WHERE s.id = 1), false)
        THEN NOT EXISTS (
            SELECT 1
              FROM team_ancestors_any(p_team) a
              JOIN kb_teams t ON t.id = a.team_id
             WHERE t.public_within_forbidden)
        ELSE false
    END;
$$;

COMMENT ON FUNCTION public_within_allowed_over(uuid) IS
$c$Policy allows a public_within row for team p_team (spec P4): the instance switch is on, and no team
in team_ancestors_any(p_team), p_team included, live or dead, forbids. T's chain, not A's: A is an
ancestor of T, so T's chain covers A's, a forbid between A and T, and a forbid on T's second
parent (PW-2). The switch is read first, inside a CASE so the order is guaranteed: a born-denied
instance pays one settings read per row and never walks the tree (PW-19).$c$;

CREATE FUNCTION anchor_subtree_holds_pool(p_anchor uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
          FROM kb_teams t
         WHERE (t.id = p_anchor OR t.id IN (SELECT d.team_id FROM team_live_descendants(p_anchor) d))
           AND t.slug = (SELECT s.gating_team_slug FROM kb_system_settings s WHERE s.id = 1));
$$;

COMMENT ON FUNCTION anchor_subtree_holds_pool(uuid) IS
$c$True when the anchor itself, or a live team beneath it, is the gating team (PW-1); 20261120000005
also counts an auto-join team. Such a team brings the whole instance into the anchor's subtree, so a public_within row
over it would read instance-wide; only an admin's instance_wide row survives it
(public_within_row_effective). A pool team below a dead team is not counted, because it confers
nothing upward (team_live_descendants does not walk through a dead team). The auto-join arm
is kept apart because it reads kb_teams.auto_join_role, which temper-substrate's pre-auto-join test
baseline omits.$c$;

CREATE FUNCTION public_within_row_effective(p_team uuid, p_anchor uuid, p_instance_wide boolean)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE
        WHEN public_within_allowed_over(p_team) THEN
                p_team <> p_anchor
            AND NOT team_is_root_or_gating(p_team)
            AND EXISTS (SELECT 1 FROM team_ancestors(p_team) a WHERE a.team_id = p_anchor)
            AND (p_instance_wide OR NOT anchor_subtree_holds_pool(p_anchor))
        ELSE false
    END;
$$;

COMMENT ON FUNCTION public_within_row_effective(uuid, uuid, boolean) IS
$c$Does one public_within row confer right now (spec P5, N9)? All of:
  * policy allows it over the team (public_within_allowed_over: the switch, then no forbid above T),
    evaluated first, inside a CASE, so a switched-off instance never walks the tree;
  * the team is not the root or the gating team: the shape trigger refuses that at write time, and
    this keeps a later gating-slug change from making such a row confer;
  * the anchor is a strict ancestor of the team along a LIVE chain: team_ancestors returns T only if
    T is live and walks only through live teams, so T dead, A dead, or every path between them dead
    each makes the row dormant (liveness);
  * the anchor's live subtree holds no auto-join or gating team, unless the row is an admin's
    instance_wide row (PW-1).
Every condition is read-time, so an act that breaks one makes the row inert at once, whatever the
open-time checks said.$c$;

CREATE FUNCTION contexts_public_within(p_profile uuid, p_teams uuid[])
RETURNS TABLE(context_id uuid) LANGUAGE sql STABLE AS $$
    WITH candidates AS MATERIALIZED (
        SELECT pw.team_id
          FROM kb_team_public_within pw
         WHERE pw.anchor_id = ANY(p_teams)
           AND public_within_row_effective(pw.team_id, pw.anchor_id, pw.instance_wide)
    )
    SELECT c.id
      FROM candidates cand
      JOIN kb_contexts c
        ON c.owner_table = 'kb_teams' AND c.owner_id = cand.team_id AND c.is_active;
$$;

COMMENT ON FUNCTION contexts_public_within(uuid, uuid[]) IS
$c$Arm 5 of the context read set (spec P6): the contexts OWNED by every team T whose public_within row
is anchored on a team in p_teams (the caller's reachable closure) and is effective. Owned contexts
only (R-b): never a context shared to T, a grant held by T or a map bound to T (N8). Not inherited
(R-a): T's children stay private. A retired context is floored by is_active; a context of a dead
team never appears, because public_within_row_effective requires T live.

The candidate rows are hoisted into a MATERIALIZED CTE so the per-row effect checks run once per
row, never per context (PW-19). p_profile is carried for the read-set signature. It applies NO reader
fence itself: every caller must, and both do (_contexts_readable_by_teams through p_reading,
resources_public_within through public_within_reading). Do not call it from anywhere else.$c$;

CREATE FUNCTION public_within_reading(p_profile uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT CASE
        WHEN COALESCE((SELECT s.public_within_enabled FROM kb_system_settings s WHERE s.id = 1), false)
        THEN EXISTS (
                 SELECT 1 FROM kb_profile_auth_links l
                  WHERE l.profile_id = p_profile
                    AND l.auth_provider <> 'auth0-m2m')
         AND NOT EXISTS (SELECT 1 FROM kb_machine_clients m WHERE m.profile_id = p_profile)
         AND NOT EXISTS (SELECT 1 FROM kb_connections k WHERE k.profile_id = p_profile)
        ELSE false
    END;
$$;

COMMENT ON FUNCTION public_within_reading(uuid) IS
$c$The machine fence (spec P6, PW-4, R-n). False whenever the instance switch is off, read first so
that every read-set call on a born-denied instance pays one settings read and nothing more. With it
on, true only for a PERSON: a profile with a human auth link
(any kb_profile_auth_links row whose provider is not the machine provider auth0-m2m) and with no
kb_machine_clients row (revoked or not; the same test as is_machine_profile) and no kb_connections
row (a connection's agent profile). Everything else is false, fail-closed: machine clients,
connection agents, stewards and auditors (machine clients themselves), and any profile with no
human auth link.

Keyed on the profile and the instance setting alone (R-n): no GUC and no request-scoped state. The fence is the principal,
never the token or the surface, so a person reads with any token that identifies them, including
the Slack mention path's act-as-the-human mint.$c$;

-- ── 4. The admin event types ──────────────────────────────────────────────────────────────────────
-- The payload schema literals below are the committed fixtures
-- crates/temper-substrate/tests/fixtures/payloads/{team_public_within_changed,
-- public_within_effect_changed,public_within_policy_changed}.v1.schema.json, in that order, and
-- `the_migration_literal_matches_the_committed_fixture` pins each pair.
INSERT INTO kb_event_types (name, payload_schema, schema_version, category) VALUES
  ('team_public_within_changed', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "TeamPublicWithinChanged",
  "description": "`team_public_within_changed` — a `public_within` row was opened, closed, lapsed or cascade-deleted\n(public within, spec P2, P3, P5). The row lets the people under the anchor team read the team's\nowned contexts. Both the team (T) and the anchor (A) are the event's `subject` references, so the\nmanagers of either side read it on the subject axis.",
  "type": "object",
  "properties": {
    "act": {
      "$ref": "#/$defs/PublicWithinAct"
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
    "admin_act": {
      "description": "True when a system admin stood in for the anchor side.",
      "type": "boolean"
    },
    "anchor_id": {
      "description": "The ancestor team whose subtree reads them (A).",
      "type": "string",
      "format": "uuid"
    },
    "instance_wide": {
      "description": "True when the row survives a pool team under the anchor (an admin's open, spec P2 rule 5).",
      "type": [
        "boolean",
        "null"
      ]
    },
    "team_id": {
      "description": "The team whose owned contexts the row opens (T).",
      "type": "string",
      "format": "uuid"
    }
  },
  "required": [
    "team_id",
    "anchor_id",
    "act",
    "admin_act"
  ],
  "$defs": {
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    },
    "PublicWithinAct": {
      "description": "What happened to the row in a [`TeamPublicWithinChanged`] record.",
      "oneOf": [
        {
          "description": "A manager of the team and a manager of the anchor (or an admin for it) opened the row.",
          "type": "string",
          "const": "opened"
        },
        {
          "description": "A manager of either side, or an admin, closed the row.",
          "type": "string",
          "const": "closed"
        },
        {
          "description": "The anchor stopped being an ancestor of the team by any path; the row was deleted and\nnever revives.",
          "type": "string",
          "const": "lapsed"
        },
        {
          "description": "The team or the anchor was erased or hard-deleted, taking the row with it.",
          "type": "string",
          "const": "cascade_deleted"
        }
      ]
    }
  }
}
$JS$::jsonb, 1, 'admin'),
  ('public_within_effect_changed', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "PublicWithinEffectChanged",
  "description": "`public_within_effect_changed` — a `public_within` row started or stopped conferring, without\nbeing opened or closed (spec P5). The cause is liveness (a team on the path was deleted or\nrestored) or policy (the instance switch, a forbid, or a pool team under the anchor). Both the\nteam and the anchor are `subject` references.",
  "type": "object",
  "properties": {
    "act": {
      "description": "The admin event type of the act that caused the change, such as `team_lifecycle_changed`.",
      "type": [
        "string",
        "null"
      ]
    },
    "act_ref": {
      "description": "The correlation id shared by the causing act's records, when it has one.",
      "type": [
        "string",
        "null"
      ],
      "format": "uuid"
    },
    "anchor_id": {
      "type": "string",
      "format": "uuid"
    },
    "cause": {
      "$ref": "#/$defs/PublicWithinCause"
    },
    "from": {
      "$ref": "#/$defs/PublicWithinEffect"
    },
    "team_id": {
      "type": "string",
      "format": "uuid"
    },
    "to": {
      "$ref": "#/$defs/PublicWithinEffect"
    }
  },
  "required": [
    "team_id",
    "anchor_id",
    "from",
    "to",
    "cause"
  ],
  "$defs": {
    "PublicWithinCause": {
      "description": "Why a row's effect changed, in a [`PublicWithinEffectChanged`] record.",
      "oneOf": [
        {
          "description": "The team, the anchor, or every live path between them, was soft-deleted or restored.",
          "type": "string",
          "const": "liveness"
        },
        {
          "description": "The instance switch, a forbid above the team, or a pool team under the anchor.",
          "type": "string",
          "const": "policy"
        }
      ]
    },
    "PublicWithinEffect": {
      "description": "Whether a `public_within` row confers, in a [`PublicWithinEffectChanged`] record.",
      "oneOf": [
        {
          "description": "The row confers reading on the anchor's subtree.",
          "type": "string",
          "const": "effective"
        },
        {
          "description": "The row exists and confers nothing until its cause is undone.",
          "type": "string",
          "const": "dormant"
        }
      ]
    }
  }
}
$JS$::jsonb, 1, 'admin'),
  ('public_within_policy_changed', $JS$
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "PublicWithinPolicyChanged",
  "description": "`public_within_policy_changed` — the instance switch (`public_within_enabled`) or a team's forbid\n(`public_within_forbidden`) changed (spec P4). The only record of the instance switch: settings\nupdates are otherwise not ledgered. A team-scope record carries the team as its `subject`\nreference; an instance-scope record carries none, and is read by system admins.",
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
    "from": {
      "description": "The value before: the switch's `enabled`, or the team's `forbidden`.",
      "type": "boolean"
    },
    "scope": {
      "$ref": "#/$defs/PublicWithinPolicyScope"
    },
    "team_id": {
      "description": "The team whose forbid changed; absent for the instance switch.",
      "type": [
        "string",
        "null"
      ],
      "format": "uuid"
    },
    "to": {
      "description": "The value after.",
      "type": "boolean"
    }
  },
  "required": [
    "scope",
    "from",
    "to"
  ],
  "$defs": {
    "ProfileId": {
      "description": "A `kb_profiles.id` value.",
      "type": "string",
      "format": "uuid"
    },
    "PublicWithinPolicyScope": {
      "description": "Which policy a [`PublicWithinPolicyChanged`] record is about.",
      "oneOf": [
        {
          "description": "`kb_system_settings.public_within_enabled`, set by a system admin.",
          "type": "string",
          "const": "instance"
        },
        {
          "description": "`kb_teams.public_within_forbidden`, set by the team's owner or an admin.",
          "type": "string",
          "const": "team"
        }
      ]
    }
  }
}
$JS$::jsonb, 1, 'admin');

-- ── 5. The instance switch's writer ───────────────────────────────────────────────────────────────
CREATE FUNCTION _admin_public_within_switch_write(
    p_emitter     uuid,
    p_actor       uuid,
    p_to          boolean,
    p_correlation uuid DEFAULT NULL
) RETURNS text
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_from boolean;
BEGIN
    SELECT s.public_within_enabled INTO v_from FROM kb_system_settings s WHERE s.id = 1 FOR UPDATE;
    IF NOT FOUND THEN
        RETURN 'absent';
    END IF;
    IF v_from = p_to THEN
        RETURN 'noop';
    END IF;
    UPDATE kb_system_settings SET public_within_enabled = p_to, updated = now() WHERE id = 1;

    PERFORM _event_append(
        'public_within_policy_changed', p_emitter, NULL, NULL,
        jsonb_strip_nulls(jsonb_build_object(
            'scope', 'instance',
            'from',  v_from,
            'to',    p_to,
            'actor', p_actor)),
        p_correlation => p_correlation);

    RETURN 'changed';
END;
$$;

COMMENT ON FUNCTION _admin_public_within_switch_write(uuid, uuid, boolean, uuid) IS
$c$Set the public_within instance switch and append its public_within_policy_changed record (scope
instance), atomically, under the settings row lock. Returns changed | noop | absent; only changed
writes or emits. The record names no team, so only system admins read it. It NEVER AUTHORIZES:
access_service::update_system_settings holds the SystemAdmin proof before calling it. Phase 4 hooks
the effect transitions here (R-l).$c$;

SELECT declare_migration(
    20261120000000,
    'additive',
    'public_within Phase 1: adds kb_system_settings.public_within_enabled (default false, no '
    'backfill), kb_teams.public_within_forbidden (default false), the kb_team_public_within table '
    'with its shape trigger, the helpers team_ancestors_any, public_within_allowed_over, '
    'anchor_subtree_holds_pool, team_is_root_or_gating, public_within_row_effective, '
    'contexts_public_within and public_within_reading, the writer _admin_public_within_switch_write, and three admin event '
    'types. New columns have defaults and nothing reads them yet, so an older binary is '
    'unaffected; no existing function changes. Plan: temper-artifacts '
    'plans/2026-11-06-public-within.md Phase 1.'
);
