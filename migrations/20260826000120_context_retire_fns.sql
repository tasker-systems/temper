-- Context retirement, event-sourced: the two mutations `20260826000110_context_retirement.sql`
-- shipped as plain UPDATEs are re-cut here as evented acts, mirroring `context_rename`
-- (`20260731000040_context_rename_fns.sql`) exactly.
--
-- THE DEFECT THIS FIXES. `kb_contexts` is a replay INPUT table restored verbatim
-- (`crates/temper-substrate/src/replay.rs:101-125`). A test proved the consequence of leaving
-- retirement un-evented: `create -> rename -> retire -> replay` restores the input table with the
-- mangled, retired slug, but then the ledger walk re-applies the EARLIER `context_renamed` event
-- on top of that verbatim restore — and `_project_context_renamed` drives the slug straight back
-- to its pre-retirement value, because nothing on the ledger recorded that a retirement ever
-- happened. Expected `("Annual Planning", "annual-planning-retired", false)`, got
-- `("Annual Planning", "annual-planning", false)`. A second-order consequence follows from
-- `UNIQUE (owner_table, owner_id, slug)`: `create -> rename -> retire -> create a NEW context
-- under the freed slug -> replay` restores both rows verbatim, then the rename projector drives
-- the retired row's slug back onto the new row's slug -> unique violation -> replay ABORTS, not
-- just mis-projects.
--
-- `kb_teams`' soft-delete gets away with staying un-evented because it touches no identity-bearing
-- column. Retirement touches `slug`, which is exactly the column `context_rename` is evented for.
-- So both `retire` and `restore` become evented mutations here, following `context_rename`'s exact
-- shape: a projector half that is a pure re-apply (never authorizes, so replayed history is not
-- re-adjudicated) and a mutation half that carries the RBAC gate as an in-transaction invariant.
--
-- BOTH verbs need events, not just retirement. With only `context_retired`, the sequence
-- `create -> rename -> retire -> restore` would replay to a RETIRED state: restore never happened
-- on the ledger, so nothing re-applies it. Both halves are required for replay to reproduce
-- whichever of the two states the ledger actually ends on.
--
-- Does NOT touch `20260826000110_context_retirement.sql`: that migration is already applied, and
-- its two read-axis predicates (`contexts_readable_by_teams`, `context_authorable_by_profile`) are
-- untouched here — this migration adds a write path, not a new read rule.

-- ── register the event types ──────────────────────────────────────────────────────────────────
-- category IS SPELLED EXPLICITLY, per `20260731000040_context_rename_fns.sql:23-28`:
-- `kb_event_types.category` has been NOT NULL with no default since
-- `20260719000010_admin_cognition_firewall_declarative.sql:96-98`, and every registration since
-- must spell it. 'domain' puts both acts in the element trail by default, alongside
-- `context_renamed`/`context_reassigned`/`citation_audited` — a retirement or restoration is
-- inspectable and challengeable like any other domain act.
--
-- NULL payload_schema keeps both out of the published-schema `TYPED_EVENT_NAMES` invariant, exactly
-- as `context_renamed`/`context_reassigned`/`citation_audited` do — no schema snapshot regenerates.
INSERT INTO kb_event_types (name, payload_schema, schema_version, category)
VALUES ('context_retired', NULL, 1, 'domain')
ON CONFLICT (name) DO NOTHING;

INSERT INTO kb_event_types (name, payload_schema, schema_version, category)
VALUES ('context_restored', NULL, 1, 'domain')
ON CONFLICT (name) DO NOTHING;

