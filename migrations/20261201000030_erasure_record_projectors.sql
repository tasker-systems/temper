-- The erasure record's projectors: one per erasure record kind (20261201000020), each writing the
-- tables of 20261201000010_erasure_records_schema.sql from its own event's payload.
--
-- THE CONSTRAINTS A LATER EDIT MUST NOT BREAK:
--
--   * EVERY PROJECTOR IS PAYLOAD-DRIVEN (spec R8). What it writes comes from its payload and, for a
--     timestamp, its own event's occurred_at, never now(). The only state it reads is the row it is
--     about to change (a flag already set, an edge already folded, a member no longer open). Replay
--     calls the same function at the event's position (replay.rs), so a live call and a replayed
--     one write the same rows.
--   * A MAP ENTRY CARRIES NO STATUS. An entry with prior_erasure and prior_event is born erased;
--     every other is born pending. An entry with only one of the two raises.
--   * THE BOUND FLAGS ARE SET ON PENDING MEMBERS ONLY, through _erasure_member_flag, which both
--     the declaration and an arrival call. A row born erased was finished by an earlier erasure,
--     which set its flags then.
--   * THE EDGE FOLD IS ONE DEFINITION. _edge_fold_apply holds the fold's statements, and both
--     _project_relationship_folded and _erasure_member_flag call it.
--   * erasure_batch_applied PROJECTS REFUSALS ONLY. It is not an authority: an erased item is
--     marked by its own authority event, never by the batch summary.
--   * A ROW MARKED erased LEAVES THE DRAIN. Every statement that marks a row erased also clears
--     next_attempt_at and lease_expires_at (kb_erasure_members_erased_is_off_the_drain).

-- ---------------------------------------------------------------------------
-- Section 1. The edge fold, shared.
-- ---------------------------------------------------------------------------
-- The statements of _project_relationship_folded (20260727000030_edge_owned_properties.sql),
-- lifted unchanged: the edge is folded with the folding event as its last_event_id, and every
-- live property the edge owns folds with it. It requires the edge to exist and does not check:
-- each caller reads the edge first and raises in its own words when it is missing.
CREATE FUNCTION _edge_fold_apply(p_event uuid, p_edge uuid)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    UPDATE kb_edges SET is_folded = true, last_event_id = p_event WHERE id = p_edge;
    -- Cascade: an edge's facets do not outlive the edge.
    UPDATE kb_properties SET is_folded = true, last_event_id = p_event
     WHERE owner_table = 'kb_edges' AND owner_id = p_edge AND NOT is_folded;
END;
$$;

COMMENT ON FUNCTION _edge_fold_apply(uuid, uuid) IS
$c$Folds one edge under p_event: kb_edges.is_folded and last_event_id, and every live kb_properties
row the edge owns. The one body of the fold, called by _project_relationship_folded and by the
erasure flag helper _erasure_member_flag. It requires the edge to exist; its callers check.$c$;

