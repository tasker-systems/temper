-- The field scrub act in SQL (field-grain scrub, build Task 4).
--
-- Spec: temper-artifacts specs/2026-10-09-field-grain-scrub-design.md S1-S5, S7, read beside
-- specs/2026-09-28-resource-erasure-design.md D4, D10, D11, D13, D14. Plan:
-- plans/2026-10-09-field-grain-scrub.md, Task 4.
--
-- 20261018100010 taught the ledger exception a second authority, resource_scrubbed, and defined the
-- scrub's family and prior predicates once (_field_scrub_property_events, _field_scrub_family_events,
-- _field_scrub_event_is_prior). This migration is the act that writes under that authority. Every
-- "which events" and "is it prior" question below is a call to those functions, never a restatement.
--
--   1. _resource_field_scrub_request_fault: the 400-class faults of a request, one definition for
--      the act and the plan.
--   2. resource_field_scrub_families: the text-free listing an operator picks a family handle from.
--   3. _resource_field_scrub_derivation: each event of the field with its paths at their scrub
--      sentinels (S7), and the paths it holds back and why.
--   4. resource_field_scrub_plan (and its internal form): the ONE computation (S5) the act consumes
--      and the survey renders.
--   5. _resource_field_scrub_rewrite_projection: the folded kb_properties rows the plan names, set
--      to what replay of the redacted events projects (S4 step 6).
--   6. resource_field_scrub_execute: the act (S4).
--   7. resource_field_scrub_survey: the plan and the listing, recording nothing (D10).
--   8. resource_erasure_refuse admits the field scrub's act and its two recorded reasons.
--
-- No key text and no value leaves any public function here (S1): the listing, the plan, the survey
-- and the act's return carry event ids, row ids, paths, counts, dates, profile ids and JSON types
-- only. The derivation, which holds the rewritten payloads, is internal to the act.

