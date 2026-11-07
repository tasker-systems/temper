-- public_within: the doors. Until now a row existed only if a test wrote it; this migration adds
-- the only writers: open, close, the forbid, and the record a hard delete leaves.
--
-- Design: temper-artifacts specs/2026-11-06-public-within-design.md (P2, P3, P4, P9).
--
--   _admin_public_within_write        opens (INSERT) or closes (DELETE) one row and appends its
--                                     team_public_within_changed record, in one statement.
--   _admin_public_within_policy_write sets or clears kb_teams.public_within_forbidden, appends
--                                     public_within_policy_changed, and one public_within_effect_changed
--                                     per row whose effect the change flips, under the rows' locks.
--   _public_within_open_locks         the open's lock set, taken in ascending id order; false when
--                                     the team gained an ancestor while the open waited.
--   public_within_audience            how many teams and people a row over an anchor reads to.
--   kb_teams_public_within_cascade    a hard-deleted team records cascade_deleted for each of its
--                                     rows before the foreign-key cascade removes them.
--
-- NONE OF THE WRITERS AUTHORIZES. Who may open, close or forbid is decided in Rust
-- (team_service), twice: on the pool, and again under these locks. What a later edit must not
-- break: the open takes a SHARE lock on every team whose forbid can reach T (team_ancestors_any(T)
-- and the root), and the forbid writer locks its team's row before it reads anything, so an open
-- and a forbid over the same team always serialize, in either order. Every rationale is in
-- COMMENT ON FUNCTION, never in a body.

-- ── 1. Open and close ─────────────────────────────────────────────────────────────────────────────
CREATE FUNCTION _admin_public_within_write(
    p_emitter       uuid,
    p_actor         uuid,
    p_team          uuid,
    p_anchor        uuid,
    p_act           text,
    p_admin_act     boolean,
    p_instance_wide boolean DEFAULT false,
    p_correlation   uuid DEFAULT NULL
) RETURNS text
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_instance_wide boolean;
BEGIN
    IF p_act = 'opened' THEN
        INSERT INTO kb_team_public_within (team_id, anchor_id, opened_by, admin_act, instance_wide)
        VALUES (p_team, p_anchor, p_actor, p_admin_act, p_instance_wide)
        ON CONFLICT (team_id, anchor_id) DO NOTHING
        RETURNING instance_wide INTO v_instance_wide;
        IF NOT FOUND THEN
            RETURN 'exists';
        END IF;
    ELSIF p_act = 'closed' THEN
        DELETE FROM kb_team_public_within
         WHERE team_id = p_team AND anchor_id = p_anchor
        RETURNING instance_wide INTO v_instance_wide;
        IF NOT FOUND THEN
            RETURN 'absent';
        END IF;
    ELSE
        RAISE EXCEPTION '_admin_public_within_write: p_act must be opened or closed, got %', p_act;
    END IF;

    PERFORM _event_append(
        'team_public_within_changed', p_emitter, NULL, NULL,
        jsonb_strip_nulls(jsonb_build_object(
            'team_id',       p_team,
            'anchor_id',     p_anchor,
            'act',           p_act,
            'admin_act',     p_admin_act,
            'instance_wide', v_instance_wide,
            'actor',         p_actor)),
        p_references => jsonb_build_array(
            jsonb_build_object('rel', 'subject',
                               'target', jsonb_build_object('kind', 'kb_teams', 'id', p_team)),
            jsonb_build_object('rel', 'subject',
                               'target', jsonb_build_object('kind', 'kb_teams', 'id', p_anchor))),
        p_correlation => p_correlation);

    RETURN p_act;
END;
$$;

COMMENT ON FUNCTION _admin_public_within_write(uuid, uuid, uuid, uuid, text, boolean, boolean, uuid) IS
$c$Open or close one public_within row and append its team_public_within_changed admin event,
atomically. opened inserts (opened_by = p_actor, admin_act and instance_wide as given) and answers
opened, or exists when the row is already there (nothing written or emitted). closed deletes and
answers closed, or absent when there is no such row (nothing written or emitted). The record names
BOTH the team and the anchor as subject references, so the managers of either side read it.
admin_act on a close says a system admin closed the row rather than a manager of either side.

