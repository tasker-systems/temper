\echo '=== FORMATION INPUT: edges homed in the map (substrate::load predicate), by edge_kind ==='
WITH m AS (SELECT id FROM kb_cogmaps WHERE name = 'Temper — self-cognition')
SELECT e.edge_kind::text AS kind, count(*) AS n, count(DISTINCT e.label) AS distinct_labels,
       round(sum(e.weight)::numeric,1) AS wsum
FROM kb_edges e, m
WHERE e.home_anchor_table='kb_cogmaps' AND e.home_anchor_id=m.id
  AND e.source_table='kb_resources' AND e.target_table='kb_resources' AND NOT e.is_folded
GROUP BY 1 ORDER BY n DESC;

\echo '=== the contradicts label within that formation input ==='
WITH m AS (SELECT id FROM kb_cogmaps WHERE name = 'Temper — self-cognition')
SELECT e.edge_kind::text AS kind, count(*) AS n
FROM kb_edges e, m
WHERE e.home_anchor_table='kb_cogmaps' AND e.home_anchor_id=m.id
  AND e.source_table='kb_resources' AND e.target_table='kb_resources' AND NOT e.is_folded
  AND e.label = 'contradicts'
GROUP BY 1;