-- ---------------------------------------------------------------------------
-- Section 1. The request's faults (S4, "Answered 400, recording nothing").
-- ---------------------------------------------------------------------------
-- NULL when the request is well formed for p_resource, otherwise the fault, which the caller raises
-- under its own prefix. The text names the field kind and ids, never a key or a value.
--
-- The family handle (S1) is the id of the first event still carrying the family's key text, which
-- _field_scrub_property_events computes. That function makes resource_created a member of R's
-- `doc_type` family (erasure D4: R's doc_type family is first seen at create), so the doc_type
-- family's handle is resource_created's id. That id is accepted as a `property` handle for that
-- family and only for it, because it is the row the family listing returns (field-grain scrub S2:
-- clearing doc_type makes resource_created.doc_type prior). Any other id is foreign: another
-- resource's event, an event that is not a property event, an edge- or block-owned property event,
-- or a later event of one of R's families.
CREATE FUNCTION _resource_field_scrub_request_fault(p_resource uuid, p_field text, p_family uuid,
                                                    p_clear boolean)
RETURNS text
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT CASE
        WHEN p_field IS NULL OR p_field NOT IN ('title', 'origin_uri', 'property', 'properties')
            THEN 'unknown field kind'
        WHEN p_clear IS NULL
            THEN 'p_clear is required'
        WHEN p_field = 'property' AND p_family IS NULL
            THEN 'field property needs a family handle'
        WHEN p_field <> 'property' AND p_family IS NOT NULL
            THEN 'field ' || p_field || ' takes no family handle'
        WHEN p_field = 'properties' AND p_clear
            THEN 'properties cannot be cleared'
        WHEN p_field = 'property'
             AND NOT EXISTS (SELECT 1 FROM _field_scrub_property_events(p_resource) pe
                              WHERE pe.event_id = p_family AND pe.handle = p_family)
            THEN 'family ' || p_family::text || ' is not a property family handle of resource '
                 || p_resource::text
    END;
$$;

-- ---------------------------------------------------------------------------
-- Section 2. The family listing (S1).
-- ---------------------------------------------------------------------------
-- One row for the title, one for the origin URI (when R carries one) and one per property family
-- of R, each family by its handle. Structure only, by construction: no column holds key text or a
-- value. `live`: some row an event of the field asserted is live (always true for the title and
-- origin URI, which a resource always holds). `unset`: the family has a property_unset, so its key
-- text up to the latest one is scrubbable in keep mode (S3). `first_by`: the profile of the first
-- event's emitter. `value_type`: jsonb_typeof of the latest value the field carries, NULL for a
-- family that was only ever unset (review focus 3).
CREATE FUNCTION resource_field_scrub_families(p_resource uuid)
RETURNS TABLE(field text, family uuid, events integer, live boolean, unset boolean,
              first_seen timestamptz, first_by uuid, value_type text)
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    WITH members AS (
        SELECT k.kind AS field, NULL::uuid AS family, f.id AS event_id, k.ord
          FROM unnest(ARRAY['title', 'origin_uri']) WITH ORDINALITY AS k(kind, ord)
         CROSS JOIN LATERAL _field_scrub_family_events(p_resource, k.kind, NULL) AS f(id)
        UNION ALL
        SELECT 'property', pe.handle, pe.event_id, 3
          FROM _field_scrub_property_events(p_resource) pe
    ), facts AS (
        SELECT m.field, m.family, m.ord, e.id, e.occurred_at, t.name AS event_type,
               en.profile_id,
               CASE WHEN m.field IN ('title', 'origin_uri') THEN e.payload -> m.field
                    WHEN t.name = 'resource_created' THEN e.payload -> 'doc_type'
                    ELSE e.payload -> 'value'
               END AS value,
               EXISTS (SELECT 1 FROM kb_properties p
                        WHERE p.owner_table = 'kb_resources' AND p.owner_id = p_resource
                          AND NOT p.is_folded AND p.asserted_by_event_id = e.id) AS asserts_live
          FROM members m
          JOIN kb_events e ON e.id = m.event_id
          JOIN kb_event_types t ON t.id = e.event_type_id
          LEFT JOIN kb_entities en ON en.id = e.emitter_entity_id
    )
    SELECT f.field, f.family, count(*)::integer,
           f.field <> 'property' OR bool_or(f.asserts_live),
           f.field = 'property' AND bool_or(f.event_type = 'property_unset'),
           (array_agg(f.occurred_at ORDER BY f.id))[1],
           (array_agg(f.profile_id ORDER BY f.id))[1],
           (array_agg(jsonb_typeof(f.value) ORDER BY f.id DESC)
                FILTER (WHERE f.value IS NOT NULL))[1]
      FROM facts f
     GROUP BY f.field, f.family, f.ord
     ORDER BY f.ord, f.family;
$$;

COMMENT ON FUNCTION resource_field_scrub_families(uuid) IS
$c$The field scrub's family listing (field-grain scrub spec S1): the title, the origin URI and each
resource-owned property family of the resource, a family by its handle (the id of the first event
still carrying its key text; resource_created for the doc_type family). Event count, live, unset,
first seen and by whom (profile id), and the latest value's JSON type. No key text and no value.$c$;

-- ---------------------------------------------------------------------------
-- Section 3. The derivation (S3, S7).
-- ---------------------------------------------------------------------------
-- For each event of the field (_field_scrub_family_events), in walk order: the payload with every
-- prior path at its scrub sentinel, the paths that changed, the facet inner-key renaming it applied
-- (for the projection rewrite; internal, it carries inner-key text), and the paths it held back
-- with the reason. The shape of _resource_erasure_payload_redaction (20261010100000), whose
-- conventions it keeps: the allowlist and its classes decide the paths (_erasure_redact_paths,
-- _erasure_path_class, _erasure_expand), a JSON null carries no text and is never rewritten, a path
-- already at its sentinel is done and not listed (the verifier's own predicate,
-- _erasure_sentinel_admits, under the scrub's authority, erasure D12), and a location with no
-- sentinel raises.
--
-- A path is the field's own when its class is: `title` for the title, `origin-uri` for the origin
-- URI, and for a property family the classes of its key and value (doc-type on resource_created and
-- on a doc_type event; the key classes `keep` of doc_type and facet are never rewritten). Whether
-- it is prior is _field_scrub_event_is_prior's answer (S3), unless p_assume_cleared, which is the
-- survey's reading of clear mode (S5): once the act's clearing event exists, every event of the
-- field is prior, so every path of the field is taken as prior without asking.
--
-- Sentinels (S7): the title, origin URI, property value and doc type take erasure's exact sentinels
-- (_erasure_sentinel_exact); a renamed key is scrubbed-key-<handle>; a facet value keeps one mark
-- per original, each inner key renamed scrubbed-facet-<id of the first of R's facet events naming
-- it>-<its position in that event's value as stored>, each inner value "erased"; a facet value with
-- no inner key takes erased:<event id>, as under erasure.
CREATE FUNCTION _resource_field_scrub_derivation(p_resource uuid, p_field text, p_family uuid,
                                                 p_assume_cleared boolean)
RETURNS TABLE(event_id uuid, event_type text, new_payload jsonb, paths jsonb, inner_keys jsonb,
              held jsonb)
LANGUAGE plpgsql STABLE
SET search_path = public, pg_temp
AS $$
DECLARE
    v_classes text[] := CASE p_field
                            WHEN 'title'      THEN ARRAY['title']
                            WHEN 'origin_uri' THEN ARRAY['origin-uri']
                            ELSE ARRAY['property-key', 'property-value', 'doc-type', 'facet-value']
                        END;
    v_facets  jsonb;
    v_ev      record;
    v_path    text;
    v_class   text;
    v_loc     record;
    v_new     jsonb;
    v_payload jsonb;
    v_paths   jsonb;
    v_inner   jsonb;
    v_held    jsonb;
BEGIN
    FOR v_ev IN
        SELECT e.id, t.name AS type, e.payload, pe.handle
          FROM _field_scrub_family_events(p_resource, p_field, p_family) AS f(id)
          JOIN kb_events e ON e.id = f.id
          JOIN kb_event_types t ON t.id = e.event_type_id
          LEFT JOIN _field_scrub_property_events(p_resource) pe ON pe.event_id = e.id
         ORDER BY e.id
    LOOP
        v_payload := v_ev.payload;
        v_paths   := '[]'::jsonb;
        v_inner   := '{}'::jsonb;
        v_held    := '[]'::jsonb;
        FOR v_path, v_class IN
            SELECT r.path, _erasure_path_class(v_ev.type, r.path, v_ev.payload)
              FROM (SELECT DISTINCT r0.path FROM _erasure_redact_paths() r0
                     WHERE r0.event_type = v_ev.type) r
             ORDER BY r.path
        LOOP
            CONTINUE WHEN v_class IS NULL OR v_class = 'keep' OR v_class <> ALL (v_classes);
            FOR v_loc IN SELECT x.at, x.val FROM _erasure_expand(v_ev.payload, v_path) x LOOP
                CONTINUE WHEN jsonb_typeof(v_loc.val) = 'null';
                CONTINUE WHEN _erasure_sentinel_admits(v_class, v_ev.id, v_ev.payload, v_loc.at,
                                                       v_loc.val, v_loc.val, 'scrub');
                IF NOT p_assume_cleared
                   AND NOT coalesce(_field_scrub_event_is_prior(p_resource, p_field, p_family,
                                                                v_ev.id, v_path), false) THEN
                    -- Held back (S5): today's value is kept; key text after the latest unset and
                    -- a facet event after the latest whole-facet fold are unreachable.
                    v_held := v_held || jsonb_build_object(
                        'path', v_path,
                        'why', CASE
                                   WHEN v_class IN ('title', 'origin-uri') THEN 'current'
                                   WHEN v_path = 'property_key' THEN 'after_latest_unset'
                                   WHEN EXISTS (SELECT 1 FROM kb_properties p
                                                 WHERE p.owner_table = 'kb_resources'
                                                   AND p.owner_id = p_resource
                                                   AND NOT p.is_folded
                                                   AND p.asserted_by_event_id = v_ev.id)
                                       THEN 'current'
                                   ELSE 'after_whole_facet_fold'
                               END);
                    CONTINUE;
                END IF;
                IF v_class = 'property-key' THEN
                    v_new := to_jsonb('scrubbed-key-' || v_ev.handle::text);
                ELSIF v_class = 'facet-value' THEN
                    IF EXISTS (SELECT 1 FROM _facet_marks(v_loc.val) fm WHERE fm.inner_key IS NULL) THEN
                        v_new := to_jsonb('erased:' || v_ev.id::text);
                    ELSE
                        IF v_facets IS NULL THEN
                            SELECT coalesce(jsonb_object_agg(x.inner_key,
                                                'scrubbed-facet-' || x.id::text || '-' || x.ord::text),
                                            '{}'::jsonb)
                              INTO v_facets
                              FROM (SELECT DISTINCT ON (fm.inner_key) fm.inner_key, pe.event_id AS id, fm.ord
                                      FROM _field_scrub_property_events(p_resource) pe
                                      JOIN kb_events e ON e.id = pe.event_id
                                     CROSS JOIN LATERAL _facet_marks(e.payload -> 'value')
                                                WITH ORDINALITY AS fm(inner_key, inner_value, ord)
                                     WHERE pe.key_text = 'facet'
                                       AND pe.event_type IN ('property_set', 'property_asserted')
                                       AND fm.inner_key IS NOT NULL
                                     ORDER BY fm.inner_key, pe.event_id, fm.ord) x;
                        END IF;
                        IF EXISTS (SELECT 1 FROM _facet_marks(v_loc.val) fm
                                    WHERE NOT v_facets ? fm.inner_key) THEN
                            RAISE EXCEPTION '_resource_field_scrub_derivation: no inner-key sentinel for a facet of event %',
                                v_ev.id;
                        END IF;
                        SELECT jsonb_object_agg(fm.inner_key, v_facets -> fm.inner_key),
                               jsonb_object_agg(v_facets ->> fm.inner_key, '"erased"'::jsonb)
                          INTO v_inner, v_new
                          FROM _facet_marks(v_loc.val) fm;
                    END IF;
                ELSE
                    v_new := _erasure_sentinel_exact(v_class, v_ev.id, v_ev.payload);
                END IF;
                IF v_new IS NULL THEN
                    -- jsonb_set with a NULL value returns NULL: never let a missing sentinel empty
                    -- a whole payload.
                    RAISE EXCEPTION '_resource_field_scrub_derivation: no sentinel for % of event %',
                        v_path, v_ev.id;
                END IF;
                IF v_new IS DISTINCT FROM v_loc.val THEN
                    v_payload := jsonb_set(v_payload, v_loc.at, v_new, false);
                    IF NOT v_paths @> to_jsonb(ARRAY[v_path]) THEN
                        v_paths := v_paths || to_jsonb(v_path);
                    END IF;
                END IF;
            END LOOP;
        END LOOP;
        IF jsonb_array_length(v_paths) > 0 OR jsonb_array_length(v_held) > 0 THEN
            event_id    := v_ev.id;
            event_type  := v_ev.type;
            new_payload := v_payload;
            paths       := v_paths;
            inner_keys  := v_inner;
            held        := v_held;
            RETURN NEXT;
        END IF;
    END LOOP;
END;
$$;

-- ---------------------------------------------------------------------------
-- Section 4. The plan: the ONE computation (S5, erasure D10).
-- ---------------------------------------------------------------------------
-- The internal form returns the plan and the derivation it was computed from; the act needs both
-- and computes them once. The derivation carries the rewritten payloads and so never leaves the
-- act: the public plan returns only the plan.
--
-- The plan (paths and ids only):
--   redacted_fields  [{event, paths}], the shape resource_erased uses;
--   kept             [{event, paths, why: current}], today's value;
--   unreachable      [{event, paths, why}], `after_latest_unset` (key text, S3) or
--                    `after_whole_facet_fold` (S3, facets);
--   folded_rows      the kb_properties ids the act rewrites: R's folded rows asserted by an event
--                    whose key or value it rewrites. Under p_clear, also the rows the clearing
--                    event would fold;
--   clears           the clearing events the act would append (S2), under p_clear;
--   refusal          NULL, or the reason the act would refuse: charter_resource and
--                    already_erased (recorded; the plan stops there, D10), sentinel_collision and
--                    projection_disagrees (recorded), nothing_prior (keep mode; answered 400).
--
-- Clear mode has one definition (plan Task 4 Step 3): the keep-mode plan run after the clearing
-- events. The act appends them and then asks for the keep-mode plan. The survey cannot append, so
-- under p_clear the derivation takes every path of the field as prior and `clears` names the events
-- the act would append; the two are held equal by a witness.
CREATE FUNCTION _resource_field_scrub_plan_with(p_resource uuid, p_field text, p_family uuid,
                                                p_clear boolean, OUT plan jsonb, OUT redact jsonb)
LANGUAGE plpgsql STABLE
SET search_path = public, pg_temp
AS $$
DECLARE
    v_erased  boolean;
    v_refusal text;
    v_fault   text;
    v_fields  jsonb;
    v_last    text;
    v_current text;
    v_clears  jsonb := '[]'::jsonb;
BEGIN
    SELECT r.erased_at IS NOT NULL INTO v_erased FROM kb_resources r WHERE r.id = p_resource;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'resource_field_scrub_plan: resource % not found', p_resource;
    END IF;
    -- The act's order: a charter refuses before an erased resource does, and both before the
    -- request is read (the block scrub's order, 20261015100010).
    v_refusal := CASE
                     WHEN EXISTS (SELECT 1 FROM kb_cogmaps c WHERE c.telos_resource_id = p_resource)
                         THEN 'charter_resource'
                     WHEN v_erased THEN 'already_erased'
                 END;
    IF v_refusal IS NOT NULL THEN
        plan := jsonb_build_object('redacted_fields', '[]'::jsonb, 'kept', '[]'::jsonb,
                                   'unreachable', '[]'::jsonb, 'folded_rows', '[]'::jsonb,
                                   'clears', '[]'::jsonb, 'refusal', v_refusal);
        redact := '[]'::jsonb;
        RETURN;
    END IF;
    v_fault := _resource_field_scrub_request_fault(p_resource, p_field, p_family, p_clear);
    IF v_fault IS NOT NULL THEN
        RAISE EXCEPTION 'resource_field_scrub_plan: %', v_fault;
    END IF;

    SELECT coalesce(jsonb_agg(jsonb_build_object(
                        'event',      d.event_id,
                        'event_type', d.event_type,
                        'payload',    d.new_payload,
                        'paths',      d.paths,
                        'inner_keys', d.inner_keys,
                        'held',       d.held)
                    ORDER BY d.event_id), '[]'::jsonb)
      INTO redact
      FROM _resource_field_scrub_derivation(p_resource, p_field, p_family, p_clear) d;

    SELECT coalesce(jsonb_agg(jsonb_build_object('event', r -> 'event', 'paths', r -> 'paths')
                              ORDER BY r ->> 'event'), '[]'::jsonb)
      INTO v_fields
      FROM jsonb_array_elements(redact) r
     WHERE jsonb_array_length(r -> 'paths') > 0;

    IF p_clear THEN
        v_clears := jsonb_build_array(CASE
            WHEN p_field IN ('title', 'origin_uri')
                THEN jsonb_build_object('event_type', 'resource_updated', 'path', p_field)
            WHEN EXISTS (SELECT 1 FROM _field_scrub_property_events(p_resource) pe
                          WHERE pe.event_id = p_family AND pe.key_text = 'doc_type')
                THEN jsonb_build_object('event_type', 'property_set', 'path', 'value')
            ELSE jsonb_build_object('event_type', 'property_unset', 'path', 'property_key')
        END);
    END IF;

    -- S7: a renamed key or facet inner key the plan writes that R's ledger already names.
    IF EXISTS (SELECT 1 FROM jsonb_array_elements(redact) r
                WHERE r -> 'paths' ? 'property_key'
                  AND EXISTS (SELECT 1 FROM _field_scrub_property_events(p_resource) pe
                               WHERE pe.key_text = r #>> '{payload,property_key}'))
       OR EXISTS (SELECT 1
                    FROM jsonb_array_elements(redact) r
                   CROSS JOIN LATERAL jsonb_each_text(r -> 'inner_keys') ik
                   WHERE EXISTS (SELECT 1
                                   FROM _field_scrub_property_events(p_resource) pe
                                   JOIN kb_events e ON e.id = pe.event_id
                                  CROSS JOIN LATERAL _facet_marks(e.payload -> 'value') fm
                                  WHERE pe.key_text = 'facet'
                                    AND pe.event_type IN ('property_set', 'property_asserted')
                                    AND fm.inner_key = ik.value)) THEN
        v_refusal := 'sentinel_collision';
    ELSIF p_field IN ('title', 'origin_uri') AND NOT p_clear THEN
        -- S3's keep-mode guard: the event producing today's value is the last carrying the field
        -- in walk order. When its value is not the projection's, the order inverted inside a
        -- transaction (erasure D14); refused, never resolved.
        SELECT e.payload ->> p_field INTO v_last
          FROM _field_scrub_family_events(p_resource, p_field, NULL) AS f(id)
          JOIN kb_events e ON e.id = f.id
         ORDER BY f.id DESC
         LIMIT 1;
        SELECT CASE p_field WHEN 'title' THEN r.title ELSE r.origin_uri END INTO v_current
          FROM kb_resources r WHERE r.id = p_resource;
        IF FOUND AND v_last IS DISTINCT FROM v_current THEN
            v_refusal := 'projection_disagrees';
        END IF;
    END IF;
    IF v_refusal IS NULL AND NOT p_clear AND jsonb_array_length(v_fields) = 0 THEN
        v_refusal := 'nothing_prior';
    END IF;

    plan := jsonb_build_object(
        'redacted_fields', v_fields,
        'kept', (SELECT coalesce(jsonb_agg(jsonb_build_object('event', x.event, 'paths', x.paths,
                                                              'why', x.why)
                                           ORDER BY x.event, x.why), '[]'::jsonb)
                   FROM (SELECT r ->> 'event' AS event, h ->> 'why' AS why,
                                jsonb_agg(h -> 'path' ORDER BY h ->> 'path') AS paths
                           FROM jsonb_array_elements(redact) r
                          CROSS JOIN LATERAL jsonb_array_elements(r -> 'held') h
                          WHERE h ->> 'why' = 'current'
                          GROUP BY 1, 2) x),
        'unreachable', (SELECT coalesce(jsonb_agg(jsonb_build_object('event', x.event, 'paths', x.paths,
                                                                     'why', x.why)
                                                  ORDER BY x.event, x.why), '[]'::jsonb)
                          FROM (SELECT r ->> 'event' AS event, h ->> 'why' AS why,
                                       jsonb_agg(h -> 'path' ORDER BY h ->> 'path') AS paths
                                  FROM jsonb_array_elements(redact) r
                                 CROSS JOIN LATERAL jsonb_array_elements(r -> 'held') h
                                 WHERE h ->> 'why' <> 'current'
                                 GROUP BY 1, 2) x),
        'folded_rows', (SELECT coalesce(jsonb_agg(p.id ORDER BY p.id), '[]'::jsonb)
                          FROM jsonb_array_elements(redact) r
                          JOIN kb_properties p ON p.asserted_by_event_id = (r ->> 'event')::uuid
                         WHERE p.owner_table = 'kb_resources' AND p.owner_id = p_resource
                           AND (p.is_folded OR p_clear)
                           AND r -> 'paths' ?| ARRAY['property_key', 'value', 'doc_type']),
        'clears', v_clears,
        'refusal', v_refusal);
END;
$$;

CREATE FUNCTION resource_field_scrub_plan(p_resource uuid, p_field text, p_family uuid,
                                          p_clear boolean)
RETURNS jsonb
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT x.plan FROM _resource_field_scrub_plan_with(p_resource, p_field, p_family, p_clear) x;
$$;

COMMENT ON FUNCTION resource_field_scrub_plan(uuid, text, uuid, boolean) IS
$c$The field scrub's ONE computation (field-grain scrub spec S5, erasure D10): which events and paths
the act redacts, which it keeps and which it cannot reach and why, which folded kb_properties rows it
rewrites, the clearing events it appends under p_clear, and the refusal verdict. The act consumes it
(through _resource_field_scrub_plan_with) and the survey renders it. Event ids, row ids and paths
only. A malformed or foreign request raises 'resource_field_scrub_plan: <fault>'.$c$;

-- ---------------------------------------------------------------------------
-- Section 5. The projection rewrite (S4 step 6, S7).
-- ---------------------------------------------------------------------------
-- Each folded row the plan names takes what its asserting event, as redacted, projects on replay:
-- the key as rewritten (a doc_type row keeps its literal key), and _property_value_normalized(<that
-- key>, <the payload's sentinel>), the normalisation _project_property_set and
-- _project_property_asserted apply (20260929040730): under a kept `tags` key a string sentinel
-- becomes ["erased:<id>"], under a renamed key it stays a string. A facet mark row takes the
-- one-key object of its renamed inner key and "erased", the row _project_property_asserted inserts
-- for a mark. is_folded and last_event_id are never touched: replay sets both as live has them.
--
-- No search-vector rebuild: _rebuild_resource_search_vector reads the title and only rows
-- `AND NOT is_folded`, and this rewrites folded rows only.
CREATE FUNCTION _resource_field_scrub_rewrite_projection(p_resource uuid, p_plan jsonb,
                                                         p_redact jsonb)
RETURNS integer
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_rows integer;
BEGIN
    UPDATE kb_properties p
       SET property_key   = t.new_key,
           property_value = t.new_value
      FROM (
        SELECT q.id, k.new_key,
               CASE
                   WHEN r ->> 'event_type' = 'resource_created'
                       THEN _property_value_normalized(k.new_key, r #> '{payload,doc_type}')
                   WHEN q.property_key = 'facet' AND jsonb_typeof(q.property_value) = 'object'
                       THEN jsonb_build_object(
                                r -> 'inner_keys' ->> (SELECT min(ok) FROM jsonb_object_keys(q.property_value) ok),
                                'erased')
                   ELSE _property_value_normalized(k.new_key, r #> '{payload,value}')
               END AS new_value
          FROM jsonb_array_elements(p_plan -> 'folded_rows') fr
          JOIN kb_properties q ON q.id = (fr #>> '{}')::uuid
          JOIN jsonb_array_elements(p_redact) r ON (r ->> 'event')::uuid = q.asserted_by_event_id
         CROSS JOIN LATERAL (SELECT CASE WHEN r ->> 'event_type' = 'resource_created'
                                         THEN q.property_key
                                         ELSE r #>> '{payload,property_key}' END AS new_key) k
         WHERE q.owner_table = 'kb_resources' AND q.owner_id = p_resource AND q.is_folded
      ) t
     WHERE p.id = t.id;
    GET DIAGNOSTICS v_rows = ROW_COUNT;
    IF v_rows <> jsonb_array_length(p_plan -> 'folded_rows') THEN
        RAISE EXCEPTION '_resource_field_scrub_rewrite_projection: % of the plan''s % rows are folded rows of resource % asserted by a redacted event',
            v_rows, jsonb_array_length(p_plan -> 'folded_rows'), p_resource;
    END IF;
    RETURN v_rows;
END;
$$;

-- ---------------------------------------------------------------------------
-- Section 6. The act (S4).
-- ---------------------------------------------------------------------------
-- block_history_scrub_execute's prologue (20261015100010): the request reference is required; the
-- resource is found; the act queue exclusive, then FOR UPDATE on R, under lock_timeout = 0, so a
-- floored writer arriving after the act queues behind it and no bound cuts the act off (erasure
-- D13); then the charter and erased raises. Every refusal RAISES with the prefix
-- `resource_field_scrub_execute: `, and the service classifies the rest of the text: the charter,
-- `already erased`, `sentinel collision` and `projection disagrees` are recorded through
-- resource_erasure_refuse; a fault, `nothing prior to scrub` and `resource % not found` record
-- nothing. Nothing is written before the last raise that can follow a write: a raise after the
-- clearing events rolls them back with the transaction.
--
-- Then, in one transaction (S4):
--   clear mode: the clearing event through its ordinary path (S2): resource_update for the title or
--     origin URI (`scrubbed-<resource id>`); property_set of doc_type to `scrubbed` for the doc_type
--     family; for any other family a property_unset of its key, appended as the fire path appends
--     it (_property_owner_anchor, _event_append) and projected by _project_property_unset. Each
--     carries the request reference as its correlation;
--   the keep-mode plan, read after the clearing event (S5's one definition of clear mode);
--   its refusals;
--   resource_scrubbed, with the references and correlation of the block scrub's record, and its
--     redaction rows (_project_resource_scrubbed_redactions);
--   ONE UPDATE of kb_events for every rewritten event: kb_events_redaction_in_trail evaluates prior
--     with the OLD key texts of the rows rewritten in its own statement, so a second statement would
--     find a renamed key outside its family and be refused;
--   the projection rewrite.
CREATE FUNCTION resource_field_scrub_execute(p_resource uuid, p_field text, p_family uuid,
                                             p_clear boolean, p_operator uuid, p_emitter uuid,
                                             p_request_ref uuid)
RETURNS jsonb
LANGUAGE plpgsql
SET search_path = public, pg_temp
SET lock_timeout TO '0'
AS $$
DECLARE
    v_found     boolean;
    v_charter   uuid;
    v_erased_ts timestamptz;
    v_fault     text;
    v_key       text;
    v_anchor_t  text;
    v_anchor_id uuid;
    v_unset     jsonb;
    v_plan      jsonb;
    v_redact    jsonb;
    v_refusal   text;
    v_fields    jsonb;
    v_payload   jsonb;
    v_ev        uuid;
BEGIN
    IF p_resource IS NULL THEN
        RAISE EXCEPTION 'resource_field_scrub_execute: p_resource is required';
    END IF;
    -- The request reference is the act's correlation id: the record and any clearing event carry
    -- it, so the act pairs with its request.
    IF p_request_ref IS NULL THEN
        RAISE EXCEPTION 'resource_field_scrub_execute: p_request_ref is required';
    END IF;

    SELECT count(*) > 0 INTO v_found FROM kb_resources r WHERE r.id = p_resource;
    IF NOT v_found THEN
        RAISE EXCEPTION 'resource_field_scrub_execute: resource % not found', p_resource;
    END IF;
    -- The act queue, exclusive, before R's row lock (20261015100010): a floored writer arriving
    -- from here on queues behind this act instead of joining the row's KEY SHARE holders.
    PERFORM pg_advisory_xact_lock(_resource_act_queue_key(p_resource));
    PERFORM 1 FROM kb_resources WHERE id = p_resource FOR UPDATE;
    SELECT c.telos_resource_id INTO v_charter FROM kb_cogmaps c WHERE c.telos_resource_id = p_resource;
    SELECT r.erased_at INTO v_erased_ts FROM kb_resources r WHERE r.id = p_resource;

    IF v_charter IS NOT NULL THEN
        RAISE EXCEPTION 'resource_field_scrub_execute: charter resource (map-grain erasure is filed task 01a0e960-0ca2-7f42-b33e-1ed19b024e6b)';
    END IF;
    IF v_erased_ts IS NOT NULL THEN
        RAISE EXCEPTION 'resource_field_scrub_execute: already erased';
    END IF;

    v_fault := _resource_field_scrub_request_fault(p_resource, p_field, p_family, p_clear);
    IF v_fault IS NOT NULL THEN
        RAISE EXCEPTION 'resource_field_scrub_execute: %', v_fault;
    END IF;

    -- ── Clear mode (S2): the clearing event, through its ordinary projector. ─────────────────
    IF p_clear THEN
        IF p_field IN ('title', 'origin_uri') THEN
            PERFORM resource_update(
                jsonb_build_object('resource_id', p_resource,
                                   p_field, 'scrubbed-' || p_resource::text),
                p_emitter, p_correlation => p_request_ref);
        ELSE
            SELECT pe.key_text INTO v_key
              FROM _field_scrub_property_events(p_resource) pe WHERE pe.event_id = p_family;
            IF v_key = 'doc_type' THEN
                -- A resource keeps a type (S2): the placeholder type, which folds the live type
                -- row. The key is the structural literal and is never renamed (erasure D4).
                PERFORM property_set(
                    jsonb_build_object(
                        'property_id',  uuid_generate_v7(),
                        'owner',        jsonb_build_object('table', 'kb_resources', 'id', p_resource),
                        'property_key', 'doc_type',
                        'value',        'scrubbed',
                        'weight',       1.0),
                    p_emitter, p_correlation => p_request_ref);
            ELSE
                -- The unset carries the real key text only inside this transaction; the plan
                -- below redacts it with the rest of the family before commit (S2).
                v_unset := jsonb_build_object(
                    'owner',        jsonb_build_object('table', 'kb_resources', 'id', p_resource),
                    'property_key', v_key);
                SELECT a.anchor_table, a.anchor_id INTO v_anchor_t, v_anchor_id
                  FROM _property_owner_anchor('kb_resources', p_resource) a;
                v_ev := _event_append('property_unset', p_emitter, v_anchor_t, v_anchor_id, v_unset,
                                      p_correlation => p_request_ref);
                PERFORM _project_property_unset(v_ev, v_unset);
            END IF;
        END IF;
    END IF;

    -- ── The ONE computation, in keep mode, after any clearing event (S5). ────────────────────
    SELECT x.plan, x.redact INTO v_plan, v_redact
      FROM _resource_field_scrub_plan_with(p_resource, p_field, p_family, false) x;
    v_refusal := v_plan ->> 'refusal';
    IF v_refusal = 'sentinel_collision' THEN
        RAISE EXCEPTION 'resource_field_scrub_execute: sentinel collision';
    ELSIF v_refusal = 'projection_disagrees' THEN
        RAISE EXCEPTION 'resource_field_scrub_execute: projection disagrees';
    ELSIF v_refusal = 'nothing_prior' THEN
        -- Keep mode only: a clearing event is a change of its own, recorded with what it made
        -- prior, even when every prior path already held its sentinel.
        IF NOT p_clear THEN
            RAISE EXCEPTION 'resource_field_scrub_execute: nothing prior to scrub';
        END IF;
    ELSIF v_refusal IS NOT NULL THEN
        RAISE EXCEPTION 'resource_field_scrub_execute: the plan refused (%)', v_refusal;
    END IF;
    v_fields := v_plan -> 'redacted_fields';

    -- ── The record (S4): field by kind and handle, never key text; cleared only when true. ───
    v_payload := jsonb_build_object(
        'subject_table',   'kb_resources',
        'subject_id',      p_resource,
        'actor',           p_operator,
        'field',           CASE WHEN p_field = 'property'
                                THEN jsonb_build_object('kind', p_field, 'family', p_family)
                                ELSE jsonb_build_object('kind', p_field) END,
        'redacted_fields', v_fields);
    IF p_clear THEN
        v_payload := v_payload || jsonb_build_object('cleared', true);
    END IF;
    v_ev := _event_append(
        'resource_scrubbed', p_emitter, NULL, NULL,
        v_payload,
        p_references => jsonb_build_array(
            jsonb_build_object('rel','subject',
                'target', jsonb_build_object('kind','kb_resources','id', p_resource)),
            jsonb_build_object('rel','request',
                'target', jsonb_build_object('kind','kb_events','id', p_request_ref))),
        p_correlation => p_request_ref);
    PERFORM _project_resource_scrubbed_redactions(v_ev, v_payload);

    -- ── The ledger rewrite: ONE statement (see the header). ─────────────────────────────────
    UPDATE kb_events e
       SET payload = r -> 'payload'
      FROM jsonb_array_elements(v_redact) r
     WHERE e.id = (r ->> 'event')::uuid
       AND jsonb_array_length(r -> 'paths') > 0;

    PERFORM _resource_field_scrub_rewrite_projection(p_resource, v_plan, v_redact);

    RETURN jsonb_build_object(
        'event_id',        v_ev,
        'redacted_fields', v_fields,
        'cleared',         p_clear);
END;
$$;

COMMENT ON FUNCTION resource_field_scrub_execute(uuid, text, uuid, boolean, uuid, uuid, uuid) IS
$c$The field scrub (field-grain scrub spec S4): every prior value of R's title, origin URI, one property
family (by handle) or every family, redacted from the ledger and the projection under one
resource_scrubbed, with today's value kept, or first cleared when p_clear. Serialized by the act
queue and R's row lock under lock_timeout 0 (erasure D13). Raises, prefixed
'resource_field_scrub_execute: ', for a charter, an erased resource, a malformed or foreign request,
a sentinel collision, a projection that disagrees, and nothing prior in keep mode. Returns the
event id, redacted_fields and cleared.$c$;

-- ---------------------------------------------------------------------------
-- Section 7. The survey (D10): the plan and the listing, recording nothing.
-- ---------------------------------------------------------------------------
-- On a charter or an erased resource it answers the plan's refusal and no listing, as the block
-- scrub's survey answers those with no per-block rows.
CREATE FUNCTION resource_field_scrub_survey(p_resource uuid, p_field text, p_family uuid,
                                            p_clear boolean)
RETURNS jsonb
LANGUAGE plpgsql STABLE
SET search_path = public, pg_temp
AS $$
DECLARE
    v_plan jsonb := resource_field_scrub_plan(p_resource, p_field, p_family, p_clear);
BEGIN
    IF v_plan ->> 'refusal' IN ('charter_resource', 'already_erased') THEN
        RETURN jsonb_build_object('families', NULL, 'plan', v_plan);
    END IF;
    RETURN jsonb_build_object(
        'families', (SELECT coalesce(jsonb_agg(to_jsonb(f)), '[]'::jsonb)
                       FROM resource_field_scrub_families(p_resource) f),
        'plan', v_plan);
END;
$$;

-- ---------------------------------------------------------------------------
-- Section 8. The refusal record admits the field scrub (S4).
-- ---------------------------------------------------------------------------
-- 20261003000210's body, CREATE OR REPLACE with the identical signature and defaults, so the six-
-- and eight-argument calls deployed binaries make still resolve. Two changes: the act `field_scrub`
-- is refusable, and its two reasons, sentinel_collision and projection_disagrees, are recorded only
-- beside it. Blocks still ride only a block_history_scrub refusal.
CREATE OR REPLACE FUNCTION public.resource_erasure_refuse(p_resource uuid, p_attempted_by uuid, p_emitter uuid, p_request_ref uuid, p_reason text, p_detail text DEFAULT NULL::text, p_act text DEFAULT NULL::text, p_blocks uuid[] DEFAULT NULL::uuid[])
 RETURNS uuid
 LANGUAGE plpgsql
 SET search_path = public, pg_temp
AS $function$
DECLARE v_ev uuid;
BEGIN
    -- The request reference is the refusal's correlation id; without one, _event_append
    -- correlates the event to itself and the refusal pairs with nothing.
    IF p_request_ref IS NULL THEN
        RAISE EXCEPTION 'resource_erasure_refuse: p_request_ref is required';
    END IF;
    IF p_reason NOT IN ('unauthorized','charter_resource','ingest_in_flight','already_erased',
                        'sentinel_collision','projection_disagrees') THEN
        RAISE EXCEPTION 'resource_erasure_refuse: % is not a resource-erasure refusal reason',
                        p_reason;
    END IF;
    IF p_act IS NOT NULL AND p_act NOT IN ('erasure','block_history_scrub','field_scrub') THEN
        RAISE EXCEPTION 'resource_erasure_refuse: % is not a refusable act', p_act;
    END IF;
    IF p_reason IN ('sentinel_collision','projection_disagrees')
       AND p_act IS DISTINCT FROM 'field_scrub' THEN
        RAISE EXCEPTION 'resource_erasure_refuse: % is recorded only for a field_scrub refusal',
                        p_reason;
    END IF;
    IF cardinality(p_blocks) > 0 AND p_act IS DISTINCT FROM 'block_history_scrub' THEN
        RAISE EXCEPTION 'resource_erasure_refuse: p_blocks is carried only by a block_history_scrub refusal';
    END IF;

    v_ev := _event_append(
        'resource_erasure_refused', p_emitter, NULL, NULL,
        jsonb_strip_nulls(jsonb_build_object(
            'subject_table', 'kb_resources',
            'subject_id',    p_resource,
            'actor',         p_attempted_by,
            'reason',        p_reason,
            'detail',        p_detail,
            'act',           p_act,
            'blocks',        CASE WHEN cardinality(p_blocks) > 0 THEN to_jsonb(p_blocks) END)),
        p_references => jsonb_build_array(
            jsonb_build_object('rel','subject',
                'target', jsonb_build_object('kind','kb_resources','id', p_resource)),
            jsonb_build_object('rel','request',
                'target', jsonb_build_object('kind','kb_events','id', p_request_ref))),
        p_correlation => p_request_ref);

    RETURN v_ev;
END;
$function$;

SELECT declare_migration(
    20261018100020,
    'additive',
    'The field scrub act (field-grain scrub spec S1-S5, S7). New functions: _resource_field_scrub_request_fault, resource_field_scrub_families (a text-free listing), _resource_field_scrub_derivation, _resource_field_scrub_plan_with, resource_field_scrub_plan, _resource_field_scrub_rewrite_projection, resource_field_scrub_execute (the act: act queue and FOR UPDATE under lock_timeout 0, appends resource_scrubbed and rewrites its events under the scrub authority 20261018100010 added) and resource_field_scrub_survey. CREATE OR REPLACE of resource_erasure_refuse with the identical signature and defaults, so the six- and eight-argument calls deployed binaries make still resolve: it also admits the act field_scrub and, beside it only, the reasons sentinel_collision and projection_disagrees, and pins search_path; every call it admitted before is admitted unchanged. No table, column, constraint, grant or event type changes.'
);
