-- Field scrub positions in replay's walk order.
--
-- Replay walks the ledger in append order: `(seq IS NULL) DESC, seq, id` (replay.rs, `WALK_ORDER`;
-- kb_events_seq from 20261116000010). The field scrub's "before" and "latest" were id order, which
-- on PG17 has no order within a millisecond (uuid_generate_v7). So an act could take an event that
-- replay walks after the family's latest unset as at or before it, or the reverse. The live act and
-- replay must agree on every such position, because the act's choice of what is prior is judged by
-- what replay rebuilds.
--
--   1. kb_event_walk_key and _event_walk_key(uuid): an event's walk position as one comparable
--      value, (recorded, seq, id). A row from before 20261116000010 has NULL seq; it is
--      (false, 0, id) and so precedes every recorded row, in id order, as the walk takes it. NULL
--      for an event that does not exist.
--   2. CREATE OR REPLACE, each with its signature, defaults, return type and search_path unchanged,
--      from its live body in 20261018100040_field_scrub_review_fixes.sql. Only the positions
--      change; every predicate, refusal and sentinel is as it was:
--        * _field_scrub_unset_boundary: the family's latest property_unset in walk order;
--        * _field_scrub_event_is_prior: the title or origin URI's last event, the key's "at or
--          before the latest unset" and the facet's "before the latest whole-facet fold", each by
--          _event_walk_key;
--        * _resource_field_scrub_derivation: the held reason's "at or before the latest unset",
--          and the order it lists events in;
--        * _resource_field_scrub_plan_with: the keep-mode guard's last event carrying the title or
--          origin URI.
--
-- A family's handle (_field_scrub_property_events, 20261018100010) stays the lowest id carrying its
-- key text. It is an identity, not a position: scrub sentinels already on the ledger name it.

-- ---------------------------------------------------------------------------
-- Section 1. An event's walk position.
-- ---------------------------------------------------------------------------
CREATE TYPE kb_event_walk_key AS (recorded boolean, seq bigint, id uuid);

COMMENT ON TYPE kb_event_walk_key IS
$c$An event's position in replay's walk order, comparable as a row: (recorded, seq, id), where
recorded is seq IS NOT NULL and seq is 0 for a row with none. Ascending order is replay's walk,
(seq IS NULL) DESC, seq, id.$c$;

CREATE FUNCTION _event_walk_key(p_event uuid)
RETURNS kb_event_walk_key
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT ROW(e.seq IS NOT NULL, coalesce(e.seq, 0), e.id)::kb_event_walk_key
      FROM kb_events e
     WHERE e.id = p_event;
$$;

COMMENT ON FUNCTION _event_walk_key(uuid) IS
$c$The walk position of one kb_events row (kb_event_walk_key): every SQL comparison of two events'
positions uses it, so the field scrub's "before" and "latest" are the order replay walks. NULL for
an id that names no event.$c$;

-- ---------------------------------------------------------------------------
-- Section 2. The field scrub's positions.
-- ---------------------------------------------------------------------------

-- The family's latest property_unset (S3, "unset first") in walk order: the boundary at or before
-- which its key text is scrubbable. NULL for a family never unset.
CREATE OR REPLACE FUNCTION _field_scrub_unset_boundary(p_resource uuid, p_handle uuid, p_keys jsonb DEFAULT NULL)
RETURNS uuid
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT pe.event_id
      FROM _field_scrub_property_events(p_resource, p_keys) pe
     WHERE pe.handle = p_handle AND pe.event_type = 'property_unset'
     ORDER BY _event_walk_key(pe.event_id) DESC
     LIMIT 1;
$$;

COMMENT ON FUNCTION _field_scrub_unset_boundary(uuid, uuid, jsonb) IS
$c$The family's latest property_unset in replay's walk order (_event_walk_key), the boundary at or
before which its key text is scrubbable (field-grain scrub spec S3); NULL for a family never unset.
_field_scrub_event_is_prior and the derivation's held reason compare against it by walk position.$c$;

-- 20261018100040 Section 2's body; the title's last event, the unset boundary and the facet
-- boundary compare by _event_walk_key.
CREATE OR REPLACE FUNCTION public._field_scrub_event_is_prior(p_resource uuid, p_kind text, p_family uuid, p_event uuid, p_path text DEFAULT NULL::text, p_keys jsonb DEFAULT NULL::jsonb)
 RETURNS boolean
 LANGUAGE plpgsql
 STABLE
 SET search_path = public, pg_temp
AS $function$
DECLARE
    v_type     text;
    v_key      text;
    v_handle   uuid;
    v_boundary uuid;
