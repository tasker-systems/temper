\set prin '019d4add-f49d-7c43-a87d-dda470e5dd9c'
\echo '=== is the Q4 exemplar a member of any LIVE region? ==='
SELECT m.region_id, r.is_folded, r.home_anchor_table,
       (SELECT count(*) FROM kb_cogmap_region_members mm
         WHERE mm.region_id = r.id AND mm.member_table='kb_resources') AS region_members
FROM kb_cogmap_region_members m
JOIN kb_cogmap_regions r ON r.id = m.region_id
WHERE m.member_id = '019fb87c-8226-72e3-8dbe-02479677dd4d';

\echo ''
\echo '=== ORPHAN CHECK: visible resources that belong to NO live region, by class ==='
WITH vis AS (
  SELECT rv.resource_id FROM resources_visible_to(:'prin'::uuid) rv
  JOIN kb_resources r ON r.id = rv.resource_id AND r.is_active AND r.ingest_state='complete'),
cls AS (
  SELECT v.resource_id, h.anchor_table,
         CASE WHEN h.anchor_table='kb_cogmaps' THEN 'distilled' ELSE 'raw' END AS class,
         (SELECT count(*) FROM kb_cogmap_regions g
           WHERE g.home_anchor_table=h.anchor_table AND g.home_anchor_id=h.anchor_id
             AND NOT g.is_folded) AS anchor_live_regions
  FROM vis v JOIN kb_resource_homes h ON h.resource_id = v.resource_id)
SELECT class,
       CASE WHEN anchor_live_regions = 0 THEN 'anchor region-less (cold-start reachable)'
            ELSE 'anchor has regions' END AS anchor_state,
       count(*) AS resources,
       count(*) FILTER (WHERE EXISTS (
         SELECT 1 FROM kb_cogmap_region_members m
         JOIN kb_cogmap_regions g ON g.id = m.region_id AND NOT g.is_folded
         WHERE m.member_id = cls.resource_id AND m.member_table='kb_resources')) AS in_a_live_region,
       count(*) FILTER (WHERE anchor_live_regions > 0 AND NOT EXISTS (
         SELECT 1 FROM kb_cogmap_region_members m
         JOIN kb_cogmap_regions g ON g.id = m.region_id AND NOT g.is_folded
         WHERE m.member_id = cls.resource_id AND m.member_table='kb_resources')) AS STRUCTURALLY_UNREACHABLE
FROM cls GROUP BY 1,2 ORDER BY 1,2;
