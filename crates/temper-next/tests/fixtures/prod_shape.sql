-- Production-shape fixture for synthesis + parity tests (WS6 chunk 2/3).
--
-- Seeds a small, comprehensive `public.*` corpus into an ISOLATED ephemeral DB (the kind
-- `#[sqlx::test(migrator = "temper_next::MIGRATOR")]` creates): the full migration chain is applied,
-- so System/Anonymous profiles + the seeded doc_types/event_types already exist. This fixture adds:
--   * 2 fixture profiles (P1 owner, P2 originator) so an originator≠owner case exists (§2).
--   * 1 team (T1) owning the team-owned context C3 (the §2-amended team branch + auto-share).
--   * 3 contexts: C1, C2 profile-owned (profile branch); C3 team-owned (team branch + kb_team_contexts).
--   * 1 event (E1) the edges reference (kb_resource_edges.{asserted_by,last}_event_id are NOT NULL).
--   * 5 resources: R1 (concept, goal target), R2 (task, carries temper-goal + the §7 key spread),
--     R3 (decision), R5 (concept, homed in the team-owned C3) — all active — and R4 (concept)
--     soft-deleted (is_active=false). So 4 active, 1 excluded.
--   * a manifest per resource.
--   * a revision per chunked resource (kb_chunks.first_revision_id is NOT NULL → FK to revisions).
--   * chunks + chunk_content with distinct-DIRECTION 768-d embeddings (so vector-parity has signal)
--     and one chunk carrying header_path/heading_depth (R2 chunk 1) for body-reconstruction parity (§8).
--     NB the embeddings must differ in DIRECTION, not just magnitude: pgvector cosine distance (`<=>`)
--     ignores magnitude, so colinear vectors (e.g. array_fill(0.11), array_fill(0.22) — both the
--     all-ones direction) are all distance 0 from each other and a cosine query cannot order them. So
--     each chunk's vector is 0.01 in every dimension EXCEPT dimension 1, which carries a distinct
--     per-chunk discriminating value (R1c0 0.1, R2c0 0.2, R2c1 0.25, R3c0 0.3, R5c0 0.5). A query that
--     loads dimension 1 then orders the resources strictly by their best (closest) chunk's value.
--   * 3 edges: a normal contains edge (R1→R2), a folded near edge (R2→R3), and an inverse-polarity
--     leads_to edge (R3→R1, cross-context); all endpoints active.
--
-- Fixed UUIDs throughout so downstream tests can assert against known ids (see common::fixture_ids).

-- Profiles ------------------------------------------------------------------------------------------
INSERT INTO public.kb_profiles (id, display_name, slug) VALUES
  ('00000000-0000-0000-00f1-000000000001', 'Fixture Owner',      'fixture-owner'),
  ('00000000-0000-0000-00f1-000000000002', 'Fixture Originator', 'fixture-originator');

-- Teams: one fixture team that owns the team-owned context C3 (§2 amended team branch). ------------
INSERT INTO public.kb_teams (id, name, slug, created_by_profile_id) VALUES
  ('00000000-0000-0000-0701-000000000001', 'Fixture Team', 'fixture-team', '00000000-0000-0000-00f1-000000000001');

-- Contexts: C1/C2 profile-owned (the profile branch), C3 team-owned (the team branch + auto-share). -
INSERT INTO public.kb_contexts (id, name, kb_owner_table, kb_owner_id) VALUES
  ('00000000-0000-0000-00c0-000000000001', 'fixture-context-one',  'kb_profiles', '00000000-0000-0000-00f1-000000000001'),
  ('00000000-0000-0000-00c0-000000000002', 'fixture-context-two',  'kb_profiles', '00000000-0000-0000-00f1-000000000001'),
  ('00000000-0000-0000-00c0-000000000003', 'fixture-team-context', 'kb_teams',    '00000000-0000-0000-0701-000000000001');

-- Event the edges reference (resource_created type already seeded by migrations) -------------------
INSERT INTO public.kb_events (id, profile_id, device_id, kb_context_id, event_type_id, payload)
VALUES (
  '00000000-0000-0000-00e0-000000000001',
  '00000000-0000-0000-00f1-000000000001',
  'fixture-device',
  '00000000-0000-0000-00c0-000000000001',
  (SELECT id FROM public.kb_event_types WHERE name = 'resource_created'),
  '{}'::jsonb
);

-- Resources -----------------------------------------------------------------------------------------
INSERT INTO public.kb_resources
  (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug, originator_profile_id, owner_profile_id, is_active, created, updated)
