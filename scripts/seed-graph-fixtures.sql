-- seed-graph-fixtures.sql (substrate / WS6 collapse)
--
-- Integration-test fixtures for graph_service::aggregator_subgraph. Loaded after
-- common::fixtures::clean_and_seed (a no-op under #[sqlx::test] isolation). All UUIDs are
-- well-known so the Rust tests reference resources by constant.
--
-- Substrate shape:
--   * resources           → kb_resources (id, title, origin_uri, is_active)
--   * ownership + home     → kb_resource_homes (anchor=kb_contexts, owner/originator profile)
--   * doc-type + stage     → kb_properties (property_key 'doc_type' / 'temper-stage', JSON-string value)
--   * edges                → kb_edges (polymorphic endpoints, homed in a context)
--   * body chunks/excerpt  → kb_content_blocks + kb_chunks + kb_chunk_content
--
-- Every property/edge/block needs a real kb_events row for its asserted_by/genesis FK; the test never
-- inspects event content, so a SINGLE genesis event (Alice-emitted, primary-context-anchored) backs all
-- of them. The personal-team AFTER-INSERT trigger fires on each kb_profiles row (harmless here).
--
-- Owner layout:   Alice (caller, primary) / Bob (cross-owner leak target)
-- Context layout: graph-test-primary / graph-test-secondary (Alice) / bob-context (Bob)
-- UUID ranges:    00c1 concepts · 00c2 direct members · 00c3 tier-3/4 · 00c4 bob · 00c5 secondary

BEGIN;

-- ─── Profiles ──────────────────────────────────────────────────────────────
INSERT INTO kb_profiles (id, handle, display_name, email) VALUES
    ('00000000-0000-0000-0088-000000000001', 'alice-graph', 'Alice (graph-test)', 'alice-graph@test.com'),
    ('00000000-0000-0000-0088-000000000002', 'bob-graph',   'Bob (graph-test)',   'bob-graph@test.com')
ON CONFLICT (id) DO UPDATE SET display_name = EXCLUDED.display_name;

INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id) VALUES
    ('00000000-0000-0000-0088-000000000101', '00000000-0000-0000-0088-000000000001', 'test-provider', 'test|alice-graph'),
    ('00000000-0000-0000-0088-000000000102', '00000000-0000-0000-0088-000000000002', 'test-provider', 'test|bob-graph')
ON CONFLICT DO NOTHING;

-- Alice's emitter entity (the genesis event's actor).
INSERT INTO kb_entities (id, profile_id, name, metadata) VALUES
    ('00000000-0000-0000-0088-000000000201', '00000000-0000-0000-0088-000000000001', 'alice-graph@web', '{}'::jsonb)
ON CONFLICT (id) DO NOTHING;

-- ─── Contexts ──────────────────────────────────────────────────────────────
INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) VALUES
    ('00000000-0000-0000-00bc-000000000001', 'kb_profiles', '00000000-0000-0000-0088-000000000001', 'graph-test-primary',   'graph-test-primary'),
    ('00000000-0000-0000-00bc-000000000002', 'kb_profiles', '00000000-0000-0000-0088-000000000001', 'graph-test-secondary', 'graph-test-secondary'),
    ('00000000-0000-0000-00bc-000000000099', 'kb_profiles', '00000000-0000-0000-0088-000000000002', 'bob-context',          'bob-context')
ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name;

-- ─── Genesis event (one, reused as every asserted_by/genesis FK) ────────────
INSERT INTO kb_events (id, event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id, payload, occurred_at)
SELECT '00000000-0000-0000-00ee-000000000001',
       (SELECT id FROM kb_event_types WHERE name = 'relationship_asserted'),
       '00000000-0000-0000-0088-000000000201',
       'kb_contexts', '00000000-0000-0000-00bc-000000000001',
       '{}'::jsonb, now()
ON CONFLICT (id) DO NOTHING;

