-- ═══════════════════════════════════════════════════════════════════════════════════════════════
-- Facets are stored at the grain they are consumed at: one row per INNER KEY.
--
-- Task 019f6d08-2b55-7ee0-b9ac-1959cf4d736b. Decision recorded 2026-07-29 (identity) and
-- 2026-07-30 (gating, ack, repair, weight rule).
--
-- ── The defect ──────────────────────────────────────────────────────────────────────────────────
-- `facet_set` appends one row per ASSERT, while `expand_facets`
-- (crates/temper-substrate/src/substrate.rs:242-271) reads one mark per INNER KEY. The two grains
-- have never matched, so re-asserting a facet accumulated a second live row and the read could not
-- tell "this facet was updated" from "this facet is genuinely multi-valued". Keying identity on the
-- inner key makes storage and consumption the same thing — and dissolves the "overwrite the whole
-- nested object vs patch the diff" question, because a multi-key assert is just N independent
-- upserts. Nothing is deleted by omission: an assert never touches a mark it did not name.
--
-- ── Why this CONFORMs rather than invents ───────────────────────────────────────────────────────
-- Fold-then-insert is already the house pattern: `_project_property_set`
-- (20260711000060_index_open_meta_tags_into_fts.sql:90) folds prior active rows for
-- (owner, property_key) then inserts. The only genuinely new thing here is extending that fold key
-- to the inner key, for `property_key = 'facet'` alone.
--
-- ── The gate, and why it is load-bearing ────────────────────────────────────────────────────────
-- `facet_set` is NOT facet-only. It is the key-agnostic assert every resource create fires for each
-- of its managed + open frontmatter properties (`writes.rs:108` → `SeedAction::PropertyAssert` →
-- `events.rs:782`, "Reuses the same key-agnostic `facet_set` query — no new SQL function"). Those
-- values are bare scalars ("task", "backlog"), which have no inner key. So the split is gated on
-- `property_key = 'facet'` and every other key keeps today's behaviour VERBATIM.
--
-- ── Strict at the door, tolerant in replay ──────────────────────────────────────────────────────
-- 41 live facet rows are double-encoded: a JSON *string* whose content is a serialized object
-- ("{\"node_label\": \"domain\"}"). All 41 parse back to objects carrying the same
-- node_label/status/priority vocabulary as the other 402 — they are well-formed facets a writer
-- JSON-encoded once too often, not junk. Every one is dated 2026-07-03 or 2026-07-06 while
-- object-shaped writes run continuously to today, so the writer that produced them is long gone.
--
-- They are REPAIRED, not folded: `_facet_marks` decodes them. The repair lives in the PROJECTOR, so
-- a replay of those historical events reproduces the decoded value forever — a one-off UPDATE would
-- be silently undone by the next replay (`kb_properties` is compared in full, folded rows included,
-- by replay.rs's PROJECTION_DUMPS). The `facet_set` WRAPPER rejects a non-object going forward, so
-- the closed bug cannot reopen. Guard at the door, tolerance in the projector: replay calls the
-- projector directly and must never fail on history it cannot change.
-- ═══════════════════════════════════════════════════════════════════════════════════════════════

-- ── Beat 1: one definition of "the marks in a facet value" ───────────────────────────────────────
-- Both projectors and the backfill below read the grain through this function rather than each
-- spelling out its own decode-then-explode. Three copies of this rule would drift, and the decode
-- arm is exactly the kind of quiet detail that gets omitted from the second copy.
--
-- A value that is neither an object nor a string encoding one yields ONE mark under the sentinel
-- key: today's whole-row behaviour, preserved. Nothing in production takes that arm (all 41
-- non-objects decode), but a projector that RAISEd there would make some historical event
-- unreplayable, and an event log you cannot replay is worse than an inert row.
CREATE FUNCTION _facet_marks(p_value jsonb)
RETURNS TABLE (inner_key text, inner_value jsonb)
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE v_decoded jsonb;
BEGIN
    v_decoded := p_value;

    -- Double-encoded object → decode. `jsonb_typeof(...) = 'object'` is checked on the RESULT, so a
    -- string holding a scalar or an array falls through to the sentinel arm untouched.
    IF jsonb_typeof(p_value) = 'string' THEN
        BEGIN
            IF jsonb_typeof((p_value #>> '{}')::jsonb) = 'object' THEN
                v_decoded := (p_value #>> '{}')::jsonb;
            END IF;
        EXCEPTION WHEN others THEN
            -- not JSON at all; keep the original and take the sentinel arm
            v_decoded := p_value;
        END;
    END IF;

    IF jsonb_typeof(v_decoded) = 'object' THEN
        RETURN QUERY SELECT kv.key, kv.value FROM jsonb_each(v_decoded) kv;
    ELSE
        RETURN QUERY SELECT NULL::text, v_decoded;
    END IF;
END;
$$;

COMMENT ON FUNCTION _facet_marks(jsonb) IS
'The marks carried by one facet value: one row per inner key, decoding a double-encoded object '
'(a JSON string holding a serialized object) on the way. A value that is neither an object nor a '
'string encoding one yields a single mark with a NULL inner_key — the pre-grain whole-row shape, '
'kept so that no historical event becomes unreplayable.';

-- ── Beat 2: the facet projectors split by inner key ──────────────────────────────────────────────
-- Both return uuid[] now: one assert can produce N rows, so a scalar return could only name one of
-- them arbitrarily — the same reads-as-complete defect the facet read shipped to kill. Non-facet
-- keys still produce exactly one element, and still use the payload's `property_id` as the row id
-- (identity-as-input, unchanged). Facet marks mint fresh ids: `kb_properties.id` carries no inbound
-- references and replay masks it (replay.rs PROJECTION_DUMPS), so a surrogate is free here, and
-- there is only one payload id to go round N rows.
DROP FUNCTION _project_property_asserted(uuid, jsonb);
CREATE FUNCTION _project_property_asserted(p_event uuid, p_payload jsonb)
RETURNS uuid[] LANGUAGE plpgsql AS $$
DECLARE v_prop uuid := (p_payload->>'property_id')::uuid;
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_owner_tbl text := p_payload#>>'{owner,table}';
        v_owner uuid := (p_payload#>>'{owner,id}')::uuid;
        v_key text := p_payload->>'property_key';
        v_weight double precision := (p_payload->>'weight')::double precision;
        v_ids uuid[] := '{}';
        v_mark record;
        v_id uuid;
BEGIN
    IF v_key <> 'facet' THEN
        -- Every non-facet key — including every frontmatter property a resource create asserts —
        -- takes the pre-existing append path, byte for byte.
        INSERT INTO kb_properties (id, owner_table, owner_id, property_key, property_value, weight,
                                   asserted_by_event_id, last_event_id, created)
        VALUES (v_prop, v_owner_tbl, v_owner, v_key, p_payload->'value', v_weight,
                p_event, p_event, v_occurred);
        RETURN ARRAY[v_prop];
    END IF;

    FOR v_mark IN SELECT * FROM _facet_marks(p_payload->'value') LOOP
        -- A mark is stored as a ONE-KEY OBJECT — {"status": "open"} — never an envelope around the
        -- key and value. That is not cosmetic: `expand_facets` explodes `property_value`'s top-level
        -- keys straight into `Facet { path, value }` (substrate.rs:242-271), so any wrapper shape
        -- would surface its own field names as facet paths and silently steer clustering by them.
        -- The one-key object is also precisely what that function's existing single-key arm already
        -- consumes, so the consumer needs no change at all.
        --
        -- Fold the prior live mark for THIS inner key only — never a sibling. `jsonb_exists` is the
        -- function spelling of `?`, used because a bare `?` in a migration is needlessly ambiguous
        -- to anything that scans SQL for placeholders.
        IF v_mark.inner_key IS NULL THEN
            -- Sentinel arm: a value that is not an object and does not decode to one. It has no
            -- inner key, so its identity is "the non-object mark on this owner".
            UPDATE kb_properties
               SET is_folded = true, last_event_id = p_event
             WHERE owner_table = v_owner_tbl AND owner_id = v_owner
               AND property_key = 'facet' AND NOT is_folded
               AND jsonb_typeof(property_value) <> 'object';
        ELSE
            UPDATE kb_properties
               SET is_folded = true, last_event_id = p_event
             WHERE owner_table = v_owner_tbl AND owner_id = v_owner
               AND property_key = 'facet' AND NOT is_folded
               AND jsonb_typeof(property_value) = 'object'
               AND jsonb_exists(property_value, v_mark.inner_key);
        END IF;

        v_id := uuid_generate_v7();
        INSERT INTO kb_properties (id, owner_table, owner_id, property_key, property_value, weight,
                                   asserted_by_event_id, last_event_id, created)
        VALUES (v_id, v_owner_tbl, v_owner, 'facet',
                CASE WHEN v_mark.inner_key IS NULL
                     THEN v_mark.inner_value
                     ELSE jsonb_build_object(v_mark.inner_key, v_mark.inner_value) END,
                v_weight, p_event, p_event, v_occurred);
        v_ids := v_ids || v_id;
    END LOOP;

    RETURN v_ids;
END;
$$;

COMMENT ON FUNCTION _project_property_asserted(uuid, jsonb) IS
'Project a property_asserted event. property_key=''facet'' splits the value into one row per inner '
'key, folding the prior live row for each key it names and leaving unnamed marks untouched '
'(patch, not replace). Every other key keeps the pre-grain append behaviour verbatim, including the '
'frontmatter properties a resource create asserts. Returns the ids written — plural, because one '
'assert can produce many rows.';

-- ── Beat 3: the same grain for property_set, which also writes this key ──────────────────────────
-- `facet` is not in MANAGED_PROPERTY_KEYS (keys.rs:42-53), so `readback::meta` surfaces it inside
-- `open_meta`, and `resource update --open-meta` writes open keys back through `property_set`
-- (db_backend.rs:1898). 29 production facet rows arrived that way. If property_set kept writing one
-- whole-object row it would re-introduce the very grain this migration removes, through a door
-- nobody thinks of as a facet writer.
--
-- property_set means "this key holds one current value", so it still folds the owner's ENTIRE live
-- facet set first — that is the difference from facet_set, and it is the correct meaning of a
-- single-valued set. It then writes the new value at the new grain.
DROP FUNCTION _project_property_set(uuid, jsonb);
CREATE FUNCTION _project_property_set(p_event uuid, p_payload jsonb)
RETURNS uuid[] LANGUAGE plpgsql AS $$
DECLARE v_prop uuid := (p_payload->>'property_id')::uuid;
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_owner_tbl text := p_payload#>>'{owner,table}';
        v_owner uuid := (p_payload#>>'{owner,id}')::uuid;
        v_key text := p_payload->>'property_key';
        v_weight double precision := (p_payload->>'weight')::double precision;
        v_ids uuid[] := '{}';
        v_mark record;
        v_id uuid;
BEGIN
    -- Replace semantics, unchanged for every key: fold the whole live set for this key first.
    UPDATE kb_properties SET is_folded = true, last_event_id = p_event
        WHERE owner_table = v_owner_tbl AND owner_id = v_owner
          AND property_key = v_key AND NOT is_folded;

    IF v_key = 'facet' THEN
        FOR v_mark IN SELECT * FROM _facet_marks(p_payload->'value') LOOP
            v_id := uuid_generate_v7();
            INSERT INTO kb_properties (id, owner_table, owner_id, property_key, property_value,
                                       weight, asserted_by_event_id, last_event_id, created)
            VALUES (v_id, v_owner_tbl, v_owner, 'facet',
                    CASE WHEN v_mark.inner_key IS NULL
                         THEN v_mark.inner_value
                         ELSE jsonb_build_object(v_mark.inner_key, v_mark.inner_value) END,
                    v_weight, p_event, p_event, v_occurred);
            v_ids := v_ids || v_id;
        END LOOP;
    ELSE
        INSERT INTO kb_properties (id, owner_table, owner_id, property_key, property_value, weight,
                                   asserted_by_event_id, last_event_id, created)
        VALUES (v_prop, v_owner_tbl, v_owner, v_key, p_payload->'value', v_weight,
                p_event, p_event, v_occurred);
        v_ids := ARRAY[v_prop];
    END IF;

    -- Carried verbatim from 20260711000060: the FTS vector is gated on the indexed open_meta keys.
    IF v_owner_tbl = 'kb_resources' AND v_key IN ('keywords', 'descriptor', 'tags') THEN
        PERFORM _rebuild_resource_search_vector(v_owner);
    END IF;
    RETURN v_ids;
END;
$$;

COMMENT ON FUNCTION _project_property_set(uuid, jsonb) IS
'Project a property_set event: fold the key''s entire live set, then insert. property_key=''facet'' '
'inserts at the inner-key grain (one row per mark) rather than one whole-object row, so the '
'open_meta round-trip cannot re-introduce the pre-grain shape. Returns the ids written.';

-- ── Beat 4: the wrappers return what they wrote, and the door refuses a malformed facet ──────────
-- The guard lives HERE and not in the projector, and the split matters: replay calls the projector
-- directly, so a projector that refused a shape would make the history that already contains it
-- unreplayable. The door refuses new bad shapes; the projector forgives old ones.
DROP FUNCTION facet_set(jsonb, uuid, jsonb, uuid, uuid);
CREATE FUNCTION facet_set(p_payload jsonb, p_emitter uuid,
                          p_metadata jsonb DEFAULT '{}'::jsonb,
                          p_invocation uuid DEFAULT NULL, p_correlation uuid DEFAULT NULL)
RETURNS uuid[] LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_anchor_tbl text; v_anchor uuid;
        v_owner_tbl text := COALESCE(p_payload#>>'{owner,table}', 'kb_resources');
        v_owner uuid := (p_payload#>>'{owner,id}')::uuid;
BEGIN
    IF p_payload->>'property_key' = 'facet'
       AND jsonb_typeof(p_payload->'value') <> 'object' THEN
        RAISE EXCEPTION 'facet_set: a facet value must be a JSON object, got %',
            jsonb_typeof(p_payload->'value')
            USING HINT = 'send {"status": "open"}, not a string containing it — 41 rows were '
                         'double-encoded that way before this guard existed';
    END IF;

    SELECT a.anchor_table, a.anchor_id INTO v_anchor_tbl, v_anchor
      FROM _property_owner_anchor(v_owner_tbl, v_owner) a;
    v_ev := _event_append('property_asserted', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_property_asserted(v_ev, p_payload);
END;
$$;

DROP FUNCTION property_set(jsonb, uuid, jsonb, uuid, uuid);
CREATE FUNCTION property_set(p_payload jsonb, p_emitter uuid,
                             p_metadata jsonb DEFAULT '{}'::jsonb,
                             p_invocation uuid DEFAULT NULL, p_correlation uuid DEFAULT NULL)
RETURNS uuid[] LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_anchor_tbl text; v_anchor uuid;
        v_owner_tbl text := COALESCE(p_payload#>>'{owner,table}', 'kb_resources');
        v_owner uuid := (p_payload#>>'{owner,id}')::uuid;
BEGIN
    IF p_payload->>'property_key' = 'facet'
       AND jsonb_typeof(p_payload->'value') <> 'object' THEN
        RAISE EXCEPTION 'property_set: a facet value must be a JSON object, got %',
            jsonb_typeof(p_payload->'value');
    END IF;

    SELECT a.anchor_table, a.anchor_id INTO v_anchor_tbl, v_anchor
      FROM _property_owner_anchor(v_owner_tbl, v_owner) a;
    v_ev := _event_append('property_set', p_emitter, v_anchor_tbl, v_anchor, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_property_set(v_ev, p_payload);
END;
$$;

-- ── Beat 5: re-project the facet history at the new grain ────────────────────────────────────────
-- NOT a hand-written UPDATE of the projection. `kb_properties` is compared IN FULL by replay
-- (replay.rs PROJECTION_DUMPS — folded rows included, id masked, ordered by natural key), so any
-- state this migration invents that a replay would not reproduce is a latent replay failure. Instead
-- the facet rows are dropped and rebuilt by running the real projectors over the real events, in
-- occurred_at order — equivalent to a replay of that slice BY CONSTRUCTION, not by argument.
--
-- Dropping projection rows is safe precisely because they are a projection: `kb_properties.id`
-- carries no inbound references, the events they derive from are untouched (kb_events is
-- append-only and nothing here writes to it), and every reader filters `NOT is_folded`.
--
-- Measured on production 2026-07-30 before writing this: 443 live facet rows from 443 events
-- (414 property_asserted + 29 property_set), zero folded, zero edge-owned. Those 443 values carry
-- 1053 marks over 1011 distinct (owner, inner key) — so this is expected to leave 1011 live rows
-- and 42 folded ones, the 42 being marks a later assert superseded.
-- It is a FUNCTION rather than an inline `DO` block for two reasons, and the first is the one that
-- matters: an inline block runs exactly once, on a database whose facet rows nobody can see from a
-- test, so the riskiest statement in this migration would ship with no witness at all. As a function
-- it is exercised against real old-grain rows by
-- `set_facet_test::the_backfill_rebuilds_old_grain_rows_from_their_events`.
--
-- Second, it survives as an operator repair primitive. It is idempotent and self-correcting by
-- construction — it derives everything from `kb_events`, which nothing here writes — so re-running
-- it can only ever restore the projection to what the ledger says it should be. Scoped to one owner
-- for targeted repair; NULL means every owner.
CREATE FUNCTION _facet_regrain_from_events(p_owner uuid DEFAULT NULL)
RETURNS TABLE (rows_before bigint, rows_live bigint, rows_folded bigint)
LANGUAGE plpgsql AS $$
DECLARE v_ev record;
BEGIN
    SELECT count(*) INTO rows_before
      FROM kb_properties
     WHERE property_key = 'facet' AND (p_owner IS NULL OR owner_id = p_owner);

    DELETE FROM kb_properties
     WHERE property_key = 'facet' AND (p_owner IS NULL OR owner_id = p_owner);

    FOR v_ev IN
        SELECT e.id, e.payload, t.name AS event_type
          FROM kb_events e
          JOIN kb_event_types t ON t.id = e.event_type_id
         WHERE e.payload->>'property_key' = 'facet'
           AND t.name IN ('property_asserted', 'property_set')
           AND (p_owner IS NULL OR (e.payload#>>'{owner,id}')::uuid = p_owner)
         ORDER BY e.occurred_at, e.id
    LOOP
        -- Dispatch per event type: the two projectors mean different things (assert patches one
        -- key; set replaces the whole facet set), and collapsing them would rewrite history.
        IF v_ev.event_type = 'property_asserted' THEN
            PERFORM _project_property_asserted(v_ev.id, v_ev.payload);
        ELSE
            PERFORM _project_property_set(v_ev.id, v_ev.payload);
        END IF;
    END LOOP;

    SELECT count(*) FILTER (WHERE NOT is_folded), count(*) FILTER (WHERE is_folded)
      INTO rows_live, rows_folded
      FROM kb_properties
     WHERE property_key = 'facet' AND (p_owner IS NULL OR owner_id = p_owner);

    -- A rebuild that produced nothing from a non-empty starting set means the event dispatch above
    -- matched no events — fail loudly rather than leave every facet silently deleted.
    IF rows_before > 0 AND rows_live = 0 THEN
        RAISE EXCEPTION 'facet regrain deleted % rows and rebuilt none', rows_before;
    END IF;

    RETURN NEXT;
END;
$$;

COMMENT ON FUNCTION _facet_regrain_from_events(uuid) IS
'Rebuild facet property rows from their events at the current grain, for one owner or (NULL) all. '
'Idempotent and derived entirely from kb_events, so re-running only ever restores the projection to '
'what the ledger says. The repair primitive behind migration 20260730000010''s backfill.';

DO $$
DECLARE v record;
BEGIN
    SELECT * INTO v FROM _facet_regrain_from_events(NULL);
    RAISE NOTICE 'facet regrain: % rows before → % live + % folded after',
        v.rows_before, v.rows_live, v.rows_folded;
END;
$$;