-- ── _project_context_retired: pure re-apply, mirrors _project_context_renamed exactly ───────────
-- Flips `is_active` to false and writes the already-mangled `to_slug` from the payload — the
-- mangle itself happened in Rust (`context_service::retire`, via `next_unique_context_slug`)
-- before the event was appended; the projector only re-applies the chosen pair. Reads only
-- `to_slug`; `from_slug` belongs to the trail, not to the projection, exactly as `from_name`/
-- `from_slug` do on `context_renamed`.
--
-- NEVER authorizes: replayed history is not re-adjudicated against present-day membership — a
-- retirement valid when it happened stays valid when it is re-applied.
CREATE FUNCTION _project_context_retired(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_context uuid := (p_payload->>'context_id')::uuid;
BEGIN
    UPDATE kb_contexts
       SET is_active = false,
           slug = p_payload->>'to_slug'
     WHERE id = v_context;
    IF NOT FOUND THEN RAISE EXCEPTION 'context_retire: context % not found', v_context; END IF;
    RETURN v_context;
END;
$$;

-- ── _project_context_restored: the mirror image ─────────────────────────────────────────────────
CREATE FUNCTION _project_context_restored(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_context uuid := (p_payload->>'context_id')::uuid;
BEGIN
    UPDATE kb_contexts
       SET is_active = true,
           slug = p_payload->>'to_slug'
     WHERE id = v_context;
    IF NOT FOUND THEN RAISE EXCEPTION 'context_restore: context % not found', v_context; END IF;
    RETURN v_context;
END;
$$;

-- ── context_retire: mutation half, copied from context_rename's body structure verbatim ─────────
-- Full 5-param act-context signature (matches every mutation fn post-20260709000050).
--
-- Authorization is an INVARIANT of this function, not a caller pre-check, for the identical reason
-- `context_rename` states it (`20260731000040_context_rename_fns.sql:56-63`): the RBAC gate lives
-- here, in the same transaction as the append+project, so there is no check-then-act window a
-- membership/ownership change could slip through. `context_service::retire` still runs the
-- identical `ContextAdminAuthority` gate up front (fast, and it renders the caller's refusal on the
-- common unauthorized case) — this guard is the atomic backstop. Only the mutation half
-- authorizes; `_project_context_retired` (the replay path) stays a pure re-apply.
--
-- The gate here is ADMIT/DENY ONLY, exactly as `context_rename`'s is: this function is reached
-- only on the race path, where the 403/404 split has already been chosen by the Rust pre-check. So
-- EVERY refusal arm raises SQLSTATE 42501 (insufficient_privilege) and the service maps the lost
-- race to 403 rather than 500. The rule mirrors `context_service::caller_administers_context`
-- exactly, same as `context_rename`'s gate.
--
-- ONE ADDITION BEYOND RENAME'S SHAPE: retiring an already-retired context must refuse rather than
-- append a second `context_retired` event onto a context that is already retired — a no-op mutation
-- gets no event. Refused with `RAISE ... USING ERRCODE = 'P0002'` (no_data_found), which
-- `context_service::map_context_write_err` maps to the same 404 the old `rows_affected() == 0`
-- check rendered for this case.
CREATE FUNCTION context_retire(p_payload jsonb, p_emitter uuid,
                               p_metadata jsonb DEFAULT '{}'::jsonb,
                               p_invocation uuid DEFAULT NULL,
                               p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
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
    SELECT owner_table, owner_id, is_active INTO v_owner_table, v_owner_id, v_is_active
      FROM kb_contexts WHERE id = v_context;
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
            IF NOT EXISTS (SELECT 1 FROM kb_team_members
                            WHERE team_id = v_owner_id AND profile_id = v_actor
                              AND role IN ('owner', 'maintainer')) THEN
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
$$;

-- ── context_restore: the mirror image ────────────────────────────────────────────────────────────
-- Same gate, same shape, same P0002 no-op refusal — mirrored on "already active" instead of
-- "already retired".
CREATE FUNCTION context_restore(p_payload jsonb, p_emitter uuid,
                                p_metadata jsonb DEFAULT '{}'::jsonb,
                                p_invocation uuid DEFAULT NULL,
                                p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_ev uuid;
    v_context uuid := (p_payload->>'context_id')::uuid;
    v_owner_table text;
    v_owner_id uuid;
    v_is_active boolean;
    v_actor uuid;
BEGIN
    SELECT owner_table, owner_id, is_active INTO v_owner_table, v_owner_id, v_is_active
      FROM kb_contexts WHERE id = v_context;
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
            IF NOT EXISTS (SELECT 1 FROM kb_team_members
                            WHERE team_id = v_owner_id AND profile_id = v_actor
                              AND role IN ('owner', 'maintainer')) THEN
                RAISE EXCEPTION 'context_restore: actor does not administer the context''s owning team'
                      USING ERRCODE = '42501';
            END IF;
        ELSE
            RAISE EXCEPTION 'context_restore: context % has unknown owner table %',
                  v_context, v_owner_table USING ERRCODE = '42501';
        END IF;
    END IF;

    v_ev := _event_append('context_restored', p_emitter, 'kb_contexts', v_context, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_context_restored(v_ev, p_payload);
END;
$$;

-- This migration declares itself, per `20260731000010`'s lagging-binary test: does a binary that
-- predates this migration keep working against the schema after it is applied? Yes. Four
-- functions are CREATEd, not replaced, so no existing signature or return type moves. The two
-- `kb_event_types` rows are inert until the paired binary calls `context_retire`/`context_restore`
-- — an older binary keeps calling the plain UPDATEs `context_service::retire`/`restore` used to run
-- and never touches these functions or these event names.
--
-- ADDITIVE rather than a rewrite of `20260826000110_context_retirement.sql`, which is already
-- applied and therefore immutable: that migration is a schema-and-predicate change (one column,
-- two `CREATE OR REPLACE` read functions) and shipped correctly as far as it went, but it could not
-- have shipped the evented write path in the same breath, because the write path did not exist
-- yet in Rust at that point in the branch's history — `context_service::retire`/`restore` still ran
-- plain UPDATEs when it landed. The un-evented version could not ship *permanently*, though,
-- because `kb_contexts` is a replay input table and an un-evented slug mutation is invisible to the
-- ledger walk that re-applies every OTHER identity-bearing mutation on top of the verbatim
-- restore — see the header above for the exact defect this produces. Two new event types and four
-- new functions; nothing pre-existing is altered or dropped.
SELECT declare_migration(
    20260826000120,
    'additive',
    'Two new CREATE FUNCTIONs per verb (projector + mutation) and two new event-type rows. Nothing pre-existing is altered or dropped, no signature or return type moves, and a binary that predates this migration calls none of it. Closes the replay hole left by the un-evented context_retire/restore shipped in 20260826000110: kb_contexts is a replay input table restored verbatim, so an un-evented slug mangle is invisible to the ledger walk, which then re-applies an earlier context_renamed event on top of the verbatim restore and drives the slug back to its pre-retirement value (worse, a freed-then-reclaimed slug collision aborts replay outright via UNIQUE (owner_table, owner_id, slug)). Eventing both retire and restore lets replay reproduce whichever of the two states the ledger actually ends on.'
);