-- ─── Resources ─────────────────────────────────────────────────────────────
INSERT INTO kb_resources (id, title, origin_uri, is_active) VALUES
    ('00000000-0000-0000-00c1-000000000001', 'Idempotency Keys',               'test://concept-idempotency',        true),
    ('00000000-0000-0000-00c1-000000000002', 'Circuit Breakers',               'test://concept-circuit-breakers',   true),
    ('00000000-0000-0000-00c1-000000000003', 'Zero-Copy Patterns',             'test://concept-zero-copy',          true),
    ('00000000-0000-0000-00c1-000000000004', 'Auth Patterns',                  'test://concept-auth-patterns',      true),
    ('00000000-0000-0000-00c1-000000000005', 'Deleted Concept',                'test://concept-deleted',            false),
    ('00000000-0000-0000-00c2-000000000001', 'OAuth Comparison',               'test://m1-oauth-comparison',        true),
    ('00000000-0000-0000-00c2-000000000002', 'Auth Middleware',                'test://m2-auth-middleware',         true),
    ('00000000-0000-0000-00c2-000000000003', 'Auth Debug Session',             'test://m3-auth-debug',              true),
    ('00000000-0000-0000-00c2-000000000004', 'Circuit Breaker Design',         'test://m4-circuit-design',          true),
    ('00000000-0000-0000-00c2-000000000005', 'Circuit Breaker Implementation', 'test://m5-circuit-impl',            true),
    ('00000000-0000-0000-00c2-000000000006', 'JWT Strategies',                 'test://m6-jwt-strategies',          true),
    ('00000000-0000-0000-00c3-000000000001', 'Token Refresh Patterns',         'test://t1-token-refresh',           true),
    ('00000000-0000-0000-00c3-000000000002', 'Session Management Research',    'test://t2-session-mgmt',            true),
    ('00000000-0000-0000-00c4-000000000001', 'Bob Secret Concept',             'test://b1-bob-concept',             true),
    ('00000000-0000-0000-00c4-000000000002', 'Bob Private Research',           'test://b2-bob-research',            true),
    ('00000000-0000-0000-00c5-000000000001', 'Secondary Context Concept',      'test://s1-secondary-concept',       true)
ON CONFLICT (id) DO UPDATE SET is_active = EXCLUDED.is_active, title = EXCLUDED.title;

-- ─── Homes (anchor context + owner/originator) ──────────────────────────────
-- (resource, context, owner). originator = owner for every fixture row.
INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
SELECT r.rid, 'kb_contexts', r.ctx, r.owner, r.owner
FROM (VALUES
    ('00000000-0000-0000-00c1-000000000001'::uuid, '00000000-0000-0000-00bc-000000000001'::uuid, '00000000-0000-0000-0088-000000000001'::uuid),
    ('00000000-0000-0000-00c1-000000000002'::uuid, '00000000-0000-0000-00bc-000000000001'::uuid, '00000000-0000-0000-0088-000000000001'::uuid),
    ('00000000-0000-0000-00c1-000000000003'::uuid, '00000000-0000-0000-00bc-000000000001'::uuid, '00000000-0000-0000-0088-000000000001'::uuid),
    ('00000000-0000-0000-00c1-000000000004'::uuid, '00000000-0000-0000-00bc-000000000001'::uuid, '00000000-0000-0000-0088-000000000001'::uuid),
    ('00000000-0000-0000-00c1-000000000005'::uuid, '00000000-0000-0000-00bc-000000000001'::uuid, '00000000-0000-0000-0088-000000000001'::uuid),
    ('00000000-0000-0000-00c2-000000000001'::uuid, '00000000-0000-0000-00bc-000000000001'::uuid, '00000000-0000-0000-0088-000000000001'::uuid),
    ('00000000-0000-0000-00c2-000000000002'::uuid, '00000000-0000-0000-00bc-000000000001'::uuid, '00000000-0000-0000-0088-000000000001'::uuid),
    ('00000000-0000-0000-00c2-000000000003'::uuid, '00000000-0000-0000-00bc-000000000001'::uuid, '00000000-0000-0000-0088-000000000001'::uuid),
    ('00000000-0000-0000-00c2-000000000004'::uuid, '00000000-0000-0000-00bc-000000000001'::uuid, '00000000-0000-0000-0088-000000000001'::uuid),
    ('00000000-0000-0000-00c2-000000000005'::uuid, '00000000-0000-0000-00bc-000000000001'::uuid, '00000000-0000-0000-0088-000000000001'::uuid),
    ('00000000-0000-0000-00c2-000000000006'::uuid, '00000000-0000-0000-00bc-000000000001'::uuid, '00000000-0000-0000-0088-000000000001'::uuid),
    ('00000000-0000-0000-00c3-000000000001'::uuid, '00000000-0000-0000-00bc-000000000001'::uuid, '00000000-0000-0000-0088-000000000001'::uuid),
    ('00000000-0000-0000-00c3-000000000002'::uuid, '00000000-0000-0000-00bc-000000000001'::uuid, '00000000-0000-0000-0088-000000000001'::uuid),
    ('00000000-0000-0000-00c4-000000000001'::uuid, '00000000-0000-0000-00bc-000000000099'::uuid, '00000000-0000-0000-0088-000000000002'::uuid),
    ('00000000-0000-0000-00c4-000000000002'::uuid, '00000000-0000-0000-00bc-000000000099'::uuid, '00000000-0000-0000-0088-000000000002'::uuid),
    ('00000000-0000-0000-00c5-000000000001'::uuid, '00000000-0000-0000-00bc-000000000002'::uuid, '00000000-0000-0000-0088-000000000001'::uuid)
) AS r(rid, ctx, owner)
ON CONFLICT (resource_id) DO NOTHING;

