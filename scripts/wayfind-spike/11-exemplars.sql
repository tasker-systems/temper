\echo '=== three largest regions under the current arm, with member titles ==='
WITH m AS (SELECT id FROM kb_cogmaps WHERE name = 'Temper — self-cognition'),
top AS (SELECT r.id, r.member_count, r.salience
        FROM kb_cogmap_regions r
        WHERE r.home_anchor_table='kb_cogmaps' AND r.home_anchor_id=(SELECT id FROM m)
          AND NOT r.is_folded
        ORDER BY r.member_count DESC, r.id LIMIT 3)
SELECT t.member_count AS sz, round(t.salience::numeric,3) AS sal, left(res.title, 88) AS member_title
FROM top t
JOIN kb_cogmap_region_members mm ON mm.region_id=t.id AND mm.member_table='kb_resources'
JOIN kb_resources res ON res.id=mm.member_id
ORDER BY t.member_count DESC, t.id, res.title;