IT NEVER AUTHORIZES: team_service decides who may open or close, before calling it, and again
under the open's locks (_public_within_open_locks) or the close's (both team rows).$c$;

-- ── 2. The open's locks ───────────────────────────────────────────────────────────────────────────
CREATE FUNCTION _public_within_open_locks(p_team uuid, p_anchor uuid)
RETURNS boolean
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_locked uuid[];
    v_id     uuid;
BEGIN
    v_locked := ARRAY(
        SELECT DISTINCT s.id
          FROM (SELECT p_team AS id
                UNION SELECT p_anchor
                UNION SELECT a.team_id FROM team_ancestors_any(p_team) a
                UNION SELECT r.id FROM kb_teams r WHERE r.slug = 'temper-system') s
         ORDER BY s.id);
    FOREACH v_id IN ARRAY v_locked
    LOOP
        IF v_id = p_team OR v_id = p_anchor THEN
            PERFORM 1 FROM kb_teams WHERE id = v_id FOR NO KEY UPDATE;
        ELSE
            PERFORM 1 FROM kb_teams WHERE id = v_id FOR SHARE;
        END IF;
    END LOOP;
    PERFORM _gating_team_lock(false);
    RETURN NOT EXISTS (
        SELECT 1 FROM team_ancestors_any(p_team) a WHERE a.team_id <> ALL (v_locked));
END;
$$;

COMMENT ON FUNCTION _public_within_open_locks(uuid, uuid) IS
$c$The locks an open takes before it decides again, in ascending id order: FOR NO KEY UPDATE on the
team and the anchor (the lock every membership writer takes, so a demotion cannot race the open),
and FOR SHARE on every other team whose forbid can reach the team: team_ancestors_any(team), live
or dead, and the root, whose forbid is the instance-wide off. Then the shared gating-team lock,
after the rows, in the order every other team door takes it.

The SHARE locks are what make the forbid race-free. _admin_public_within_policy_write locks its
team's row FOR NO KEY UPDATE before it reads anything, which conflicts with SHARE: a forbid
committed first is seen by the open's re-check; an open committed first is seen by the forbid's
transition scan, and recorded.

Answers false when the team's ancestors, read again once the locks are held, include a team that is
not locked: a link committed while the open waited gave the team a new ancestor whose forbid would
not serialize with this open. The caller then refuses with a retryable conflict and writes nothing.
That re-read is sufficient because every new ancestor link names a child that is the team or one of
its ancestors, and each of those is locked here, as every link writer locks the child. Any other
concurrent change to the tree is not serialized here; every structural condition is also checked at
read time (public_within_row_effective).$c$;

-- ── 3. The audience ───────────────────────────────────────────────────────────────────────────────
CREATE FUNCTION public_within_audience(p_anchor uuid)
RETURNS TABLE(teams bigint, principals bigint)
LANGUAGE sql STABLE AS $$
    WITH sub AS MATERIALIZED (
        SELECT d.team_id FROM team_live_descendants(p_anchor) d
    )
    SELECT (SELECT count(*) FROM sub),
           (SELECT count(DISTINCT m.profile_id)
              FROM kb_team_members m
              JOIN sub ON sub.team_id = m.team_id
             WHERE public_within_reading(m.profile_id));
$$;

COMMENT ON FUNCTION public_within_audience(uuid) IS
$c$Who an effective public_within row over p_anchor reads to, as counts: the live teams in the
anchor's subtree (the anchor itself, if live, and every team below it through live links), and the
distinct PEOPLE on them (public_within_reading: a machine or a connection agent never reads through
the arm, so it is not counted). The same reach rule as the read arm: a reader holds the anchor in
profile_reachable_teams exactly when they sit on one of these teams. A count, never a list: T's
members watch it grow without learning who is under the anchor.$c$;

