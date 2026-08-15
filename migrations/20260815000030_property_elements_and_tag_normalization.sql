-- The owner-agnostic element relation, and the write-time `tags` normalization (design §6.2/§6.3
-- and §7, ruled 2026-08-15). Reasoning, measurements and residue: task
-- 01a00502-a774-7001-b5b2-0ce462158f1c and
-- docs/superpowers/specs/2026-08-14-property-conventions-and-predicate-container-design.md.
--
-- Traps a later edit breaks silently:
--   1. The view has NO `tags` branch — one rule for all 70 live keys, which is the point.
--   2. Normalization is scoped to `tags`. Widening it rewrites `date`, `descriptor` and 61 others.
--   3. It does NOT forgive every shape. Normalizing onto an already-live equal value is a
--      `uq_kb_properties_active` duplicate and RAISES out of the projector, which replay calls
--      directly. Unreachable today: nothing asserts `tags` (`create_resource` fires `PropertySet`,
--      which folds), and prod's 467 `tags` events are all arrays. Witness:
--      `property_elements.rs::normalizing_can_collide_with_an_existing_live_row_and_that_raises`.
--   4. The `facet` predicate WIDENS — see its comment.
--
-- Additive: one CREATE VIEW, one CREATE FUNCTION, three CREATE OR REPLACE at byte-identical
-- signatures. No DROP.

-- ── The shape convention, owner-agnostic (§6.2) ─────────────────────────────────────────────────
-- Explode arrays, pass everything else whole. An empty array yields NO rows, so this cannot
-- witness a key's mere PRESENCE — a `has_key` predicate must read `kb_properties` directly.
CREATE VIEW kb_property_elements AS
    SELECT p.owner_table, p.owner_id, p.property_key, elem.value AS element, p.weight
      FROM kb_properties p
      CROSS JOIN LATERAL jsonb_array_elements(
        CASE jsonb_typeof(p.property_value)
          WHEN 'array' THEN p.property_value
          ELSE jsonb_build_array(p.property_value)
        END) AS elem(value)
     WHERE NOT p.is_folded;

COMMENT ON VIEW kb_property_elements IS
    'Live kb_properties at ELEMENT grain, for every owner table: an array-valued row becomes one row per element, any other shape becomes one row holding itself. A predicate reads this rather than the table, so it can neither lose a shape convention nor wrongly inherit one. An empty array yields NO rows, so a key-existence predicate must read kb_properties directly.';

-- ── The write-time rule, in one place because both projectors need it (§7) ─────────────────────
-- A bare-string `tags` value is ONE tag. Owner-agnostic: a shape convention applies to every owner.
CREATE FUNCTION _property_value_normalized(p_key text, p_value jsonb)
RETURNS jsonb LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE WHEN p_key = 'tags' AND jsonb_typeof(p_value) = 'string'
                THEN jsonb_build_array(p_value)
                ELSE p_value END;
$$;

COMMENT ON FUNCTION _property_value_normalized(text, jsonb) IS
    'The stored shape of a property value: a bare-string tags value becomes a one-element array, every other key and shape is returned verbatim. One definition because both projectors write kb_properties.';

-- ── Both projectors, body-only ──────────────────────────────────────────────────────────────────

