-- The field scrub's consolidated review fixes (field-grain scrub, after build Task 8's reviews).
--
-- Spec: temper-artifacts specs/2026-10-09-field-grain-scrub-design.md S2, S3, S4, S5, S6.3, S7, S8,
-- S9. Plan: plans/2026-10-09-field-grain-scrub.md (Global constraints). Every function replaced
-- here is CREATE OR REPLACE with its signature, defaults, return type and search_path unchanged,
-- starting from its live body (20261018100010, 20261018100020, 20261018100030); no overload is
-- added beside any of them.
--
--   1. Exact scrub sentinels (S7, S6.3). The row verifier admits a renamed key or a facet inner key
--      by shape; a forged or buggy statement could therefore split one family across two sentinels,
--      merge two families into one, or rename a family onto another family's sentinel-shaped key
--      text. The derivation's sentinel becomes ONE function, _field_scrub_sentinel, fed by
--      _field_scrub_key_sentinel (scrubbed-key-<handle>) and _field_scrub_facet_inner_keys (the
--      inner-key mapping the derivation computed inline, now extracted so both callers share it).
--      kb_events_redaction_in_trail requires every location a scrub record changed to hold exactly
--      that sentinel, computed over the ledger as it stood before the statement (the OLD key texts
--      and OLD values of the statement's own rows), and that a record renaming any key of a family
--      renames every prior key of that family in the same statement, so a second statement under
--      a second record cannot split the family either. The derivation's "already done" test becomes
--      the same equality, so a family whose key text merely has a sentinel's shape (a key typed as
--      scrubbed-key-<F>) is renamed to its own scrubbed-key-<G> when G is scrubbed, which unblocks
--      F's scrub: the collision check reads the key texts as they stand, and after G's scrub none is
--      scrubbed-key-<F>.
--   2. Key text is prior only when its event asserts no live row (S6.3, "none of its asserted
--      kb_properties rows is live"). An event whose id sorts at or before the latest unset but whose
--      projection landed after it (an inversion across transactions) keeps its live row, and its key
--      text is held as `current`, not renamed under a live row that keeps the real key.
--      _field_scrub_unset_boundary is the "latest unset" both _field_scrub_event_is_prior and the
--      derivation's held reason read.
--   3. The search vector (S8 amended). Renaming a `tags`, `keywords` or `descriptor` family removes
--      the rebuilds its events triggered live, so replay could leave a different vector. The act
--      rebuilds R's vector at its end, and the replay arm of resource_scrubbed rebuilds the
--      subject's at the event's position: both compute it over the same state.
--   4. Clear mode is not repeatable (S2, S4). A clear of a field already cleared (the placeholder
--      title or origin URI, a doc_type already `scrubbed`, a family with no live row) is answered
--      `resource_field_scrub_execute: already cleared` and records nothing; the plan's refusal is
--      `already_cleared`. One predicate, _resource_field_scrub_already_cleared, for both.
--   5. The sweep's ledger arm checks the location (S9). A scrub-authorised row closes a ledger
--      finding only while the event's value at the row's path is a sentinel _erasure_sentinel_admits
--      admits under `scrub`: a record and rows written without the rewrite close nothing.
--   6. The family listing is empty for a charter or an erased resource, as the survey's is (S1, S5).
--   7. A comment on kb_event_field_redactions.authority records the cost of a mislabelled row.

-- ---------------------------------------------------------------------------
-- Section 1. The scrub's sentinels, one definition (S7).
-- ---------------------------------------------------------------------------

-- The placeholder clear mode sets for the title or origin URI (S2).
CREATE FUNCTION _field_scrub_placeholder(p_resource uuid)
RETURNS text
LANGUAGE sql IMMUTABLE
SET search_path = public, pg_temp
AS $$
    SELECT 'scrubbed-' || p_resource::text;
$$;

-- A renamed key's sentinel: scrubbed-key-<the family's handle> (S7). NULL for a NULL handle.
CREATE FUNCTION _field_scrub_key_sentinel(p_handle uuid)
RETURNS text
LANGUAGE sql IMMUTABLE
SET search_path = public, pg_temp
AS $$
    SELECT 'scrubbed-key-' || p_handle::text;
$$;

-- The facet inner-key mapping of R (S7): each inner key any of R's facet events names, to
-- scrubbed-facet-<id of the first of R's facet events naming it>-<its position in that event's
-- value as stored>. The body the derivation computed inline (20261018100020 Section 3), extracted
-- so the statement verifier applies the same mapping. p_keys and p_values supply the ORIGINAL key
-- text and value by event id, for events a statement has already rewritten (as p_keys does for
-- _field_scrub_property_events); the act's plan passes neither.
CREATE FUNCTION _field_scrub_facet_inner_keys(p_resource uuid, p_keys jsonb DEFAULT NULL,
                                              p_values jsonb DEFAULT NULL)
RETURNS jsonb
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT coalesce(jsonb_object_agg(x.inner_key,
                        'scrubbed-facet-' || x.id::text || '-' || x.ord::text),
                    '{}'::jsonb)
      FROM (SELECT DISTINCT ON (fm.inner_key) fm.inner_key, pe.event_id AS id, fm.ord
              FROM _field_scrub_property_events(p_resource, p_keys) pe
              JOIN kb_events e ON e.id = pe.event_id
             CROSS JOIN LATERAL _facet_marks(coalesce(p_values -> e.id::text, e.payload -> 'value'))
                        WITH ORDINALITY AS fm(inner_key, inner_value, ord)
             WHERE pe.key_text = 'facet'
               AND pe.event_type IN ('property_set', 'property_asserted')
               AND fm.inner_key IS NOT NULL
             ORDER BY fm.inner_key, pe.event_id, fm.ord) x;
$$;

-- The sentinel the field scrub writes at one location (S7), from the location's class, its event,
-- the event's payload and the location's ORIGINAL value: a renamed key is scrubbed-key-<p_handle>;
-- a facet value keeps one mark per original, each inner key renamed through p_facets
-- (_field_scrub_facet_inner_keys) and each inner value "erased", or erased:<event id> for a facet
-- value with no inner key; every other class takes erasure's exact sentinel. NULL when no sentinel
-- exists (a facet mark p_facets does not name, a class with no exact sentinel). The ONE definition:
-- the derivation writes it and kb_events_redaction_in_trail requires it.
CREATE FUNCTION _field_scrub_sentinel(p_class text, p_event uuid, p_payload jsonb, p_old jsonb,
                                      p_handle uuid, p_facets jsonb)
RETURNS jsonb
LANGUAGE sql IMMUTABLE
SET search_path = public, pg_temp
AS $$
    SELECT CASE
        WHEN p_class = 'property-key'
            THEN to_jsonb(_field_scrub_key_sentinel(p_handle))
        WHEN p_class = 'facet-value' THEN
            CASE
                WHEN EXISTS (SELECT 1 FROM _facet_marks(p_old) fm WHERE fm.inner_key IS NULL)
                    THEN to_jsonb('erased:' || p_event::text)
                WHEN EXISTS (SELECT 1 FROM _facet_marks(p_old) fm
                              WHERE NOT coalesce(p_facets ? fm.inner_key, false))
                    THEN NULL
                ELSE (SELECT jsonb_object_agg(p_facets ->> fm.inner_key, '"erased"'::jsonb)
                        FROM _facet_marks(p_old) fm)
            END
        ELSE _erasure_sentinel_exact(p_class, p_event, p_payload)
    END;
$$;

COMMENT ON FUNCTION _field_scrub_sentinel(text, uuid, jsonb, jsonb, uuid, jsonb) IS
$c$The field scrub's sentinel at one location (field-grain scrub spec S7): scrubbed-key-<handle> for a
renamed key, the facet mark mapping of _field_scrub_facet_inner_keys for a facet value (erased:<event>
for a facet value with no inner key), erasure's exact sentinel for the title, origin URI, property
value and doc type. The ONE definition: _resource_field_scrub_derivation writes it and
kb_events_redaction_in_trail admits nothing else.$c$;

-- ---------------------------------------------------------------------------
-- Section 2. Key text is prior only without a live row (S3, S6.3).
-- ---------------------------------------------------------------------------

-- The family's latest property_unset (S3, "unset first"): the boundary at or before which its key
-- text is scrubbable. NULL for a family never unset.
CREATE FUNCTION _field_scrub_unset_boundary(p_resource uuid, p_handle uuid, p_keys jsonb DEFAULT NULL)
RETURNS uuid
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT pe.event_id
      FROM _field_scrub_property_events(p_resource, p_keys) pe
     WHERE pe.handle = p_handle AND pe.event_type = 'property_unset'
     ORDER BY pe.event_id DESC
     LIMIT 1;
$$;

-- 20261018100010 Section 5's body, with one change: key text at or before the latest unset is prior
-- only when its event asserts no live row (S6.3's "none of its asserted kb_properties rows is live"
-- applies to every path, the key's included). A property_asserted whose id sorts before the unset
-- but whose projection landed after it holds a live row under the real key; renaming its event's key
-- would leave that row live under a key its event no longer names, and replay would fold it at the
-- renamed unset.
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
        -- the field in walk order (ORDER BY id); every earlier event carrying it is prior.
        RETURN (p_path IS NULL OR p_path = p_kind)
           AND EXISTS (SELECT 1 FROM _field_scrub_family_events(p_resource, p_kind, NULL) f
                        WHERE f = p_event)
           AND EXISTS (SELECT 1 FROM _field_scrub_family_events(p_resource, p_kind, NULL) f
                        WHERE f > p_event);
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
        RETURN v_boundary IS NOT NULL AND p_event <= v_boundary
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
         ORDER BY pe.event_id DESC
         LIMIT 1;
        RETURN v_boundary IS NOT NULL AND p_event < v_boundary;
    END IF;
    RETURN true;
END;
$function$;

COMMENT ON FUNCTION _field_scrub_event_is_prior(uuid, text, uuid, uuid, text, jsonb) IS
$c$Whether one event's value at one path is prior under field-grain scrub spec S3, for a scrub of
p_kind (title, origin_uri, property with its family handle, or properties): the title or origin URI
before the last event carrying it; a property value none of whose asserted rows is live; key text at
or before the family's latest property_unset (_field_scrub_unset_boundary) whose event asserts no
live row; a facet value fully folded and before the latest whole-facet fold. The ONE definition: the
act's plan and kb_events_redaction_in_trail both call it. p_keys supplies original key text for
events a statement has already renamed.$c$;

-- ---------------------------------------------------------------------------
-- Section 3. The derivation writes the one sentinel and holds an inverted key as current.
-- ---------------------------------------------------------------------------
-- 20261018100020 Section 3's body, with three changes:
--   * each location's sentinel is _field_scrub_sentinel's (Section 1), the facet mapping read from
--     _field_scrub_facet_inner_keys instead of computed inline;
--   * a location is done only when it already holds THIS location's sentinel. Before, any value
--     _erasure_sentinel_admits admitted under `scrub` was done, so a family whose key text had a
--     sentinel's shape (scrubbed-key-<F>, typed by the owner) was never renamed by its own scrub
--     and blocked F's for good;
--   * key text at or before the latest unset whose event asserts a live row is held `current`
--     (Section 2); key text after the latest unset stays `after_latest_unset`.
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
                                        AND v_ev.id <= _field_scrub_unset_boundary(p_resource, v_ev.handle)
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

-- ---------------------------------------------------------------------------
-- Section 4. The statement verifier requires the exact sentinel (S6.3, S7).
-- ---------------------------------------------------------------------------
-- 20261018100010 Section 7's body. The trail loop is unchanged, and so is the scrub loop's family-
-- and-prior check. The scrub loop adds one check: every location a scrub record's row names on an
-- event this statement changed either is unchanged or holds _field_scrub_sentinel's value for it,
-- computed over the ledger as it stood before the statement: the handle from the OLD key texts
-- (v_keys), the facet mapping from the OLD key texts and values (v_keys, v_values). The row
-- verifier admits a renamed key or facet inner key by shape, because its n cannot see the family;
-- this is where the family is seen. So one statement cannot split a family across two key
-- sentinels, merge two families into one, rename a family onto another's sentinel-shaped key text,
-- or map two inner keys onto one. A third check holds a family's key rename whole (below), so two
-- statements cannot split it either. Both new checks read only while the subject is not erased.
CREATE OR REPLACE FUNCTION public.kb_events_redaction_in_trail()
 RETURNS trigger
 LANGUAGE plpgsql
 SET search_path = public, pg_temp
AS $function$
DECLARE
    v_subject uuid;
    v_record  uuid;
    v_payload jsonb;
    v_keys    jsonb;
    v_values  jsonb;
    v_scrubbed uuid;
    v_facets  jsonb;
BEGIN
    FOR v_subject IN
        SELECT DISTINCT (e.payload ->> 'subject_id')::uuid
          FROM new_rows n
          JOIN old_rows o ON o.id = n.id
          JOIN kb_event_field_redactions r ON r.event_id = n.id
          JOIN kb_events e ON e.id = r.redacted_by
         WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
           AND e.created = now()
    LOOP
        IF EXISTS (
            WITH trail AS MATERIALIZED (
                SELECT ts.event_id FROM _resource_erasure_trail_scope(v_subject) ts
            )
            SELECT 1
              FROM new_rows n
              JOIN old_rows o ON o.id = n.id
              JOIN kb_event_field_redactions r ON r.event_id = n.id
              JOIN kb_events e ON e.id = r.redacted_by
             WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
               AND e.created = now()
               AND (e.payload ->> 'subject_id')::uuid = v_subject
               AND NOT EXISTS (SELECT 1 FROM trail t WHERE t.event_id = n.id)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
    END LOOP;

    FOR v_record IN
        SELECT DISTINCT r.redacted_by
          FROM new_rows n
          JOIN old_rows o ON o.id = n.id
          JOIN kb_event_field_redactions r ON r.event_id = n.id AND r.authority = 'scrub'
          JOIN kb_events e ON e.id = r.redacted_by
         WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
           AND e.created = now()
    LOOP
        IF v_keys IS NULL THEN
            SELECT coalesce(jsonb_object_agg(o.id::text, o.payload -> 'property_key'), '{}'::jsonb)
              INTO v_keys
              FROM old_rows o
             WHERE o.payload ? 'property_key';
            SELECT coalesce(jsonb_object_agg(o.id::text, o.payload -> 'value'), '{}'::jsonb)
              INTO v_values
              FROM old_rows o
             WHERE o.payload ? 'value';
        END IF;
        SELECT e.payload INTO v_payload FROM kb_events e WHERE e.id = v_record;
        v_scrubbed := (v_payload ->> 'subject_id')::uuid;
        IF EXISTS (
            SELECT 1
              FROM new_rows n
              JOIN old_rows o ON o.id = n.id
              JOIN kb_event_field_redactions r
                ON r.event_id = n.id AND r.authority = 'scrub' AND r.redacted_by = v_record
             WHERE (n.payload, n.metadata) IS DISTINCT FROM (o.payload, o.metadata)
               AND NOT coalesce(_field_scrub_event_is_prior(
                       v_scrubbed,
                       v_payload #>> '{field,kind}',
                       (v_payload #>> '{field,family}')::uuid,
                       n.id, r.path, v_keys), false)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
        -- The exact sentinel (S7). Metadata paths need no arm: none is a scrubbed field's own path,
        -- so the prior check above has already refused any scrub row naming one. Read only while
        -- the subject is not erased: on an erased subject a scrub row authorises nothing
        -- (kb_events_append_only), so a change there stands on an erasure row, under erasure's
        -- sentinels; that is the state when one transaction scrubs R and then erases it.
        v_facets := _field_scrub_facet_inner_keys(v_scrubbed, v_keys, v_values);
        IF EXISTS (SELECT 1 FROM kb_resources s WHERE s.id = v_scrubbed AND s.erased_at IS NULL)
           AND EXISTS (
            SELECT 1
              FROM new_rows n
              JOIN old_rows o ON o.id = n.id
              JOIN kb_event_types t ON t.id = o.event_type_id
              JOIN kb_event_field_redactions r
                ON r.event_id = n.id AND r.authority = 'scrub' AND r.redacted_by = v_record
              LEFT JOIN _field_scrub_property_events(v_scrubbed, v_keys) pe ON pe.event_id = n.id
             CROSS JOIN LATERAL _erasure_expand(o.payload, r.path) x
             WHERE n.payload IS DISTINCT FROM o.payload
               AND r.path NOT LIKE 'metadata.%'
               AND (n.payload #> x.at) IS DISTINCT FROM x.val
               AND (n.payload #> x.at) IS DISTINCT FROM _field_scrub_sentinel(
                       _erasure_path_class(t.name, r.path, o.payload), o.id, o.payload, x.val,
                       pe.handle, v_facets)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
        -- A family's key text is renamed whole, in one statement. Exactness alone holds one
        -- statement to one sentinel per family, but the handle is the lowest id STILL carrying the
        -- text: a statement renaming only part of a family's prior key text would leave the rest a
        -- family with a new handle, which a later statement could rename to a second sentinel. So
        -- when this record renames any key of a family, every event of that family whose key text
        -- is prior must carry a renamed key once the statement is done. The act renames them all
        -- in its one UPDATE (see resource_field_scrub_execute's header).
        IF EXISTS (SELECT 1 FROM kb_resources s WHERE s.id = v_scrubbed AND s.erased_at IS NULL)
           AND EXISTS (
            SELECT 1
              FROM _field_scrub_property_events(v_scrubbed, v_keys) pe
              JOIN kb_events cur ON cur.id = pe.event_id
             WHERE pe.handle IN (
                       SELECT pr.handle
                         FROM new_rows n
                         JOIN old_rows o ON o.id = n.id
                         JOIN kb_event_field_redactions r
                           ON r.event_id = n.id AND r.authority = 'scrub'
                          AND r.redacted_by = v_record AND r.path = 'property_key'
                         JOIN _field_scrub_property_events(v_scrubbed, v_keys) pr
                           ON pr.event_id = n.id
                        WHERE n.payload -> 'property_key' IS DISTINCT FROM o.payload -> 'property_key')
               AND cur.payload ->> 'property_key' IS NOT DISTINCT FROM pe.key_text
               AND coalesce(_field_scrub_event_is_prior(
                       v_scrubbed,
                       v_payload #>> '{field,kind}',
                       (v_payload #>> '{field,family}')::uuid,
                       pe.event_id, 'property_key', v_keys), false)) THEN
            RAISE EXCEPTION 'event ledger is append-only';
        END IF;
    END LOOP;
    RETURN NULL;
END;
$function$;

-- ---------------------------------------------------------------------------
-- Section 5. Clear mode is not repeatable (S2, S4).
-- ---------------------------------------------------------------------------
-- Whether a clear of p_field (and p_family) would find today's value already cleared: the title or
-- origin URI already the placeholder; a doc_type family with no live type row but the placeholder
-- type `scrubbed`; any other family with no live row of its key on R. Under `properties`, which is
-- never cleared, false. The ONE definition: the act raises `already cleared` on it before it appends
-- anything, and the plan answers `already_cleared` on it, so the survey and the act agree.
CREATE FUNCTION _resource_field_scrub_already_cleared(p_resource uuid, p_field text, p_family uuid)
RETURNS boolean
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT CASE
        WHEN p_field = 'title' THEN
            EXISTS (SELECT 1 FROM kb_resources r
                     WHERE r.id = p_resource AND r.title = _field_scrub_placeholder(p_resource))
        WHEN p_field = 'origin_uri' THEN
            EXISTS (SELECT 1 FROM kb_resources r
                     WHERE r.id = p_resource AND r.origin_uri = _field_scrub_placeholder(p_resource))
        WHEN p_field = 'property' THEN
            NOT EXISTS (SELECT 1
                          FROM _field_scrub_property_events(p_resource) pe
                          JOIN kb_properties p
                            ON p.owner_table = 'kb_resources' AND p.owner_id = p_resource
                           AND NOT p.is_folded AND p.property_key = pe.key_text
                         WHERE pe.event_id = p_family
                           AND (pe.key_text <> 'doc_type'
                                OR p.property_value IS DISTINCT FROM '"scrubbed"'::jsonb))
        ELSE false
    END;
$$;

-- 20261018100020 Section 4's body, with one change: under p_clear, a field already cleared answers
-- the refusal `already_cleared` with an empty plan, after the charter, the erased resource and the
-- request's faults, in the act's order. Like `nothing_prior`, it is a 400 that records nothing.
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
$function$;

COMMENT ON FUNCTION resource_field_scrub_plan(uuid, text, uuid, boolean) IS
$c$The field scrub's ONE computation (field-grain scrub spec S5, erasure D10): which events and paths
the act redacts, which it keeps and which it cannot reach and why, which folded kb_properties rows it
rewrites, the clearing events it appends under p_clear, and the refusal verdict: charter_resource,
already_erased, sentinel_collision and projection_disagrees (recorded by the act), nothing_prior
(keep mode) and already_cleared (clear mode on a field already cleared), both answered 400. The act
consumes it (through _resource_field_scrub_plan_with) and the survey renders it. Event ids, row ids
and paths only. A malformed or foreign request raises 'resource_field_scrub_plan: <fault>'.$c$;

-- ---------------------------------------------------------------------------
-- Section 6. The act: already cleared, the placeholder by its one definition, the search vector.
-- ---------------------------------------------------------------------------
-- 20261018100020 Section 6's body, with three changes:
--   * after the request's faults, a clear of a field already cleared raises
--     `resource_field_scrub_execute: already cleared` before anything is appended (Section 5), so a
--     retried clear records nothing;
--   * the placeholder title or origin URI is _field_scrub_placeholder's;
--   * R's search vector is rebuilt last, after the projection rewrite. Renaming a `tags`,
--     `keywords` or `descriptor` family removes the rebuilds its events triggered live: replay of
--     the renamed events rebuilds nothing, so a vector a later non-rebuilding event left stale live
--     (a property_asserted of `keywords`) would differ from replay's. The replay arm of
--     resource_scrubbed rebuilds the subject's vector at the event's position, over the state the
--     live act ends in, so both compute the same vector. A rebuild is a pure function of the
--     resource's title, live chunks and live rows, so a rebuild of a family the act did not rename
--     is a no-op.
CREATE OR REPLACE FUNCTION public.resource_field_scrub_execute(p_resource uuid, p_field text, p_family uuid, p_clear boolean, p_operator uuid, p_emitter uuid, p_request_ref uuid)
 RETURNS jsonb
 LANGUAGE plpgsql
 SET search_path = public, pg_temp
 SET lock_timeout TO '0'
AS $function$
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
    -- A clear of a field already cleared appends nothing and records nothing: it is a 400, like
    -- keep mode with nothing prior. Read under R's row lock, so a concurrent clear has landed.
    IF p_clear AND _resource_field_scrub_already_cleared(p_resource, p_field, p_family) THEN
        RAISE EXCEPTION 'resource_field_scrub_execute: already cleared';
    END IF;

    -- ── Clear mode (S2): the clearing event, through its ordinary projector. ─────────────────
    IF p_clear THEN
        IF p_field IN ('title', 'origin_uri') THEN
            PERFORM resource_update(
                jsonb_build_object('resource_id', p_resource,
                                   p_field, _field_scrub_placeholder(p_resource)),
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

    -- ── The search vector, over the state the act leaves (S8 amended; see above). ───────────
    PERFORM _rebuild_resource_search_vector(p_resource);

    RETURN jsonb_build_object(
        'event_id',        v_ev,
        'redacted_fields', v_fields,
        'cleared',         p_clear);
END;
$function$;

COMMENT ON FUNCTION resource_field_scrub_execute(uuid, text, uuid, boolean, uuid, uuid, uuid) IS
$c$The field scrub (field-grain scrub spec S4): every prior value of R's title, origin URI, one property
family (by handle) or every family, redacted from the ledger and the projection under one
resource_scrubbed, with today's value kept, or first cleared when p_clear; R's search vector is
rebuilt last. Serialized by the act queue and R's row lock under lock_timeout 0 (erasure D13).
Raises, prefixed 'resource_field_scrub_execute: ', for a charter, an erased resource, a malformed or
foreign request, a clear of a field already cleared ('already cleared'), a sentinel collision, a
projection that disagrees, and nothing prior in keep mode. Returns the event id, redacted_fields and
cleared.$c$;

-- ---------------------------------------------------------------------------
-- Section 7. The family listing is empty for a charter or an erased resource (S1, S5).
-- ---------------------------------------------------------------------------
-- 20261018100020 Section 2's body, gated: the survey answers a charter or an erased resource with
-- its refusal and no listing, and the listing door now answers the same resource with no rows,
-- instead of the structure of a ledger the act will never touch. resource_field_scrub_family_flags
-- reads this listing, so its rows follow.
CREATE OR REPLACE FUNCTION public.resource_field_scrub_families(p_resource uuid)
 RETURNS TABLE(field text, family uuid, events integer, live boolean, unset boolean, first_seen timestamp with time zone, first_by uuid, value_type text)
 LANGUAGE sql
 STABLE
 SET search_path = public, pg_temp
AS $function$
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
     WHERE EXISTS (SELECT 1 FROM kb_resources r
                    WHERE r.id = p_resource AND r.erased_at IS NULL)
       AND NOT EXISTS (SELECT 1 FROM kb_cogmaps c WHERE c.telos_resource_id = p_resource)
     GROUP BY f.field, f.family, f.ord
     ORDER BY f.ord, f.family;
$function$;

COMMENT ON FUNCTION resource_field_scrub_families(uuid) IS
$c$The field scrub's family listing (field-grain scrub spec S1): the title, the origin URI and each
resource-owned property family of the resource, a family by its handle (the id of the first event
still carrying its key text; resource_created for the doc_type family). Event count, live, unset,
first seen and by whom (profile id), and the latest value's JSON type. No key text and no value. No
rows for a charter or an erased resource, which the act refuses before it reads a family.$c$;

-- ---------------------------------------------------------------------------
-- Section 8. The sweep's ledger arm checks the location (S9).
-- ---------------------------------------------------------------------------
-- 20261018100030 Section 1's body, with two changes:
--   * a row of authority `scrub` closes a ledger finding only while the event's value at the row's
--     path is a sentinel _erasure_sentinel_admits admits under `scrub`, at every location of the
--     path (the scrub's paths are scalar: one location each). The verifier checks the sentinel at
--     the rewrite; closure, which reads long after it, checks that the rewrite happened. A record and
--     its rows written without the rewrite (projected by hand, or by a transaction that rolled the
--     UPDATE back and kept the rows) close nothing. A scrubbed path an erasure later re-renamed (a
--     scrubbed-key-<handle> numbered erased-key-<n>) fails this check and closes under the
--     erasure's own row, which lists it;
--   * the placeholder title or origin URI is _field_scrub_placeholder's.
-- Erasure rows are unchanged. Their paths include `[*]` paths where the row means some location
-- under the path was rewritten and others were kept by rule (a local source, a kept label), whose
-- per-location rule reads the payload the act was changing; and an erasure row counts only on an
-- erased subject, which only the act sets. Holding them to a per-location check here is a change to
-- erasure's closure that no finding of this review asked for and no suite of this branch exercises.
CREATE OR REPLACE FUNCTION sensitivity.place_closure(p_surface text, p_target uuid, p_hash text, p_path text)
 RETURNS text
 LANGUAGE sql
 STABLE
 SET search_path = public, pg_temp
AS $function$
    SELECT CASE
        WHEN p_surface IN ('kb_events.payload', 'kb_events.metadata') THEN
            CASE WHEN EXISTS (SELECT 1 FROM kb_event_field_redactions r
                                JOIN kb_events e ON e.id = r.redacted_by
                                JOIN kb_event_types t ON t.id = e.event_type_id
                               WHERE r.event_id = p_target
                                 AND sensitivity.ledger_path_covers(r.path, p_surface, p_path)
                                 AND t.name = CASE r.authority
                                                  WHEN 'erasure' THEN 'resource_erased'
                                                  WHEN 'scrub'   THEN 'resource_scrubbed'
                                              END
                                 AND e.payload -> 'redacted_fields' @> jsonb_build_array(jsonb_build_object(
                                         'event', p_target::text, 'paths', jsonb_build_array(r.path)))
                                 AND EXISTS (SELECT 1 FROM kb_resources s
                                              WHERE s.id = (e.payload ->> 'subject_id')::uuid
                                                AND (r.authority = 'scrub' OR s.erased_at IS NOT NULL))
                                 AND (r.authority = 'erasure'
                                      OR coalesce((
                                          SELECT bool_and(_erasure_sentinel_admits(
                                                     _erasure_path_class(tt.name, r.path, ev.payload),
                                                     ev.id, ev.payload, x.at, x.val, x.val, 'scrub'))
                                            FROM kb_events ev
                                            JOIN kb_event_types tt ON tt.id = ev.event_type_id
                                           CROSS JOIN LATERAL _erasure_expand(
                                                     CASE WHEN r.path LIKE 'metadata.%'
                                                          THEN ev.metadata ELSE ev.payload END,
                                                     CASE WHEN r.path LIKE 'metadata.%'
                                                          THEN substr(r.path, 10) ELSE r.path END) x
                                           WHERE ev.id = p_target), false)))
                 THEN 'sentinel' END
        WHEN p_surface NOT IN ('kb_block_content.content', 'kb_chunk_content.content', 'kb_chunks.header_path',
                               'kb_resources.title', 'kb_resources.origin_uri', 'kb_properties.property_key',
                               'kb_edges.label', 'kb_citation_audits.reason', 'kb_remote_sources.uri',
                               'kb_properties.property_value') THEN NULL
        WHEN x.present IS NULL THEN 'row_missing'
        -- The resource erasure act's D4 sentinels (20260929040730, steps 9a, 9b, 9d).
        WHEN (p_surface = 'kb_resources.title'           AND x.unit = 'erased-' || p_target::text)
          OR (p_surface = 'kb_resources.origin_uri'      AND x.unit = 'erased:' || p_target::text)
          OR (p_surface = 'kb_properties.property_key'   AND x.unit ~ '^erased-key-[0-9]+$')
          OR (p_surface = 'kb_properties.property_value' AND x.unit = '"erased"') THEN 'sentinel'
        -- The field scrub's sentinels (field-grain scrub S2, S7; 20261018100020).
        WHEN (p_surface IN ('kb_resources.title', 'kb_resources.origin_uri')
              AND x.unit = _field_scrub_placeholder(p_target))
          OR (p_surface = 'kb_properties.property_key'
              AND x.unit ~ '^scrubbed-key-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$')
          OR (p_surface = 'kb_properties.property_value'
              AND x.unit ~ '^\{"scrubbed-facet-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}-[1-9][0-9]*": "erased"\}$')
          OR (p_surface = 'kb_properties.property_value'
              AND EXISTS (SELECT 1 FROM kb_properties p
                           WHERE p.id = p_target
                             AND p.property_value IN (to_jsonb('erased:' || p.asserted_by_event_id::text),
                                                      jsonb_build_array('erased:' || p.asserted_by_event_id::text))))
            THEN 'sentinel'
        WHEN coalesce(x.unit, '') = ''
          OR (p_surface = 'kb_properties.property_value' AND x.unit IN ('{}', '[]', '""', 'null')) THEN 'content_empty'
        WHEN o.content_hash <> p_hash THEN 'changed'
    END
      FROM (SELECT 1) one
      LEFT JOIN LATERAL (
          SELECT true, bc.content FROM kb_block_content bc
           WHERE p_surface = 'kb_block_content.content' AND bc.block_revision_id = p_target
          UNION ALL
          SELECT true, cc.content FROM kb_chunk_content cc
           WHERE p_surface = 'kb_chunk_content.content' AND cc.chunk_id = p_target
          UNION ALL
          SELECT true, c.header_path FROM kb_chunks c
           WHERE p_surface = 'kb_chunks.header_path' AND c.id = p_target
          UNION ALL
          SELECT true, r.title FROM kb_resources r
           WHERE p_surface = 'kb_resources.title' AND r.id = p_target
          UNION ALL
          SELECT true, r.origin_uri FROM kb_resources r
           WHERE p_surface = 'kb_resources.origin_uri' AND r.id = p_target
          UNION ALL
          SELECT true, p.property_key FROM kb_properties p
           WHERE p_surface = 'kb_properties.property_key' AND p.id = p_target
          UNION ALL
          SELECT true, p.property_value::text FROM kb_properties p
           WHERE p_surface = 'kb_properties.property_value' AND p.id = p_target
          UNION ALL
          SELECT true, e.label FROM kb_edges e
           WHERE p_surface = 'kb_edges.label' AND e.id = p_target
          UNION ALL
          SELECT true, a.reason FROM kb_citation_audits a
           WHERE p_surface = 'kb_citation_audits.reason' AND a.id = p_target
          UNION ALL
          SELECT true, s.uri FROM kb_remote_sources s
           WHERE p_surface = 'kb_remote_sources.uri' AND s.id = p_target
      ) x (present, unit) ON true
      LEFT JOIN sensitivity.place_observations o ON o.surface = p_surface AND o.target_id = p_target;
$function$;

COMMENT ON FUNCTION sensitivity.place_closure(text, uuid, text, text) IS
'How a finding''s place closes, from what it holds now (sweep D2 as amended): row_missing, sentinel, content_empty, or on a mutable surface changed by the sweep''s own later observation; NULL while it is open, and on a surface this function does not know. A ledger place (kb_events.payload, kb_events.metadata) closes as sentinel when a kb_event_field_redactions row on its event covers p_path and the row''s authorising event, of its authority''s type, lists that event and path: a resource_erased for an erased subject, or a resource_scrubbed for a resource, erased since or not, whose rewrite the event still holds (its value at the path is a sentinel the scrub admits; field-grain scrub S9); never otherwise. A projection place closes as sentinel on the erasure''s D4 sentinels and on the field scrub''s (S7): scrubbed-<resource id>, scrubbed-key-<uuid>, erased:<the row''s asserting event> and a scrubbed-facet mark. On an erasure row''s `[*]` path the row means some location under it was rewritten, not every one. p_path is read only on the ledger.';

-- ---------------------------------------------------------------------------
-- Section 9. The authority column records what a mislabelled row costs.
-- ---------------------------------------------------------------------------
COMMENT ON COLUMN kb_event_field_redactions.authority IS
$c$The authorising event's type: `erasure` (resource_erased, whose subject must be erased) or `scrub`
(resource_scrubbed, whose subject must not be). Written by that type's projector. A structural
token, never a value. The DEFAULT 'erasure' is the backfill and the reading of a binary that names
no authority; a writer must never lean on it for a scrub. A scrub row mislabelled `erasure` would
occupy the key (event_id, path, 'erasure'): a later erasure of that path would collide on
kb_event_field_redactions_pkey, which resource_erasure_service reads as a concurrent completion
(erasure, cut 2 PR 3, ruling 9) and retries, colliding again every time.$c$;

SELECT declare_migration(
    20261018100040,
    'additive',
    'The field scrub''s review fixes (field-grain scrub spec S2-S9). New functions: _field_scrub_placeholder, _field_scrub_key_sentinel, _field_scrub_facet_inner_keys (the facet inner-key mapping the derivation computed inline), _field_scrub_sentinel (the one per-location sentinel), _field_scrub_unset_boundary and _resource_field_scrub_already_cleared. CREATE OR REPLACE, each with its signature, defaults, return type and search_path unchanged: _field_scrub_event_is_prior (key text is prior only when its event asserts no live row), _resource_field_scrub_derivation (writes _field_scrub_sentinel; a location is done only at its own exact sentinel; an inverted live key is held current), kb_events_redaction_in_trail (on a subject not erased, a scrub record''s changed locations must hold the exact sentinel, computed over the pre-statement ledger, and a record renaming any key of a family renames all of its prior key text in that statement; the trail check and the family-and-prior check are unchanged), _resource_field_scrub_plan_with (refusal already_cleared under p_clear), resource_field_scrub_execute (raises ''already cleared'' before appending anything; rebuilds R''s search vector last), resource_field_scrub_families (no rows for a charter or an erased resource) and sensitivity.place_closure (a scrub row closes a ledger finding only while the event still holds a scrub sentinel at its path; erasure rows unchanged). Comments on resource_field_scrub_plan, resource_field_scrub_execute, resource_field_scrub_families, _field_scrub_event_is_prior, sensitivity.place_closure and kb_event_field_redactions.authority. Every change narrows what is admitted, closed or listed, or adds a 400 the deployed binary answers as an internal error until it is upgraded (a clear of a field already cleared, which the deployed binary would have re-applied). No table, column, constraint, grant or event type changes.'
);
