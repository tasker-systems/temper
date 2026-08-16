-- The ordering operator: `PropertyOp::Compare` (§12: open keys, closed operators).
-- Spec: 01a00a79-1dd7-7ae2-9d34-b801eb509904. Task: 01a001af-02a9-7c02-9977-195aeecadefb.
-- Additive: two `CREATE OR REPLACE FUNCTION` (ungated bodies), two new `IMMUTABLE` helper
-- functions. No DROP, no signature change, no default change. Gated wrappers delegate unchanged.
--
-- Two extractions (boyscout — this PR): the wire-parse and the operator dispatch were duplicated
-- identically across four bodies. Both are `IMMUTABLE` (pure functions of their arguments, no
-- table reads) so both inline — measured plan-identical to the inline CTE, Index Only Scan on
-- uq_kb_properties_active preserved. The measured blocker (20260808000020:308) was about a
-- `STABLE` body with a correlated sublink; neither helper has one. A future PropertyOp arm is
-- now a one-place edit for both, not two. Differential witness in property_predicate_parity.rs.

-- The wire-parse: one jsonb argument → (property_key, op, vals, direction, bound) rows.
-- IMMUTABLE; raise-guards stop jsonb_array_elements raising on a non-array.
CREATE OR REPLACE FUNCTION _temper_property_predicates_parse(p_props jsonb)
RETURNS TABLE (property_key text, op text, vals jsonb, direction text, bound jsonb)
LANGUAGE sql IMMUTABLE AS $$
    SELECT q->>'key'             AS property_key,
           q->'op'->>'op'        AS op,
           CASE jsonb_typeof(q->'op'->'values')
             WHEN 'array' THEN q->'op'->'values' ELSE '[]'::jsonb END AS vals,
           q->'op'->>'direction' AS direction,
           q->'op'->'value'       AS bound
      FROM jsonb_array_elements(
             CASE jsonb_typeof(p_props)
               WHEN 'array' THEN p_props ELSE '[]'::jsonb END) AS q
$$;

-- The operator dispatch: does this property_value match this predicate?
-- IMMUTABLE (all scalars/jsonb, no table reads); the `contains` EXISTS is over a function,
-- not a table, so it does not block inlining. Fail-closed: unknown op → false, missing
-- value → bound is NULL → jsonb_typeof guard is falsy, missing direction → inner ELSE false.
CREATE OR REPLACE FUNCTION _temper_property_op_match(
    p_op text, p_vals jsonb, p_direction text, p_bound jsonb, p_property_value jsonb
) RETURNS boolean
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE p_op
             WHEN 'has_key'  THEN true
             WHEN 'contains' THEN EXISTS (
               SELECT 1 FROM jsonb_array_elements(p_vals) AS v
                WHERE p_property_value @> v)
             WHEN 'compare' THEN
                   jsonb_typeof(p_property_value) = jsonb_typeof(p_bound)
               AND CASE p_direction
                     WHEN 'gt'  THEN p_property_value >  p_bound
                     WHEN 'gte' THEN p_property_value >= p_bound
                     WHEN 'lt'  THEN p_property_value <  p_bound
                     WHEN 'lte' THEN p_property_value <= p_bound
                     ELSE false END
             ELSE false END
$$;

COMMENT ON FUNCTION _temper_property_predicates_parse(jsonb) IS
    'Wire-parse: one jsonb argument → (property_key, op, vals, direction, bound) rows. IMMUTABLE, inlines. Extracted from four identical copies so a future PropertyOp arm is a one-place parse edit.';
COMMENT ON FUNCTION _temper_property_op_match(text, jsonb, text, jsonb, jsonb) IS
    'Operator dispatch: does property_value match this predicate? IMMUTABLE (no table reads), inlines. compare is type-guarded by jsonb_typeof. Extracted from two identical copies so a future PropertyOp arm is a one-place dispatch edit. See 20260816000010 and property_predicate_parity.rs.';

