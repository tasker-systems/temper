-- `EdgeFilter`'s third axis: edge property predicates, applied inside the walk so they constrain a
-- HOP and not the returned set.
--
-- Design: docs/superpowers/specs/2026-08-14-property-conventions-and-predicate-container-design.md §6.5
-- Task 01a000c2-033c-7451-8b13-b7aa7469d217 · PR #679, which carry the reasoning and the measurements.
--
-- Three things a later edit can break silently:
--   1. NO `DEFAULT` on the widened signatures. A default on the added parameter makes the 8-arity
--      incumbent ambiguous and every existing call to it an error `[measured — 2026-08-15, pg18]`.
--   2. Both incumbent arities delegate; one body of the walk exists (20260808000030).
--   3. Failing closed needs the guard in `adj` AND the ones in `predicates` — see both.
--
-- Additive: one CREATE VIEW, two CREATE FUNCTION, two CREATE OR REPLACE at byte-identical
-- signatures. No DROP.

-- A predicate reads the view, never `kb_properties` raw (design §6.3). A VIEW and not a predicate
-- function because a `LANGUAGE sql STABLE` body containing a sublink does not inline
-- `[measured — 20260808000020:308]`. `property_value` is exposed unchanged so the element-exploding
-- relation the resource half needs can be built over it.
CREATE VIEW kb_edge_properties AS
    SELECT p.owner_id AS edge_id, p.property_key, p.property_value, p.weight
      FROM kb_properties p
     WHERE p.owner_table = 'kb_edges' AND NOT p.is_folded;

COMMENT ON VIEW kb_edge_properties IS
    'Live properties owned by an edge: kb_properties where owner_table = ''kb_edges'' and NOT is_folded. A predicate reads this rather than the table, so it can neither lose a convention nor wrongly inherit one. property_value is exposed unchanged; the element-exploding relation is a separate, later object.';

CREATE FUNCTION __temper_ungated_follow_from(
    p_visible_ids     uuid[],
    p_seed_ids        uuid[],
    p_depth           int,
    p_gamma           double precision,
    p_edge_kinds      text[],
    p_labels          text[],
    p_bound_ids       uuid[],
    p_limit           int,
    p_edge_properties jsonb)
RETURNS TABLE (resource_id uuid, graph_score real, via jsonb)
LANGUAGE sql STABLE AS $$
  WITH RECURSIVE
  -- `PropertyOp` is internally tagged inside a field named `op`, hence `q->'op'->>'op'`. The
  -- `CASE`s are raise-guards: `jsonb_array_elements` errors on a non-array, and a `WHERE` beside a
  -- lateral SRF runs after the expansion that already raised.
  predicates AS (
    SELECT q->>'key'      AS property_key,
           q->'op'->>'op' AS op,
           CASE jsonb_typeof(q->'op'->'values')
             WHEN 'array' THEN q->'op'->'values' ELSE '[]'::jsonb END AS vals
      FROM jsonb_array_elements(
             CASE jsonb_typeof(p_edge_properties)
               WHEN 'array' THEN p_edge_properties ELSE '[]'::jsonb END) AS q
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
       -- Fail-closed lives here and cannot move into `predicates`: that CTE is empty for an absent
       -- argument and a malformed one alike, so the distinction does not survive the parse.
       AND (p_edge_properties IS NULL OR (
             jsonb_typeof(p_edge_properties) = 'array'
             AND NOT EXISTS (                       -- no listed predicate FAILS to match
               SELECT 1 FROM predicates q
                WHERE NOT EXISTS (
                  SELECT 1 FROM kb_edge_properties ep
                   WHERE ep.edge_id = e.id
                     AND ep.property_key = q.property_key
                     AND CASE q.op
                           WHEN 'has_key'  THEN true
                           WHEN 'contains' THEN EXISTS (   -- OR within one predicate's values
                             SELECT 1 FROM jsonb_array_elements(q.vals) AS v
                              WHERE ep.property_value @> v)
                           ELSE false                      -- an operator the closed set lacks
                         END))))
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

CREATE OR REPLACE FUNCTION __temper_ungated_follow_from(
    p_visible_ids uuid[],
    p_seed_ids    uuid[],
    p_depth       int,
    p_gamma       double precision,
    p_edge_kinds  text[] DEFAULT NULL,
    p_labels      text[] DEFAULT NULL,
    p_bound_ids   uuid[] DEFAULT NULL,
    p_limit       int    DEFAULT NULL)
RETURNS TABLE (resource_id uuid, graph_score real, via jsonb)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id, c.graph_score, c.via
      FROM __temper_ungated_follow_from(
             p_visible_ids, p_seed_ids, p_depth, p_gamma,
             p_edge_kinds, p_labels, p_bound_ids, p_limit, NULL::jsonb) c;
$$;

CREATE FUNCTION query_follow_from(
    p_principal       uuid,
    p_seed_ids        uuid[],
    p_depth           int,
    p_gamma           double precision,
    p_edge_kinds      text[],
    p_labels          text[],
    p_bound_ids       uuid[],
    p_limit           int,
    p_edge_properties jsonb)
RETURNS TABLE (resource_id uuid, graph_score real, via jsonb)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id, c.graph_score, c.via
      FROM __temper_ungated_follow_from(
             ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v),
             p_seed_ids, p_depth, p_gamma,
             p_edge_kinds, p_labels, p_bound_ids, p_limit, p_edge_properties) c;
