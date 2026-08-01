\echo '=== the global telos-default lens (formation fields) ==='
SELECT name, w_express, w_contains, w_leads_to, w_near, w_prop, w_cos, knn_k, cos_floor, resolution
FROM kb_cogmap_lenses WHERE home_anchor_table IS NULL ORDER BY name;

\echo '=== live region shape per region-bearing anchor ==='
SELECT r.home_anchor_table AS kind,
       coalesce(c.name, x.name) AS anchor,
       count(*) AS live_regions,
       count(*) FILTER (WHERE r.member_count <= 1) AS singletons,
       max(r.member_count) AS largest,
       round(avg(r.member_count)::numeric, 2) AS mean_members
FROM kb_cogmap_regions r
LEFT JOIN kb_cogmaps  c ON r.home_anchor_table='kb_cogmaps'  AND c.id = r.home_anchor_id
LEFT JOIN kb_contexts x ON r.home_anchor_table='kb_contexts' AND x.id = r.home_anchor_id
WHERE NOT r.is_folded
GROUP BY 1,2 ORDER BY live_regions DESC;

\echo '=== the map: live nodes homed, and lens id used by the selector ==='
WITH m AS (SELECT id FROM kb_cogmaps WHERE name = 'Temper — self-cognition')
SELECT (SELECT count(*) FROM kb_resource_homes h, m
        WHERE h.anchor_table='kb_cogmaps' AND h.anchor_id=m.id) AS homed_rows,
       (SELECT id FROM kb_cogmap_lenses WHERE name='telos-default' AND home_anchor_table IS NULL) AS telos_default_lens_id,
       (SELECT id FROM m) AS cogmap_id;
