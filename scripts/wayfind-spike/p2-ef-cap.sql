\set ON_ERROR_STOP on
SELECT '[1,2]'::vector <=> '[1,3]'::vector AS warmup;

\echo '=== CORRECTED corpus shape ==='
SELECT (SELECT count(*) FROM kb_chunks WHERE is_current) AS current_chunks,
       (SELECT count(DISTINCT resource_id) FROM kb_chunks WHERE is_current) AS resources;

-- a real document vector as a LITERAL, so ORDER BY compares against a constant and HNSW engages
SELECT embedding::text AS qv FROM kb_chunks WHERE is_current ORDER BY md5(id::text) LIMIT 1 \gset

\echo ''
\echo '=== plan MUST be an Index Scan on idx_kb_chunks_embedding ==='
EXPLAIN (COSTS OFF) SELECT resource_id FROM kb_chunks WHERE is_current ORDER BY embedding <=> :'qv'::vector LIMIT 100;

\echo ''
\echo '=== chunks returned and distinct resources, LIMIT 100, per ef_search ==='
SET hnsw.ef_search = 40;
SELECT 'ef=40 (PRODUCTION DEFAULT)' AS arm, count(*) AS chunks, count(DISTINCT resource_id) AS resources
  FROM (SELECT resource_id FROM kb_chunks WHERE is_current ORDER BY embedding <=> :'qv'::vector LIMIT 100) t;
SET hnsw.ef_search = 100;
SELECT 'ef=100', count(*), count(DISTINCT resource_id)
  FROM (SELECT resource_id FROM kb_chunks WHERE is_current ORDER BY embedding <=> :'qv'::vector LIMIT 100) t;
SET hnsw.ef_search = 400;
SELECT 'ef=400', count(*), count(DISTINCT resource_id)
  FROM (SELECT resource_id FROM kb_chunks WHERE is_current ORDER BY embedding <=> :'qv'::vector LIMIT 100) t;
RESET hnsw.ef_search;

\echo ''
\echo '=== exact scan, same vector, same LIMIT ==='
SET enable_indexscan = off; SET enable_bitmapscan = off;
SELECT 'exact' AS arm, count(*) AS chunks, count(DISTINCT resource_id) AS resources
  FROM (SELECT resource_id FROM kb_chunks WHERE is_current ORDER BY embedding <=> :'qv'::vector LIMIT 100) t;
RESET enable_indexscan; RESET enable_bitmapscan;
