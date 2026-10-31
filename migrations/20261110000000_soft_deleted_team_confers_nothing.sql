-- A soft-deleted team confers nothing: no authority over it or its property, no content it owns,
-- and nothing is written onto it.
--
-- =================================================================================================
-- WHAT CHANGES
-- =================================================================================================
--
-- `20260703000001_team_metadata_soft_delete.sql` says membership in a soft-deleted team confers
-- nothing anywhere. The read side kept that (`profile_effective_teams`, `team_ancestors`); several
-- authority probes did not, and content a dead team owned stayed reachable through other teams.
--
-- 1. AUTHORITY. The owning-team probe in context_rename / context_retire / context_restore /
--    context_reassign, and context_reassign's target-team probe, now require the team to be live.
--    A former owner or maintainer of a dead team no longer administers its contexts. A system admin
--    is still exempt from the owner side, so an admin can move a dead team's context out: that is
--    the recovery path.
--
-- 2. CONTENT. New STABLE `context_is_live(uuid)`: the context is active and, if a team owns it, that
--    team is active. A context owned by a dead team reads as RETIRED. It replaces the retired floor in
--    contexts_readable_by_teams arms 3 (shares) and 4 (grants), context_authorable_by_profile's grant
--    arm and can()'s kb_contexts subject floor. Arm 2 (members of the owner) already held it through
--    profile_reachable_teams, and the authorable team arm through its own `t.is_active`. What
--    survives is what survives retirement: a resource's home owner, and a grant on the resource
--    itself (resources_visible_to / can_modify_resource arms that do not go through the context).
--
-- 3. NO WRITES ONTO A DEAD TEAM. context_reassign refuses a dead target team for everyone;
--    _admin_team_membership_write refuses a dead team as a backstop to the Rust lock helper; the
--    three auto-join writers skip dead teams. steward_team_contexts and cogmaps_share_a_team read
--    live teams only.
--
-- Every function here keeps its signature and return type; each is a body-only CREATE OR REPLACE of
-- its current definition with the change above and nothing else.
--
-- Design: temper-artifacts specs/2026-10-31-soft-deleted-team-confers-nothing-design.md.

CREATE OR REPLACE FUNCTION context_is_live(p_context uuid)
 RETURNS boolean
 LANGUAGE sql
 STABLE
AS $$
    -- Retired (is_active = false), or owned by a soft-deleted team: confers nothing through the
    -- container. A team owner absent from kb_teams cannot happen (owners are checked on write) and
    -- reads as not live.
    SELECT EXISTS (
        SELECT 1
          FROM kb_contexts c
         WHERE c.id = p_context AND c.is_active
           AND ( c.owner_table <> 'kb_teams'
                 OR EXISTS (SELECT 1 FROM kb_teams t WHERE t.id = c.owner_id AND t.is_active) )
    );
$$;

CREATE OR REPLACE FUNCTION public.context_reassign(p_payload jsonb, p_emitter uuid, p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL::uuid, p_correlation uuid DEFAULT NULL::uuid)
 RETURNS uuid
 LANGUAGE plpgsql
AS $function$
DECLARE
    v_ev uuid;
    v_context uuid := (p_payload->>'context_id')::uuid;
    v_to_table text := p_payload->>'to_owner_table';
    v_to_id uuid := (p_payload->>'to_owner_id')::uuid;
    v_owner_table text;
    v_owner_id uuid;
    v_actor uuid;