-- ── 4. The forbid ─────────────────────────────────────────────────────────────────────────────────
CREATE FUNCTION _admin_public_within_policy_write(
    p_emitter     uuid,
    p_actor       uuid,
    p_team        uuid,
    p_forbidden   boolean,
    p_correlation uuid DEFAULT NULL
) RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_from        boolean;
    v_is_root     boolean;
    v_correlation uuid := coalesce(p_correlation, uuid_generate_v7());
    v_before      jsonb;
    v_changes     jsonb := '[]'::jsonb;
    v_row         record;
    v_system      uuid;
BEGIN
    SELECT t.public_within_forbidden, t.slug = 'temper-system'
      INTO v_from, v_is_root
      FROM kb_teams t
     WHERE t.id = p_team AND t.is_active
       FOR NO KEY UPDATE;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('outcome', 'absent');
    END IF;
    IF v_from = p_forbidden THEN
        RETURN jsonb_build_object('outcome', 'noop', 'changes', '[]'::jsonb);
    END IF;

    PERFORM 1
       FROM kb_team_public_within pw
      WHERE v_is_root
         OR EXISTS (SELECT 1 FROM team_ancestors_any(pw.team_id) a WHERE a.team_id = p_team)
      ORDER BY pw.team_id, pw.anchor_id
        FOR UPDATE;

    SELECT coalesce(jsonb_object_agg(pw.team_id::text || ':' || pw.anchor_id::text,
                                     public_within_row_effective(pw.team_id, pw.anchor_id,
                                                                 pw.instance_wide)),
                    '{}'::jsonb)
      INTO v_before
      FROM kb_team_public_within pw
     WHERE v_is_root
        OR EXISTS (SELECT 1 FROM team_ancestors_any(pw.team_id) a WHERE a.team_id = p_team);

    UPDATE kb_teams SET public_within_forbidden = p_forbidden WHERE id = p_team;

    PERFORM _event_append(
        'public_within_policy_changed', p_emitter, NULL, NULL,
        jsonb_strip_nulls(jsonb_build_object(
            'scope',   'team',
            'team_id', p_team,
            'from',    v_from,
            'to',      p_forbidden,
            'actor',   p_actor)),
        p_references => jsonb_build_array(
            jsonb_build_object('rel', 'subject',
                               'target', jsonb_build_object('kind', 'kb_teams', 'id', p_team))),
        p_correlation => v_correlation);

    SELECT e.id INTO v_system
      FROM kb_entities e
      JOIN kb_profiles p ON p.id = e.profile_id
     WHERE p.handle = 'system' AND e.name = 'system';

    FOR v_row IN
        SELECT pw.team_id, pw.anchor_id,
               (v_before ->> (pw.team_id::text || ':' || pw.anchor_id::text))::boolean AS was,
               public_within_row_effective(pw.team_id, pw.anchor_id, pw.instance_wide) AS now
          FROM kb_team_public_within pw
         WHERE v_before ? (pw.team_id::text || ':' || pw.anchor_id::text)
         ORDER BY pw.team_id, pw.anchor_id
    LOOP
        CONTINUE WHEN v_row.was = v_row.now;
        PERFORM _event_append(
            'public_within_effect_changed', v_system, NULL, NULL,
            jsonb_build_object(
                'team_id',   v_row.team_id,
                'anchor_id', v_row.anchor_id,
                'from',      CASE WHEN v_row.was THEN 'effective' ELSE 'dormant' END,
                'to',        CASE WHEN v_row.now THEN 'effective' ELSE 'dormant' END,
                'cause',     'policy',
                'act',       'public_within_policy_changed',
                'act_ref',   v_correlation),
            p_references => jsonb_build_array(
                jsonb_build_object('rel', 'subject',
                                   'target', jsonb_build_object('kind', 'kb_teams', 'id', v_row.team_id)),
                jsonb_build_object('rel', 'subject',
                                   'target', jsonb_build_object('kind', 'kb_teams', 'id', v_row.anchor_id))),
            p_correlation => v_correlation);
        v_changes := v_changes || jsonb_build_array(jsonb_build_object(
            'team_id',   v_row.team_id,
            'anchor_id', v_row.anchor_id,
            'effective', v_row.now));
    END LOOP;

    RETURN jsonb_build_object('outcome', 'changed', 'correlation_id', v_correlation,
                              'changes', v_changes);