$$;

CREATE OR REPLACE FUNCTION query_follow_from(
    p_principal   uuid,
    p_seed_ids    uuid[],
    p_depth       int,
    p_gamma       double precision,
    p_edge_kinds  text[] DEFAULT NULL,
    p_labels      text[] DEFAULT NULL,
    p_bound_ids   uuid[] DEFAULT NULL,
    p_limit       int    DEFAULT NULL)
RETURNS TABLE (resource_id uuid, graph_score real, via jsonb)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id, c.graph_score, c.via
      FROM query_follow_from(
             p_principal, p_seed_ids, p_depth, p_gamma,
             p_edge_kinds, p_labels, p_bound_ids, p_limit, NULL::jsonb) c;
$$;

COMMENT ON FUNCTION __temper_ungated_follow_from(
        uuid[], uuid[], int, double precision, text[], text[], uuid[], int, jsonb) IS
    'The walk, widened with p_edge_properties: EdgeFilter''s third axis, applied inside `adj` because it constrains which edge may be TRAVERSED. Shape is the serialization of Rust''s Vec<PropertyPredicate>. AND across the list, OR within one predicate''s values; NULL narrows nothing, a malformed argument narrows to zero. Carries NO DEFAULT on any parameter, and that is load-bearing: a default makes the 8-arity incumbent ambiguous and every existing call to it an error.';

COMMENT ON FUNCTION query_follow_from(
        uuid, uuid[], int, double precision, text[], text[], uuid[], int, jsonb) IS
    'Gated wrapper over the widened __temper_ungated_follow_from: computes resources_visible_to(p_principal) once and passes it down. Carries p_edge_properties so a direct caller can express every axis the core can. No defaults, for the reason recorded on the core.';

SELECT declare_migration(
    20260815000010,
    'additive',
    'Gives EdgeFilter a third axis: property predicates over an edge''s own kb_properties rows, applied inside the walk because an edge predicate constrains a HOP and has no set-shaped substitute. One CREATE VIEW (kb_edge_properties -- a predicate reads the view, never the table), two CREATE FUNCTION (widened core and gated wrapper), two CREATE OR REPLACE at byte-identical signatures re-pointing both incumbent arities to delegate. No DROP. The widened signatures carry NO DEFAULT on any parameter: measured on pg18, a default on the added parameter makes the incumbent 8-arity call ambiguous and therefore breaks search_graph_expand, the compiler''s emitted call and four test call sites at run time. Task 01a000c2-033c-7451-8b13-b7aa7469d217, PR #679, design docs/superpowers/specs/2026-08-14-property-conventions-and-predicate-container-design.md.'
);