BEGIN
    -- Existence + current owner (the owner drives the "administers the context" half of the gate).
    -- FOR NO KEY UPDATE since 20261022100000: the person act holds the same lock on every estate
    -- context while it erases them, so this waits for it and then reads the erased state.
    SELECT owner_table, owner_id INTO v_owner_table, v_owner_id
      FROM kb_contexts WHERE id = v_context
       FOR NO KEY UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'context_reassign: context % not found', v_context;
    END IF;

    -- The acting principal IS the emitter; kb_entities.profile_id (NOT NULL) is the human/machine
    -- behind the actor. Authorize that profile, not the emitter entity.
    SELECT profile_id INTO v_actor FROM kb_entities WHERE id = p_emitter;
    IF v_actor IS NULL THEN
        RAISE EXCEPTION 'context_reassign: emitter % has no profile', p_emitter
              USING ERRCODE = '42501';
    END IF;

    -- Nothing is transferred INTO a soft-deleted team, by anyone (soft-delete design D4): a row
    -- written onto a dead team would come back unreviewed if it were ever restored. A system admin
    -- may still move a dead team's context OUT (the owner side below exempts them): that is the
    -- recovery path.
    IF v_to_table = 'kb_teams'
       AND NOT EXISTS (SELECT 1 FROM kb_teams WHERE id = v_to_id AND is_active) THEN
        RAISE EXCEPTION 'context_reassign: target team % not found', v_to_id
              USING ERRCODE = '42501';
    END IF;

    IF NOT is_system_admin(v_actor) THEN
        -- Target-team side: must be a team, non-gating, and the actor owner/maintainer on it.
        IF v_to_table IS DISTINCT FROM 'kb_teams' THEN
            RAISE EXCEPTION 'context_reassign: transfer target must be a team'
                  USING ERRCODE = '42501';
        END IF;
        IF EXISTS (SELECT 1 FROM kb_teams t
                     JOIN kb_system_settings s ON t.slug = s.gating_team_slug
                    WHERE t.id = v_to_id) THEN
            RAISE EXCEPTION 'context_reassign: cannot transfer into the gating team'
                  USING ERRCODE = '42501';
        END IF;
        IF NOT EXISTS (SELECT 1 FROM kb_team_members tm
                         JOIN kb_teams t ON t.id = tm.team_id AND t.is_active
                        WHERE tm.team_id = v_to_id AND tm.profile_id = v_actor
                          AND tm.role IN ('owner', 'maintainer')) THEN
            RAISE EXCEPTION 'context_reassign: actor lacks owner/maintainer on the target team'
                  USING ERRCODE = '42501';
        END IF;

        -- Context side: the actor must administer the CURRENT owner (own it directly, or
        -- owner/maintainer on the owning team — matching `caller_administers_context`).
        IF v_owner_table = 'kb_profiles' THEN
            IF v_owner_id IS DISTINCT FROM v_actor THEN
                RAISE EXCEPTION 'context_reassign: actor does not own the context'
                      USING ERRCODE = '42501';
            END IF;
        ELSIF v_owner_table = 'kb_teams' THEN
            IF NOT EXISTS (SELECT 1 FROM kb_team_members tm
                             JOIN kb_teams t ON t.id = tm.team_id AND t.is_active
                            WHERE tm.team_id = v_owner_id AND tm.profile_id = v_actor
                              AND tm.role IN ('owner', 'maintainer')) THEN
                RAISE EXCEPTION 'context_reassign: actor does not administer the context''s owning team'
                      USING ERRCODE = '42501';
            END IF;
        ELSE
            RAISE EXCEPTION 'context_reassign: context % has unknown owner table %',
                  v_context, v_owner_table USING ERRCODE = '42501';
        END IF;
    END IF;

    -- An erased context stays where the erasure left it (R7): moving it out of its erased owner's
    -- estate would take it out of the scope a later run of the act rewrites. After the gate, so
    -- only a caller who could otherwise move it learns why not. SQLSTATE TE001, rendered 410.
    IF EXISTS (SELECT 1 FROM kb_events e
                 JOIN kb_event_types t ON t.id = e.event_type_id
                WHERE t.name = 'context_erased'
                  AND e.producing_anchor_table = 'kb_contexts'
                  AND e.producing_anchor_id = v_context) THEN
        RAISE EXCEPTION 'context_reassign: context % was erased', v_context
              USING ERRCODE = 'TE001';
    END IF;

    v_ev := _event_append('context_reassigned', p_emitter, 'kb_contexts', v_context, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_context_reassigned(v_ev, p_payload);
END;
$function$;

CREATE OR REPLACE FUNCTION public.context_rename(p_payload jsonb, p_emitter uuid, p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL::uuid, p_correlation uuid DEFAULT NULL::uuid)
 RETURNS uuid
 LANGUAGE plpgsql
AS $function$
DECLARE
    v_ev uuid;
    v_context uuid := (p_payload->>'context_id')::uuid;
    v_owner_table text;
    v_owner_id uuid;
    v_is_active boolean;
    v_actor uuid;
BEGIN
    -- Existence + current owner + current activation. The owner drives the "administers the
    -- context" gate; the activation flag drives the state refusal directly below. One read, the
    -- shape `context_retire` uses (20260826000120:124-128).
    -- FOR NO KEY UPDATE since 20261022100000: the person act holds the same lock on every estate
    -- context while it erases them, so this waits for it and then reads the erased state.
    SELECT owner_table, owner_id, is_active INTO v_owner_table, v_owner_id, v_is_active
      FROM kb_contexts WHERE id = v_context
       FOR NO KEY UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'context_rename: context % not found', v_context;
    END IF;

    -- The floor this migration exists for. Before the authorization gate deliberately: whether the
    -- row is renameable at all does not depend on who is asking, and a caller who does not
    -- administer it must still get 42501 rather than learning its retirement state.
    IF NOT v_is_active THEN
        RAISE EXCEPTION 'context_rename: context % is retired', v_context
              USING ERRCODE = 'P0002';
    END IF;

    -- The acting principal IS the emitter; kb_entities.profile_id (NOT NULL) is the human/machine
    -- behind the actor. Authorize that profile, not the emitter entity.
    SELECT profile_id INTO v_actor FROM kb_entities WHERE id = p_emitter;
    IF v_actor IS NULL THEN
        RAISE EXCEPTION 'context_rename: emitter % has no profile', p_emitter
              USING ERRCODE = '42501';
    END IF;

    IF NOT is_system_admin(v_actor) THEN
        -- Context side: the actor must administer the owner (own it directly, or owner/maintainer
        -- on the owning team -- matching `caller_administers_context`).
        IF v_owner_table = 'kb_profiles' THEN
            IF v_owner_id IS DISTINCT FROM v_actor THEN
                RAISE EXCEPTION 'context_rename: actor does not own the context'
                      USING ERRCODE = '42501';
            END IF;
        ELSIF v_owner_table = 'kb_teams' THEN
            IF NOT EXISTS (SELECT 1 FROM kb_team_members tm
                             JOIN kb_teams t ON t.id = tm.team_id AND t.is_active
                            WHERE tm.team_id = v_owner_id AND tm.profile_id = v_actor
                              AND tm.role IN ('owner', 'maintainer')) THEN
                RAISE EXCEPTION 'context_rename: actor does not administer the context''s owning team'
                      USING ERRCODE = '42501';
            END IF;
        ELSE
            RAISE EXCEPTION 'context_rename: context % has unknown owner table %',
                  v_context, v_owner_table USING ERRCODE = '42501';
        END IF;
    END IF;

    v_ev := _event_append('context_renamed', p_emitter, 'kb_contexts', v_context, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_context_renamed(v_ev, p_payload);
END;
$function$;

CREATE OR REPLACE FUNCTION public.context_retire(p_payload jsonb, p_emitter uuid, p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL::uuid, p_correlation uuid DEFAULT NULL::uuid)
 RETURNS uuid
 LANGUAGE plpgsql
AS $function$
DECLARE
    v_ev uuid;
    v_context uuid := (p_payload->>'context_id')::uuid;
    v_owner_table text;
    v_owner_id uuid;
    v_is_active boolean;
    v_actor uuid;
BEGIN
    -- Existence + current owner + current activation, in one read. The owner drives the
    -- "administers the context" gate; the activation flag drives the no-op refusal below.
    -- FOR NO KEY UPDATE since 20261022100000: the person act holds the same lock on every estate
    -- context while it erases them, so this waits for it and then reads the erased state.
    SELECT owner_table, owner_id, is_active INTO v_owner_table, v_owner_id, v_is_active
      FROM kb_contexts WHERE id = v_context
       FOR NO KEY UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'context_retire: context % not found', v_context;
    END IF;

    IF NOT v_is_active THEN
        RAISE EXCEPTION 'context_retire: context % is already retired', v_context
              USING ERRCODE = 'P0002';
    END IF;

    -- The acting principal IS the emitter; kb_entities.profile_id (NOT NULL) is the human/machine
    -- behind the actor. Authorize that profile, not the emitter entity.
    SELECT profile_id INTO v_actor FROM kb_entities WHERE id = p_emitter;
    IF v_actor IS NULL THEN
        RAISE EXCEPTION 'context_retire: emitter % has no profile', p_emitter
              USING ERRCODE = '42501';
    END IF;

    IF NOT is_system_admin(v_actor) THEN
        -- Context side: the actor must administer the owner (own it directly, or owner/maintainer
        -- on the owning team — matching `caller_administers_context`).
        IF v_owner_table = 'kb_profiles' THEN
            IF v_owner_id IS DISTINCT FROM v_actor THEN
                RAISE EXCEPTION 'context_retire: actor does not own the context'
                      USING ERRCODE = '42501';
            END IF;
        ELSIF v_owner_table = 'kb_teams' THEN
            IF NOT EXISTS (SELECT 1 FROM kb_team_members tm
                             JOIN kb_teams t ON t.id = tm.team_id AND t.is_active
                            WHERE tm.team_id = v_owner_id AND tm.profile_id = v_actor
                              AND tm.role IN ('owner', 'maintainer')) THEN
                RAISE EXCEPTION 'context_retire: actor does not administer the context''s owning team'
                      USING ERRCODE = '42501';
            END IF;
        ELSE
            RAISE EXCEPTION 'context_retire: context % has unknown owner table %',
                  v_context, v_owner_table USING ERRCODE = '42501';
        END IF;
    END IF;

    v_ev := _event_append('context_retired', p_emitter, 'kb_contexts', v_context, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_context_retired(v_ev, p_payload);
END;
$function$;

CREATE OR REPLACE FUNCTION public.context_restore(p_payload jsonb, p_emitter uuid, p_metadata jsonb DEFAULT '{}'::jsonb, p_invocation uuid DEFAULT NULL::uuid, p_correlation uuid DEFAULT NULL::uuid)
 RETURNS uuid
 LANGUAGE plpgsql
AS $function$
DECLARE
    v_ev uuid;
    v_context uuid := (p_payload->>'context_id')::uuid;
    v_owner_table text;
    v_owner_id uuid;
    v_is_active boolean;
    v_actor uuid;
BEGIN
    -- FOR NO KEY UPDATE since 20261022100000: the person act holds the same lock on every estate
    -- context while it erases them, so this waits for it and then reads the erased state.
    SELECT owner_table, owner_id, is_active INTO v_owner_table, v_owner_id, v_is_active
      FROM kb_contexts WHERE id = v_context
       FOR NO KEY UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'context_restore: context % not found', v_context;
    END IF;


    IF v_is_active THEN
        RAISE EXCEPTION 'context_restore: context % is already active', v_context
              USING ERRCODE = 'P0002';
    END IF;

    SELECT profile_id INTO v_actor FROM kb_entities WHERE id = p_emitter;
    IF v_actor IS NULL THEN
        RAISE EXCEPTION 'context_restore: emitter % has no profile', p_emitter
              USING ERRCODE = '42501';
    END IF;

    IF NOT is_system_admin(v_actor) THEN
        IF v_owner_table = 'kb_profiles' THEN
            IF v_owner_id IS DISTINCT FROM v_actor THEN
                RAISE EXCEPTION 'context_restore: actor does not own the context'
                      USING ERRCODE = '42501';
            END IF;
        ELSIF v_owner_table = 'kb_teams' THEN
            IF NOT EXISTS (SELECT 1 FROM kb_team_members tm
                             JOIN kb_teams t ON t.id = tm.team_id AND t.is_active
                            WHERE tm.team_id = v_owner_id AND tm.profile_id = v_actor
                              AND tm.role IN ('owner', 'maintainer')) THEN
                RAISE EXCEPTION 'context_restore: actor does not administer the context''s owning team'
                      USING ERRCODE = '42501';
            END IF;
        ELSE
            RAISE EXCEPTION 'context_restore: context % has unknown owner table %',
                  v_context, v_owner_table USING ERRCODE = '42501';
        END IF;
    END IF;

    -- An erased context never comes back (R7). Refused whoever asks, an instance admin included.
    -- After the authorization gate, so only a caller who administers the context learns it was
    -- erased. Keyed by the context's own context_erased event, not by its owner, so a co-admin of
    -- an erased person's personal team is refused too. SQLSTATE TE001, which
    -- map_context_write_err renders as 410.
    IF EXISTS (SELECT 1 FROM kb_events e
                 JOIN kb_event_types t ON t.id = e.event_type_id
                WHERE t.name = 'context_erased'
                  AND e.producing_anchor_table = 'kb_contexts'
                  AND e.producing_anchor_id = v_context) THEN
        RAISE EXCEPTION 'context_restore: context % was erased', v_context
              USING ERRCODE = 'TE001';
    END IF;

    v_ev := _event_append('context_restored', p_emitter, 'kb_contexts', v_context, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_context_restored(v_ev, p_payload);
END;
$function$;

CREATE OR REPLACE FUNCTION public._admin_team_membership_write(p_emitter uuid, p_actor uuid, p_via text, p_act text, p_team uuid, p_profile uuid, p_role team_role, p_expected_role team_role, p_expected_source team_member_source, p_new_source team_member_source, p_guard_last_owner boolean, p_invited_by uuid DEFAULT NULL::uuid, p_correlation uuid DEFAULT NULL::uuid)
 RETURNS text
 LANGUAGE plpgsql
 SET search_path TO 'public', 'pg_temp'
AS $function$
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
    -- Nothing is written onto a soft-deleted team (soft-delete design D4). The Rust lock helper
    -- answers 404 before this; this is the backstop for any other caller.
    IF NOT EXISTS (SELECT 1 FROM kb_teams WHERE id = p_team AND is_active) THEN
        RAISE EXCEPTION 'team membership write: team % is not live', p_team;
    END IF;
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
$function$;

CREATE OR REPLACE FUNCTION public.auto_join_reconcile()
 RETURNS TABLE(team_slug text, profile_handle text)
 LANGUAGE sql
AS $function$
    WITH ins AS (
        INSERT INTO kb_team_members (team_id, profile_id, role)
        SELECT t.id, p.id, t.auto_join_role
          FROM kb_teams t
         CROSS JOIN kb_profiles p
         WHERE t.auto_join_role IS NOT NULL AND t.is_active
           AND has_system_access(p.id)
           AND NOT EXISTS (SELECT 1 FROM kb_machine_clients mc WHERE mc.profile_id = p.id)
           AND NOT EXISTS (
               SELECT 1 FROM kb_team_members m
                WHERE m.team_id = t.id AND m.profile_id = p.id)
        ON CONFLICT (team_id, profile_id) DO NOTHING
        RETURNING team_id, profile_id
    )
    SELECT t.slug, pr.handle
      FROM ins
      JOIN kb_teams t     ON t.id = ins.team_id
      JOIN kb_profiles pr ON pr.id = ins.profile_id
     ORDER BY t.slug, pr.handle;
$function$;

CREATE OR REPLACE FUNCTION public.backfill_auto_join_team(p_team uuid)
 RETURNS void
 LANGUAGE plpgsql
AS $function$
DECLARE
    v_role team_role;
BEGIN
    SELECT auto_join_role INTO v_role FROM kb_teams WHERE id = p_team AND is_active;
    IF v_role IS NULL THEN
        RETURN;  -- not a live auto-join team; nothing to backfill
    END IF;
    INSERT INTO kb_team_members (team_id, profile_id, role)
    SELECT p_team, p.id, v_role
      FROM kb_profiles p
     WHERE has_system_access(p.id)
       AND NOT EXISTS (SELECT 1 FROM kb_machine_clients mc WHERE mc.profile_id = p.id)
    ON CONFLICT (team_id, profile_id) DO NOTHING;
END;
$function$;

CREATE OR REPLACE FUNCTION public.ensure_auto_join_memberships(p_profile uuid)
 RETURNS void
 LANGUAGE plpgsql
AS $function$
BEGIN
    IF NOT has_system_access(p_profile) THEN
        RETURN;  -- not eligible (standing not approved); enroll nothing
    END IF;
    IF EXISTS (SELECT 1 FROM kb_machine_clients mc WHERE mc.profile_id = p_profile) THEN
        RETURN;  -- a machine: auto-join never enrolls one
    END IF;
    INSERT INTO kb_team_members (team_id, profile_id, role)
    SELECT t.id, p_profile, t.auto_join_role
      FROM kb_teams t
     WHERE t.auto_join_role IS NOT NULL AND t.is_active
    ON CONFLICT (team_id, profile_id) DO NOTHING;
END;
$function$;

CREATE OR REPLACE FUNCTION public.steward_team_contexts(p_cogmap uuid)
 RETURNS TABLE(context_id uuid)
 LANGUAGE sql
 STABLE
AS $function$
    -- The UNION of every joined team's reachable contexts: the candidate set of contexts where a
    -- resource/event change could have landed that this cogmap might need to distill. This is a
    -- CHANGE-DETECTION scope, NOT a visibility gate — it only feeds event counts and the latest-event
    -- watermark; no resource content is read through it. Cross-team leak-safety is enforced DOWNSTREAM
    -- at the resource grain by the cogmap-principal distillation read (resources_accessible_to_cogmap,
    -- the producer-intersection). Union is deliberate: as a "did anything I might need to look at
    -- change?" trigger, over-approximation is safe (a wasted tick), while an intersection would
    -- UNDER-trigger — miss a distillable-but-grant-visible change whose home context isn't shared to
    -- every team — and silently stale the map.
    --
    -- A team reaches a context it OWNS directly (ownership does not inherit down the team DAG)...
    SELECT c.id AS context_id
      FROM kb_team_cogmaps tc
      JOIN kb_contexts c ON c.owner_table = 'kb_teams' AND c.owner_id = tc.team_id
      JOIN kb_teams t ON t.id = tc.team_id AND t.is_active
     WHERE tc.cogmap_id = p_cogmap
    UNION
    -- ...or one SHARED to it or any of its ANCESTORS (shares inherit down the team DAG via
    -- team_ancestors, the same up-traversal vis_team / anchor_readable_by_profile use for context reach).
    SELECT ktc.context_id
      FROM kb_team_cogmaps tc
      CROSS JOIN LATERAL team_ancestors(tc.team_id) a
      JOIN kb_team_contexts ktc ON ktc.team_id = a.team_id
     WHERE tc.cogmap_id = p_cogmap;
$function$;

CREATE OR REPLACE FUNCTION public.cogmaps_share_a_team(p_cogmap_a uuid, p_cogmap_b uuid)
 RETURNS boolean
 LANGUAGE sql
 STABLE
AS $function$
    SELECT EXISTS (
        SELECT 1
        FROM kb_team_cogmaps a
        JOIN kb_team_cogmaps b ON a.team_id = b.team_id
        JOIN kb_teams t ON t.id = a.team_id AND t.is_active
        WHERE a.cogmap_id = p_cogmap_a AND b.cogmap_id = p_cogmap_b
    );
$function$;

CREATE OR REPLACE FUNCTION public.contexts_readable_by_teams(p_profile uuid, p_teams uuid[])
 RETURNS TABLE(context_id uuid)
 LANGUAGE sql
 STABLE
AS $function$
    -- 1. personal context
    SELECT c.id
    FROM kb_contexts c
    WHERE c.owner_table = 'kb_profiles' AND c.owner_id = p_profile
      AND c.is_active

    UNION

    -- 2. context OWNED by an enclosing team.
    SELECT c.id
    FROM kb_contexts c
    WHERE c.owner_table = 'kb_teams' AND c.owner_id = ANY(p_teams)
      AND c.is_active

    UNION

    -- 3. context SHARED to an enclosing team. context_is_live: retired, or owned by a soft-deleted
    --    team, confers nothing through a share (soft-delete design D3).
    SELECT tc.context_id
    FROM kb_team_contexts tc
    WHERE tc.team_id = ANY(p_teams)
      AND context_is_live(tc.context_id)

    UNION

    -- 4. explicit read-grant on the context (profile-anchored, or team-anchored on a reachable team);
    --    floored by context_is_live like arm 3.
    SELECT g.subject_id
    FROM kb_access_grants g
    WHERE g.subject_table = 'kb_contexts' AND g.can_read
      AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
         OR (g.principal_table = 'kb_teams' AND g.principal_id = ANY(p_teams)) )
      AND context_is_live(g.subject_id);
$function$;

CREATE OR REPLACE FUNCTION public.context_authorable_by_profile(p_profile uuid, p_context uuid)
 RETURNS boolean
 LANGUAGE sql
 STABLE
AS $function$
    SELECT EXISTS (
        -- personal-owned: the owner authors their own context
        SELECT 1
        FROM kb_contexts c
        WHERE c.id = p_context
          AND c.owner_table = 'kb_profiles' AND c.owner_id = p_profile
          AND c.is_active

        UNION ALL

        -- team-owned: DIRECT membership in the OWNING team, with an authoring role.
        --
        -- NARROWED. This arm previously ancestor-expanded, so transitive membership in an enclosing
        -- team conferred write on that team's context. Read inherits up the enclosure chain;
        -- mutation does not. `watcher` is read-only. Carried forward verbatim from the definition
        -- this CREATE OR REPLACE supersedes: it states a live invariant, and dropping it invites
        -- the next reader to re-widen the arm.
        SELECT 1
        FROM kb_contexts c
        JOIN kb_team_members tm ON tm.team_id = c.owner_id AND tm.profile_id = p_profile
        JOIN kb_teams t ON t.id = c.owner_id AND t.is_active
        WHERE c.id = p_context
          AND c.owner_table = 'kb_teams'
          AND tm.role IN ('owner', 'maintainer', 'member')
          AND c.is_active
    )
    -- explicit write-grant, floored here because profile_explicit_grant is subject-polymorphic
    -- and cannot know a context is retired, or owned by a soft-deleted team (context_is_live,
    -- soft-delete design D3).
    OR ( profile_explicit_grant(p_profile, 'write', 'kb_contexts', p_context)
         AND context_is_live(p_context) );
$function$;

CREATE OR REPLACE FUNCTION public.can(p_principal_table text, p_principal_id uuid, p_action text, p_subject_table text, p_subject_id uuid)
 RETURNS boolean
 LANGUAGE sql
 STABLE
AS $function$
    SELECT CASE p_principal_table
        WHEN 'kb_profiles' THEN
            (
                profile_explicit_grant(p_principal_id, p_action, p_subject_table, p_subject_id)
                -- Subject-liveness floor: the explicit arm answers a subject only while a live
                -- row backs it. Kinds with no liveness column are admitted (ELSE true).
                AND CASE p_subject_table
                        WHEN 'kb_resources' THEN EXISTS (
                            SELECT 1 FROM kb_resources r
                             WHERE r.id = p_subject_id AND r.is_active)
                        WHEN 'kb_contexts' THEN context_is_live(p_subject_id)
                        ELSE true
                    END
            )
            OR derived_access_profile(p_principal_id, p_action, p_subject_table, p_subject_id)
        WHEN 'kb_cogmaps' THEN
            p_subject_table = 'kb_resources' AND p_action = 'read'
            AND p_subject_id IN (SELECT resource_id FROM resources_accessible_to_cogmap(p_principal_id))
        ELSE false
    END;
$function$;

SELECT declare_migration(
    20261110000000,
    'additive',
    'New STABLE function context_is_live(uuid) and body-only CREATE OR REPLACE, signatures and return '
    'types unchanged, of context_reassign, context_rename, context_retire, context_restore, '
    '_admin_team_membership_write, auto_join_reconcile, backfill_auto_join_team, '
    'ensure_auto_join_memberships, steward_team_contexts, cogmaps_share_a_team, '
    'contexts_readable_by_teams, context_authorable_by_profile and can. Behaviour change, '
    'deliberate: a soft-deleted team confers no authority over its contexts, a context it owns reads '
    'as retired, and nothing is written onto it. No row is rewritten. Design: temper-artifacts '
    'specs/2026-10-31-soft-deleted-team-confers-nothing-design.md.'
);
