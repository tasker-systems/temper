-- The owner-agnostic element relation, and the write-time `tags` normalization that lets it be
-- universal.
--
-- Design: docs/superpowers/specs/2026-08-14-property-conventions-and-predicate-container-design.md
-- §6.2/§6.3 and §7 (ruled 2026-08-15). Task 01a00502-a774-7001-b5b2-0ce462158f1c, which carries the
-- reasoning, the measurements and the residue.
--
-- Three things a later edit can break silently:
--   1. The view has NO `tags` branch, and that is the point — one rule for all 70 live keys.
--   2. Normalization is scoped to `tags`. Widening it to every scalar silently rewrites `date`,
--      `descriptor` and the 61 unrecognized keys.
--   3. It lives in the PROJECTORS, so nothing bypasses it and replay converges history. That does
--      not contradict 20260730000010's *"the door refuses new bad shapes; the projector forgives
--      old ones"* — normalizing forgives every shape; it refuses none.
--
-- Additive: one CREATE VIEW, one CREATE FUNCTION, three CREATE OR REPLACE at byte-identical
-- signatures. No DROP.

-- ── The shape convention, owner-agnostic (§6.2) ─────────────────────────────────────────────────
-- Explode arrays, pass everything else through whole. An empty array contributes no rows, so this
-- relation cannot witness a key's mere presence — the case a `has_key` predicate must not read here.
--
-- `[measured on prod — 2026-08-15]` This shape's cost is MEASURED, not carried from
-- `20260808000020`'s plain-EXISTS finding: against the real corpus the exploding form and the
-- incumbent expression read an identical 26,970 blocks per call, at 34.17 ms vs 34.71 ms mean over
-- three calls each (σ 2.7 / 2.1) — a difference inside one standard deviation of either.
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
    'Live kb_properties at ELEMENT grain, for every owner table: an array-valued row becomes one row per element, any other shape becomes one row holding itself. A predicate reads this rather than the table, so it can neither lose a shape convention nor wrongly inherit one (design 6.3). Universal by construction — there is no per-key branch, which is what the retirement of the tags whitespace-split bought. An empty array yields NO rows, so this relation cannot answer whether a key is merely PRESENT; a key-existence predicate must read kb_properties directly.';

-- ── The write-time rule, in one place because two projectors need it (§7) ───────────────────────
-- A bare-string `tags` value is ONE tag: `"concept design"` stores as `["concept design"]`. The
-- whitespace split this replaces existed to agree with FTS, and FTS does not split — it delegates to
-- a tokenizer that splits differently. Owner-agnostic: a shape convention applies to every owner.
CREATE FUNCTION _property_value_normalized(p_key text, p_value jsonb)
RETURNS jsonb LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE WHEN p_key = 'tags' AND jsonb_typeof(p_value) = 'string'
                THEN jsonb_build_array(p_value)
                ELSE p_value END;
$$;

COMMENT ON FUNCTION _property_value_normalized(text, jsonb) IS
    'The stored shape of a property value. Today: a bare-string `tags` value becomes a one-element array, and every other key and shape is returned verbatim. One definition because both projectors write kb_properties and a rule honoured by one of them is not a rule. Owner-agnostic, because a shape convention applies to every owner (design 5).';

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
-- `tags` loses its `regexp_split_to_array` arm and `facet` loses its own copy of
-- `owner_table = … AND NOT is_folded`. Body change only; the signature is byte-identical.
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
       -- AND-containment, both sides folded. A resource with no `tags` aggregates to NULL and
       -- `NULL @> $` is NULL, so it is correctly excluded; `tags: []` yields the empty array and is
       -- likewise excluded for any non-empty filter.
       AND (p_tags IS NULL OR (
             SELECT coalesce(array_agg(DISTINCT lower(pe.element #>> '{}')), '{}')
               FROM kb_property_elements pe
              WHERE pe.owner_table = 'kb_resources' AND pe.owner_id = r.id
                AND pe.property_key = 'tags'
           ) @> ARRAY(SELECT lower(x) FROM unnest(p_tags) AS x))
       -- `jsonb_typeof` is what fails CLOSED; the `CASE` is what keeps jsonb_array_elements from
       -- RAISING whatever order the planner takes the conjuncts in. Neither substitutes for the
       -- other. The key guard is the third: jsonb_build_object RAISES on a null key.
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
    'Gives the property layer its owner-agnostic element relation and rules what a bare-string tags value MEANS (task 01a00502-a774-7001-b5b2-0ce462158f1c, design section 6.2/6.3 and section 7). Three objects. kb_property_elements exposes live kb_properties at element grain for every owner table — an array becomes one row per element, any other shape becomes one row holding itself — which is the view a predicate reads instead of the table, so it can neither lose a shape convention nor wrongly inherit one. It has NO per-key branch and that is the point: one rule for all 70 live property keys. _property_value_normalized carries the section 7 ruling (decided 2026-08-15, Pete): a bare-string tags value is normalized AT WRITE and is ONE tag, so tags "concept design" stores as ["concept design"]. It lives in ONE function because both projectors write kb_properties and a rule honoured by one of them is not a rule, and it is scoped to tags because widening it to every scalar would silently rewrite date, descriptor and the 61 unrecognized keys. The normalization sits in the PROJECTORS rather than at a door or in Rust so that nothing can bypass it — not a direct SQL caller, not the scenario loader, not replay — and so replay CONVERGES historical events instead of reproducing them; that does not contradict 20260730000010''s rule that the door refuses new bad shapes while the projector forgives old ones, because normalizing forgives every shape and refuses none. The whitespace split it retires existed only to agree with FTS, and FTS does not split — it delegates to a tokenizer that splits differently (to_tsvector(''english'',''ci-auth deploy'') yields ci, auth, ci-auth, deploy while regexp_split_to_array yields {ci-auth, deploy}), so the split centralized an answer that did not achieve its own goal. Both of __temper_ungated_find_resources_with''s property predicates now read the view: tags loses its regexp_split_to_array arm and facet loses its own copy of the owner/liveness predicate. Measured on prod before the change: over all 428 live tags rows the view form and the incumbent expression disagree on ZERO, because the only shape they treat differently is the bare string and zero bare-string tags exist. Its COST is measured too rather than carried from 20260808000020, whose view-versus-function finding was taken on a plain doc_type EXISTS and which an element-exploding shape inherits nothing from: against the real prod corpus the two forms read an identical 26,970 blocks per call at 34.17 ms versus 34.71 ms mean over three calls each, a difference inside one standard deviation of either. That measurement used pg_stat_statements, which IS now installed and collecting on prod (616 statements) — 20260814000020 applied, so the repeated note that it is unavailable is stale. The behaviour change is therefore real but currently uninstantiated: someone who wrote tags "ci auth" meaning two tags now has one. Body change only on all three functions, at byte-identical signatures. One CREATE VIEW, one CREATE FUNCTION, three CREATE OR REPLACE, no DROP.'
);
