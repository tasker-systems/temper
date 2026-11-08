-- public_within: every structural and admin act that can change whether a row confers records the
-- change, and a row whose anchor is no longer above its team is deleted.
--
-- Design: temper-artifacts specs/2026-11-06-public-within-design.md.
--
--   _public_within_effects_lock       the one lock that orders effect computations with each other
--                                     and with an open.
--   _public_within_effects_before     snapshots the effect of every row an act on some teams can
--                                     reach, before the act writes.
--   _public_within_transitions        after the act: deletes each snapshotted row whose anchor is no
--                                     longer an ancestor of its team by any path (lapsed), records
--                                     each row whose effect flipped, and answers both lists.
--   _admin_team_parent_link_act       link/unlink, its record and its transitions; answers jsonb.
--   _admin_team_lifecycle_act         delete/restore/rename/auto-join, its record and transitions.
--   _admin_gating_team_slug_write     repoints the gating team and records the transitions.
--   _admin_team_parent_link_write,    unchanged signatures; now body-only wrappers over the two
--   _admin_team_lifecycle_write       *_act functions, so any caller gets the transitions.
--   _admin_public_within_policy_write the forbid writer, now on the same two helpers.
--   _public_within_open_locks         now also takes the effects lock, last.
--
-- None of these authorizes. Every rationale is in COMMENT ON FUNCTION, never in a body.

-- ── 1. The lock ───────────────────────────────────────────────────────────────────────────────────
CREATE FUNCTION _public_within_effects_lock()
RETURNS void
LANGUAGE plpgsql VOLATILE
SET search_path = public, pg_temp
AS $$
BEGIN
    PERFORM _gating_team_lock(false);
    PERFORM pg_advisory_xact_lock(hashtextextended('public_within_effects:', 0));
END;
$$;

COMMENT ON FUNCTION _public_within_effects_lock() IS
$c$Until the transaction ends: the shared side of _gating_team_lock, then one exclusive advisory key
for public_within effects. Taken by every act that computes effect transitions
(_public_within_effects_before) and by every open (_public_within_open_locks), so that a row's
transitions are computed one act at a time: an open committed first is in a later act's snapshot,
and an act committed first is seen by a later open's re-check. Without it, an act that changes a row's
effect through a team the open does not lock (an auto-join set below the anchor, a pool attached
below it) could miss a row opened concurrently, and leave its change of effect unrecorded.

The order is fixed for every holder: team rows first (each act locks the teams it writes before it
calls this), then the gating lock, then this key; after it, no team row is locked. An act that
repoints the gating slug or renames a team takes the gating lock exclusively before its row work, and
the shared request here is then its own. The key is per database, like every advisory lock, and the
public_within_effects: prefix keeps it out of the other hashtextextended keys.$c$;

-- ── 2. Before ─────────────────────────────────────────────────────────────────────────────────────
CREATE FUNCTION _public_within_effects_before(p_team_under uuid[], p_anchor_over uuid[])
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_all  boolean;
    v_snap jsonb;
BEGIN
    PERFORM _public_within_effects_lock();
    IF NOT EXISTS (SELECT 1 FROM kb_team_public_within) THEN
        RETURN '{}'::jsonb;
    END IF;
    v_all := EXISTS (SELECT 1 FROM kb_teams r
                      WHERE r.id = ANY(p_team_under) AND r.slug = 'temper-system');
    WITH RECURSIVE down AS (
        SELECT t.id AS team_id FROM kb_teams t WHERE t.id = ANY(p_team_under)
        UNION
        SELECT tp.child_id FROM kb_teams_parents tp JOIN down ON tp.parent_id = down.team_id
    ), up AS (
        SELECT t.id AS team_id FROM kb_teams t WHERE t.id = ANY(p_anchor_over)
        UNION
        SELECT tp.parent_id FROM kb_teams_parents tp JOIN up ON tp.child_id = up.team_id
    )
    SELECT coalesce(jsonb_object_agg(
               pw.team_id::text || ':' || pw.anchor_id::text,
               jsonb_build_object(
                   'effective', public_within_row_effective(pw.team_id, pw.anchor_id,
                                                            pw.instance_wide),
                   'live',      EXISTS (SELECT 1 FROM team_ancestors(pw.team_id) a
                                         WHERE a.team_id = pw.anchor_id))),
           '{}'::jsonb)
      INTO v_snap
      FROM kb_team_public_within pw
     WHERE v_all
        OR pw.team_id IN (SELECT team_id FROM down)
        OR pw.anchor_id IN (SELECT team_id FROM up);
    RETURN v_snap;
