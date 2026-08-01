\set prin '019d4add-f49d-7c43-a87d-dda470e5dd9c'
\echo '=== who put each member in its region: declared edge, shared facet, or similarity alone? ==='
WITH m AS (SELECT id FROM kb_cogmaps WHERE name = 'Temper — self-cognition'),
reg AS (SELECT r.id, r.member_count FROM kb_cogmap_regions r
        WHERE r.home_anchor_table='kb_cogmaps'
          AND r.home_anchor_id=(SELECT id FROM m)
          AND NOT r.is_folded AND r.member_count > 1),
mem AS (SELECT mm.region_id, mm.member_id FROM kb_cogmap_region_members mm
        JOIN reg ON reg.id = mm.region_id WHERE mm.member_table='kb_resources'),
by_edge AS (
  SELECT DISTINCT a.region_id, a.member_id
  FROM mem a
  JOIN mem b ON b.region_id = a.region_id AND b.member_id <> a.member_id
  JOIN kb_edges e
    ON e.home_anchor_table='kb_cogmaps'
   AND e.home_anchor_id=(SELECT id FROM m)
   AND NOT e.is_folded
   AND ((e.source_id=a.member_id AND e.target_id=b.member_id)
     OR (e.source_id=b.member_id AND e.target_id=a.member_id))),
by_facet AS (
  SELECT DISTINCT a.region_id, a.member_id
  FROM mem a
  JOIN mem b ON b.region_id=a.region_id AND b.member_id <> a.member_id
  JOIN kb_properties pa ON pa.owner_table='kb_resources' AND pa.owner_id=a.member_id
   AND pa.property_key='facet' AND NOT pa.is_folded
  JOIN kb_properties pb ON pb.owner_table='kb_resources' AND pb.owner_id=b.member_id
   AND pb.property_key='facet' AND NOT pb.is_folded
   AND pb.property_value = pa.property_value)
SELECT count(*) AS members_in_multi_regions,
       count(*) FILTER (WHERE e.member_id IS NOT NULL) AS by_declared_edge,
       count(*) FILTER (WHERE e.member_id IS NULL AND f.member_id IS NOT NULL) AS by_facet_only,
       count(*) FILTER (WHERE e.member_id IS NULL AND f.member_id IS NULL) AS similarity_only,
       count(DISTINCT mem.region_id) FILTER (WHERE e.member_id IS NULL AND f.member_id IS NULL)
         AS regions_with_similarity_only_members
FROM mem
LEFT JOIN by_edge  e ON e.region_id=mem.region_id AND e.member_id=mem.member_id
LEFT JOIN by_facet f ON f.region_id=mem.region_id AND f.member_id=mem.member_id;
