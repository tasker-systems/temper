-- `ResourceFilter`'s open-key property predicates: the resource-side sibling of `20260815000010`'s
-- edge slot, reading `property_value` WHOLE. 67 of the 70 live keys become narrowable.
--
-- Design: docs/superpowers/specs/2026-08-14-property-conventions-and-predicate-container-design.md
-- §6.3 and §9. Task 01a00502-a774-7001-b5b2-0ce462158f1c carries the grain ruling and its
-- measurements.
--
-- Four things a later edit can break silently:
--   1. NO `DEFAULT` on the widened signatures. A default on the added parameter makes the incumbent
--      arity ambiguous and every existing call to it an error `[measured — 2026-08-15, pg18]`.
--   2. This predicate reads `kb_resource_properties` (value WHOLE), NOT `kb_property_elements`.
--      An array-shaped probe matches under the whole grain and matches NOTHING under the element
--      one — 1,228 array-shaped rows on prod. Both operators read the one relation.
--   3. `p_properties` is LAST and deliberately not beside `p_facets`, which is also `jsonb`. Two
--      adjacent `jsonb` narrowings is the one transposition this signature could not catch.
--   4. Failing closed needs the `jsonb_typeof` test AND both `CASE` guards AND the `ELSE false`.
--
-- Additive: one CREATE VIEW, two CREATE FUNCTION, two CREATE OR REPLACE at byte-identical
-- signatures. No DROP.

-- ── The owner-scoped relation, the sibling of kb_edge_properties ────────────────────────────────
-- Homes `owner_table` and `is_folded`. `kb_resource_doc_type`'s comment records a hand-written copy
-- dropping the first of those once already, which `20260806000020` had to restore.
CREATE VIEW kb_resource_properties AS
    SELECT p.owner_id AS resource_id, p.property_key, p.property_value, p.weight
      FROM kb_properties p
     WHERE p.owner_table = 'kb_resources' AND NOT p.is_folded;

COMMENT ON VIEW kb_resource_properties IS
    'Live properties owned by a resource: kb_properties where owner_table = ''kb_resources'' and NOT is_folded, with property_value exposed WHOLE. The relation BOTH members of the closed operator set read — contains against the whole value, has_key as a row-existence test. Deliberately not kb_property_elements: an array-shaped containment probe matches here and matches nothing there, and an empty array is a row here and no rows there. kb_property_elements serves the tags and facet predicates, whose semantics are AND-containment over elements.';

-- ── The selection core, widened ─────────────────────────────────────────────────────────────────
CREATE FUNCTION __temper_ungated_find_resources_with(
    p_visible_ids    uuid[],
    p_doc_types      text[],
    p_tags           text[],
    p_facets         jsonb,
    p_stage          text,
    p_status         text,
    p_owner_profile  uuid,
    p_owner_handle   text,
    p_title_contains text,
    p_anchor_table   varchar,
    p_anchor_id      uuid,
    p_anchor_reader  uuid,
    p_properties     jsonb)
RETURNS TABLE (resource_id uuid)
LANGUAGE sql STABLE AS $$
  -- The wire shape parsed once into columns, exactly as the edge sibling parses it. `PropertyOp` is
  -- internally tagged inside a field named `op`, hence `q->'op'->>'op'`.
  WITH predicates AS (
    SELECT q->>'key'      AS property_key,
           q->'op'->>'op' AS op,
           CASE jsonb_typeof(q->'op'->'values')
             WHEN 'array' THEN q->'op'->'values' ELSE '[]'::jsonb END AS vals
      FROM jsonb_array_elements(
             CASE jsonb_typeof(p_properties)
               WHEN 'array' THEN p_properties ELSE '[]'::jsonb END) AS q
  )
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
       -- `coalesce` makes it the EMPTY ARRAY — NOT because it is NULL. Drop the `coalesce` and
       -- every tag filter becomes NULL and matches nothing.
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
       -- Open key space, closed operator set. AND across the list, OR within one predicate's
       -- values. NULL narrows nothing; a malformed argument narrows to zero, and the fail-closed
       -- test must stay HERE rather than in `predicates`, which is empty for an absent argument and
       -- a malformed one alike — the distinction does not survive the parse.
       --
       -- `has_key` needs no value test: the row's existence in the view IS the answer, which is why
       -- it can be asked of a `[]`-valued key that the element relation cannot see at all.
       AND (p_properties IS NULL OR (
             jsonb_typeof(p_properties) = 'array'
             AND NOT EXISTS (                       -- no listed predicate FAILS to match
               SELECT 1 FROM predicates q
                WHERE NOT EXISTS (
                  SELECT 1 FROM kb_resource_properties rp
                   WHERE rp.resource_id = r.id
                     AND rp.property_key = q.property_key
                     AND CASE q.op
                           WHEN 'has_key'  THEN true
                           WHEN 'contains' THEN EXISTS (   -- OR within one predicate's values
                             SELECT 1 FROM jsonb_array_elements(q.vals) AS v
                              WHERE rp.property_value @> v)
                           ELSE false                      -- an operator the closed set lacks
                         END))))
       AND (p_anchor_id IS NULL OR (
             COALESCE(anchor_readable_by_profile(p_anchor_reader, p_anchor_table, p_anchor_id),
                      false)
             AND h.anchor_table = p_anchor_table
             AND h.anchor_id = p_anchor_id));
