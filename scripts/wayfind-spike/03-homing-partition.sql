-- P2b — doc_type lives in kb_properties (as unified_search's corpus CTE reads it), not a table.
\set prin '019d4add-f49d-7c43-a87d-dda470e5dd9c'

\echo '=== B. doc_type composition by homing class ==='
WITH vis AS (
  SELECT rv.resource_id FROM resources_visible_to(:'prin'::uuid) rv
  JOIN kb_resources r ON r.id = rv.resource_id AND r.is_active AND r.ingest_state = 'complete'),
cls AS (
  SELECT v.resource_id,
         CASE WHEN bool_or(hh.anchor_table = 'kb_cogmaps') THEN 'distilled (cogmap)'
              ELSE 'raw (context)' END AS class
  FROM vis v LEFT JOIN kb_resource_homes hh ON hh.resource_id = v.resource_id
  GROUP BY v.resource_id)
SELECT cls.class,
       COALESCE((SELECT p.property_value #>> '{}' FROM kb_properties p
                  WHERE p.owner_table='kb_resources' AND p.owner_id=cls.resource_id
                    AND p.property_key='doc_type' AND NOT p.is_folded LIMIT 1), '(none)') AS doc_type,
       count(*) AS n
FROM cls GROUP BY 1,2 HAVING count(*) >= 5 ORDER BY 1, 3 DESC;

\echo ''
\echo '=== C. chunk count + indexed length by homing class — the Stage-2 length lever ==='
WITH vis AS (
  SELECT rv.resource_id FROM resources_visible_to(:'prin'::uuid) rv
  JOIN kb_resources r ON r.id = rv.resource_id AND r.is_active AND r.ingest_state = 'complete'),
cls AS (
  SELECT v.resource_id,
         CASE WHEN bool_or(hh.anchor_table = 'kb_cogmaps') THEN 'distilled (cogmap)'
              ELSE 'raw (context)' END AS class
  FROM vis v LEFT JOIN kb_resource_homes hh ON hh.resource_id = v.resource_id
  GROUP BY v.resource_id)
SELECT cls.class,
       count(*) AS resources,
       round(avg(x.nc)::numeric, 2)                        AS mean_chunks,
       percentile_disc(0.5) WITHIN GROUP (ORDER BY x.nc)   AS p50_chunks,
       percentile_disc(0.9) WITHIN GROUP (ORDER BY x.nc)   AS p90_chunks,
       max(x.nc)                                           AS max_chunks,
       round(avg(x.len)::numeric, 0)                       AS mean_chars,
       percentile_disc(0.5) WITHIN GROUP (ORDER BY x.len)  AS p50_chars
FROM cls
CROSS JOIN LATERAL (
  SELECT (SELECT count(*) FROM kb_chunks c WHERE c.resource_id = cls.resource_id AND c.is_current) AS nc,
         (SELECT COALESCE(sum(length(c.content)),0) FROM kb_chunks c
           WHERE c.resource_id = cls.resource_id AND c.is_current) AS len) x
GROUP BY 1 ORDER BY 2 DESC;
