-- Plan-1 substrate verification. Run after 01→03. search_path pinned like 04_scenarios.sql.
SET search_path = temper_next, public;
\echo '== T1: readout columns present =='
SELECT string_agg(column_name, ',' ORDER BY column_name) AS got
FROM information_schema.columns
WHERE table_schema='temper_next' AND table_name='kb_cogmap_regions'
  AND column_name IN ('telos_alignment','reference_standing','centrality','content_cohesion','internal_tension');
-- EXPECT: centrality,content_cohesion,internal_tension,reference_standing,telos_alignment

\echo '== T2: telos-default lens exists and the seeded region points at it =='
SELECT l.name AS lens_name, l.selection_kind,
       (r.lens_id = l.id) AS region_linked
FROM kb_cogmap_lenses l
JOIN kb_cogmap_regions r ON r.lens_id = l.id
WHERE l.name = 'telos-default';
-- EXPECT: telos-default | homed | t

\echo '== T3: kb_properties accepts an edge owner =='
SELECT pg_get_constraintdef(oid) LIKE '%kb_edges%' AS edges_allowed
FROM pg_constraint
WHERE conrelid='temper_next.kb_properties'::regclass AND contype='c'
  AND pg_get_constraintdef(oid) LIKE '%owner_table%';
-- EXPECT: t
