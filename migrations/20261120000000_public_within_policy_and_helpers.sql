-- public_within: the schema, the policy and the helpers. Nothing reads through them yet.
--
-- Design: temper-artifacts specs/2026-11-06-public-within-design.md.
--
-- A `public_within(T, A)` row lets the people under the anchor team A read the contexts T owns. This
-- migration adds the row, the forbid over it, and the predicates that say whether a row confers; the
-- read arm that consults them is 20261120000010. With no row, every read set answers as before.
-- There is no instance setting: a forbid on the root is the admin's instance-wide off.
--
--   kb_teams.public_within_forbidden           a forbid over everything beneath a team; on the root,
--                                              over every team.
--   kb_team_public_within                      the rows; a trigger and CHECKs refuse barred shapes.
--   team_is_root_or_gating, team_ancestors_any, public_within_allowed_over,
--   anchor_subtree_holds_pool (20261120000005 adds its auto-join arm), public_within_row_effective,
--   contexts_public_within, public_within_reading   the predicates, one COMMENT ON each.
--   team_public_within_changed, public_within_effect_changed, public_within_policy_changed.
--
-- `team_live_descendants` (20261119000020) is reused, not redefined. Every rationale lives in
-- COMMENT ON FUNCTION, never in a body: the guard scans read pg_get_functiondef.

-- ── 1. The forbid ─────────────────────────────────────────────────────────────────────────────────
ALTER TABLE kb_teams
    ADD COLUMN public_within_forbidden boolean NOT NULL DEFAULT false;

COMMENT ON COLUMN kb_teams.public_within_forbidden IS
$c$A forbid: no public_within row confers for any team beneath this one, by any path,
live or dead (public_within_allowed_over walks team_ancestors_any). A lower owner cannot lift it;
only leaving the subtree does. On the root (temper-system) it is the instance-wide off: every
row, an admin's instance_wide row included, confers nothing while it is set, whether or not the
team's tree is linked under the root (public_within_allowed_over reads the root's flag directly,
since anyone may create a parentless team). Only a system admin may set it on the root, and an
owner elsewhere; never on a personal team. No door sets it yet; the one that does will enforce
those rules, not this column.$c$;

-- ── 2. The rows ───────────────────────────────────────────────────────────────────────────────────
CREATE TABLE kb_team_public_within (
    team_id       uuid        NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    anchor_id     uuid        NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    opened_by     uuid            NULL REFERENCES kb_profiles(id) ON DELETE SET NULL,
    opened_at     timestamptz NOT NULL DEFAULT now(),
    admin_act     boolean     NOT NULL DEFAULT false,
    instance_wide boolean     NOT NULL DEFAULT false,
    PRIMARY KEY (team_id, anchor_id),
    CONSTRAINT kb_team_public_within_not_self CHECK (team_id <> anchor_id),
    CONSTRAINT kb_team_public_within_instance_wide_is_admin CHECK (NOT instance_wide OR admin_act)
);

CREATE INDEX idx_kb_team_public_within_anchor ON kb_team_public_within (anchor_id);

COMMENT ON TABLE kb_team_public_within IS
$c$public_within(T, A): the people under anchor team A read the contexts team T owns, while the row
is effective (public_within_row_effective). Opened by both sides, closed by either. A structural
lapse deletes the row; liveness and policy causes leave it dormant. Owned contexts only: a share to
T, a grant held by T and a map bound to T are never exposed.$c$;
COMMENT ON COLUMN kb_team_public_within.team_id IS 'T: the team whose owned contexts are opened.';
COMMENT ON COLUMN kb_team_public_within.anchor_id IS
    'A: a strict ancestor of T whose subtree reads them.';
COMMENT ON COLUMN kb_team_public_within.opened_by IS
    'Who opened the row. Required on insert (the shape guard); NULL only once that profile is '
    'deleted, when the ledger record keeps the act.';
COMMENT ON COLUMN kb_team_public_within.admin_act IS
    'A system admin stood in for the anchor side. An admin never stands in for the T side.';
COMMENT ON COLUMN kb_team_public_within.instance_wide IS
$c$Opened by an admin over an anchor that is the root or whose subtree holds an auto-join or gating
team. Only such a row survives a pool under A at read time. Implies admin_act (a CHECK).$c$;

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
or whose anchor is personal, and an insert that names no opener. The
guard runs on an insert and on a change of team or anchor, never on any other update (so the
opened_by SET NULL cascade and a later flag change pass), and not under replay (temper.replaying),
which restores rows as they stand: a row whose opener was since deleted, or whose team has since
become the gating team, is restored, not re-judged (the kb_events precedent). The constraint name on each refusal is
what Rust renders the reason from. The anchor may be the root or the gating team: that is an
admin's open, decided by the door, not here.$c$;

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
team between it and T, must not lift it.

