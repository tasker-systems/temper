-- Context ownership transfer: event-sourced owner change on kb_contexts.
-- Mirrors resource_reassign; this moves (owner_table, owner_id) in place.
-- Additive only: one event-type row + two functions. No table changes.
--
-- kb_contexts is a replay INPUT table (restored verbatim), not a projection, so this
-- projector is an idempotent re-apply on replay (see the context-transfer plan's
-- replay-roundtrip test). This is why an evented context mutation is safe even though
-- context create/share/unshare are un-evented.

-- _event_append raises unless the event name is seeded. NULL payload_schema keeps it
-- out of the published-schema TYPED_EVENT_NAMES invariant (as resource_reassigned).
INSERT INTO kb_event_types (name, payload_schema, schema_version)
VALUES ('context_reassigned', NULL, 1)
ON CONFLICT (name) DO NOTHING;

-- Projection half: set the context's owner to (to_owner_table, to_owner_id).
-- The UNIQUE(owner_table, owner_id, slug) constraint is the backstop for a slug
-- collision under the new owner (the service pre-checks and returns 409 first).
CREATE FUNCTION _project_context_reassigned(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_context uuid := (p_payload->>'context_id')::uuid;
BEGIN
    UPDATE kb_contexts
       SET owner_table = p_payload->>'to_owner_table',
           owner_id    = (p_payload->>'to_owner_id')::uuid
     WHERE id = v_context;
    IF NOT FOUND THEN RAISE EXCEPTION 'context_reassign: context % not found', v_context; END IF;
    RETURN v_context;
END;
$$;

-- Mutation half: authorize, append the event anchored to the context itself, then project.
-- Full 5-param act-context signature (matches every mutation fn post-20260709000050).
--
-- Authorization is an INVARIANT of this function, not a caller pre-check: the RBAC gate lives
-- here, in the same transaction as the append+project, so there is no check-then-act window a
-- membership/ownership change could slip through. `context_service::reassign` still runs the
-- identical `can_share` gate up front (fast, clean 403 + it skips the event append on the common
-- unauthorized case) — this guard is the atomic backstop that makes that pre-check advisory. Only
-- the mutation half authorizes; the projector (`_project_context_reassigned`, the replay path)
-- stays a pure re-apply, so historical events never re-authorize on replay.
--
-- The rule mirrors `context_service::can_share` exactly: a system admin bypasses; otherwise the
-- acting profile (resolved from the emitter entity) must administer the CURRENT context owner AND
-- hold owner/maintainer on the NON-gating target team. Authorization failures raise SQLSTATE 42501
-- (insufficient_privilege) so the service maps the race path to 403 rather than 500.
CREATE FUNCTION context_reassign(p_payload jsonb, p_emitter uuid,
                                 p_metadata jsonb DEFAULT '{}'::jsonb,
                                 p_invocation uuid DEFAULT NULL,
                                 p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
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
    SELECT owner_table, owner_id INTO v_owner_table, v_owner_id
      FROM kb_contexts WHERE id = v_context;
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
        IF NOT EXISTS (SELECT 1 FROM kb_team_members
                        WHERE team_id = v_to_id AND profile_id = v_actor
                          AND role IN ('owner', 'maintainer')) THEN
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
            IF NOT EXISTS (SELECT 1 FROM kb_team_members
                            WHERE team_id = v_owner_id AND profile_id = v_actor
                              AND role IN ('owner', 'maintainer')) THEN
                RAISE EXCEPTION 'context_reassign: actor does not administer the context''s owning team'
                      USING ERRCODE = '42501';
            END IF;
        ELSE
            RAISE EXCEPTION 'context_reassign: context % has unknown owner table %',
                  v_context, v_owner_table USING ERRCODE = '42501';
        END IF;
    END IF;

    v_ev := _event_append('context_reassigned', p_emitter, 'kb_contexts', v_context, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_context_reassigned(v_ev, p_payload);
END;
$$;