BEGIN
    IF p_kind IN ('title', 'origin_uri') THEN
        -- S3, "Title and origin URI": the event producing today's value is the last event carrying
        -- the field in walk order (_event_walk_key); every earlier event carrying it is prior.
        RETURN (p_path IS NULL OR p_path = p_kind)
           AND EXISTS (SELECT 1 FROM _field_scrub_family_events(p_resource, p_kind, NULL) f
                        WHERE f = p_event)
           AND EXISTS (SELECT 1 FROM _field_scrub_family_events(p_resource, p_kind, NULL) f
                        WHERE _event_walk_key(f) > _event_walk_key(p_event));
    END IF;
    IF p_kind IS NULL OR p_kind NOT IN ('property', 'properties') THEN
        RETURN false;
    END IF;

    SELECT pe.event_type, pe.key_text, pe.handle
      INTO v_type, v_key, v_handle
      FROM _field_scrub_property_events(p_resource, p_keys) pe
     WHERE pe.event_id = p_event;
    IF NOT FOUND OR (p_kind = 'property' AND v_handle IS DISTINCT FROM p_family) THEN
        RETURN false;
    END IF;
    IF p_path IS NOT NULL AND p_path <> ALL (CASE v_type
                                                 WHEN 'resource_created' THEN ARRAY['doc_type']
                                                 WHEN 'property_unset'   THEN ARRAY['property_key']
                                                 ELSE ARRAY['property_key', 'value']
                                             END) THEN
        RETURN false;
    END IF;

    IF p_path = 'property_key' THEN
        -- S3, "Property key text: unset first": key text is scrubbable for every event of the
        -- family at or before its latest property_unset, the unset included, whose asserted rows
        -- are none of them live (S6.3). A family never unset has no scrubbable key text.
        v_boundary := _field_scrub_unset_boundary(p_resource, v_handle, p_keys);
        RETURN v_boundary IS NOT NULL
           AND _event_walk_key(p_event) <= _event_walk_key(v_boundary)
           AND NOT EXISTS (SELECT 1 FROM kb_properties p
                            WHERE p.owner_table = 'kb_resources' AND p.owner_id = p_resource
                              AND NOT p.is_folded AND p.asserted_by_event_id = p_event);
    END IF;

    -- S3, "Property values": an event is prior when none of the kb_properties rows it asserted
    -- (asserted_by_event_id) is live. A property_unset asserts none. Read as the projection stands
    -- now, after any clearing events (S6.3).
    IF EXISTS (SELECT 1 FROM kb_properties p
                WHERE p.owner_table = 'kb_resources' AND p.owner_id = p_resource
                  AND NOT p.is_folded AND p.asserted_by_event_id = p_event) THEN
        RETURN false;
    END IF;
    IF v_key = 'facet' THEN
        -- S3, "Facets": every mark folded (above) AND before the latest whole-facet fold, a
        -- property_set or property_unset of `facet`, both of which fold by property_key whatever
        -- the inner key. After it, an inner-key fold could leave a renamed mark live on replay.
        SELECT pe.event_id INTO v_boundary
          FROM _field_scrub_property_events(p_resource, p_keys) pe
         WHERE pe.handle = v_handle AND pe.event_type IN ('property_set', 'property_unset')
         ORDER BY _event_walk_key(pe.event_id) DESC
         LIMIT 1;
        RETURN v_boundary IS NOT NULL AND _event_walk_key(p_event) < _event_walk_key(v_boundary);
    END IF;
    RETURN true;
END;
$function$;

-- 20261018100040 Section 3's body; the held reason compares with the unset boundary by
-- _event_walk_key, and events are listed in walk order.
CREATE OR REPLACE FUNCTION public._resource_field_scrub_derivation(p_resource uuid, p_field text, p_family uuid, p_assume_cleared boolean)
 RETURNS TABLE(event_id uuid, event_type text, new_payload jsonb, paths jsonb, inner_keys jsonb, held jsonb)
 LANGUAGE plpgsql
 STABLE
 SET search_path = public, pg_temp
