-- Context rename: event-sourced (name, slug) change on kb_contexts.
-- Mirrors context_reassign (20260715000010); this moves the identity pair in place.
-- Additive only: one event-type row + two functions. No table changes.
--
-- Renumbered from 20260730000020 while this branch was open: 20260731000010/20 landed on main
-- first, and a migration must sit above main's high-water mark. Never applied anywhere but a dev
-- database under the old number, so there is no applied file being edited — the rule that forbids
-- that is intact.
--
-- kb_contexts is a replay INPUT table (restored verbatim), not a projection, so this
-- projector is an idempotent re-apply on replay — the same property context_reassign states
-- (20260715000010:5-8), and the same reason an evented context mutation is safe even though
-- context create/share/unshare are un-evented.
--
-- kb_contexts has no `updated` column (context_service.rs:41 synthesizes it as
-- `c.created AS "updated!"`), so a rename bumps no row-level timestamp. Attributability rests
-- ENTIRELY on the event trail. That is why the payload carries from_name/from_slug beside
-- to_name/to_slug: the projector never reads the from_* pair, but without it nothing on disk
-- could say what the context used to be called — the row keeps no before-image and no mtime.

-- _event_append raises unless the event name is seeded. NULL payload_schema keeps it out of the
-- published-schema TYPED_EVENT_NAMES invariant (as context_reassigned and citation_audited), so
-- no schema snapshot regenerates. `category` is spelled explicitly because 20260719000010:96-98
-- dropped the column default with the standing rule at :80 — "Every future event-type
-- registration must spell `category` explicitly." The template's three-column INSERT predates
-- that migration and would now fail the NOT NULL; 20260724000110:55-57 is the current shape.
INSERT INTO kb_event_types (name, payload_schema, schema_version, category)
VALUES ('context_renamed', NULL, 1, 'domain')
ON CONFLICT (name) DO NOTHING;

-- Projection half: set the context's name and slug to (to_name, to_slug).
-- The UNIQUE(owner_table, owner_id, slug) constraint is the backstop for a slug collision under
-- the same owner (the service pre-checks and returns 409 first).
--
-- Reads only the to_* pair; the from_* pair belongs to the trail, not to the projection.
-- This half NEVER authorizes, so replayed history is not re-adjudicated against present-day
-- membership — a rename valid when it happened stays valid when it is re-applied.
CREATE FUNCTION _project_context_renamed(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_context uuid := (p_payload->>'context_id')::uuid;
BEGIN
    UPDATE kb_contexts
       SET name = p_payload->>'to_name',
           slug = p_payload->>'to_slug'
     WHERE id = v_context;
    IF NOT FOUND THEN RAISE EXCEPTION 'context_rename: context % not found', v_context; END IF;
    RETURN v_context;
END;
$$;

-- Mutation half: authorize, append the event anchored to the context itself, then project.
-- Full 5-param act-context signature (matches every mutation fn post-20260709000050).
--
-- Authorization is an INVARIANT of this function, not a caller pre-check: the RBAC gate lives
-- here, in the same transaction as the append+project, so there is no check-then-act window a
-- membership/ownership change could slip through. `context_service::rename` still runs the
-- identical administration gate up front (fast, and it renders the caller's refusal + skips the
-- event append on the common unauthorized case) — this guard is the atomic backstop that makes
-- that pre-check advisory. Only the mutation half authorizes; the projector
-- (`_project_context_renamed`, the replay path) stays a pure re-apply, so historical events
-- never re-authorize on replay.
--
-- The gate here is ADMIT/DENY ONLY. It deliberately does NOT reproduce the 403/404 split the
-- Rust pre-check renders: this function is reached only on the race path, where that split has
-- already been chosen. So EVERY refusal arm raises SQLSTATE 42501 (insufficient_privilege) and
-- the service maps the lost race to 403 rather than 500.
--
-- The rule mirrors `context_service::caller_administers_context` exactly: a system admin
-- bypasses; otherwise the acting profile (resolved from the emitter entity) must administer the
-- context's owner — own it directly, or hold owner/maintainer by DIRECT membership on the owning
-- team (matching `team_service::role_on_team`, which does not walk enclosing teams).
--
-- NO TARGET-TEAM HALF, deliberately. context_reassign's gate also checks a destination team
-- (20260715000010:77-93) because a transfer HAS one. A rename has no target: it moves two columns
-- of the same row under the same, unchanged owner. Copying that half in would refuse every
-- rename, since the payload carries no to_owner_table for it to read.
CREATE FUNCTION context_rename(p_payload jsonb, p_emitter uuid,
                               p_metadata jsonb DEFAULT '{}'::jsonb,
                               p_invocation uuid DEFAULT NULL,
                               p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_ev uuid;
    v_context uuid := (p_payload->>'context_id')::uuid;
    v_owner_table text;
    v_owner_id uuid;
    v_actor uuid;
BEGIN
    -- Existence + current owner (the owner drives the "administers the context" gate).
    SELECT owner_table, owner_id INTO v_owner_table, v_owner_id
      FROM kb_contexts WHERE id = v_context;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'context_rename: context % not found', v_context;
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
        -- on the owning team — matching `caller_administers_context`).
        IF v_owner_table = 'kb_profiles' THEN
            IF v_owner_id IS DISTINCT FROM v_actor THEN
                RAISE EXCEPTION 'context_rename: actor does not own the context'
                      USING ERRCODE = '42501';
            END IF;
        ELSIF v_owner_table = 'kb_teams' THEN
            IF NOT EXISTS (SELECT 1 FROM kb_team_members
                            WHERE team_id = v_owner_id AND profile_id = v_actor
                              AND role IN ('owner', 'maintainer')) THEN
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
$$;

-- This migration declares itself, per 20260731000010. The class question is the lagging-binary
-- test: does a binary that predates this migration keep working against the schema after it is
-- applied? Yes. Two functions are CREATEd, not replaced, so no existing signature or return type
-- moves — which is the specific failure 20260730000010 shipped, where `CREATE OR REPLACE` changed
-- `facet_set` from `uuid` to `uuid[]` and the running binary decoded by type.
--
-- The one arm worth stating rather than waving past is the `kb_event_types` row. An older binary
-- gains an event-type name it has no `EventKind` arm for, which would matter if it had to replay a
-- `context_renamed` EVENT — `from_canonical_name` would not resolve it. But applying this migration
-- creates no such event: only `context_rename` does, and only the paired binary calls it. So the
-- exposure needs a NEW binary to write history and then a ROLLBACK to an old one, which is the
-- ordinary shape-forward risk every event type carries, not a property of applying this schema
-- ahead of its binary.
SELECT declare_migration(
    20260731000030,
    'additive',
    'Two new CREATE FUNCTIONs and one event-type row. Nothing pre-existing is altered or dropped, no signature or return type moves, and a binary that predates this migration calls none of it. The kb_event_types row is inert until the paired binary fires context_rename.'
);