VALUES
  -- R1: the goal target (concept), active.
  ('00000000-0000-0000-00a0-000000000001',
   '00000000-0000-0000-00c0-000000000001',
   (SELECT id FROM public.kb_doc_types WHERE name = 'concept'),
   'temper://fixture/goal-doc', 'Goal Doc', 'goal-doc',
   '00000000-0000-0000-00f1-000000000001', '00000000-0000-0000-00f1-000000000001',
   true, '2026-01-01T00:00:01Z', '2026-01-01T00:00:01Z'),
  -- R2: task carrying temper-goal + §7 key spread; originator≠owner.
  ('00000000-0000-0000-00a0-000000000002',
   '00000000-0000-0000-00c0-000000000001',
   (SELECT id FROM public.kb_doc_types WHERE name = 'task'),
   'temper://fixture/task-doc', 'Task Doc', 'task-doc',
   '00000000-0000-0000-00f1-000000000002', '00000000-0000-0000-00f1-000000000001',
   true, '2026-01-01T00:00:02Z', '2026-01-01T00:00:02Z'),
  -- R3: decision, active.
  ('00000000-0000-0000-00a0-000000000003',
   '00000000-0000-0000-00c0-000000000002',
   (SELECT id FROM public.kb_doc_types WHERE name = 'decision'),
   'temper://fixture/decision-doc', 'Decision Doc', 'decision-doc',
   '00000000-0000-0000-00f1-000000000001', '00000000-0000-0000-00f1-000000000001',
   true, '2026-01-01T00:00:03Z', '2026-01-01T00:00:03Z'),
  -- R4: soft-deleted (must be excluded by synthesis, §0).
  ('00000000-0000-0000-00a0-000000000004',
   '00000000-0000-0000-00c0-000000000002',
   (SELECT id FROM public.kb_doc_types WHERE name = 'concept'),
   'temper://fixture/deleted-doc', 'Deleted Doc', 'deleted-doc',
   '00000000-0000-0000-00f1-000000000001', '00000000-0000-0000-00f1-000000000001',
   false, '2026-01-01T00:00:04Z', '2026-01-01T00:00:04Z'),
  -- R5: active, homed in the team-owned context C3 (so the team branch + auto-share are exercised).
  ('00000000-0000-0000-00a0-000000000005',
   '00000000-0000-0000-00c0-000000000003',
   (SELECT id FROM public.kb_doc_types WHERE name = 'concept'),
   'temper://fixture/team-doc', 'Team Doc', 'team-doc',
   '00000000-0000-0000-00f1-000000000001', '00000000-0000-0000-00f1-000000000001',
   true, '2026-01-01T00:00:05Z', '2026-01-01T00:00:05Z');

-- Manifests -----------------------------------------------------------------------------------------
INSERT INTO public.kb_resource_manifests (resource_id, body_hash, managed_meta, open_meta) VALUES
  ('00000000-0000-0000-00a0-000000000001', 'sha256:r1',
   jsonb_build_object(
     'temper-title', 'Goal Doc', 'temper-slug', 'goal-doc',
     'temper-id', '00000000-0000-0000-00a0-000000000001'),
   '{}'::jsonb),
  -- R2 carries the full §7 key spread: title/slug/id (die), context (die), goal (edge),
  -- type (reconcile-to-doctype), stage/mode/effort (properties), plus open_meta keys.
  ('00000000-0000-0000-00a0-000000000002', 'sha256:r2',
   jsonb_build_object(
     'temper-title', 'Task Doc', 'temper-slug', 'task-doc',
     'temper-id', '00000000-0000-0000-00a0-000000000002',
     'temper-context', 'fixture-context-one',
     'temper-goal', 'goal-doc', 'temper-type', 'task',
     'temper-stage', 'doing', 'temper-mode', 'build', 'temper-effort', 'M'),
   jsonb_build_object('custom-key', 'custom-value', 'another-key', 'another-value')),
  ('00000000-0000-0000-00a0-000000000003', 'sha256:r3',
   jsonb_build_object(
     'temper-title', 'Decision Doc', 'temper-slug', 'decision-doc',
     'temper-id', '00000000-0000-0000-00a0-000000000003'),
   '{}'::jsonb),
  ('00000000-0000-0000-00a0-000000000004', 'sha256:r4', '{}'::jsonb, '{}'::jsonb),
  ('00000000-0000-0000-00a0-000000000005', 'sha256:r5',
   jsonb_build_object(
     'temper-title', 'Team Doc', 'temper-slug', 'team-doc',
     'temper-id', '00000000-0000-0000-00a0-000000000005'),
   '{}'::jsonb);

-- Revisions (kb_chunks.first_revision_id FK target) ------------------------------------------------
INSERT INTO public.kb_resource_revisions (id, resource_id, body_hash, chunk_count) VALUES
  ('00000000-0000-0000-0bb0-000000000001', '00000000-0000-0000-00a0-000000000001', 'sha256:r1', 1),
  ('00000000-0000-0000-0bb0-000000000002', '00000000-0000-0000-00a0-000000000002', 'sha256:r2', 2),
  ('00000000-0000-0000-0bb0-000000000003', '00000000-0000-0000-00a0-000000000003', 'sha256:r3', 1),
  ('00000000-0000-0000-0bb0-000000000005', '00000000-0000-0000-00a0-000000000005', 'sha256:r5', 1);

