-- P5 — two remaining questions.
--  (1) FTS MATCH RATE by class: does a short document simply fail to match at all?
--      (P3 showed ts_rank is kind-neutral AMONG matches; this asks who gets to be a match.)
--  (2) REGION-ARM SCOPE COMPOSITION: at width 3, are the winning regions' members
--      distilled-majority? If yes, raw dominance is a RANKING loss, not an admission loss.
--      Probe family: document vectors (counterfactual). Deterministic by md5.
\set prin '019d4add-f49d-7c43-a87d-dda470e5dd9c'

\echo '=== 1. FTS match rate by class, over the 20 real query strings ==='
WITH q(qi, txt) AS (VALUES
  (1,'what picks the map that a routing query gets sent to'),
  (2,'who gets recorded as having graded a citation'),
  (3,'what is an outcome register'),
  (4,'what does the additive-only migration gate do'),
  (5,'how does temper decide which resources a principal can see'),
  (6,'what is a cognitive map telos'),
  (7,'why are machine principals registered rather than discovered'),
  (8,'what makes evidential standing three axes instead of one number'),
  (9,'what prevents a refusal from naming the subject it refused'),
  (10,'what is the difference between a context and a cognitive map'),
  (11,'why does wayfind emit no ledger event'),
  (12,'how are act span fields recorded for write commands'),
  (13,'how do I run the integration tests'),
  (14,'what changed in the context rename design'),
  (15,'how does the vault projection cache work'),
  (16,'what is the sqlx query cache regeneration ritual'),
  (17,'how do I set up docker postgres locally'),
  (18,'explain eigenvalues and eigenvectors'),
  (19,'how does storyteller model narrative agents'),
  (20,'what is the tasker orchestration actor model')),
vis AS (
  SELECT rv.resource_id FROM resources_visible_to(:'prin'::uuid) rv
  JOIN kb_resources r ON r.id = rv.resource_id AND r.is_active AND r.ingest_state='complete'),
cls AS (
  SELECT v.resource_id,
         CASE WHEN bool_or(hh.anchor_table='kb_cogmaps') THEN 'distilled' ELSE 'raw' END AS class
  FROM vis v LEFT JOIN kb_resource_homes hh ON hh.resource_id = v.resource_id
  GROUP BY v.resource_id)
SELECT cls.class,
       count(*)                                                       AS class_resources,
       count(*) FILTER (WHERE si.search_vector @@ websearch_to_tsquery('english', q.txt)) AS fts_matches,
       round(100.0 * count(*) FILTER (WHERE si.search_vector @@ websearch_to_tsquery('english', q.txt))
             / count(*), 2)                                           AS pct_matching,
       round(avg(length(si.search_vector))::numeric, 1)               AS mean_lexemes
FROM q CROSS JOIN cls
JOIN kb_resource_search_index si ON si.resource_id = cls.resource_id
GROUP BY 1 ORDER BY 1;

\echo ''
\echo '=== 1b. FTS match rate by LEXEME bucket, within class (is it length, or is it kind?) ==='
WITH q(txt) AS (VALUES
  ('what picks the map that a routing query gets sent to'),('who gets recorded as having graded a citation'),
  ('what is an outcome register'),('what does the additive-only migration gate do'),
  ('how does temper decide which resources a principal can see'),('what is a cognitive map telos'),
  ('why are machine principals registered rather than discovered'),
  ('what makes evidential standing three axes instead of one number'),
  ('what prevents a refusal from naming the subject it refused'),
  ('what is the difference between a context and a cognitive map'),
  ('why does wayfind emit no ledger event'),('how are act span fields recorded for write commands'),
  ('how do I run the integration tests'),('what changed in the context rename design'),
  ('how does the vault projection cache work'),('what is the sqlx query cache regeneration ritual'),
  ('how do I set up docker postgres locally'),('explain eigenvalues and eigenvectors'),
  ('how does storyteller model narrative agents'),('what is the tasker orchestration actor model')),
vis AS (
  SELECT rv.resource_id FROM resources_visible_to(:'prin'::uuid) rv
  JOIN kb_resources r ON r.id = rv.resource_id AND r.is_active AND r.ingest_state='complete'),
cls AS (
  SELECT v.resource_id,
         CASE WHEN bool_or(hh.anchor_table='kb_cogmaps') THEN 'distilled' ELSE 'raw' END AS class
  FROM vis v LEFT JOIN kb_resource_homes hh ON hh.resource_id = v.resource_id
  GROUP BY v.resource_id)
SELECT cls.class,
       CASE WHEN length(si.search_vector) < 100 THEN 'a:<100'
            WHEN length(si.search_vector) < 200 THEN 'b:100-199'
            WHEN length(si.search_vector) < 400 THEN 'c:200-399'
            ELSE 'd:400+' END AS lexeme_bucket,
       count(*) AS observations,
       round(100.0 * count(*) FILTER (WHERE si.search_vector @@ websearch_to_tsquery('english', q.txt))
             / count(*), 2) AS pct_matching
FROM q CROSS JOIN cls
JOIN kb_resource_search_index si ON si.resource_id = cls.resource_id
GROUP BY 1,2 ORDER BY 1,2;

\echo ''
\echo '=== 2. region-arm SCOPE composition at width 3 (document-vector probes) ==='
WITH vis AS (
  SELECT rv.resource_id FROM resources_visible_to(:'prin'::uuid) rv
  JOIN kb_resources r ON r.id = rv.resource_id AND r.is_active AND r.ingest_state='complete'),
homed AS (
  SELECT DISTINCT ON (h.resource_id) h.resource_id, h.anchor_id
  FROM kb_resource_homes h JOIN vis v ON v.resource_id = h.resource_id),
pick AS (
  SELECT c.id AS chunk_id, c.embedding,
         row_number() OVER (PARTITION BY homed.anchor_id ORDER BY md5(c.id::text)) AS rn
  FROM kb_chunks c JOIN homed ON homed.resource_id = c.resource_id
  WHERE c.is_current),
probes AS (SELECT chunk_id, embedding FROM pick WHERE rn <= 2),
won AS (
  SELECT p.chunk_id, s.region_id, s.home_anchor_table
  FROM probes p
  CROSS JOIN LATERAL wayfind_region_scores(:'prin'::uuid, NULL, p.embedding, 3) s
  WHERE s.in_top_n),
mem AS (
  SELECT w.chunk_id, w.home_anchor_table AS region_kind, m.member_id
  FROM won w
  JOIN kb_cogmap_region_members m ON m.region_id = w.region_id AND m.member_table='kb_resources'
  WHERE m.member_id IN (SELECT resource_id FROM resources_visible_to(:'prin'::uuid)))
SELECT region_kind,
       count(*)                              AS member_slots_across_probes,
       count(DISTINCT member_id)             AS distinct_resources,
       round(count(*)::numeric
             / (SELECT count(*) FROM probes), 2) AS mean_members_per_probe
FROM mem GROUP BY 1 ORDER BY 1;
