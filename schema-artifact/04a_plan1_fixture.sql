-- Plan-1 substrate verification. Run after 01→03. search_path pinned like 04_scenarios.sql.
SET search_path = temper_next, public;
\echo '== T1: readout columns present =='
SELECT string_agg(column_name, ',' ORDER BY column_name) AS got
FROM information_schema.columns
WHERE table_schema='temper_next' AND table_name='kb_cogmap_regions'
  AND column_name IN ('telos_alignment','reference_standing','centrality','content_cohesion','internal_tension');
-- EXPECT: centrality,content_cohesion,internal_tension,reference_standing,telos_alignment