END;
$$;

COMMENT ON FUNCTION _public_within_effects_before(uuid[], uuid[]) IS
$c$The first half of the one definition of a public_within transition. Takes
_public_within_effects_lock, then snapshots every row an act can flip: each row whose team is at or
below a team in p_team_under, or whose anchor is at or above a team in p_anchor_over, walked through
every link, live or dead; every row when the root is in p_team_under (its forbid is the instance-wide
off and reaches trees not linked under it). For each row it records whether the row confers
(public_within_row_effective, the one effect definition) and whether its anchor is above its team
along a live chain (the liveness clause of that definition, kept apart only so a transition can say
which kind of cause flipped it).

Why those two sets suffice. A row (T, A) confers from: the forbids on T's ancestors (any
liveness) and the root's; whether T is the root or the gating team; a live chain from A down to T;
and no pool in A's live subtree. So:
  * a forbid on X can flip only rows with T at or below X: p_team_under = {X};
  * a delete, restore, auto-join change or rename of X, or the gating team moving to or from X, can
    flip only rows with A at or above X (X on the chain from A to T, X in A's subtree, or X = T,
    whose anchors are above it): p_anchor_over = {X};
  * a link or unlink of parent P over child C can change T's forbidding ancestors only for T at or
    below C, and the chain, the pool and any path from A only for A at or above P:
    p_team_under = {C}, p_anchor_over = {P}. Neither set depends on the link itself, so the same
    rows are read before and after.
The walks ignore liveness, so they are a superset of the live walks the effect reads. The narrow
scope is what keeps an act's hold on the effects lock short: rows it cannot flip are never read.

Call it after the act has locked the teams it writes and before it writes; pass the result to
_public_within_transitions after the write. An empty object means no row can be reached, and
_public_within_transitions then does nothing. With no public_within row at all it answers that at
once, after the lock and without walking the tree: the lock is still taken, because a concurrent
open may be about to commit the first row.$c$;

-- ── 3. After ──────────────────────────────────────────────────────────────────────────────────────
CREATE FUNCTION _public_within_transitions(p_before jsonb, p_act text, p_correlation uuid)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_system  uuid;
    v_row     record;
    v_lapsed  jsonb := '[]'::jsonb;
    v_changes jsonb := '[]'::jsonb;
