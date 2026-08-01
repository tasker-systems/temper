-- P0 — verify the DEPLOYED objects before reasoning about them (standard §2).
\echo '=== signatures present ==='
SELECT p.proname || '(' || pg_get_function_arguments(p.oid) || ')' AS sig
FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
WHERE n.nspname = 'public'
  AND p.proname IN ('wayfind_region_scores','wayfind_scope_ids','wayfind_region_diagnostics',
                    'wayfind_scope_reach','unified_search','search_fts_candidates',
                    'search_vector_candidates','search_graph_expand')
ORDER BY 1;

\echo ''
\echo '=== wayfind_region_scores: constants + round-robin present? ==='
SELECT
  (prosrc ~ 'map_rank')                        AS has_map_rank,
  (prosrc ~ 'ORDER BY map_rank ASC')           AS has_round_robin_order,
  (prosrc ~ '0\.4::float8\s+AS alpha')         AS alpha_0_4,
  (prosrc ~ '0\.6::float8\s+AS beta')          AS beta_0_6,
  (prosrc ~ '0\.05::float8 AS kappa')          AS kappa_0_05,
  (prosrc ~ '1\.0::float8  AS prior_cogmap')   AS prior_cogmap_1_0,
  (prosrc ~ '0\.6::float8  AS prior_context')  AS prior_context_0_6,
  (prosrc ~ '3   AS default_n')                AS default_n_3,
  (prosrc ~ 'percent_rank\(\) OVER \(PARTITION BY c\.home_anchor_table') AS per_kind_percent_rank
FROM pg_proc WHERE proname = 'wayfind_region_scores';

\echo ''
\echo '=== wayfind_scope_ids: does region_ids bound members per region? ==='
SELECT
  (prosrc ~ 'wayfind_region_scores')            AS delegates_scoring,
  (prosrc ~ 'kb_cogmap_region_members')         AS derefs_members,
  (prosrc ~ 'thin_threshold')                   AS has_cold_start,
  (prosrc ~ '0   AS thin_threshold')            AS thin_threshold_0,
  -- any LIMIT inside the member deref would bound material per slot
  (SELECT count(*) FROM regexp_matches(prosrc, 'LIMIT', 'g')) AS limit_count
FROM pg_proc WHERE proname = 'wayfind_scope_ids';

\echo ''
\echo '=== search_fts_candidates: scope-aware? top-k? ==='
SELECT
  pg_get_function_arguments(oid)      AS args,
  (prosrc ~ 'p_scope_ids')            AS scope_aware,
  (prosrc ~ 'LIMIT')                  AS has_topk,
  (prosrc ~ 'ts_rank\(')              AS uses_ts_rank,
  (prosrc ~ 'ts_rank\([^)]*, 32\)')   AS ts_rank_norm_32,
  (prosrc ~ 'ingest_state')           AS gates_ingest_state
FROM pg_proc WHERE proname = 'search_fts_candidates';

\echo ''
\echo '=== unified_search: blend weights ==='
SELECT
  (prosrc ~ '1\.0::float8 AS w_fts')   AS w_fts_1_0,
  (prosrc ~ '1\.0::float8 AS w_vec')   AS w_vec_1_0,
  (prosrc ~ '0\.5::float8 AS w_graph') AS w_graph_0_5,
  (prosrc ~ '100 AS vector_k')         AS vector_k_100,
  (prosrc ~ '20 AS auto_seed_n')       AS auto_seed_n_20,
  (prosrc ~ 'p_scope_ids')             AS scope_threaded
FROM pg_proc WHERE proname = 'unified_search';

\echo ''
\echo '=== migration ledger head (what is actually applied) ==='
SELECT version, description, applied_at
FROM kb_migration_ledger ORDER BY version DESC LIMIT 6;
