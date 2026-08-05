\set ON_ERROR_STOP on
SELECT '[1,2]'::vector <=> '[1,3]'::vector AS w \gset w
\echo '=== is the migration applied on prod, and how did it go? ==='
SELECT version, state, class, state_reason FROM migration_current WHERE version = 20260804000030;
\echo ''
\echo '=== is the pin actually in force on the deployed function? ==='
SELECT proname, proconfig FROM pg_proc WHERE proname = 'search_vector_candidates';
\echo ''
\echo '=== session default unchanged (the pin must NOT have leaked) ==='
SELECT current_setting('hnsw.ef_search') AS session_ef_search;
