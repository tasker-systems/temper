\set ON_ERROR_STOP on
\echo '=== force pgvector library load, THEN read its GUCs (lazy _PG_init registration) ==='
SELECT '[1,2]'::vector <=> '[1,3]'::vector AS warmup;

SELECT name, setting, boot_val, source FROM pg_settings WHERE name LIKE 'hnsw.%' ORDER BY 1;

\echo ''
\echo '=== pgvector version on prod ==='
SELECT extname, extversion FROM pg_extension WHERE extname = 'vector';

\echo ''
\echo '=== server version ==='
SELECT version();

\echo ''
\echo '=== HNSW index: build params (reloptions) and predicate ==='
SELECT c.relname AS index_name,
       c.reloptions,
       pg_get_indexdef(c.oid) AS indexdef
  FROM pg_class c
  JOIN pg_index i ON i.indexrelid = c.oid
 WHERE c.relname = 'idx_kb_chunks_embedding';

\echo ''
\echo '=== corpus shape: current chunks and their concentration ==='
SELECT count(*) AS current_chunks,
       count(DISTINCT resource_id) AS resources_with_chunks,
       round(avg(n)::numeric, 2) AS mean_chunks_per_resource,
       max(n) AS max_chunks_one_resource,
       percentile_cont(0.5) WITHIN GROUP (ORDER BY n) AS median_chunks
  FROM (SELECT resource_id, count(*) AS n
          FROM kb_chunks WHERE is_current GROUP BY resource_id) t,
       LATERAL (SELECT 1) x;
