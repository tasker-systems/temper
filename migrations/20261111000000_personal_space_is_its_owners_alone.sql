-- Personal space is its owner's alone: a system admin's arm does not reach a personal context.
--
-- Task 01a191b4-d041-737e-8c41-9696c502a50f, ruled 2026-10-31, the follow-up to 20261110000000. A
-- system admin may see that a personal team exists and which principals it admits, at what role,
-- for audit and compliance, and nothing else of personal space; and acts on personal space only
-- through erasure.
--
-- Personal space is a context owned by a profile (`@handle`) or by a personal team
-- (`kb_teams.personal_of`), and the resources homed in one. Its owner is that profile:
--
--   context_personal_owner(context)   the one statement: whose personal space, or NULL;
--   resource_personal_owner(resource) the same for the resource's home context;
--   context_is_personal / resource_is_personal   the boolean shorthands.
--
-- The four context acts re-implement their gate in plpgsql (the atomic half of the service check),
-- each with an `is_system_admin` bypass. That bypass now stops at someone else's personal context,
-- which leaves only its owner, through the ordinary arm. An admin's own personal space is theirs.
-- context_reassign additionally refuses moving any context into someone else's personal team,
-- the Rust gate's rule since 20261110000000, which its admin bypass skipped.
--
-- Erasure is untouched: the person act and resource erasure never call these four functions.
-- Bodies are carried verbatim from 20261110000100 (a soft-deleted team confers nothing) apart from
-- those lines.

CREATE OR REPLACE FUNCTION context_personal_owner(p_context uuid)
RETURNS uuid LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT CASE WHEN c.owner_table = 'kb_profiles' THEN c.owner_id ELSE t.personal_of END
      FROM kb_contexts c
      LEFT JOIN kb_teams t ON c.owner_table = 'kb_teams' AND t.id = c.owner_id
     WHERE c.id = p_context;
$$;
COMMENT ON FUNCTION context_personal_owner(uuid) IS
  'The profile whose personal space a context is (its owning profile, or its personal team''s '
  'personal_of), or NULL for a shared context.';

CREATE OR REPLACE FUNCTION context_is_personal(p_context uuid)
RETURNS boolean LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT context_personal_owner(p_context) IS NOT NULL;
$$;
COMMENT ON FUNCTION context_is_personal(uuid) IS
  'Whether a context is personal space: owned by a profile, or by a team carrying personal_of.';

CREATE OR REPLACE FUNCTION resource_personal_owner(p_resource uuid)
RETURNS uuid LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT context_personal_owner(h.anchor_id)
      FROM kb_resource_homes h
     WHERE h.resource_id = p_resource AND h.anchor_table = 'kb_contexts'
     LIMIT 1;
$$;
COMMENT ON FUNCTION resource_personal_owner(uuid) IS
  'The profile whose personal space a resource is homed in (context_personal_owner of its home '
  'context), or NULL.';

CREATE OR REPLACE FUNCTION resource_is_personal(p_resource uuid)
RETURNS boolean LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT resource_personal_owner(p_resource) IS NOT NULL;
$$;
COMMENT ON FUNCTION resource_is_personal(uuid) IS
  'Whether a resource is homed in personal space.';

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

    -- The admin arm stops at someone else's personal space (20261111000000): only its owner acts
    -- on it, through the ordinary arm below.
    IF NOT is_system_admin(v_actor)
       OR COALESCE(context_personal_owner(v_context) <> v_actor, false) THEN
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

    -- The admin arm stops at someone else's personal space (20261111000000): only its owner acts
    -- on it, through the ordinary arm below.
    IF NOT is_system_admin(v_actor)
       OR COALESCE(context_personal_owner(v_context) <> v_actor, false) THEN
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

    -- The admin arm stops at someone else's personal space (20261111000000): only its owner acts
    -- on it, through the ordinary arm below.
    IF NOT is_system_admin(v_actor)
       OR COALESCE(context_personal_owner(v_context) <> v_actor, false) THEN
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

    -- A personal team is acted on by its owner alone (20261110000000): no one else, an instance admin
    -- included, moves a context into one.
    IF EXISTS (SELECT 1 FROM kb_teams t
                WHERE v_to_table = 'kb_teams' AND t.id = v_to_id
                  AND t.personal_of IS NOT NULL AND t.personal_of IS DISTINCT FROM v_actor) THEN
        RAISE EXCEPTION 'context_reassign: cannot transfer into someone else''s personal team'
              USING ERRCODE = '42501';
    END IF;

    -- The admin arm stops at someone else's personal space (20261111000000): only its owner acts
    -- on it, through the ordinary arm below.
    IF NOT is_system_admin(v_actor)
       OR COALESCE(context_personal_owner(v_context) <> v_actor, false) THEN
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

SELECT declare_migration(
    20261111000000,
    'additive',
    'Adds context_personal_owner, context_is_personal, resource_personal_owner and '
    'resource_is_personal, and replaces context_retire, context_restore, '
    'context_rename and context_reassign (signatures unchanged) so their system-admin bypass does not '
    'apply to someone else''s personal context, and context_reassign refuses moving a context into someone else''s '
    'personal team. No table, column or constraint changes. A binary without this change keeps working '
    'for every act these allow; a system admin acting on someone''s personal context is refused (42501).'
);
