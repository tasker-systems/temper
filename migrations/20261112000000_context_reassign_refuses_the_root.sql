-- Only a system admin transfers a context into temper-system (team DAG review G14).
--
-- temper-system is every personal team's parent, and reading inherits down the team tree, so a
-- context it owns reaches every profile on the instance. context_reassign has refused a non-admin
-- transfer into the GATING team since 20260715000010, but the gating team and temper-system are the
-- same team only when an instance points kb_system_settings.gating_team_slug at temper-system. It is
-- NULL by default and may name any shared team, and then a non-admin owner or maintainer of
-- temper-system could move a context into everyone's reach.
--
-- This is the plpgsql twin of the Rust two-sided gate (`authz::two_sided::TwoSidedAuthority`),
-- which refuses the root beside the gating team in the same release
-- (`access_service::is_root_or_gating_team`). The Rust gate is the pre-check; this is the atomic
-- enforcement, so the two must agree.
--
-- A body-only CREATE OR REPLACE of context_reassign as 20261110000100 left it, with one added
-- refusal on the non-admin target side and nothing else. Signature and return type unchanged. A
-- system admin is unaffected.

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
        -- Target-team side: must be a team, neither the gating team nor the root, and the actor
        -- owner/maintainer on it. The root (temper-system) reaches every profile, as the gating
        -- team does, and the two are different teams unless an instance makes them one (G14).
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
        IF EXISTS (SELECT 1 FROM kb_teams t WHERE t.id = v_to_id AND t.slug = 'temper-system') THEN
            RAISE EXCEPTION 'context_reassign: cannot transfer into temper-system'
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
    20261112000000,
    'additive',
    'Body-only CREATE OR REPLACE of context_reassign, signature and return type unchanged: a '
    'non-admin may no longer transfer a context into temper-system, as they may not into the gating '
    'team. Behaviour change, deliberate (team DAG review G14). No row is rewritten.'
);