-- ── `__temper_ungated_find_resources_with`: compare arm + extracted helpers ───────────────────
CREATE OR REPLACE FUNCTION __temper_ungated_find_resources_with(
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
  WITH predicates AS (
    SELECT * FROM _temper_property_predicates_parse(p_properties)
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
       AND (p_tags IS NULL OR (
             SELECT coalesce(array_agg(DISTINCT lower(pe.element #>> '{}')), '{}')
               FROM kb_property_elements pe
              WHERE pe.owner_table = 'kb_resources' AND pe.owner_id = r.id
                AND pe.property_key = 'tags'
           ) @> ARRAY(SELECT lower(x) FROM unnest(p_tags) AS x))
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
       -- Fail-closed stays HERE (not in the parse function): an absent argument and a malformed
       -- one both yield an empty predicates set, so the distinction does not survive the parse.
       AND (p_properties IS NULL OR (
             jsonb_typeof(p_properties) = 'array'
             AND NOT EXISTS (
               SELECT 1 FROM predicates q
                WHERE NOT EXISTS (
                  SELECT 1 FROM kb_resource_properties rp
                   WHERE rp.resource_id = r.id
                     AND rp.property_key = q.property_key
                     AND _temper_property_op_match(q.op, q.vals, q.direction, q.bound, rp.property_value)))))
       AND (p_anchor_id IS NULL OR (
             COALESCE(anchor_readable_by_profile(p_anchor_reader, p_anchor_table, p_anchor_id),
                      false)
             AND h.anchor_table = p_anchor_table
             AND h.anchor_id = p_anchor_id));
$$;

-- ── `__temper_ungated_follow_from`: compare arm + extracted helpers ────────────────────────────
CREATE OR REPLACE FUNCTION __temper_ungated_follow_from(
    p_visible_ids      uuid[],
    p_seed_ids         uuid[],
    p_depth            int,
    p_gamma            double precision,
    p_edge_kinds       text[],
    p_labels           text[],
    p_bound_ids        uuid[],
    p_limit            int,
    p_edge_properties  jsonb)
RETURNS TABLE (resource_id uuid, graph_score real, via jsonb)
LANGUAGE sql STABLE AS $$
  WITH RECURSIVE
  predicates AS (
    SELECT * FROM _temper_property_predicates_parse(p_edge_properties)
  ),
  admitted AS (
    SELECT v.id
      FROM unnest(p_visible_ids) AS v(id)
     WHERE p_bound_ids IS NULL
        OR EXISTS (SELECT 1 FROM unnest(p_bound_ids) AS b(id) WHERE b.id = v.id)
  ),
  adj AS (
    SELECT e.source_id, e.target_id, e.weight,
           e.edge_kind::text AS edge_kind, e.label, e.polarity::text AS polarity
      FROM kb_edges e
      JOIN admitted sa ON sa.id = e.source_id
      JOIN admitted ta ON ta.id = e.target_id
     WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
       AND NOT e.is_folded
       AND (p_edge_kinds IS NULL OR array_length(p_edge_kinds, 1) IS NULL
            OR e.edge_kind::text = ANY(p_edge_kinds))
       AND (p_labels IS NULL OR array_length(p_labels, 1) IS NULL
            OR e.label = ANY(p_labels))
       AND (p_edge_properties IS NULL OR (
             jsonb_typeof(p_edge_properties) = 'array'
             AND NOT EXISTS (
               SELECT 1 FROM predicates q
                WHERE NOT EXISTS (
                  SELECT 1 FROM kb_edge_properties ep
                   WHERE ep.edge_id = e.id
                     AND ep.property_key = q.property_key
                     AND _temper_property_op_match(q.op, q.vals, q.direction, q.bound, ep.property_value)))))
  ),
  walk AS (
    SELECT s.id AS node, 1.0::double precision AS score, 0 AS hop, ARRAY[s.id] AS path,
           s.id AS seed_id,
           NULL::uuid AS e_source, NULL::uuid AS e_target,
           NULL::text AS e_kind, NULL::text AS e_label, NULL::text AS e_polarity
      FROM unnest(p_seed_ids) AS s(id)
     WHERE EXISTS (SELECT 1 FROM admitted a WHERE a.id = s.id)
    UNION ALL
    SELECT nb.node, w.score * p_gamma * nb.weight, w.hop + 1, w.path || nb.node,
           w.seed_id, nb.source_id, nb.target_id, nb.edge_kind, nb.label, nb.polarity
      FROM walk w
      JOIN LATERAL (
        SELECT adj.target_id AS node, adj.weight, adj.source_id, adj.target_id,
               adj.edge_kind, adj.label, adj.polarity
          FROM adj WHERE adj.source_id = w.node
        UNION ALL
        SELECT adj.source_id AS node, adj.weight, adj.source_id, adj.target_id,
               adj.edge_kind, adj.label, adj.polarity
          FROM adj WHERE adj.target_id = w.node
      ) nb ON true
     WHERE w.hop < p_depth
       AND NOT nb.node = ANY(w.path)
  ),
  ranked AS (
    SELECT w.node, MAX(w.score)::real AS graph_score
      FROM walk w
     WHERE w.hop > 0
     GROUP BY w.node
     ORDER BY MAX(w.score) DESC, w.node
     LIMIT p_limit
  )
  SELECT r.node, r.graph_score,
         (SELECT jsonb_agg(DISTINCT jsonb_build_object(
                   'seed_id',   w.seed_id,
                   'source_id', w.e_source,
                   'target_id', w.e_target,
                   'edge_kind', w.e_kind,
                   'label',     w.e_label,
                   'polarity',  w.e_polarity))
            FROM walk w
           WHERE w.node = r.node AND w.hop > 0)
    FROM ranked r
   ORDER BY r.graph_score DESC, r.node;
$$;

SELECT declare_migration(
    20260816000010,
    'additive',
    'Widens the closed PropertyOp set with Compare { direction, value } (OrdOp: gt/gte/lt/lte). Three rulings (spec 01a00a79): one variant, no Between; jsonb native ordering type-guarded by jsonb_typeof; type inferred from the caller''s bound. Also extracts two IMMUTABLE helpers — the wire-parse and the operator dispatch — from four duplicated copies into one each (both inline, measured plan-identical, Index Only Scan preserved). probe_count for compare is 1; the 256 probe ceiling is unchanged. No DROP, no signature change. Differential witness in property_predicate_parity.rs gains compare cases including both type-guard directions.'
);