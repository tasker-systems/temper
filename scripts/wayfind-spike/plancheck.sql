\set ON_ERROR_STOP on
\echo '=== does the LATERAL form actually engage the HNSW index? ==='
EXPLAIN (COSTS OFF)
WITH q(qi, emb) AS (VALUES (1, (SELECT embedding FROM kb_chunks WHERE is_current LIMIT 1)))
SELECT t.* FROM q CROSS JOIN LATERAL (
  SELECT c.id, c.resource_id FROM kb_chunks c WHERE c.is_current
   ORDER BY c.embedding <=> q.emb LIMIT 100) t;