-- Chunks (distinct-DIRECTION 768-d embeddings; R2 chunk 1 carries heading metadata) ----------------
-- Each embedding is 0.01 in every dimension EXCEPT dimension 1, which carries a distinct per-chunk
-- value. Distinct DIRECTIONS (not just magnitudes) are required: cosine distance (`<=>`) is
-- magnitude-invariant, so colinear vectors would all be distance 0 and a cosine query could not order
-- them. With a query that loads dimension 1, the per-resource best (closest) chunk strictly orders the
-- resources by that value: R5(0.5) < R3(0.3) < R2(0.25 via chunk1) < R1(0.1) by ascending cosine
-- distance. See readback::vector_search + the vector_parity test.
INSERT INTO public.kb_chunks
  (id, resource_id, chunk_index, version, header_path, heading_depth, content_hash, embedding, is_current, first_revision_id)
VALUES
  ('00000000-0000-0000-0cc0-000000000001', '00000000-0000-0000-00a0-000000000001', 0, 1,
   '', 0, 'hash-r1-c0',
   (SELECT array_agg(CASE WHEN i = 1 THEN 0.1::real ELSE 0.01::real END ORDER BY i)
      FROM generate_series(1,768) AS i)::vector,
   true, '00000000-0000-0000-0bb0-000000000001'),
  ('00000000-0000-0000-0cc0-000000000002', '00000000-0000-0000-00a0-000000000002', 0, 1,
   '', 0, 'hash-r2-c0',
   (SELECT array_agg(CASE WHEN i = 1 THEN 0.2::real ELSE 0.01::real END ORDER BY i)
      FROM generate_series(1,768) AS i)::vector,
   true, '00000000-0000-0000-0bb0-000000000002'),
  ('00000000-0000-0000-0cc0-000000000003', '00000000-0000-0000-00a0-000000000002', 1, 1,
   'Intro > Goals', 2, 'hash-r2-c1',
   (SELECT array_agg(CASE WHEN i = 1 THEN 0.25::real ELSE 0.01::real END ORDER BY i)
      FROM generate_series(1,768) AS i)::vector,
   true, '00000000-0000-0000-0bb0-000000000002'),
  ('00000000-0000-0000-0cc0-000000000004', '00000000-0000-0000-00a0-000000000003', 0, 1,
   '', 0, 'hash-r3-c0',
   (SELECT array_agg(CASE WHEN i = 1 THEN 0.3::real ELSE 0.01::real END ORDER BY i)
      FROM generate_series(1,768) AS i)::vector,
   true, '00000000-0000-0000-0bb0-000000000003'),
  ('00000000-0000-0000-0cc0-000000000005', '00000000-0000-0000-00a0-000000000005', 0, 1,
   '', 0, 'hash-r5-c0',
   (SELECT array_agg(CASE WHEN i = 1 THEN 0.5::real ELSE 0.01::real END ORDER BY i)
      FROM generate_series(1,768) AS i)::vector,
   true, '00000000-0000-0000-0bb0-000000000005');

INSERT INTO public.kb_chunk_content (chunk_id, content) VALUES
  ('00000000-0000-0000-0cc0-000000000001', 'Goal body text.'),
  ('00000000-0000-0000-0cc0-000000000002', 'Task intro paragraph.'),
  ('00000000-0000-0000-0cc0-000000000003', 'Task goals section body.'),
  ('00000000-0000-0000-0cc0-000000000004', 'Decision body text.'),
  ('00000000-0000-0000-0cc0-000000000005', 'Team body text.');

-- Edges: 1 normal (contains R1→R2), 1 folded (near R2→R3), 1 inverse-polarity (leads_to R3→R1, the
-- cross-context case: R3 in C2, R1 in C1 — homes at the SOURCE's context); all endpoints active. The
-- contains R1→R2 edge IS R2's materialized temper-goal edge, so it proves the §7 mint dedup.
INSERT INTO public.kb_resource_edges
  (id, source_resource_id, target_resource_id, edge_kind, polarity, label, weight, asserted_by_event_id, last_event_id, is_folded)
VALUES
  ('00000000-0000-0000-0dd0-000000000001',
   '00000000-0000-0000-00a0-000000000001', '00000000-0000-0000-00a0-000000000002',
   'contains', 'forward', 'parent_of', 1.0,
   '00000000-0000-0000-00e0-000000000001', '00000000-0000-0000-00e0-000000000001', false),
  ('00000000-0000-0000-0dd0-000000000002',
   '00000000-0000-0000-00a0-000000000002', '00000000-0000-0000-00a0-000000000003',
   'near', 'forward', '', 0.5,
   '00000000-0000-0000-00e0-000000000001', '00000000-0000-0000-00e0-000000000001', true),
  ('00000000-0000-0000-0dd0-000000000003',
   '00000000-0000-0000-00a0-000000000003', '00000000-0000-0000-00a0-000000000001',
   'leads_to', 'inverse', 'derived_from', 0.7,
   '00000000-0000-0000-00e0-000000000001', '00000000-0000-0000-00e0-000000000001', false);
