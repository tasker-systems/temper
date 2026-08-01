-- P4b — order-statistic hypothesis, probes inlined (read-only transaction forbids CREATE VIEW).
\set prin '019d4add-f49d-7c43-a87d-dda470e5dd9c'

\echo '=== A. best-of-N (shipped) vs length-controlled counterfactuals, by class ==='
WITH vis AS (
  SELECT rv.resource_id FROM resources_visible_to(:'prin'::uuid) rv
  JOIN kb_resources r ON r.id = rv.resource_id AND r.is_active AND r.ingest_state='complete'),
cls AS (
  SELECT v.resource_id,
         CASE WHEN bool_or(hh.anchor_table='kb_cogmaps') THEN 'distilled' ELSE 'raw' END AS class
  FROM vis v LEFT JOIN kb_resource_homes hh ON hh.resource_id = v.resource_id
  GROUP BY v.resource_id),
homed AS (
  SELECT DISTINCT ON (h.resource_id) h.resource_id, h.anchor_id
  FROM kb_resource_homes h JOIN vis v ON v.resource_id = h.resource_id),
pick AS (
  SELECT c.id AS chunk_id, c.embedding, homed.anchor_id,
         row_number() OVER (PARTITION BY homed.anchor_id ORDER BY md5(c.id::text)) AS rn
  FROM kb_chunks c JOIN homed ON homed.resource_id = c.resource_id
  WHERE c.is_current),
probes AS (SELECT chunk_id, embedding FROM pick WHERE rn <= 2),
per AS (
  SELECT p.chunk_id, cls.class, cls.resource_id,
         count(*)                                   AS n_chunks,
         1.0 - MIN(c.embedding <=> p.embedding)/2.0 AS best_of_n,
         1.0 - AVG(c.embedding <=> p.embedding)/2.0 AS mean_chunk,
         1.0 - MIN(c.embedding <=> p.embedding) FILTER (WHERE c.chunk_index = 0)/2.0 AS first_chunk
  FROM probes p
  CROSS JOIN cls
  JOIN kb_chunks c ON c.resource_id = cls.resource_id AND c.is_current
  GROUP BY p.chunk_id, cls.class, cls.resource_id)
SELECT class,
       count(DISTINCT resource_id)        AS resources,
       round(avg(n_chunks)::numeric,2)    AS mean_chunks,
       round(avg(best_of_n)::numeric,4)   AS shipped_best_of_n,
       round(avg(mean_chunk)::numeric,4)  AS cf_mean_chunk,
       round(avg(first_chunk)::numeric,4) AS cf_first_chunk
FROM per GROUP BY 1 ORDER BY 1;

\echo ''
\echo '=== B. artifact test: best-of-N by chunk-count bucket, WITHIN class ==='
WITH vis AS (
  SELECT rv.resource_id FROM resources_visible_to(:'prin'::uuid) rv
  JOIN kb_resources r ON r.id = rv.resource_id AND r.is_active AND r.ingest_state='complete'),
cls AS (
  SELECT v.resource_id,
         CASE WHEN bool_or(hh.anchor_table='kb_cogmaps') THEN 'distilled' ELSE 'raw' END AS class
  FROM vis v LEFT JOIN kb_resource_homes hh ON hh.resource_id = v.resource_id
  GROUP BY v.resource_id),
homed AS (
  SELECT DISTINCT ON (h.resource_id) h.resource_id, h.anchor_id
  FROM kb_resource_homes h JOIN vis v ON v.resource_id = h.resource_id),
pick AS (
  SELECT c.id AS chunk_id, c.embedding, homed.anchor_id,
         row_number() OVER (PARTITION BY homed.anchor_id ORDER BY md5(c.id::text)) AS rn
  FROM kb_chunks c JOIN homed ON homed.resource_id = c.resource_id
  WHERE c.is_current),
probes AS (SELECT chunk_id, embedding FROM pick WHERE rn <= 2),
per AS (
  SELECT cls.class, count(*) AS n_chunks,
         1.0 - MIN(c.embedding <=> p.embedding)/2.0 AS best_of_n,
         1.0 - AVG(c.embedding <=> p.embedding)/2.0 AS mean_chunk
  FROM probes p CROSS JOIN cls
  JOIN kb_chunks c ON c.resource_id = cls.resource_id AND c.is_current
  GROUP BY p.chunk_id, cls.class, cls.resource_id)
SELECT class,
       CASE WHEN n_chunks = 1 THEN 'a:1'
            WHEN n_chunks <= 3 THEN 'b:2-3'
            WHEN n_chunks <= 7 THEN 'c:4-7'
            WHEN n_chunks <= 15 THEN 'd:8-15'
            ELSE 'e:16+' END AS chunk_bucket,
       count(*) AS observations,
       round(avg(best_of_n)::numeric,4)  AS shipped_best_of_n,
       round(avg(mean_chunk)::numeric,4) AS cf_mean_chunk
FROM per GROUP BY 1,2 ORDER BY 1,2;
