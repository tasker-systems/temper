-- The erasure record's projectors: one per erasure record kind (20261201000020), each writing the
-- tables of 20261201000010_erasure_records_schema.sql from its own event's payload.
--
-- THE CONSTRAINTS A LATER EDIT MUST NOT BREAK:
--
--   * EVERY PROJECTOR IS PAYLOAD-DRIVEN (spec R8). What it writes comes from its payload and, for a
--     timestamp, its own event's occurred_at (read from kb_events), never now(). Beyond that, the
--     only state it reads is the row it is about to change (a flag already set, an edge already
--     folded, a member no longer open). Replay calls the same function at the event's position
--     (replay.rs), so a live call and a replayed one write the same rows.
--   * A MAP ENTRY CARRIES NO STATUS. An entry with prior_erasure and prior_event is born erased;
--     every other is born pending. An entry with only one of the two raises.
--   * A DECLARATION'S CLAIMS ARE VERIFIED, NEVER TRUSTED. _project_erasure_declared compares the
--     payload's counts and checksum with the map it inserts, and raises on a mismatch or on a
--     field the declaration's kind does not carry. The checks read only the payload and the rows
--     the call inserts, so a payload that projected live projects the same way in replay.
--   * THE BOUND FLAGS ARE SET ON PENDING MEMBERS ONLY, through _erasure_member_flag, which both
--     the declaration and an arrival call. A row born erased was finished by an earlier erasure,
--     which set its flags then.
--   * THE EDGE FOLD IS ONE DEFINITION. _edge_fold_apply holds the fold's statements, and both
--     _project_relationship_folded and _erasure_member_flag call it.
--   * erasure_batch_applied PROJECTS REFUSALS ONLY. It is not an authority: an erased item is
--     marked by its own authority event, never by the batch summary.
--   * A ROW MARKED erased LEAVES THE DRAIN. Every statement that marks a row erased also clears
--     next_attempt_at and lease_expires_at (kb_erasure_members_erased_is_off_the_drain).
--   * resource_erased's MEMBERSHIP IS DECIDED BY ITS PAYLOAD ALONE. A first record carries
--     `targets`; a completion record's keys are within subject_table, subject_id, actor and
--     redacted_fields; any other shape raises. The backfill re-projects first records only.

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
-- of the map's pending members. The payload's counts and checksum are verified against the map it
-- just inserted, never stored on trust, and a field that does not belong to the declaration's kind
-- raises:
--
--   * a legacy declaration carries no manifest, no request, no counts, no checksum and no holders;
--   * counts belong to an estate only. An estate carries counts and a checksum. Its counts name
--     only the five member tables and must include every table its map holds; a count for a table
--     the map does not hold may be absent, and reads as 0. Each count equals the map's entries of
--     that table;
--   * a checksum, on an estate or on any other non-legacy declaration, equals
--     erasure_map_checksum over the rows just inserted.
--
-- Each is a writer's defect, and the projector refuses it rather than dropping, defaulting or
-- trusting what it was given. An honest historical payload never raises: the checks read only the
-- payload and the rows this call inserted, so replay re-runs them over the same inputs, and every
-- legacy declaration the backfill writes carries none of the fields they test.
CREATE FUNCTION _project_erasure_declared(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_erasure  uuid    := (p_payload->>'erasure_id')::uuid;
    v_kind     text    := p_payload->>'kind';
    v_legacy   boolean := coalesce((p_payload->>'legacy')::boolean, false);
    v_map      jsonb   := coalesce(p_payload->'map', '[]'::jsonb);
    v_counts   jsonb   := nullif(p_payload->'counts', 'null'::jsonb);
    v_checksum text    := p_payload->>'checksum';
    v_holders  jsonb   := coalesce(nullif(p_payload->'holders', 'null'::jsonb), '[]'::jsonb);
    v_member   record;
BEGIN
    IF v_erasure IS NULL THEN
        RAISE EXCEPTION 'erasure_declared event % carries no erasure_id', p_event
            USING ERRCODE = 'not_null_violation';
    END IF;
    IF EXISTS (SELECT 1 FROM jsonb_array_elements(v_map) AS m(entry)
                WHERE (m.entry ? 'prior_erasure') <> (m.entry ? 'prior_event')) THEN
        RAISE EXCEPTION 'erasure_declared event %: a map entry names prior_erasure or prior_event without the other', p_event
            USING ERRCODE = 'check_violation';
    END IF;
    IF v_legacy AND p_payload ?| ARRAY['request_ref', 'manifest_version', 'manifest_digest'] THEN
        RAISE EXCEPTION 'erasure_declared event %: a legacy declaration carries a manifest or a request', p_event
            USING ERRCODE = 'check_violation';
    END IF;
    IF v_legacy AND (v_counts IS NOT NULL OR v_checksum IS NOT NULL
                     OR jsonb_array_length(v_holders) > 0) THEN
        RAISE EXCEPTION 'erasure_declared event %: a legacy declaration carries counts, a checksum or holders', p_event
            USING ERRCODE = 'check_violation';
    END IF;
    IF v_kind IS DISTINCT FROM 'estate' AND v_counts IS NOT NULL THEN
        RAISE EXCEPTION 'erasure_declared event %: only an estate declaration carries counts', p_event
            USING ERRCODE = 'check_violation';
    END IF;

    IF v_kind = 'estate' THEN
        IF v_counts IS NULL THEN
            RAISE EXCEPTION 'erasure_declared event %: an estate declaration carries no counts', p_event
                USING ERRCODE = 'not_null_violation';
        END IF;
        IF v_checksum IS NULL THEN
            RAISE EXCEPTION 'erasure_declared event %: an estate declaration carries no checksum', p_event
                USING ERRCODE = 'not_null_violation';
        END IF;
        IF EXISTS (SELECT 1 FROM jsonb_array_elements(v_map) AS m(entry)
                    WHERE NOT v_counts ? (m.entry->>'table')) THEN
            RAISE EXCEPTION 'erasure_declared event %: the estate counts omit a table its map holds', p_event
                USING ERRCODE = 'check_violation';
        END IF;
        IF EXISTS (SELECT 1 FROM jsonb_object_keys(v_counts) AS k(tbl)
                    WHERE k.tbl NOT IN ('kb_resources', 'kb_contexts', 'kb_cogmaps',
                                        'kb_blobs', 'kb_edges')) THEN
            RAISE EXCEPTION 'erasure_declared event %: the estate counts name a table that is not a member table', p_event
                USING ERRCODE = 'check_violation';
        END IF;
        IF EXISTS (SELECT 1
                     FROM unnest(ARRAY['kb_resources', 'kb_contexts', 'kb_cogmaps',
                                       'kb_blobs', 'kb_edges']) AS t(tbl)
                    WHERE coalesce((v_counts->>t.tbl)::integer, 0)
                          <> (SELECT count(*) FROM jsonb_array_elements(v_map) AS m(entry)
                               WHERE m.entry->>'table' = t.tbl)) THEN
            RAISE EXCEPTION 'erasure_declared event %: the estate counts are not the map''s', p_event
                USING ERRCODE = 'check_violation';
        END IF;
    END IF;

    INSERT INTO kb_erasures (id, kind, subject_table, subject_id, request_ref,
                             manifest_version, manifest_digest, legacy, declared_event_id)
    VALUES (v_erasure,
            v_kind,
            p_payload->>'subject_table',
            (p_payload->>'subject_id')::uuid,
            (p_payload->>'request_ref')::uuid,
            p_payload->>'manifest_version',
            p_payload->>'manifest_digest',
            v_legacy,
            p_event);

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
      FROM jsonb_array_elements(v_map) WITH ORDINALITY AS m(entry, ord)
     ORDER BY m.ord;

    IF v_checksum IS NOT NULL AND v_checksum IS DISTINCT FROM erasure_map_checksum(v_erasure) THEN
        RAISE EXCEPTION 'erasure_declared event %: the checksum is not the map''s (erasure_map_checksum)', p_event
            USING ERRCODE = 'check_violation';
    END IF;

    IF v_kind = 'estate' THEN
        INSERT INTO kb_erasure_estate_snapshots (erasure_id, resource_count, context_count,
                                                 cogmap_count, blob_count, edge_count, checksum)
        VALUES (v_erasure,
                coalesce((v_counts->>'kb_resources')::integer, 0),
                coalesce((v_counts->>'kb_contexts')::integer, 0),
                coalesce((v_counts->>'kb_cogmaps')::integer, 0),
                coalesce((v_counts->>'kb_blobs')::integer, 0),
                coalesce((v_counts->>'kb_edges')::integer, 0),
                v_checksum);
    END IF;

    INSERT INTO kb_erasure_former_holders (erasure_id, profile_id)
    SELECT v_erasure, h.profile::uuid
      FROM jsonb_array_elements_text(v_holders) AS h(profile);

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
pending members (_erasure_member_flag). It verifies what the payload claims about the map and
raises on a mismatch or a field that does not belong: a legacy payload carries no manifest,
request, counts, checksum or holders; only an estate carries counts, and an estate carries counts
and a checksum, its counts naming only member tables, including every table its map holds (an
absent one reads as 0), and each equal to the map's entries of that table; a checksum present on
any declaration equals erasure_map_checksum of the rows just inserted. Payload-driven; replay calls
it at the event's position.$c$;

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
-- item is missing or already erased raises: the batch locked exactly those open rows. A refused
-- entry that names no erasure raises too. Erased entries are not projected here; each is marked by
-- its own authority event.
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
        IF cardinality(v_erasures) = 0 THEN
            RAISE EXCEPTION 'erasure_batch_applied event %: the refusal of % % names no erasure',
                            p_event, v_table, v_entry->>'member'
                USING ERRCODE = 'check_violation';
        END IF;

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
Raises when a named row is missing or erased, or when a refused entry names no erasure. Erased
entries are not projected here: the batch is not an authority, and each erased item is marked by
its own authority event.$c$;

-- ---------------------------------------------------------------------------
-- Section 7. resource_erased's membership: the legacy erasure.
-- ---------------------------------------------------------------------------
-- A resource_erased names no erasure, so each is recorded as a legacy one (spec D9, R21). The
-- branch is decided by the payload alone, never by the ledger: every first-record writer since
-- 20260929040730_resource_erasure_act.sql builds `targets`, and the completion pass
-- (resource_erasure_execute, 20261015100020) builds only subject_table, subject_id, actor and
-- redacted_fields. The Rust contract does not guarantee the key (ResourceErased.targets is
-- skipped when empty), so a payload of neither shape raises rather than being read as a
-- completion.
--
--   * A FIRST RECORD creates the legacy resource erasure whose id and declared_event_id are this
--     event's, with one member, erased by this event. Both inserts are ON CONFLICT DO NOTHING, so
--     a re-projection (_erasure_legacy_backfill) is a no-op.
--   * A COMPLETION RECORD marks erased every open row of its subject in legacy erasures, and takes
--     it off the drain. It never touches a non-legacy erasure, and it has no id bound:
--     uuid_generate_v7() has no order within a millisecond (20261116000010), so an id comparison
--     is not append order.
CREATE FUNCTION _project_resource_erased_membership(p_event uuid, p_payload jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_subject uuid := (p_payload->>'subject_id')::uuid;
BEGIN
    IF p_payload ? 'targets' THEN
        INSERT INTO kb_erasures (id, kind, subject_table, subject_id, legacy, declared_event_id)
        VALUES (p_event, 'resource', 'kb_resources', v_subject, true, p_event)
        ON CONFLICT DO NOTHING;
        INSERT INTO kb_erasure_members (erasure_id, member_table, member_id, status,
                                        erased_by_event_id)
        VALUES (p_event, 'kb_resources', v_subject, 'erased', p_event)
        ON CONFLICT DO NOTHING;
    ELSIF NOT EXISTS (SELECT 1 FROM jsonb_object_keys(p_payload) AS k(key)
                       WHERE k.key NOT IN ('subject_table', 'subject_id', 'actor',
                                           'redacted_fields')) THEN
        UPDATE kb_erasure_members m
           SET status             = 'erased',
               erased_by_event_id = p_event,
               next_attempt_at    = NULL,
               lease_expires_at   = NULL
          FROM kb_erasures e
         WHERE e.id = m.erasure_id
           AND e.legacy
           AND m.member_table = 'kb_resources'
           AND m.member_id    = v_subject
           AND m.status      <> 'erased';
    ELSE
        RAISE EXCEPTION '_project_resource_erased_membership: resource_erased % is neither a first record nor a completion record', p_event
            USING ERRCODE = 'check_violation';
    END IF;
END;
$$;

COMMENT ON FUNCTION _project_resource_erased_membership(uuid, jsonb) IS
$c$Projects a resource_erased payload onto the erasure record as a legacy erasure (spec D9). A first
record (its payload carries targets) creates the legacy resource erasure whose id and
declared_event_id are this event's, with one member erased by this event; both inserts are ON
CONFLICT DO NOTHING. A completion record (its keys within subject_table, subject_id, actor and
redacted_fields) marks erased every open row of its subject in legacy erasures and clears their
drain columns. Any other shape raises. Payload-driven; called second by
_project_resource_erased_redactions, and by _erasure_legacy_backfill.$c$;

-- Signature unchanged: resource_erasure_execute and the replay arm call it as before. The field
-- redactions run first, so a payload they refuse is refused with the same message as before.
CREATE OR REPLACE FUNCTION _project_resource_erased_redactions(p_event uuid, p_payload jsonb)
RETURNS void
LANGUAGE sql
SET search_path = public, pg_temp
AS $$
    SELECT _project_field_redactions(p_event, p_payload, 'erasure');
    SELECT _project_resource_erased_membership(p_event, p_payload);
$$;

COMMENT ON FUNCTION _project_resource_erased_redactions(uuid, jsonb) IS
$c$The resource_erased projection: its redacted_fields as kb_event_field_redactions rows, authority
erasure (_project_field_redactions), then its legacy erasure record
(_project_resource_erased_membership). Called by resource_erasure_execute after each append and by
the replay arm at the event's position.$c$;

-- ---------------------------------------------------------------------------
-- Section 8. The legacy backfill.
-- ---------------------------------------------------------------------------
-- Whether an erased resource still owes a completion pass is derived from the ledger as it stands,
-- which no projector may do (spec R8). So this function derives it once and records what it
-- derives in an event (spec R21): a legacy erasure_declared, under the canonical system actor,
-- whose one-row map makes the resource's member pending.
--
--   1. Every resource_erased that is not completion-shaped is re-projected through
--      _project_resource_erased_membership, in replay's walk order (replay.rs, `replay`). A first
--      record projects; a record of neither shape raises there, so the migration fails on it
--      rather than a later replay. A completion record is never re-projected: a first record's
--      row is born erased and a completion record touches only open legacy rows, so the only open
--      legacy row is one this function declared, and re-projecting an earlier completion over it
--      on a later call would wrongly close it.
--   2. Each erased resource, in id order (the person act's lock order), is locked as the acts lock
--      it: the act queue key, then the row FOR UPDATE.
--   3. One with a completion pass owed and no open legacy row gets one erasure_declared, projected
--      at once as the live path projects it.
--   4. An erased resource with no resource_erased record raises, and so does any raise inside the
--      derivation. Inside the migration, either aborts the deploy.
--
-- IT TAKES NO TABLE LOCK. The only locks it takes are step 2's, the ones the erasure acts take, so
-- it waits only for an erasure act on a husk, and every other write runs beside it. The acts and
-- this function take those locks one resource at a time in id order, so they cannot deadlock.
--
-- THE RACE IT LEAVES, AND ITS REMEDY. A first erasure that committed under the replaced wrapper
-- after step 1 read the ledger is recorded by nothing: its resource is erased and has no `erased`
-- member. Every erasure that begins after this migration commits runs the re-issued wrapper and
-- records its own. The check that finds such a straggler (P14 in the slice plan):
--
--   SELECT r.id FROM kb_resources r
--    WHERE r.erased_at IS NOT NULL
--      AND NOT EXISTS (SELECT 1 FROM kb_erasure_members m
--                       WHERE m.member_table = 'kb_resources' AND m.member_id = r.id
--                         AND m.status = 'erased');
--
-- and the remedy is to run `SELECT _erasure_legacy_backfill()` again, at any time: a first record
-- is re-projected ON CONFLICT DO NOTHING, an owed pass is declared once, and an erasure the drain
-- or a completion pass has since closed is left as it is.
-- It is idempotent: a second call appends nothing.
CREATE FUNCTION _erasure_legacy_backfill()
RETURNS integer LANGUAGE plpgsql AS $$
DECLARE
    v_record   record;
    v_resource uuid;
    v_emitter  uuid;
    v_erasure  uuid;
    v_payload  jsonb;
    v_ev       uuid;
    v_appended integer := 0;
BEGIN
    FOR v_record IN
        SELECT e.id, e.payload
          FROM kb_events e
          JOIN kb_event_types t ON t.id = e.event_type_id
         WHERE t.name = 'resource_erased'
           AND EXISTS (SELECT 1 FROM jsonb_object_keys(e.payload) AS k(key)
                        WHERE k.key NOT IN ('subject_table', 'subject_id', 'actor',
                                            'redacted_fields'))
         ORDER BY (e.xact_id IS NULL) DESC, e.xact_id, e.seq, e.id
    LOOP
        PERFORM _project_resource_erased_membership(v_record.id, v_record.payload);
    END LOOP;

    FOR v_resource IN
        SELECT r.id FROM kb_resources r WHERE r.erased_at IS NOT NULL ORDER BY r.id
    LOOP
        PERFORM pg_advisory_xact_lock(_resource_act_queue_key(v_resource));
        PERFORM 1 FROM kb_resources r WHERE r.id = v_resource FOR UPDATE;

        IF NOT EXISTS (SELECT 1
                         FROM kb_events e
                         JOIN kb_event_types t ON t.id = e.event_type_id
                        WHERE t.name = 'resource_erased'
                          AND e.payload->>'subject_id' = v_resource::text) THEN
            RAISE EXCEPTION '_erasure_legacy_backfill: erased resource % has no resource_erased record', v_resource
                USING ERRCODE = 'no_data_found';
        END IF;

        CONTINUE WHEN resource_erasure_completion_fields(v_resource) = '[]'::jsonb;
        CONTINUE WHEN EXISTS (SELECT 1
                                FROM kb_erasure_members m
                                JOIN kb_erasures e ON e.id = m.erasure_id
                               WHERE e.legacy
                                 AND m.member_table = 'kb_resources'
                                 AND m.member_id    = v_resource
                                 AND m.status      <> 'erased');

        IF v_emitter IS NULL THEN
            v_emitter := (SELECT e.id FROM kb_entities e
                            JOIN kb_profiles p ON p.id = e.profile_id
                           WHERE p.handle = 'system' AND e.name = 'system');
            IF v_emitter IS NULL THEN
                RAISE EXCEPTION '_erasure_legacy_backfill: the canonical system actor is not seeded'
                    USING ERRCODE = 'no_data_found';
            END IF;
        END IF;

        v_erasure := uuid_generate_v7();
        v_payload := jsonb_build_object(
                'erasure_id',    v_erasure,
                'kind',          'resource',
                'subject_table', 'kb_resources',
                'subject_id',    v_resource,
                'legacy',        true,
                'map',           jsonb_build_array(jsonb_build_object(
                                     'table', 'kb_resources',
                                     'id',    v_resource)));
        v_ev := _event_append(
            'erasure_declared', v_emitter, NULL, NULL, v_payload,
            p_references => jsonb_build_array(
                jsonb_build_object('rel', 'subject',
                    'target', jsonb_build_object('kind', 'kb_resources', 'id', v_resource))),
            p_correlation => v_erasure);
        PERFORM _project_erasure_declared(v_ev, v_payload);
        v_appended := v_appended + 1;
    END LOOP;

    RETURN v_appended;
END;
$$;

COMMENT ON FUNCTION _erasure_legacy_backfill() IS
$c$Records every old-door resource erasure as a legacy erasure (spec D9, R21), and returns the number
of erasure_declared events it appended. It re-projects each resource_erased that is not
completion-shaped through _project_resource_erased_membership in replay's walk order (a first
record projects, a record of neither shape raises; never a completion record), then, for each
erased resource in id order under the acts' locks (the act queue key, then FOR UPDATE), appends one
legacy erasure_declared under the canonical system actor when resource_erasure_completion_fields is
non-empty and the resource has no open legacy row, and projects it. An erased resource with no
resource_erased record raises. It takes no table lock. Idempotent and safe to run at any time: run
again, it records a first erasure that committed under the replaced wrapper after the migration's
read (an erased resource with no erased member). Called once by
20261201000030_erasure_record_projectors.sql.$c$;

SELECT _erasure_legacy_backfill();

SELECT declare_migration(
    20261201000030,
    'additive',
    'Adds the projectors _project_erasure_declared, _project_erasure_member_arrived, '
    '_project_erasure_scope_sealed, _project_erasure_reopened, _project_erasure_completed and '
    '_project_erasure_batch_applied, the helpers _erasure_member_flag and _edge_fold_apply, and '
    're-issues _project_relationship_folded over _edge_fold_apply with the same signature, the '
    'same statements and the same raise. Adds _project_resource_erased_membership, which records '
    'each resource_erased as a legacy erasure, and re-issues _project_resource_erased_redactions '
    'with the same signature to call it after the field redactions, whose checks and messages are '
    'unchanged. Adds _erasure_legacy_backfill and runs it once: it projects every existing '
    'resource_erased first record, taking no table lock, and appends one legacy '
    'erasure_declared, under the canonical system actor, for each erased resource that still '
    'owes a completion pass. It raises, aborting '
    'the migration, on an erased resource with no resource_erased record or on a raise inside '
    'resource_erasure_completion_fields. A binary without this change writes resource_erased '
    'through the same SQL act, which now also records its legacy erasure.'
);