BEGIN
    IF p_before IS NULL OR p_before = '{}'::jsonb THEN
        RETURN jsonb_build_object('lapsed', v_lapsed, 'changes', v_changes);
    END IF;

    SELECT e.id INTO v_system
      FROM kb_entities e
      JOIN kb_profiles p ON p.id = e.profile_id
     WHERE p.handle = 'system' AND e.name = 'system';

    FOR v_row IN
        WITH gone AS (
            DELETE FROM kb_team_public_within pw
             WHERE p_before ? (pw.team_id::text || ':' || pw.anchor_id::text)
               AND NOT EXISTS (SELECT 1 FROM team_ancestors_any(pw.team_id) a
                                WHERE a.team_id = pw.anchor_id)
            RETURNING pw.team_id, pw.anchor_id, pw.admin_act, pw.instance_wide
        )
        SELECT * FROM gone ORDER BY team_id, anchor_id
    LOOP
        PERFORM _event_append(
            'team_public_within_changed', v_system, NULL, NULL,
            jsonb_build_object(
                'team_id',       v_row.team_id,
                'anchor_id',     v_row.anchor_id,
                'act',           'lapsed',
                'admin_act',     v_row.admin_act,
                'instance_wide', v_row.instance_wide),
            p_references => jsonb_build_array(
                jsonb_build_object('rel', 'subject',
                                   'target', jsonb_build_object('kind', 'kb_teams', 'id', v_row.team_id)),
                jsonb_build_object('rel', 'subject',
                                   'target', jsonb_build_object('kind', 'kb_teams', 'id', v_row.anchor_id))),
            p_correlation => p_correlation);
        v_lapsed := v_lapsed || jsonb_build_array(jsonb_build_object(
            'team_id', v_row.team_id, 'anchor_id', v_row.anchor_id));
    END LOOP;

    FOR v_row IN
        SELECT s.team_id, s.anchor_id, s.was, s.was_live,
               public_within_row_effective(s.team_id, s.anchor_id, s.instance_wide) AS now,
               EXISTS (SELECT 1 FROM team_ancestors(s.team_id) a
                        WHERE a.team_id = s.anchor_id) AS now_live
          FROM (SELECT pw.team_id, pw.anchor_id, pw.instance_wide,
                       (p_before -> (pw.team_id::text || ':' || pw.anchor_id::text)
                                 ->> 'effective')::boolean AS was,
                       (p_before -> (pw.team_id::text || ':' || pw.anchor_id::text)
                                 ->> 'live')::boolean AS was_live
                  FROM kb_team_public_within pw
                 WHERE p_before ? (pw.team_id::text || ':' || pw.anchor_id::text)) s
         ORDER BY s.team_id, s.anchor_id
    LOOP
        CONTINUE WHEN v_row.was = v_row.now;
        PERFORM _event_append(
            'public_within_effect_changed', v_system, NULL, NULL,
            jsonb_strip_nulls(jsonb_build_object(
                'team_id',   v_row.team_id,
                'anchor_id', v_row.anchor_id,
                'from',      CASE WHEN v_row.was THEN 'effective' ELSE 'dormant' END,
                'to',        CASE WHEN v_row.now THEN 'effective' ELSE 'dormant' END,
                'cause',     CASE WHEN v_row.was_live IS DISTINCT FROM v_row.now_live
                                  THEN 'liveness' ELSE 'policy' END,
                'act',       p_act,
                'act_ref',   p_correlation)),
            p_references => jsonb_build_array(
                jsonb_build_object('rel', 'subject',
                                   'target', jsonb_build_object('kind', 'kb_teams', 'id', v_row.team_id)),
                jsonb_build_object('rel', 'subject',
                                   'target', jsonb_build_object('kind', 'kb_teams', 'id', v_row.anchor_id))),
            p_correlation => p_correlation);
        v_changes := v_changes || jsonb_build_array(jsonb_build_object(
            'team_id',   v_row.team_id,
            'anchor_id', v_row.anchor_id,
            'effective', v_row.now));
    END LOOP;

    RETURN jsonb_build_object('lapsed', v_lapsed, 'changes', v_changes);
END;
$$;