-- ─── doc_type property (graph_subgraph_nodes reads property_value #>> '{}') ──
INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, asserted_by_event_id, last_event_id)
SELECT 'kb_resources', d.rid, 'doc_type', to_jsonb(d.dt),
       '00000000-0000-0000-00ee-000000000001', '00000000-0000-0000-00ee-000000000001'
FROM (VALUES
    ('00000000-0000-0000-00c1-000000000001'::uuid, 'concept'),
    ('00000000-0000-0000-00c1-000000000002'::uuid, 'concept'),
    ('00000000-0000-0000-00c1-000000000003'::uuid, 'concept'),
    ('00000000-0000-0000-00c1-000000000004'::uuid, 'concept'),
    ('00000000-0000-0000-00c1-000000000005'::uuid, 'concept'),
    ('00000000-0000-0000-00c2-000000000001'::uuid, 'research'),
    ('00000000-0000-0000-00c2-000000000002'::uuid, 'task'),
    ('00000000-0000-0000-00c2-000000000003'::uuid, 'session'),
    ('00000000-0000-0000-00c2-000000000004'::uuid, 'research'),
    ('00000000-0000-0000-00c2-000000000005'::uuid, 'task'),
    ('00000000-0000-0000-00c2-000000000006'::uuid, 'research'),
    ('00000000-0000-0000-00c3-000000000001'::uuid, 'research'),
    ('00000000-0000-0000-00c3-000000000002'::uuid, 'research'),
    ('00000000-0000-0000-00c4-000000000001'::uuid, 'concept'),
    ('00000000-0000-0000-00c4-000000000002'::uuid, 'research'),
    ('00000000-0000-0000-00c5-000000000001'::uuid, 'concept')
) AS d(rid, dt)
ON CONFLICT DO NOTHING;

-- m2 (task) carries temper-stage; non-task doctypes never surface stage even if set.
INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, asserted_by_event_id, last_event_id)
VALUES ('kb_resources', '00000000-0000-0000-00c2-000000000002', 'temper-stage', '"in-progress"'::jsonb,
        '00000000-0000-0000-00ee-000000000001', '00000000-0000-0000-00ee-000000000001')
ON CONFLICT DO NOTHING;

-- ─── Edges (homed in graph-test-primary) ────────────────────────────────────
--   c1→m1,m2,m3 (relates_to/near/forward); c2→m4,m5,m1; c4→m6; m6→t1, t1→t2 (depends_on/leads_to/inverse);
--   m2→b2 (references/near/forward — cross-owner leak attempt, dropped by the visibility gate).
INSERT INTO kb_edges (id, source_table, source_id, target_table, target_id, edge_kind, polarity, label, weight,
                      home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id, is_folded)
SELECT e.eid, 'kb_resources', e.src, 'kb_resources', e.tgt, e.kind::edge_kind, e.pol::edge_polarity, e.lbl, 1.0,
       'kb_contexts', '00000000-0000-0000-00bc-000000000001',
       '00000000-0000-0000-00ee-000000000001', '00000000-0000-0000-00ee-000000000001', false
