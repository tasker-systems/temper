\set ON_ERROR_STOP on
\echo '=== every HNSW/ivfflat index in the database ==='
SELECT c.relname AS index_name, t.relname AS table_name, am.amname, pg_get_indexdef(c.oid) AS def
  FROM pg_class c
  JOIN pg_index i  ON i.indexrelid=c.oid
  JOIN pg_class t  ON t.oid=i.indrelid
  JOIN pg_am am    ON am.oid=c.relam
 WHERE am.amname IN ('hnsw','ivfflat');

\echo ''
\echo '=== wayfind_region_scores: the lines around each LIMIT ==='
SELECT string_agg(ln, E'\n') AS ctx FROM (
  SELECT l AS ln FROM regexp_split_to_table(
    (SELECT prosrc FROM pg_proc WHERE proname='wayfind_region_scores'), E'\n')
    WITH ORDINALITY AS s(l, n)
  WHERE l ~* 'limit|<=>|FROM kb_'
) x;
