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

\echo '== T4: content_cohesion = mean member-to-centroid cosine =='
DO $fx$
DECLARE
  r_a uuid; r_b uuid; b_a uuid; b_b uuid; reg uuid;
  ev uuid; et uuid; ent uuid;
  -- two unit-ish vectors: e1=[1,0,0,...], e2=[0,1,0,...] in 768-dim
  v1 vector := ('[1,' || array_to_string(array_fill(0::float8, ARRAY[767]), ',') || ']')::vector;
  v2 vector := ('[0,1,' || array_to_string(array_fill(0::float8, ARRAY[766]), ',') || ']')::vector;
BEGIN
  SELECT id INTO et FROM kb_event_types WHERE name='region_materialized';
  SELECT emitter_entity_id INTO ent FROM kb_events LIMIT 1;   -- reuse any seeded entity
  INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id)
    VALUES (et, ent, 'kb_cogmaps', (SELECT id FROM kb_cogmaps LIMIT 1)) RETURNING id INTO ev;
  INSERT INTO kb_resources (title, origin_uri) VALUES ('fx: A','temper://fx/a') RETURNING id INTO r_a;
  INSERT INTO kb_resources (title, origin_uri) VALUES ('fx: B','temper://fx/b') RETURNING id INTO r_b;
  INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
    VALUES (r_a,0,ev,ev) RETURNING id INTO b_a;
  INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
    VALUES (r_b,0,ev,ev) RETURNING id INTO b_b;
  INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash, embedding, is_current)
    VALUES (b_a, r_a, 0, 'h-a', v1, true);
  INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash, embedding, is_current)
    VALUES (b_b, r_b, 0, 'h-b', v2, true);
  INSERT INTO kb_cogmap_regions
    (cogmap_id, lens_id, centroid, salience, label, member_count, asserted_by_event_id, last_event_id)
    VALUES ((SELECT id FROM kb_cogmaps LIMIT 1), (SELECT id FROM kb_cogmap_lenses LIMIT 1),
            v1, 0.0, 'fx', 2, ev, ev) RETURNING id INTO reg;
  INSERT INTO kb_cogmap_region_members (region_id, member_table, member_id)
    VALUES (reg,'kb_resources',r_a),(reg,'kb_resources',r_b);
  -- centroid of e1,e2 = [0.5,0.5,0,...]; cos(e1,centroid)=cos(e2,centroid)=0.7071; mean=0.7071
  RAISE NOTICE 'content_cohesion=% (EXPECT ~0.7071)', round(cogmap_region_content_cohesion(reg)::numeric, 4);
END $fx$;

\echo '== T5: telos_alignment = cosine(centroid, telos embedding) =='
-- NOTE (GD-2 deviation from plan): the plan tested the real seeded onboarding region with a
-- bound-check accepting NULL, but the seed embeds NO chunks, so that path is always NULL (verified:
-- 0 embedded telos chunks). A self-contained deterministic fixture proves the function's correctness
-- unfakeably instead. The fn takes (p_region, p_cogmap) separately, so we reuse the T4 region
-- (centroid v1) against a fixture cogmap whose telos resource (r_a) carries embedding v1 ⇒ cos=1.0.
DO $fx5$
DECLARE reg uuid; r_a uuid; fxmap uuid;
BEGIN
  SELECT id INTO reg FROM kb_cogmap_regions WHERE label='fx';
  SELECT id INTO r_a FROM kb_resources WHERE origin_uri='temper://fx/a';  -- one current chunk, embedding v1=[1,0,...]
  INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('fx-telos-map', r_a) RETURNING id INTO fxmap;
  -- region centroid = v1; telos (r_a) pooled embedding = v1; cos(v1,v1) = 1.0
  RAISE NOTICE 'telos_alignment=% (EXPECT ~1.0)', round(cogmap_region_telos_alignment(reg, fxmap)::numeric, 4);
END $fx5$;

\echo '== T6: reference_standing / centrality / internal_tension exist and compute =='
DO $fx6$
DECLARE r_a uuid; r_b uuid; reg uuid; ev uuid;
BEGIN
  SELECT id INTO r_a FROM kb_resources WHERE origin_uri='temper://fx/a';
  SELECT id INTO r_b FROM kb_resources WHERE origin_uri='temper://fx/b';
  SELECT id INTO reg FROM kb_cogmap_regions WHERE label='fx';
  SELECT id INTO ev FROM kb_events ORDER BY occurred_at DESC LIMIT 1;
  -- a declared leads_to edge A->B, weight 0.8, homed in the fixture cogmap
  INSERT INTO kb_edges (source_table, source_id, target_table, target_id, edge_kind, label, weight,
                        home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id)
    VALUES ('kb_resources', r_a, 'kb_resources', r_b, 'leads_to', 'depends_on', 0.8,
            'kb_cogmaps', (SELECT id FROM kb_cogmaps LIMIT 1), ev, ev);
  RAISE NOTICE 'reference_standing=% centrality=% tension=%',
    cogmap_region_reference_standing(reg),
    round(cogmap_region_centrality(reg)::numeric,4),
    cogmap_region_internal_tension(reg, ARRAY['contradicts']);
END $fx6$;
-- EXPECT: reference_standing=0 centrality=1.6000 tension=0   (2 members × 0.8 internal weight; no opposed edge)