FROM (VALUES
    ('00000000-0000-0000-00e0-000000000001'::uuid, '00000000-0000-0000-00c1-000000000001'::uuid, '00000000-0000-0000-00c2-000000000001'::uuid, 'near',     'forward', 'relates_to'),
    ('00000000-0000-0000-00e0-000000000002'::uuid, '00000000-0000-0000-00c1-000000000001'::uuid, '00000000-0000-0000-00c2-000000000002'::uuid, 'near',     'forward', 'relates_to'),
    ('00000000-0000-0000-00e0-000000000003'::uuid, '00000000-0000-0000-00c1-000000000001'::uuid, '00000000-0000-0000-00c2-000000000003'::uuid, 'near',     'forward', 'relates_to'),
    ('00000000-0000-0000-00e0-000000000004'::uuid, '00000000-0000-0000-00c1-000000000002'::uuid, '00000000-0000-0000-00c2-000000000004'::uuid, 'near',     'forward', 'relates_to'),
    ('00000000-0000-0000-00e0-000000000005'::uuid, '00000000-0000-0000-00c1-000000000002'::uuid, '00000000-0000-0000-00c2-000000000005'::uuid, 'near',     'forward', 'relates_to'),
    ('00000000-0000-0000-00e0-000000000006'::uuid, '00000000-0000-0000-00c1-000000000002'::uuid, '00000000-0000-0000-00c2-000000000001'::uuid, 'near',     'forward', 'relates_to'),
    ('00000000-0000-0000-00e0-000000000007'::uuid, '00000000-0000-0000-00c1-000000000004'::uuid, '00000000-0000-0000-00c2-000000000006'::uuid, 'near',     'forward', 'relates_to'),
    ('00000000-0000-0000-00e0-000000000008'::uuid, '00000000-0000-0000-00c2-000000000006'::uuid, '00000000-0000-0000-00c3-000000000001'::uuid, 'leads_to', 'inverse', 'depends_on'),
    ('00000000-0000-0000-00e0-000000000009'::uuid, '00000000-0000-0000-00c3-000000000001'::uuid, '00000000-0000-0000-00c3-000000000002'::uuid, 'leads_to', 'inverse', 'depends_on'),
    ('00000000-0000-0000-00e1-000000000001'::uuid, '00000000-0000-0000-00c2-000000000002'::uuid, '00000000-0000-0000-00c4-000000000002'::uuid, 'near',     'forward', 'references')
) AS e(eid, src, tgt, kind, pol, lbl)
ON CONFLICT DO NOTHING;

-- ─── Body chunks for excerpt derivation (c1, m1) ────────────────────────────
INSERT INTO kb_content_blocks (id, resource_id, seq, is_folded, genesis_event_id, last_event_id) VALUES
    ('00000000-0000-0000-00cb-000000000001', '00000000-0000-0000-00c1-000000000001', 0, false, '00000000-0000-0000-00ee-000000000001', '00000000-0000-0000-00ee-000000000001'),
    ('00000000-0000-0000-00cb-000000000002', '00000000-0000-0000-00c2-000000000001', 0, false, '00000000-0000-0000-00ee-000000000001', '00000000-0000-0000-00ee-000000000001')
ON CONFLICT (id) DO NOTHING;

INSERT INTO kb_chunks (id, block_id, resource_id, chunk_index, version, header_path, heading_depth, content_hash, embedding, is_current) VALUES
    ('00000000-0000-0000-00cc-000000000001', '00000000-0000-0000-00cb-000000000001', '00000000-0000-0000-00c1-000000000001',
     0, 1, '', 0, md5('c1-body'), ('[' || repeat('0,', 767) || '0]')::vector, true),
    ('00000000-0000-0000-00cc-000000000002', '00000000-0000-0000-00cb-000000000002', '00000000-0000-0000-00c2-000000000001',
     0, 1, '', 0, md5('m1-body'), ('[' || repeat('0,', 767) || '0]')::vector, true)
ON CONFLICT (id) DO NOTHING;

INSERT INTO kb_chunk_content (chunk_id, content) VALUES
    ('00000000-0000-0000-00cc-000000000001',
     'Idempotency keys let retries be safe.' || E'\n\n' ||
     'Further discussion of retry semantics lives in the follow-up chunk and is intentionally not part of the excerpt.'),
    ('00000000-0000-0000-00cc-000000000002',
     repeat('OAuth comparison notes repeating for word-boundary truncation coverage. ', 8))
ON CONFLICT (chunk_id) DO UPDATE SET content = EXCLUDED.content;

COMMIT;
