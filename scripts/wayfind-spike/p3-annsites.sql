\set ON_ERROR_STOP on
\echo '=== every deployed function doing a top-k ANN (<=> with LIMIT) — the copy set ==='
SELECT p.proname,
       (prosrc ~ '<=>')                       AS uses_distance,
       (SELECT count(*) FROM regexp_matches(prosrc,'LIMIT','g')) AS limit_count,
       p.proconfig                            AS function_scoped_gucs
  FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
 WHERE n.nspname='public' AND p.prosrc ~ '<=>'
 ORDER BY 1;
