\echo '=== ts_rank flag actually deployed in search_fts_candidates (settle it by printing the body) ==='
SELECT substring(pg_get_functiondef(p.oid) from 'ts_rank\([^;]{0,120}') AS ts_rank_call
FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
WHERE p.proname='search_fts_candidates' AND n.nspname='public';

\echo '=== search_vector_candidates: does the unscoped branch apply LIMIT inside ann? ==='
SELECT count(*) FILTER (WHERE d ~ 'LIMIT') AS mentions_limit,
       count(*) FILTER (WHERE d ~ 'p_scope_ids') AS mentions_scope
FROM (SELECT pg_get_functiondef(p.oid) AS d
      FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
      WHERE p.proname='search_vector_candidates' AND n.nspname='public') t;