COMMENT ON FUNCTION _public_within_transitions(jsonb, text, uuid) IS
$c$The second half of the one definition of a public_within transition, run after an act wrote,
over the rows _public_within_effects_before snapshotted. Answers jsonb
{lapsed: [{team_id, anchor_id}], changes: [{team_id, anchor_id, effective}]}.

  * Structural lapse: a row whose anchor is no longer in team_ancestors_any(team) (no path, live or
    dead, runs from the anchor down to the team) is DELETED and recorded as
    team_public_within_changed lapsed. It never revives: re-linking the teams does not bring it back,
    and a reopen needs both sides again. Liveness and policy never delete a row.
  * Every other row whose effect flipped is recorded as public_within_effect_changed, from and to,
    with cause liveness when its live-chain clause flipped and policy otherwise (a forbid, the
    gating team, or a pool at the anchor), act = p_act (the causing act's event type, or NULL) and
    act_ref = p_correlation.

Every record names both teams as subject references, so the managers of either side read it, and
carries p_correlation, the causing act's correlation id. Every record is emitted by the system
entity, never the actor: an act's answer names a row only to a manager of its team or anchor or an
admin, and a record emitted by the actor would reach the actor's own history in the ledger, which is
not gated by subject, and name every row the answer withheld. Who caused it is the act's own record,
joined by the correlation id.

A row deleted or closed by another transaction since the snapshot is skipped. It never authorizes.$c$;

-- ── 4. Link and unlink ────────────────────────────────────────────────────────────────────────────
CREATE FUNCTION _admin_team_parent_link_act(
    p_emitter     uuid,
    p_actor       uuid,
    p_act         text,
    p_parent      uuid,
    p_child       uuid,
    p_correlation uuid DEFAULT NULL
) RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_rows        integer;
    v_before      jsonb;
    v_correlation uuid;
BEGIN
    IF p_act NOT IN ('link', 'unlink') THEN
        RAISE EXCEPTION '_admin_team_parent_link_act: p_act must be link or unlink, got %', p_act;
    END IF;
    PERFORM 1 FROM kb_teams WHERE id IN (p_parent, p_child) ORDER BY id FOR NO KEY UPDATE;
    v_before := _public_within_effects_before(ARRAY[p_child], ARRAY[p_parent]);

    IF p_act = 'link' THEN
        INSERT INTO kb_teams_parents (child_id, parent_id)
        VALUES (p_child, p_parent)
        ON CONFLICT DO NOTHING;
        GET DIAGNOSTICS v_rows = ROW_COUNT;
        IF v_rows = 0 THEN
            RETURN jsonb_build_object('outcome', 'exists');
        END IF;
    ELSE
        DELETE FROM kb_teams_parents WHERE child_id = p_child AND parent_id = p_parent;
        GET DIAGNOSTICS v_rows = ROW_COUNT;
        IF v_rows = 0 THEN
            RETURN jsonb_build_object('outcome', 'absent');
        END IF;
    END IF;

    v_correlation := coalesce(p_correlation, uuid_generate_v7());
    PERFORM _event_append(
        'team_parent_link_changed', p_emitter, NULL, NULL,
        jsonb_strip_nulls(jsonb_build_object(
            'parent_team_id', p_parent,
            'child_team_id',  p_child,
            'act',            CASE p_act WHEN 'link' THEN 'linked' ELSE 'unlinked' END,
            'actor',          p_actor)),
        p_references => jsonb_build_array(
            jsonb_build_object('rel', 'subject', 'target', jsonb_build_object('kind', 'kb_teams', 'id', p_parent)),
            jsonb_build_object('rel', 'principal', 'target', jsonb_build_object('kind', 'kb_teams', 'id', p_child))),
        p_correlation => v_correlation);

    RETURN jsonb_build_object(
               'outcome',        CASE p_act WHEN 'link' THEN 'linked' ELSE 'unlinked' END,
               'correlation_id', v_correlation)
        || _public_within_transitions(v_before, 'team_parent_link_changed', v_correlation);
END;
$$;

COMMENT ON FUNCTION _admin_team_parent_link_act(uuid, uuid, text, uuid, uuid, uuid) IS
$c$Insert (link) or delete (unlink) one kb_teams_parents row, append its team_parent_link_changed
admin event, and record what it did to every public_within row, atomically. Answers jsonb
{outcome: linked | unlinked | exists | absent, correlation_id, lapsed, changes}; only linked and
unlinked write or emit. Covers create-with-parents, attach, detach and reparent.

It locks both team rows (ascending, a no-op when the caller holds them, as every door does), then
snapshots (_public_within_effects_before: teams at or below the child, anchors at or above the
parent), writes, records, and runs _public_within_transitions. An unlink that leaves an anchor with no path to its team deletes the row
(lapsed); a link or unlink that changes a forbid above a team, a pool below an anchor or the only
live path between them records the change of effect. The link record carries p_correlation, or,
when there is none, a fresh id that the transitions share. The id is minted whether or not a row
was in reach, so the actor's own record says nothing about rows near the teams.
IT NEVER AUTHORIZES: team_service decides who may, before calling it.$c$;

CREATE OR REPLACE FUNCTION _admin_team_parent_link_write(
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
BEGIN
    RETURN _admin_team_parent_link_act(p_emitter, p_actor, p_act, p_parent, p_child,
                                       p_correlation) ->> 'outcome';
END;
$$;

COMMENT ON FUNCTION _admin_team_parent_link_write(uuid, uuid, text, uuid, uuid, uuid) IS
$c$Insert (p_act = 'link') or delete (p_act = 'unlink') one kb_teams_parents row and append its
team_parent_link_changed admin event, atomically. Returns linked | unlinked | exists | absent; only
the first two write or emit. A wrapper over _admin_team_parent_link_act, which also records the
public_within transitions; this keeps the text answer for an older caller. It NEVER AUTHORIZES.$c$;

-- ── 5. The lifecycle ──────────────────────────────────────────────────────────────────────────────
CREATE FUNCTION _admin_team_lifecycle_act(
    p_emitter     uuid,
    p_actor       uuid,
    p_team        uuid,
    p_act         text,
    p_to          text DEFAULT NULL,
    p_correlation uuid DEFAULT NULL
) RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_from        text;
    v_to          text;
    v_before      jsonb := '{}'::jsonb;
    v_correlation uuid;
BEGIN
    IF p_act = 'created' THEN
        IF NOT EXISTS (SELECT 1 FROM kb_teams WHERE id = p_team AND is_active) THEN
            RETURN jsonb_build_object('outcome', 'absent');
        END IF;
    ELSIF p_act = 'deleted' THEN
        PERFORM 1 FROM kb_teams
          WHERE id = p_team AND is_active AND slug <> 'temper-system'
            FOR NO KEY UPDATE;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('outcome', 'absent');
        END IF;
        v_before := _public_within_effects_before('{}'::uuid[], ARRAY[p_team]);
        UPDATE kb_teams SET is_active = false WHERE id = p_team;
    ELSIF p_act = 'restored' THEN
        PERFORM 1 FROM kb_teams WHERE id = p_team AND NOT is_active FOR NO KEY UPDATE;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('outcome', 'absent');
        END IF;
        v_before := _public_within_effects_before('{}'::uuid[], ARRAY[p_team]);
        UPDATE kb_teams SET is_active = true WHERE id = p_team;
    ELSIF p_act = 'renamed' THEN
        IF p_to IS NULL THEN
            RAISE EXCEPTION '_admin_team_lifecycle_act: renamed needs the new slug in p_to';
        END IF;
        PERFORM _gating_team_lock(true);
        SELECT slug INTO v_from FROM kb_teams WHERE id = p_team AND is_active FOR UPDATE;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('outcome', 'absent');
        END IF;
        IF v_from = p_to THEN
            RETURN jsonb_build_object('outcome', 'noop');
        END IF;
        v_before := _public_within_effects_before('{}'::uuid[], ARRAY[p_team]);
        UPDATE kb_teams SET slug = p_to WHERE id = p_team;
        UPDATE kb_system_settings SET gating_team_slug = p_to WHERE gating_team_slug = v_from;
        v_to := p_to;
    ELSIF p_act = 'auto_join_changed' THEN
        SELECT auto_join_role::text INTO v_from FROM kb_teams
         WHERE id = p_team AND is_active FOR NO KEY UPDATE;
        IF NOT FOUND THEN
            RETURN jsonb_build_object('outcome', 'absent');
        END IF;
        IF v_from IS NOT DISTINCT FROM p_to THEN
            RETURN jsonb_build_object('outcome', 'noop');
        END IF;
        v_before := _public_within_effects_before('{}'::uuid[], ARRAY[p_team]);
        UPDATE kb_teams SET auto_join_role = p_to::team_role WHERE id = p_team;
        v_to := p_to;
    ELSE
        RAISE EXCEPTION '_admin_team_lifecycle_act: p_act must be created, deleted, restored, '
                        'renamed or auto_join_changed, got %', p_act;
    END IF;

    v_correlation := coalesce(p_correlation, uuid_generate_v7());
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
        p_correlation => v_correlation);

    RETURN jsonb_build_object('outcome', 'changed', 'correlation_id', v_correlation)
        || _public_within_transitions(v_before, 'team_lifecycle_changed', v_correlation);
END;
$$;

COMMENT ON FUNCTION _admin_team_lifecycle_act(uuid, uuid, uuid, text, text, uuid) IS
$c$Perform one team lifecycle act (created | deleted | restored | renamed | auto_join_changed) on
kb_teams, append its team_lifecycle_changed admin event, and record what it did to every
public_within row, atomically. Answers jsonb {outcome: changed | noop | absent, correlation_id,
lapsed, changes}; only changed writes or emits. created appends only and changes no row's effect.

Each writing act locks its team row first (rename: the exclusive gating lock, then the row, as
before), decides absent or noop, then snapshots (_public_within_effects_before: anchors at or above
the team), writes, records, and runs _public_within_transitions. A delete or restore flips rows through liveness; an
auto-join change flips rows whose anchor's live subtree gains or loses a pool; a rename moves the
gating slug with it, so it flips nothing unless it changed which team is the root or the gating team
(the door refuses the root; renaming onto a gating slug that no team holds makes the team the gating
team, and those flips are recorded too). The lifecycle record carries p_correlation, or, when there is none, a fresh id
that the transitions share, minted whether or not a row was in reach, so the actor's own record says
nothing about rows near the team. IT NEVER AUTHORIZES.$c$;

CREATE OR REPLACE FUNCTION _admin_team_lifecycle_write(
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
BEGIN
    RETURN _admin_team_lifecycle_act(p_emitter, p_actor, p_team, p_act, p_to,
                                     p_correlation) ->> 'outcome';
END;
$$;

COMMENT ON FUNCTION _admin_team_lifecycle_write(uuid, uuid, uuid, text, text, uuid) IS
$c$Perform one team lifecycle act (created | deleted | restored | renamed | auto_join_changed) on
kb_teams and append its team_lifecycle_changed admin event, atomically. Returns changed | noop |
absent; only changed writes or emits. A wrapper over _admin_team_lifecycle_act, which also records
the public_within transitions; this keeps the text answer for an older caller. It NEVER
AUTHORIZES.$c$;

-- ── 6. The gating slug ────────────────────────────────────────────────────────────────────────────
CREATE FUNCTION _admin_gating_team_slug_write(p_slug text, p_correlation uuid DEFAULT NULL)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_from        text;
    v_before      jsonb;
    v_correlation uuid;
BEGIN
    PERFORM _gating_team_lock(true);
    SELECT gating_team_slug INTO v_from FROM kb_system_settings WHERE id = 1 FOR UPDATE;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('outcome', 'absent');
    END IF;
    IF v_from IS NOT DISTINCT FROM p_slug THEN
        RETURN jsonb_build_object('outcome', 'noop');
    END IF;
    IF NOT EXISTS (SELECT 1 FROM kb_teams t WHERE t.slug = p_slug) THEN
        RETURN jsonb_build_object('outcome', 'no_team');
    END IF;
    v_before := _public_within_effects_before(
        '{}'::uuid[],
        ARRAY(SELECT t.id FROM kb_teams t WHERE t.slug = v_from OR t.slug = p_slug));
    UPDATE kb_system_settings SET gating_team_slug = p_slug, updated = now() WHERE id = 1;
    v_correlation := coalesce(p_correlation, uuid_generate_v7());
    RETURN jsonb_build_object('outcome', 'changed', 'correlation_id', v_correlation)
        || _public_within_transitions(v_before, NULL, v_correlation);
END;
$$;

COMMENT ON FUNCTION _admin_gating_team_slug_write(text, uuid) IS
$c$Repoint the instance's gating team (kb_system_settings.gating_team_slug) and record what it did
to every public_within row, atomically. Answers jsonb {outcome: changed | noop | absent,
correlation_id, lapsed, changes}, or {outcome: no_team} (nothing written) when no team holds the
slug once the lock is held: a rename committed since the caller's pre-check took it, and a slug
naming no team would let any manager become the gating team by renaming onto it. The gating team counts as a pool under any anchor above it, and
can never be a row's team, so moving it flips rows anchored above the old or the new gating team, and
rows whose team is either; the snapshot covers every anchor at or above either team.

It takes the exclusive gating lock before the settings row (the order a rename takes them in), so
the shared request inside _public_within_effects_before is its own; a rename takes the same exclusive
lock, so the existence re-check cannot be raced. The settings trigger
(gating_team_is_never_personal) still refuses a personal team. There is no settings event type, so
each effect record carries act NULL and only the correlation id. IT NEVER AUTHORIZES: only a system
admin reaches it (access_service::update_system_settings).$c$;

-- ── 7. The forbid, on the same helpers ────────────────────────────────────────────────────────────
CREATE OR REPLACE FUNCTION _admin_public_within_policy_write(
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
    v_correlation uuid := coalesce(p_correlation, uuid_generate_v7());
    v_before      jsonb;
    v_after       jsonb;
BEGIN
    SELECT t.public_within_forbidden
      INTO v_from
      FROM kb_teams t
     WHERE t.id = p_team AND t.is_active
       FOR NO KEY UPDATE;
    IF NOT FOUND THEN
        RETURN jsonb_build_object('outcome', 'absent');
    END IF;
    IF v_from = p_forbidden THEN
        RETURN jsonb_build_object('outcome', 'noop', 'changes', '[]'::jsonb);
    END IF;

    v_before := _public_within_effects_before(ARRAY[p_team], '{}'::uuid[]);

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

    v_after := _public_within_transitions(v_before, 'public_within_policy_changed', v_correlation);

    RETURN jsonb_build_object('outcome', 'changed', 'correlation_id', v_correlation,
                              'changes', v_after -> 'changes', 'lapsed', v_after -> 'lapsed');
END;
$$;

COMMENT ON FUNCTION _admin_public_within_policy_write(uuid, uuid, uuid, boolean, uuid) IS
$c$Set or clear one team's public_within forbid, and record what it did, atomically. Answers jsonb:
{outcome: absent} when there is no live team (nothing written); {outcome: noop} when the flag
already holds the value (nothing written or emitted); otherwise {outcome: changed, correlation_id,
changes: [{team_id, anchor_id, effective}], lapsed}, one change per row whose effect the forbid
flipped. A forbid never moves a link, so lapsed is empty unless a row had already lost its anchor.

It locks the team row first, then computes the transitions with the shared helpers
(_public_within_effects_before over the team, then _public_within_transitions), so a forbid and every
structural act share one definition of a transition. The rows they consider include every row whose
team has p_team in team_ancestors_any (live or dead: a forbid fails closed, so a team below a dead team
is still under it), and, on the root (temper-system, the instance-wide off), every row. It appends
public_within_policy_changed (the team as subject; on the root only an admin reads it), emitted by
the caller; each effect record is the system entity's (see _public_within_transitions), all under one
correlation id.

The team lock comes first and conflicts with the SHARE lock _public_within_open_locks takes, so a
concurrent open is either refused by its re-check or committed before this scan and recorded here.
IT NEVER AUTHORIZES: who may forbid (a team's direct owner, or a system admin; on the root and the
gating team a system admin only; on a personal team nobody) is decided in team_service, twice.$c$;

-- ── 8. The open takes the effects lock ────────────────────────────────────────────────────────────
CREATE OR REPLACE FUNCTION _public_within_open_locks(p_team uuid, p_anchor uuid)
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
    PERFORM _public_within_effects_lock();
    RETURN NOT EXISTS (
        SELECT 1 FROM team_ancestors_any(p_team) a WHERE a.team_id <> ALL (v_locked));
END;
$$;

COMMENT ON FUNCTION _public_within_open_locks(uuid, uuid) IS
$c$The locks an open takes before it decides again, in ascending id order: FOR NO KEY UPDATE on the
team and the anchor (the lock every membership writer takes, so a demotion cannot race the open),
and FOR SHARE on every other team whose forbid can reach the team: team_ancestors_any(team), live
or dead, and the root, whose forbid is the instance-wide off. Then _public_within_effects_lock (the
shared gating lock, then the effects key), after the rows, in the order every other holder takes them.

The SHARE locks are what make the forbid race-free. _admin_public_within_policy_write locks its
team's row FOR NO KEY UPDATE before it reads anything, which conflicts with SHARE: a forbid
committed first is seen by the open's re-check; an open committed first is seen by the forbid's
transition scan, and recorded. The effects key does the same for every other act that computes
transitions, including those that change a row's effect through a team this open does not lock (an
auto-join set or a pool attached below the anchor): such an act committed first is seen by the
re-check, and an open committed first is in the act's snapshot, and recorded.

Answers false when the team's ancestors, read again once the locks are held, include a team that is
not locked: a link committed while the open waited gave the team a new ancestor whose forbid would
not serialize with this open. The caller then refuses with a retryable conflict and writes nothing.
That re-read is sufficient because every new ancestor link names a child that is the team or one of
its ancestors, and each of those is locked here, as every link writer locks the child.$c$;

-- ── 9. Rows that lapsed before this migration ─────────────────────────────────────────────────────
-- Until now an unlink left a row whose anchor it cut off. Each such row is deleted and recorded as
-- lapsed now, so no later link can revive it.
DO $$
DECLARE
    v_before jsonb;
BEGIN
    IF NOT EXISTS (SELECT 1 FROM kb_team_public_within pw
                    WHERE NOT EXISTS (SELECT 1 FROM team_ancestors_any(pw.team_id) a
                                       WHERE a.team_id = pw.anchor_id)) THEN
        RETURN;
    END IF;
    SELECT jsonb_object_agg(pw.team_id::text || ':' || pw.anchor_id::text,
                            jsonb_build_object('effective', false, 'live', false))
      INTO v_before
      FROM kb_team_public_within pw
     WHERE NOT EXISTS (SELECT 1 FROM team_ancestors_any(pw.team_id) a
                        WHERE a.team_id = pw.anchor_id);
    PERFORM _public_within_transitions(v_before, NULL, uuid_generate_v7());
END;
$$;

SELECT declare_migration(
    20261122000000,
    'additive',
    'public_within transitions: adds _public_within_effects_lock, _public_within_effects_before, '
    '_public_within_transitions, _admin_team_parent_link_act, _admin_team_lifecycle_act and '
    '_admin_gating_team_slug_write, and body-only CREATE OR REPLACE (signatures and return types '
    'unchanged) of _admin_team_parent_link_write and _admin_team_lifecycle_write (now wrappers over '
    'the *_act functions), _admin_public_within_policy_write and _public_within_open_locks. Every '
    'structural act now records each public_within row whose effect it flips, and deletes a row whose '
    'anchor is no longer above its team; a one-time step deletes and records, as lapsed, every '
    'existing row whose anchor is already no ancestor of its team (rows exist only since the '
    'unreleased public_within doors). The parent-link and lifecycle records now always carry a '
    'correlation id. Signatures and answers are unchanged, so an older binary is unaffected.'
);