$$;

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
    SELECT c.resource_id
      FROM __temper_ungated_find_resources_with(
             p_visible_ids, p_doc_types, p_tags, p_facets, p_stage, p_status,
             p_owner_profile, p_owner_handle, p_title_contains,
             p_anchor_table, p_anchor_id, p_anchor_reader, NULL::jsonb) c;
$$;

-- ── The gated wrapper, widened to match ─────────────────────────────────────────────────────────
CREATE FUNCTION query_find_resources_with(
    p_principal      uuid,
    p_doc_types      text[],
    p_tags           text[],
    p_facets         jsonb,
    p_stage          text,
    p_status         text,
    p_owner_profile  uuid,
    p_owner_handle   text,
    p_title_contains text,
    p_anchor_table   varchar,
    p_anchor_id      uuid,
    p_properties     jsonb)
RETURNS TABLE (resource_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id
      FROM __temper_ungated_find_resources_with(
             ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v),
             p_doc_types, p_tags, p_facets, p_stage, p_status,
             p_owner_profile, p_owner_handle, p_title_contains,
             p_anchor_table, p_anchor_id, p_principal, p_properties) c;
$$;

CREATE OR REPLACE FUNCTION query_find_resources_with(
    p_principal      uuid,
    p_doc_types      text[]  DEFAULT NULL,
    p_tags           text[]  DEFAULT NULL,
    p_facets         jsonb   DEFAULT NULL,
    p_stage          text    DEFAULT NULL,
    p_status         text    DEFAULT NULL,
    p_owner_profile  uuid    DEFAULT NULL,
    p_owner_handle   text    DEFAULT NULL,
    p_title_contains text    DEFAULT NULL,
    p_anchor_table   varchar DEFAULT NULL,
    p_anchor_id      uuid    DEFAULT NULL)
RETURNS TABLE (resource_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id
      FROM query_find_resources_with(
             p_principal, p_doc_types, p_tags, p_facets, p_stage, p_status,
             p_owner_profile, p_owner_handle, p_title_contains,
             p_anchor_table, p_anchor_id, NULL::jsonb) c;
$$;

COMMENT ON FUNCTION __temper_ungated_find_resources_with(
        uuid[], text[], text[], jsonb, text, text, uuid, text, text, varchar, uuid, uuid, jsonb) IS
    'The selection core, widened with p_properties: ResourceFilter''s open-key slot, over an OPEN key space and a CLOSED operator set. Shape is the serialization of Rust''s Vec<PropertyPredicate>, the same type EdgeFilter.properties carries, and contains means the same thing in both containers — property_value @> v, the value WHOLE. Reads kb_resource_properties and NOT kb_property_elements: an array-shaped probe matches the first and nothing in the second, and has_key needs a relation in which a []-valued key is still a row. AND across the list, OR within one predicate''s values; NULL narrows nothing, a malformed argument narrows to zero. Carries NO DEFAULT on any parameter, and that is load-bearing: a default makes the 12-arity incumbent ambiguous and every existing call to it an error. p_properties sits LAST rather than beside p_facets because both are jsonb, and two adjacent jsonb narrowings is the one transposition a positional call could make without a type error.';

COMMENT ON FUNCTION query_find_resources_with(
        uuid, text[], text[], jsonb, text, text, uuid, text, text, varchar, uuid, jsonb) IS
    'Gated wrapper over the widened __temper_ungated_find_resources_with: computes resources_visible_to(p_principal) once and passes the principal down as p_anchor_reader. Carries p_properties so a direct caller can express every narrowing the core can. No defaults, for the reason recorded on the core.';

SELECT declare_migration(
    20260815000040,
    'additive',
    'Gives ResourceFilter open-key property predicates, making 67 of the 70 live property keys narrowable for the first time. One CREATE VIEW (kb_resource_properties -- live resource-owned kb_properties with property_value exposed WHOLE, the owner-scoped sibling of kb_edge_properties), two CREATE FUNCTION (widened selection core and gated wrapper, both 1 arity above their incumbents), two CREATE OR REPLACE at byte-identical signatures re-pointing both incumbents to delegate. No DROP. The widened signatures carry NO DEFAULT on any parameter: measured on pg18, a default on the added parameter makes the incumbent arity ambiguous and breaks every existing call at run time. The grain is RULED and is not the element view: contains reads property_value whole, so it means the same thing here as in EdgeFilter.properties, an array-shaped probe keeps matching the 1,228 array-shaped rows on prod that the element grain would silently drop, and has_key can be asked of the 11 []-valued rows the element relation cannot see. kb_property_elements is unchanged and continues to serve tags and facet, whose semantics are AND-containment over elements. p_properties is the LAST parameter rather than adjacent to p_facets because both are jsonb. Ruling, measurements and residue live in task 01a00502-a774-7001-b5b2-0ce462158f1c and docs/superpowers/specs/2026-08-14-property-conventions-and-predicate-container-design.md sections 6.3 and 9 — deliberately not here, because an applied migration cannot have its prose corrected.'
);