NEVER USE IT FOR REACH. A forbid counts on every ancestor, live or dead; reach counts live teams
only (team_ancestors' is_active floor), because a dead team confers nothing.$c$;

CREATE FUNCTION public_within_allowed_over(p_team uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT NOT EXISTS (
        SELECT 1
          FROM team_ancestors_any(p_team) a
          JOIN kb_teams t ON t.id = a.team_id
         WHERE t.public_within_forbidden)
       AND NOT EXISTS (
        SELECT 1 FROM kb_teams r WHERE r.slug = 'temper-system' AND r.public_within_forbidden);
$$;

COMMENT ON FUNCTION public_within_allowed_over(uuid) IS
$c$Policy allows a public_within row for team p_team: no team in team_ancestors_any(p_team),
p_team included, live or dead, forbids. T's chain, not A's: A is an ancestor of T, so T's chain
covers A's, a forbid between A and T, and a forbid on T's second parent. A forbid on the root (the
instance-wide off) is read directly as well as along the walk: a parentless team's chain never
reaches the root, and the off must reach it too. It runs only for a candidate row.$c$;

CREATE FUNCTION anchor_subtree_holds_pool(p_anchor uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
          FROM kb_teams t
         WHERE (t.id = p_anchor OR t.id IN (SELECT d.team_id FROM team_live_descendants(p_anchor) d))
           AND ( t.slug = 'temper-system'
              OR t.slug = (SELECT s.gating_team_slug FROM kb_system_settings s WHERE s.id = 1)));
$$;

COMMENT ON FUNCTION anchor_subtree_holds_pool(uuid) IS
$c$True when the anchor is the root (temper-system), or the anchor itself or a live team beneath it
is the gating team; 20261120000005 also counts an auto-join team. Each brings the whole instance
into the anchor's subtree (every personal team sits under the root), so a public_within row over it
would read instance-wide; only an admin's instance_wide row survives it
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
$c$Does one public_within row confer right now? All of:
  * policy allows it over the team (public_within_allowed_over: no forbid above T, the root's
    included), evaluated first, inside a CASE;
  * the team is not the root or the gating team: the shape trigger refuses that at write time, and
    this keeps a later gating-slug change from making such a row confer;
  * the anchor is a strict ancestor of the team along a LIVE chain: team_ancestors returns T only if
    T is live and walks only through live teams, so T dead, A dead, or every path between them dead
    each makes the row dormant (liveness);
  * the anchor is not the root and its live subtree holds no auto-join or gating team
    (anchor_subtree_holds_pool), unless the row is an admin's instance_wide row.
Every condition is read-time, so an act that breaks one makes the row inert at once, whatever the
open-time checks said.$c$;

CREATE FUNCTION public_within_reading(p_profile uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
               SELECT 1 FROM kb_profile_auth_links l
                WHERE l.profile_id = p_profile
                  AND l.auth_provider <> 'auth0-m2m')
       AND NOT EXISTS (SELECT 1 FROM kb_machine_clients m WHERE m.profile_id = p_profile)
       AND NOT EXISTS (SELECT 1 FROM kb_connections k WHERE k.profile_id = p_profile);
$$;

COMMENT ON FUNCTION public_within_reading(uuid) IS
$c$The machine fence: machine principals never read through public_within. True only for a
PERSON: a profile with a human auth link
(any kb_profile_auth_links row whose provider is not the machine provider auth0-m2m) and with no
kb_machine_clients row (revoked or not; the same test as is_machine_profile) and no kb_connections
row (a connection's agent profile). Everything else is false, fail-closed: machine clients,
connection agents, stewards and auditors (machine clients themselves), and any profile with no
human auth link.

Keyed on the profile alone: no GUC, no setting and no request-scoped state. Called only by
contexts_public_within, and only when a candidate row exists. The fence is the principal,
never the token or the surface, so a person reads with any token that identifies them, including
the Slack mention path's act-as-the-human mint.$c$;

CREATE FUNCTION contexts_public_within(p_profile uuid, p_teams uuid[])
RETURNS TABLE(context_id uuid) LANGUAGE plpgsql STABLE ROWS 10 AS $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM kb_team_public_within pw WHERE pw.anchor_id = ANY(p_teams)) THEN
        RETURN;
    END IF;
    IF NOT public_within_reading(p_profile) THEN
        RETURN;
    END IF;
    RETURN QUERY
    SELECT c.id
      FROM kb_team_public_within pw
      JOIN kb_contexts c
        ON c.owner_table = 'kb_teams' AND c.owner_id = pw.team_id AND c.is_active
     WHERE pw.anchor_id = ANY(p_teams)
       AND public_within_row_effective(pw.team_id, pw.anchor_id, pw.instance_wide)
       AND NOT EXISTS (SELECT 1 FROM kb_connections k WHERE k.home_context_id = c.id);
END;
$$;

COMMENT ON FUNCTION contexts_public_within(uuid, uuid[]) IS
$c$The public_within arm of the context read set: the owned, live, non-intake contexts of every team T
whose public_within row is anchored on a team in p_teams (the caller's reachable closure) and is
effective. Owned contexts only: never a context shared to T, a grant held by T or a map bound to T.
Not inherited: T's children stay private. A retired context is floored by is_active; a context of
a dead team never appears, because public_within_row_effective requires T live. A connection's
home context (its intake: what an external system sends it) is never opened, in any connection
state, revoked included: the exclusion keys on kb_connections.home_context_id. Each connection's
home is a context created for it alone, so no ordinary working context is affected, and T's own
members read the intake exactly as before.

THE MACHINE FENCE LIVES HERE, and it is lazy. The first statement asks for a candidate row (a row
anchored on a team in the closure: an index probe on anchor_id). Only when one exists is the reader
fence (public_within_reading(p_profile)) evaluated, once, and only for a person are the per-row
effect checks run. With no candidate row, a read costs that one probe: no fence call, no tree walk
and no context scan. PL/pgSQL, not SQL, so that callers do not inline (and plan, on every call) a
body that almost always returns at its first statement. Every path through the public_within arm comes through here, so a machine reads nothing
through it whatever rows exist.$c$;

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
  "description": "`team_public_within_changed` — a `public_within` row was opened, closed, lapsed or cascade-deleted.\nThe row lets the people under the anchor team read the team's owned contexts. Both the team (T)\nand the anchor (A) are the event's `subject` references, so the managers of either side read it\non the subject axis.",
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
      "description": "True when the row survives a pool team under the anchor (only an admin opens such a row).",
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
  "description": "`public_within_effect_changed` — a `public_within` row started or stopped conferring, without\nbeing opened or closed. The cause is liveness (a team on the path was deleted or restored) or\npolicy (a forbid, the root's included, or a pool at the anchor). Both the team and the\nanchor are `subject` references.",
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
          "description": "A forbid above the team (the root's included), or a pool at the anchor.",
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
  "description": "`public_within_policy_changed` — a team's forbid (`public_within_forbidden`) changed. A forbid\non the root (`temper-system`) is the instance-wide off: it makes every row confer nothing,\nwhether or not the team's tree is linked under the root. The record carries the team as its\n`subject` reference.",
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
      "description": "The forbid before.",
      "type": "boolean"
    },
    "scope": {
      "$ref": "#/$defs/PublicWithinPolicyScope"
    },
    "team_id": {
      "description": "The team whose forbid changed.",
      "type": "string",
      "format": "uuid"
    },
    "to": {
      "description": "The value after.",
      "type": "boolean"
    }
  },
  "required": [
    "scope",
    "team_id",
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
          "description": "`kb_teams.public_within_forbidden`, set by the team's owner or an admin; on the root, by a\nsystem admin only.",
          "type": "string",
          "const": "team"
        }
      ]
    }
  }
}
$JS$::jsonb, 1, 'admin');

SELECT declare_migration(
    20261120000000,
    'additive',
    'public_within schema, policy and helpers: adds kb_teams.public_within_forbidden (default '
    'false; on the root, the instance-wide off), the kb_team_public_within table with its shape '
    'trigger and CHECKs, the helpers team_ancestors_any, public_within_allowed_over, '
    'anchor_subtree_holds_pool, team_is_root_or_gating, public_within_row_effective, '
    'contexts_public_within and public_within_reading, and three admin event types. The new column '
    'has a default and no existing function changes, so an older binary is unaffected. A row needs '
    'both sides; with none, nothing reads through the arm.'
);