AS $function$
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
         ORDER BY _event_walk_key(e.id)
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
            IF v_class = 'facet-value' AND v_facets IS NULL THEN
                v_facets := _field_scrub_facet_inner_keys(p_resource);
            END IF;
            FOR v_loc IN SELECT x.at, x.val FROM _erasure_expand(v_ev.payload, v_path) x LOOP
                CONTINUE WHEN jsonb_typeof(v_loc.val) = 'null';
                v_new := _field_scrub_sentinel(v_class, v_ev.id, v_ev.payload, v_loc.val,
                                               v_ev.handle, v_facets);
                -- Done: the location already holds its own sentinel (a NULL sentinel is never done).
                CONTINUE WHEN v_loc.val = v_new;
                IF NOT p_assume_cleared
                   AND NOT coalesce(_field_scrub_event_is_prior(p_resource, p_field, p_family,
                                                                v_ev.id, v_path), false) THEN
                    -- Held back (S5): today's value is kept; key text after the latest unset and
                    -- a facet event after the latest whole-facet fold are unreachable. Key text at
                    -- or before the latest unset is held only for a live row its event asserted,
                    -- which is today's value.
                    v_held := v_held || jsonb_build_object(
                        'path', v_path,
                        'why', CASE
                                   WHEN v_class IN ('title', 'origin-uri') THEN 'current'
                                   WHEN v_path = 'property_key'
                                        AND _event_walk_key(v_ev.id)
                                            <= _event_walk_key(_field_scrub_unset_boundary(p_resource, v_ev.handle))
                                       THEN 'current'
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
                IF v_class = 'facet-value'
                   AND NOT EXISTS (SELECT 1 FROM _facet_marks(v_loc.val) fm WHERE fm.inner_key IS NULL) THEN
                    IF EXISTS (SELECT 1 FROM _facet_marks(v_loc.val) fm
                                WHERE NOT v_facets ? fm.inner_key) THEN
                        RAISE EXCEPTION '_resource_field_scrub_derivation: no inner-key sentinel for a facet of event %',
                            v_ev.id;
                    END IF;
                    SELECT jsonb_object_agg(fm.inner_key, v_facets -> fm.inner_key)
                      INTO v_inner
                      FROM _facet_marks(v_loc.val) fm;
                END IF;
                IF v_new IS NULL THEN
                    -- jsonb_set with a NULL value returns NULL: never let a missing sentinel empty
                    -- a whole payload.
                    RAISE EXCEPTION '_resource_field_scrub_derivation: no sentinel for % of event %',
                        v_path, v_ev.id;
                END IF;
                v_payload := jsonb_set(v_payload, v_loc.at, v_new, false);
                IF NOT v_paths @> to_jsonb(ARRAY[v_path]) THEN
                    v_paths := v_paths || to_jsonb(v_path);
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
$function$;

-- 20261018100040 Section 5's body; the keep-mode guard takes the last event in walk order.
CREATE OR REPLACE FUNCTION public._resource_field_scrub_plan_with(p_resource uuid, p_field text, p_family uuid, p_clear boolean, OUT plan jsonb, OUT redact jsonb)
 RETURNS record
 LANGUAGE plpgsql
 STABLE
 SET search_path = public, pg_temp
AS $function$
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
    -- Clear mode on a field already cleared: the act raises `already cleared` here, before it
    -- appends anything; the plan answers it with nothing to do.
    IF p_clear AND _resource_field_scrub_already_cleared(p_resource, p_field, p_family) THEN
        plan := jsonb_build_object('redacted_fields', '[]'::jsonb, 'kept', '[]'::jsonb,
                                   'unreachable', '[]'::jsonb, 'folded_rows', '[]'::jsonb,
                                   'clears', '[]'::jsonb, 'refusal', 'already_cleared');
        redact := '[]'::jsonb;
        RETURN;
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
        -- in walk order (_event_walk_key). When its value is not the projection's, an append ran
        -- before another whose projection it then landed after (replay.rs, `replay`, the walk
        -- order's residual); refused, never resolved. resource_update (20260709000050) takes no
        -- lock on R before it appends; its first lock is _resource_write_guard inside
        -- _project_resource_updated (20261117000010), after its seq is drawn. It is safe only
        -- under a caller that locks R before the append, as every live caller does: the update
        -- path's lock_resource_for_write (FOR NO KEY UPDATE) and the temper-services write floor
        -- (FOR KEY SHARE). A caller without one can append, wait on a scrub's FOR UPDATE and
        -- project after the scrub's own appends, so live keeps its value and replay keeps the
        -- scrub's.
        SELECT e.payload ->> p_field INTO v_last
          FROM _field_scrub_family_events(p_resource, p_field, NULL) AS f(id)
          JOIN kb_events e ON e.id = f.id
         ORDER BY _event_walk_key(f.id) DESC
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
$function$;

SELECT declare_migration(
    20261201000040,
    'additive',
    'Adds the composite type kb_event_walk_key and the function _event_walk_key(uuid). '
    'CREATE OR REPLACE of _field_scrub_unset_boundary, _field_scrub_event_is_prior, '
    '_resource_field_scrub_derivation and _resource_field_scrub_plan_with with their signatures, '
    'defaults, return types and search_path unchanged: each compares and orders events by replay''s '
    'walk position instead of by id. Every predicate, refusal and sentinel is unchanged; a field '
    'scrub plan can differ only where two events of one resource sort one way by id and the other '
    'by append order. No table, column, constraint, grant or event type changes.'
);