CREATE OR REPLACE FUNCTION _project_relationship_folded(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_edge uuid := (p_payload->>'edge_id')::uuid;
BEGIN
    PERFORM 1 FROM kb_edges WHERE id = v_edge;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'relationship_fold: edge % not found', v_edge;
    END IF;
    PERFORM _edge_fold_apply(p_event, v_edge);
    RETURN v_edge;
END;
$$;

-- ---------------------------------------------------------------------------
-- Section 2. The bound flags of one member.
-- ---------------------------------------------------------------------------
-- A resource and a context are flagged inactive and nothing else: no `updated` stamp and no
-- last_event_id, and a context's name and slug are left as they stand (_project_context_erased,
-- 20261022100000, rewrites them from its own payload, which this event does not carry). An edge
-- is folded unless it is folded already, so an earlier fold keeps its own event. Cogmaps and
-- blobs carry no flag this helper sets.
CREATE FUNCTION _erasure_member_flag(p_event uuid, p_member_table text, p_member_id uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_folded boolean;
BEGIN
    CASE p_member_table
        WHEN 'kb_resources' THEN
            UPDATE kb_resources SET is_active = false WHERE id = p_member_id AND is_active;
        WHEN 'kb_contexts' THEN
            UPDATE kb_contexts SET is_active = false WHERE id = p_member_id AND is_active;
        WHEN 'kb_edges' THEN
            SELECT is_folded INTO v_folded FROM kb_edges WHERE id = p_member_id;
            IF NOT FOUND THEN
                RAISE EXCEPTION 'erasure member flag: mapped edge % not found', p_member_id
                    USING ERRCODE = 'no_data_found';
            END IF;
            IF NOT v_folded THEN
                PERFORM _edge_fold_apply(p_event, p_member_id);
            END IF;
        ELSE
            NULL;
    END CASE;
END;
$$;

COMMENT ON FUNCTION _erasure_member_flag(uuid, text, uuid) IS
$c$Sets the bound flag of one pending erasure member under p_event: kb_resources.is_active and
kb_contexts.is_active go false, and an unfolded edge folds through _edge_fold_apply. A mapped edge
that does not exist raises. Cogmaps and blobs carry no flag here. Called by
_project_erasure_declared and _project_erasure_member_arrived.$c$;

-- ---------------------------------------------------------------------------
-- Section 3. erasure_declared.
-- ---------------------------------------------------------------------------
-- The erasure row, the estate snapshot (estates only), the map, the former holders, and the flags
-- of the map's pending members. A legacy payload is projected with legacy = true and no manifest
-- and no request, whatever else it carries.
CREATE FUNCTION _project_erasure_declared(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_erasure uuid    := (p_payload->>'erasure_id')::uuid;
    v_kind    text    := p_payload->>'kind';
    v_legacy  boolean := coalesce((p_payload->>'legacy')::boolean, false);
    v_counts  jsonb   := p_payload->'counts';
    v_member  record;
BEGIN
    IF v_erasure IS NULL THEN
        RAISE EXCEPTION 'erasure_declared event % carries no erasure_id', p_event
            USING ERRCODE = 'not_null_violation';
    END IF;
    IF EXISTS (SELECT 1 FROM jsonb_array_elements(coalesce(p_payload->'map', '[]'::jsonb)) AS m(entry)
                WHERE (m.entry ? 'prior_erasure') <> (m.entry ? 'prior_event')) THEN
        RAISE EXCEPTION 'erasure_declared event %: a map entry names prior_erasure or prior_event without the other', p_event
            USING ERRCODE = 'check_violation';
    END IF;

    INSERT INTO kb_erasures (id, kind, subject_table, subject_id, request_ref,
                             manifest_version, manifest_digest, legacy, declared_event_id)
    VALUES (v_erasure,
            v_kind,
            p_payload->>'subject_table',
            (p_payload->>'subject_id')::uuid,
            CASE WHEN v_legacy THEN NULL ELSE (p_payload->>'request_ref')::uuid END,
            CASE WHEN v_legacy THEN NULL ELSE p_payload->>'manifest_version' END,
            CASE WHEN v_legacy THEN NULL ELSE p_payload->>'manifest_digest' END,
            v_legacy,
            p_event);

    IF v_kind = 'estate' THEN
        IF v_counts IS NULL THEN
            RAISE EXCEPTION 'erasure_declared event %: an estate declaration carries no counts', p_event
                USING ERRCODE = 'not_null_violation';
        END IF;
        INSERT INTO kb_erasure_estate_snapshots (erasure_id, resource_count, context_count,
                                                 cogmap_count, blob_count, edge_count, checksum)
        VALUES (v_erasure,
                coalesce((v_counts->>'kb_resources')::integer, 0),
                coalesce((v_counts->>'kb_contexts')::integer, 0),
                coalesce((v_counts->>'kb_cogmaps')::integer, 0),
                coalesce((v_counts->>'kb_blobs')::integer, 0),
                coalesce((v_counts->>'kb_edges')::integer, 0),
                p_payload->>'checksum');
    END IF;

    -- One statement: the self-FK to a container member is checked at its end, so the entries'
    -- order does not matter here (the payload lists containers first all the same).
    INSERT INTO kb_erasure_members (erasure_id, member_table, member_id, parent_table, parent_id,
                                    status, prior_erasure_id, erased_by_event_id,
                                    next_attempt_at, lease_expires_at)
    SELECT v_erasure,
           m.entry->>'table',
           (m.entry->>'id')::uuid,
           m.entry->'parent'->>'table',
           (m.entry->'parent'->>'id')::uuid,
           CASE WHEN m.entry ? 'prior_event' THEN 'erased' ELSE 'pending' END,
           (m.entry->>'prior_erasure')::uuid,
           (m.entry->>'prior_event')::uuid,
           NULL,
           NULL
      FROM jsonb_array_elements(coalesce(p_payload->'map', '[]'::jsonb)) WITH ORDINALITY AS m(entry, ord)
     ORDER BY m.ord;

    INSERT INTO kb_erasure_former_holders (erasure_id, profile_id)
    SELECT v_erasure, h.profile::uuid
      FROM jsonb_array_elements_text(coalesce(p_payload->'holders', '[]'::jsonb)) AS h(profile);

    FOR v_member IN
        SELECT m.member_table, m.member_id
          FROM kb_erasure_members m
         WHERE m.erasure_id = v_erasure AND m.status = 'pending'
         ORDER BY m.member_table COLLATE "C", m.member_id
    LOOP
        PERFORM _erasure_member_flag(p_event, v_member.member_table, v_member.member_id);
    END LOOP;
END;
$$;

COMMENT ON FUNCTION _project_erasure_declared(uuid, jsonb) IS
$c$Projects an erasure_declared payload: the kb_erasures row (declared_event_id = this event), the
estate snapshot for an estate, one kb_erasure_members row per map entry (born erased when the entry
names prior_erasure and prior_event, else pending), the former holders, and the bound flags of the
pending members (_erasure_member_flag). A legacy payload yields legacy = true with no manifest and
no request_ref. Payload-driven; replay calls it at the event's position.$c$;

-- ---------------------------------------------------------------------------
-- Section 4. erasure_member_arrived.
-- ---------------------------------------------------------------------------
CREATE FUNCTION _project_erasure_member_arrived(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_erasure uuid := (p_payload->>'erasure_id')::uuid;
    v_table   text := p_payload->'member'->>'table';
    v_member  uuid := (p_payload->'member'->>'id')::uuid;
BEGIN
    INSERT INTO kb_erasure_members (erasure_id, member_table, member_id, parent_table, parent_id,
                                    status, late)
    VALUES (v_erasure,
            v_table,
            v_member,
            p_payload->'parent'->>'table',
            (p_payload->'parent'->>'id')::uuid,
            'pending',
            true);
    PERFORM _erasure_member_flag(p_event, v_table, v_member);
END;
$$;

COMMENT ON FUNCTION _project_erasure_member_arrived(uuid, jsonb) IS
$c$Projects an erasure_member_arrived payload: one late, pending kb_erasure_members row, and its
bound flag (_erasure_member_flag). Payload-driven; replay calls it at the event's position.$c$;

-- ---------------------------------------------------------------------------
-- Section 5. erasure_scope_sealed, erasure_reopened, erasure_completed.
-- ---------------------------------------------------------------------------
CREATE FUNCTION _project_erasure_scope_sealed(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_erasure uuid := (p_payload->>'erasure_id')::uuid;
BEGIN
    UPDATE kb_erasures SET scope_sealed_event_id = p_event WHERE id = v_erasure;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'erasure_scope_sealed event %: erasure % not found', p_event, v_erasure
            USING ERRCODE = 'no_data_found';
    END IF;
END;
$$;

COMMENT ON FUNCTION _project_erasure_scope_sealed(uuid, jsonb) IS
    'Projects an erasure_scope_sealed payload: kb_erasures.scope_sealed_event_id = this event. '
    'Raises when the erasure does not exist.';

CREATE FUNCTION _project_erasure_reopened(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_erasure uuid := (p_payload->>'erasure_id')::uuid;
BEGIN
    UPDATE kb_erasures SET completed_event_id = NULL WHERE id = v_erasure;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'erasure_reopened event %: erasure % not found', p_event, v_erasure
            USING ERRCODE = 'no_data_found';
    END IF;
END;
$$;

COMMENT ON FUNCTION _project_erasure_reopened(uuid, jsonb) IS
    'Projects an erasure_reopened payload: kb_erasures.completed_event_id is cleared, so the '
    'erasure is no longer complete. Raises when the erasure does not exist.';

-- This body sets the erasure's column only. The completion step's by-hash half has no projection
-- here.
CREATE FUNCTION _project_erasure_completed(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_erasure uuid := (p_payload->>'erasure_id')::uuid;
BEGIN
    UPDATE kb_erasures SET completed_event_id = p_event WHERE id = v_erasure;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'erasure_completed event %: erasure % not found', p_event, v_erasure
            USING ERRCODE = 'no_data_found';
    END IF;
END;
$$;

COMMENT ON FUNCTION _project_erasure_completed(uuid, jsonb) IS
    'Projects an erasure_completed payload: kb_erasures.completed_event_id = this event, the '
    'latest completion. Raises when the erasure does not exist.';

-- ---------------------------------------------------------------------------
-- Section 6. erasure_batch_applied.
-- ---------------------------------------------------------------------------
-- Each refused entry is written to the item's open row in every erasure it names: pending ->
-- refused, or refused -> refused with the new reason. first_refused_at takes this event's
-- occurred_at only when it is unset, so a retry keeps the first. A named erasure whose row of the
-- item is missing or already erased raises: the batch locked exactly those open rows. Erased
-- entries are not projected here; each is marked by its own authority event.
CREATE FUNCTION _project_erasure_batch_applied(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_table       text := p_payload->>'member_table';
    v_occurred_at timestamptz;
    v_entry       jsonb;
    v_erasures    uuid[];
    v_rows        integer;
BEGIN
    SELECT occurred_at INTO v_occurred_at FROM kb_events WHERE id = p_event;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'erasure_batch_applied: event % not found', p_event
            USING ERRCODE = 'no_data_found';
    END IF;

    FOR v_entry IN
        SELECT e.entry FROM jsonb_array_elements(coalesce(p_payload->'entries', '[]'::jsonb)) AS e(entry)
    LOOP
        CONTINUE WHEN (v_entry->>'outcome') IS DISTINCT FROM 'refused';

        SELECT coalesce(array_agg(DISTINCT x::uuid), '{}')
          INTO v_erasures
          FROM jsonb_array_elements_text(coalesce(v_entry->'erasures', '[]'::jsonb)) AS x;

        UPDATE kb_erasure_members
           SET status           = 'refused',
               refusal_reason   = v_entry->>'reason',
               refusal_sqlstate = v_entry->>'sqlstate',
               refusal_routine  = v_entry->>'routine',
               first_refused_at = coalesce(first_refused_at, v_occurred_at)
         WHERE member_table = v_table
           AND member_id    = (v_entry->>'member')::uuid
           AND erasure_id   = ANY (v_erasures)
           AND status      <> 'erased';
        GET DIAGNOSTICS v_rows = ROW_COUNT;

        IF v_rows <> cardinality(v_erasures) THEN
            RAISE EXCEPTION 'erasure_batch_applied event %: % % is open in % of the % erasures it names',
                            p_event, v_table, v_entry->>'member', v_rows, cardinality(v_erasures)
                USING ERRCODE = 'check_violation';
        END IF;
    END LOOP;
END;
$$;

COMMENT ON FUNCTION _project_erasure_batch_applied(uuid, jsonb) IS
$c$Projects the refusals of an erasure_batch_applied payload onto the item's open row in each erasure
the entry names, keeping first_refused_at once set (it takes this event's occurred_at otherwise).
Raises when a named row is missing or erased. Erased entries are not projected here: the batch is
not an authority, and each erased item is marked by its own authority event.$c$;

SELECT declare_migration(
    20261201000030,
    'additive',
    'Adds the projectors _project_erasure_declared, _project_erasure_member_arrived, '
    '_project_erasure_scope_sealed, _project_erasure_reopened, _project_erasure_completed and '
    '_project_erasure_batch_applied, the helpers _erasure_member_flag and _edge_fold_apply, and '
    're-issues _project_relationship_folded over _edge_fold_apply with the same signature, the '
    'same statements and the same raise. Nothing appends the erasure record kinds yet, so a binary '
    'without this change is unaffected.'
);
