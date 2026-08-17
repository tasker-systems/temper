-- Decompose `__temper_ungated_follow_from`'s walk: an `undirected` CTE over `adj` replaces the
-- duplicated `JOIN LATERAL`, and carrying the edge ID instead of five denormalized columns rebuilds
-- `via` by joining `kb_edges` at the end. Three density issues, both candidates measured.
--
-- Task 01a0057e-bbaa-7d93-bd21-21cb9fab5101. Measurement and reasoning are in the task body and PR;
-- this file carries only what an operator needs to audit the shape change.
--
-- Additive: one CREATE OR REPLACE at a byte-identical signature. No DROP, no new parameter.
-- ═══════════════════════════════════════════════════════════════════════════════════════════════

CREATE OR REPLACE FUNCTION __temper_ungated_follow_from(
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
    SELECT e.id AS edge_id, e.source_id, e.target_id, e.weight
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
  -- Undirected adjacency: both orientations once, so `walk` joins plainly and the duplicated LATERAL
  -- disappears. The edge's own source/target are recovered at the end by joining `kb_edges` on the
  -- edge ID carried through the recursion — one column instead of five.
  undirected AS (
    SELECT adj.source_id AS from_node, adj.target_id AS to_node,
           adj.weight, adj.edge_id
      FROM adj
    UNION ALL
    SELECT adj.target_id AS from_node, adj.source_id AS to_node,
           adj.weight, adj.edge_id
      FROM adj
  ),
  walk AS (
    SELECT s.id AS node, 1.0::double precision AS score, 0 AS hop, ARRAY[s.id] AS path,
           s.id AS seed_id,
           NULL::uuid AS e_id
      FROM unnest(p_seed_ids) AS s(id)
     WHERE EXISTS (SELECT 1 FROM admitted a WHERE a.id = s.id)
    UNION ALL
    SELECT u.to_node, w.score * p_gamma * u.weight, w.hop + 1, w.path || u.to_node,
           w.seed_id, u.edge_id
      FROM walk w
      JOIN undirected u ON u.from_node = w.node
     WHERE w.hop < p_depth
       AND NOT u.to_node = ANY(w.path)
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
         -- Every parent, deduplicated at the EDGE grain: the edge ID is a 1:1 handle for the
         -- (source, target, kind, label, polarity) tuple, so DISTINCT on the jsonb object and
         -- DISTINCT on e_id are the same set. Joining `kb_edges` here is a PK lookup.
         (SELECT jsonb_agg(DISTINCT jsonb_build_object(
                   'seed_id',   w.seed_id,
                   'source_id', e.source_id,
                   'target_id', e.target_id,
                   'edge_kind', e.edge_kind::text,
                   'label',     e.label,
                   'polarity',  e.polarity::text))
            FROM walk w
            JOIN kb_edges e ON e.id = w.e_id
           WHERE w.node = r.node AND w.hop > 0)
    FROM ranked r
   ORDER BY r.graph_score DESC, r.node;
$$;

SELECT declare_migration(
    20260817000010,
    'additive',
    'Decomposes __temper_ungated_follow_from''s walk (task 01a0057e-bbaa-7d93-bd21-21cb9fab5101): an undirected CTE over adj replaces the duplicated JOIN LATERAL (two near-identical seven-column SELECTs whose only difference was which endpoint joined w.node), and carrying the edge ID through the recursion instead of five denormalized columns (e_source, e_target, e_kind, e_label, e_polarity) rebuilds via by joining kb_edges on its PK at the end. One CREATE OR REPLACE at a byte-identical signature and return type — no DROP, no new parameter, no DEFAULT. Rows, scores and via contents are unchanged: measured against the incumbent on a 500-node synthetic graph (25 hubs, 574 edges, depth 2, 25 hub seeds, limit 50, 5 alternating runs), all three candidates (A only, B only, A+B) produced identical (resource_id, graph_score, via) with 0 diffs both directions. A+B landed at 9.0ms median vs the incumbent''s 24.8ms — 63% faster (2.7x). B alone barely moved (22.7ms, 8% faster), confirming the LATERAL was the dominant cost on this corpus rather than the correlated via subquery the +20% prod baseline named; on a larger corpus with more via tuples the edge-ID join may matter more, and it costs nothing here. The 15-test regression boundary in crates/temper-substrate/tests/search_graph_expand.rs passes unedited.'
);