CREATE OR REPLACE FUNCTION _project_property_asserted(p_event uuid, p_payload jsonb)
RETURNS uuid[] LANGUAGE plpgsql AS $$
DECLARE v_prop uuid := (p_payload->>'property_id')::uuid;
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_owner_tbl text := p_payload#>>'{owner,table}';
        v_owner uuid := (p_payload#>>'{owner,id}')::uuid;
        v_key text := p_payload->>'property_key';
        v_value jsonb := _property_value_normalized(p_payload->>'property_key',
                                                    p_payload->'value');
        v_weight double precision := (p_payload->>'weight')::double precision;
        v_ids uuid[] := '{}';
        v_mark record;
        v_id uuid;
BEGIN
    IF v_key <> 'facet' THEN
        INSERT INTO kb_properties (id, owner_table, owner_id, property_key, property_value, weight,
                                   asserted_by_event_id, last_event_id, created)
        VALUES (v_prop, v_owner_tbl, v_owner, v_key, v_value, v_weight,
                p_event, p_event, v_occurred);
        RETURN ARRAY[v_prop];
    END IF;

    FOR v_mark IN SELECT * FROM _facet_marks(v_value) LOOP
        -- A mark is stored as a ONE-KEY OBJECT — {"status": "open"} — never an envelope around the
        -- key and value: `expand_facets` explodes `property_value`'s top-level keys straight into
        -- `Facet { path, value }`, so a wrapper would surface its own field names as facet paths.
        --
        -- Fold the prior live mark for THIS inner key only — never a sibling.
        IF v_mark.inner_key IS NULL THEN
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

CREATE OR REPLACE FUNCTION _project_property_set(p_event uuid, p_payload jsonb)
RETURNS uuid[] LANGUAGE plpgsql AS $$
DECLARE v_prop uuid := (p_payload->>'property_id')::uuid;
        v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_owner_tbl text := p_payload#>>'{owner,table}';
        v_owner uuid := (p_payload#>>'{owner,id}')::uuid;
        v_key text := p_payload->>'property_key';
        v_value jsonb := _property_value_normalized(p_payload->>'property_key',
                                                    p_payload->'value');
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
        FOR v_mark IN SELECT * FROM _facet_marks(v_value) LOOP
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
        VALUES (v_prop, v_owner_tbl, v_owner, v_key, v_value, v_weight,
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

-- ── The selection act's two property predicates now read the view ───────────────────────────────
-- `tags` loses its `regexp_split_to_array` arm. `facet` loses its own copy of
-- `owner_table = … AND NOT is_folded` AND changes meaning slightly — see its comment. Body change
-- only; the signature is byte-identical.
CREATE OR REPLACE FUNCTION __temper_ungated_find_resources_with(
    p_visible_ids    uuid[],
    p_doc_types      text[]  DEFAULT NULL,
    p_tags           text[]  DEFAULT NULL,
    p_facets         jsonb   DEFAULT NULL,
    p_stage          text    DEFAULT NULL,
    p_status         text    DEFAULT NULL,
    p_owner_profile  uuid    DEFAULT NULL,
    p_owner_handle   text    DEFAULT NULL,
    p_title_contains text    DEFAULT NULL,
    p_anchor_table   varchar DEFAULT NULL,
    p_anchor_id      uuid    DEFAULT NULL,
    p_anchor_reader  uuid    DEFAULT NULL)
RETURNS TABLE (resource_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT r.id
      FROM kb_resources r
      JOIN kb_resources_live lv                    ON lv.id = r.id
      JOIN unnest(p_visible_ids) AS v(resource_id) ON v.resource_id = r.id
      JOIN kb_resource_homes h                     ON h.resource_id = r.id
     WHERE (p_doc_types IS NULL OR EXISTS (
             SELECT 1 FROM kb_resource_doc_type dt
              WHERE dt.resource_id = r.id AND dt.doc_type = ANY(p_doc_types)))
       AND (p_stage IS NULL OR EXISTS (
             SELECT 1 FROM kb_resource_workflow_props wp
              WHERE wp.resource_id = r.id AND wp.stage = p_stage))
       AND (p_status IS NULL OR EXISTS (
             SELECT 1 FROM kb_resource_workflow_props wp
              WHERE wp.resource_id = r.id AND wp.status = p_status))
       AND (p_owner_profile IS NULL OR h.owner_profile_id = p_owner_profile)
       AND (p_owner_handle IS NULL OR EXISTS (
             SELECT 1 FROM kb_profiles p
              WHERE p.id = h.owner_profile_id AND p.handle = p_owner_handle))
       AND (p_title_contains IS NULL OR r.title ILIKE '%' || p_title_contains || '%')
       -- AND-containment, both sides folded. A resource with no `tags` is excluded because the
       -- `coalesce` makes it the EMPTY ARRAY — NOT because it is NULL, which is what the
       -- incumbent's comment said `[corrected — 2026-08-15]`. Drop the `coalesce` and every tag
       -- filter becomes NULL and matches nothing.
       AND (p_tags IS NULL OR (
             SELECT coalesce(array_agg(DISTINCT lower(pe.element #>> '{}')), '{}')
               FROM kb_property_elements pe
              WHERE pe.owner_table = 'kb_resources' AND pe.owner_id = r.id
                AND pe.property_key = 'tags'
           ) @> ARRAY(SELECT lower(x) FROM unnest(p_tags) AS x))
       -- Three guards, distinct jobs: `jsonb_typeof` fails closed, the `CASE` stops
       -- `jsonb_array_elements` raising, the null-key test stops `jsonb_build_object` raising.
       --
       -- Reading `pe.element` rather than `fp.property_value` WIDENS this: an ARRAY-shaped facet
       -- value explodes, and a one-element array wrapping an object does not contain that object
       -- while the element does (jsonb's array-containment exception is primitives only).
       -- Reachable via `_facet_marks`' sentinel arm; accepted, and all 1,281 live facet rows on
       -- prod are objects `[measured — 2026-08-15]`.
       AND (p_facets IS NULL OR (
             jsonb_typeof(p_facets) = 'array'
             AND NOT EXISTS (
               SELECT 1 FROM jsonb_array_elements(
                 CASE jsonb_typeof(p_facets) WHEN 'array' THEN p_facets ELSE '[]'::jsonb END) AS f
                WHERE NOT EXISTS (
                  SELECT 1 FROM kb_property_elements pe
                   WHERE pe.owner_table = 'kb_resources' AND pe.owner_id = r.id
                     AND pe.property_key = 'facet'
                     AND f->>'key' IS NOT NULL
                     AND pe.element @> jsonb_build_object(f->>'key', f->>'value')))))
       AND (p_anchor_id IS NULL OR (
             COALESCE(anchor_readable_by_profile(p_anchor_reader, p_anchor_table, p_anchor_id),
                      false)
             AND h.anchor_table = p_anchor_table
             AND h.anchor_id = p_anchor_id));
$$;

SELECT declare_migration(
    20260815000030,
    'additive',
    'Gives the property layer its owner-agnostic element relation (kb_property_elements: live kb_properties at element grain, arrays exploded, every other shape passed whole) and rules that a bare-string tags value is ONE tag, normalized at write by _property_value_normalized in both projectors. Repoints find_resources_with''s tags and facet predicates onto the view, deleting the last SQL copy of the tags whitespace-split; the third copy was Rust (filtered_visible_page) and goes in the same change. Additive: one CREATE VIEW, one CREATE FUNCTION, three CREATE OR REPLACE at byte-identical signatures, no DROP. Two behaviour changes, NOT no-ops: normalizing onto an already-live equal value is a uq_kb_properties_active duplicate and raises out of the projector (unreachable today — nothing asserts tags, and prod''s 467 tags events are all arrays); and the facet predicate widens for an array-shaped facet value, of which prod has none. Reasoning, measurements and residue live in task 01a00502-a774-7001-b5b2-0ce462158f1c and docs/superpowers/specs/2026-08-14-property-conventions-and-predicate-container-design.md sections 6.2, 6.3, 7 and 9 — deliberately not here, because an applied migration cannot have its prose corrected.'
);
