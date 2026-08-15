-- `p_facets` fails closed. It failed open, and its comment said otherwise.
--
-- Task 01a00510-e583-78b1-bdc5-f08f4ca483a8. Body change only: CREATE OR REPLACE at a byte-identical
-- signature, no DROP, no parameter moves.
--
-- Two defects in one slot, opposite in direction, both measured on prod `[2026-08-15]`:
--
--   1. A NON-ARRAY argument normalized to '[]', and `NOT EXISTS` over zero elements is TRUE — so a
--      malformed filter returned an UNNARROWED page. Over five faceted resources: 5 unfiltered,
--      5 malformed, 0 for a well-formed non-match.
--   2. A well-formed array whose element lacks `key` RAISED `argument 1: key must not be null`,
--      because `jsonb_build_object` refuses a null key. Data-dependent: it fires only for a caller
--      who can see a resource carrying a live `facet`, so it is a 500 for some principals and a
--      silent 0 for others.
--
-- Unreachable through the compiler, whose `selection_narrowings_for` always builds an array of
-- `{key,value}`; this function is directly callable, which is why the guards exist at all.
--
-- `p_tags` is NOT changed: measured, its row-side normalization already yields '{}' for every
-- non-array/non-string stored value — matches nothing, raises nothing.
--
-- 20260814000010's inline comment still says "Fail closed: a non-array narrows to nothing". It is
-- applied and cannot be corrected; the correction lives here and in this migration's ledger entry.
--
-- Additive: one CREATE OR REPLACE at a byte-identical signature. No DROP.

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
       AND (p_tags IS NULL OR (
             SELECT coalesce(array_agg(DISTINCT lower(t)), '{}')
               FROM kb_properties tp
               CROSS JOIN LATERAL jsonb_array_elements_text(
                 CASE jsonb_typeof(tp.property_value)
                   WHEN 'array'  THEN tp.property_value
                   WHEN 'string' THEN to_jsonb(
                     regexp_split_to_array(trim(tp.property_value #>> '{}'), '\s+'))
                   ELSE '[]'::jsonb
                 END) AS t
              WHERE tp.owner_table = 'kb_resources' AND tp.owner_id = r.id
                AND tp.property_key = 'tags' AND NOT tp.is_folded
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
                  SELECT 1 FROM kb_properties fp
                   WHERE fp.owner_table = 'kb_resources' AND fp.owner_id = r.id
                     AND fp.property_key = 'facet' AND NOT fp.is_folded
                     AND f->>'key' IS NOT NULL
                     AND fp.property_value @> jsonb_build_object(f->>'key', f->>'value')))))
       AND (p_anchor_id IS NULL OR (
             COALESCE(anchor_readable_by_profile(p_anchor_reader, p_anchor_table, p_anchor_id),
                      false)
             AND h.anchor_table = p_anchor_table
             AND h.anchor_id = p_anchor_id));
$$;

SELECT declare_migration(
    20260815000020,
    'additive',
    'Fixes __temper_ungated_find_resources_with''s p_facets slot, which failed OPEN and described itself as failing closed (task 01a00510-e583-78b1-bdc5-f08f4ca483a8). Two defects in one slot, opposite in direction, both measured on prod: a NON-ARRAY argument normalized to ''[]'' and NOT EXISTS over zero elements is TRUE, so a malformed filter returned an unnarrowed page (over five faceted resources: 5 unfiltered, 5 malformed, 0 for a well-formed non-match); and a well-formed array whose element lacks ''key'' RAISED ''argument 1: key must not be null'' from jsonb_build_object, data-dependently — a 500 for a caller who can see a faceted resource and a silent 0 for one who cannot. Three guards now, with distinct jobs: jsonb_typeof fails closed, the CASE prevents the raise from jsonb_array_elements whatever order the planner takes the conjuncts in, and the null-key test prevents the raise from jsonb_build_object. p_tags is deliberately unchanged: measured, its row-side normalization already yields ''{}'' for every non-array/non-string stored value. Body change only, CREATE OR REPLACE at a byte-identical signature, no DROP. 20260814000010''s inline comment remains stale by necessity — it is applied and cannot be corrected.'
);