END;
$$;

COMMENT ON FUNCTION _admin_public_within_policy_write(uuid, uuid, uuid, boolean, uuid) IS
$c$Set or clear one team's public_within forbid, and record what it did, atomically. Answers jsonb:
{outcome: absent} when there is no live team (nothing written); {outcome: noop} when the flag
already holds the value (nothing written or emitted); otherwise {outcome: changed, correlation_id,
changes: [{team_id, anchor_id, effective}]}, one entry per row whose effect the change flipped.

The rows it considers are every row the forbid can reach: those whose team has p_team in
team_ancestors_any (live or dead: a forbid fails closed, so a team below a dead team is still
under it), and, on the root (temper-system, the instance-wide off), every row. That is wider than
team_live_descendants(p_team): a forbid above a dead team reaches the live rows below it too. It
locks the team row first, then those rows FOR UPDATE in key order, reads each row's effect, flips
the flag, and reads it again. It appends public_within_policy_changed (the team as subject; on the
root only an admin reads it) and one public_within_effect_changed per flipped row (cause policy,
both sides as subject), all under one correlation id, which is each effect record's act_ref.

The policy record is emitted by the caller; each effect record by the system entity. An effect
record names a row (both teams), and the act's answer names a row only to a manager of its team or
anchor or an admin: emitted by the caller, the record would reach the caller's own actor history of
the ledger, which is not gated by subject, and name every row the answer withheld. Who caused it is
the policy record, joined by the correlation id.

The team lock comes first and conflicts with the SHARE lock _public_within_open_locks takes, so a
concurrent open is either refused by its re-check or committed before this scan and recorded here.
IT NEVER AUTHORIZES: who may forbid (a team's direct owner, or a system admin; on the root and the
gating team a system admin only; on a personal team nobody) is decided in team_service, twice.$c$;

-- ── 5. Hard delete ────────────────────────────────────────────────────────────────────────────────
CREATE FUNCTION kb_teams_public_within_cascade() RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_emitter uuid;
    v_row     record;
BEGIN
    IF current_setting('temper.replaying', true) = 'on' THEN
        RETURN OLD;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM kb_team_public_within pw
                    WHERE pw.team_id = OLD.id OR pw.anchor_id = OLD.id) THEN
        RETURN OLD;
    END IF;
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
                                   'target', jsonb_build_object('kind', 'kb_teams', 'id', v_row.anchor_id))));
    END LOOP;
    RETURN OLD;
END;
$$;

COMMENT ON FUNCTION kb_teams_public_within_cascade() IS
$c$Before a kb_teams row is hard-deleted, each public_within row naming it (as the team or the
anchor) is deleted here and recorded as team_public_within_changed cascade_deleted, emitted by the
system entity, with no actor: the foreign-key cascade would otherwise remove it unrecorded. Deleting
the rows here, not leaving them to the cascade, records each row once even when one statement
deletes both of its teams. No door hard-deletes a team today (teams are soft-deleted, and erasure
scrubs a personal team, which never holds a row); this keeps any later one, or a manual delete,
on the record. Skipped under replay (temper.replaying), which restores rows as they stand.$c$;

CREATE TRIGGER kb_teams_public_within_cascade
    BEFORE DELETE ON kb_teams
    FOR EACH ROW EXECUTE FUNCTION kb_teams_public_within_cascade();

SELECT declare_migration(
    20261121000000,
    'additive',
    'public_within doors: adds _admin_public_within_write (open/close a row and record it), '
    '_admin_public_within_policy_write (set or clear a team''s forbid and record each row whose '
    'effect it flips), _public_within_open_locks, public_within_audience, and a BEFORE DELETE '
    'trigger on kb_teams recording cascade_deleted for each row of a hard-deleted team. New '
    'functions and one new trigger; no existing function, column or row changes, and no door '
    'hard-deletes a team, so an older binary is unaffected.'
);
