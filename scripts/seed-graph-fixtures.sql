-- seed-graph-fixtures.sql
--
-- Integration-test fixtures for graph_service tests. Designed to be loaded
-- after crates/temper-api/tests/common/fixtures::clean_and_seed which strips
-- test-generated data. All UUIDs are well-known so Rust tests can reference
-- resources by constant.
--
-- Owner layout:
--   Alice (caller) — primary test profile
--   Bob            — secondary profile, used for cross-owner leak checks
--
-- Context layout:
--   graph-test-primary   — main context under test
--   graph-test-secondary — second context, used for multi-context isolation
--
-- Scenario IDs embedded in UUID ranges:
--   00c1 — concepts (aggregator nodes)
--   00c2 — direct members (tier 2)
--   00c3 — tier-3 nodes
--   00c4 — bob-owned resources
--   00c5 — resources in the secondary context

BEGIN;

-- ─── Profiles ──────────────────────────────────────────────────────────────

INSERT INTO kb_profiles (id, display_name, email, slug) VALUES
    ('00000000-0000-0000-0088-000000000001', 'Alice (graph-test)', 'alice-graph@test.com', 'alice-graph'),
    ('00000000-0000-0000-0088-000000000002', 'Bob (graph-test)',   'bob-graph@test.com',   'bob-graph')
ON CONFLICT (id) DO UPDATE SET display_name = EXCLUDED.display_name;

INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id) VALUES
    ('00000000-0000-0000-0088-000000000101', '00000000-0000-0000-0088-000000000001', 'test-provider', 'test|alice-graph'),
    ('00000000-0000-0000-0088-000000000102', '00000000-0000-0000-0088-000000000002', 'test-provider', 'test|bob-graph')
ON CONFLICT DO NOTHING;

-- ─── Contexts (owned by Alice) ─────────────────────────────────────────────

INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id) VALUES
    ('00000000-0000-0000-00bc-000000000001', 'graph-test-primary',   'kb_profiles', '00000000-0000-0000-0088-000000000001'),
    ('00000000-0000-0000-00bc-000000000002', 'graph-test-secondary', 'kb_profiles', '00000000-0000-0000-0088-000000000001')
ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name;

-- Bob owns his own primary context (needed so Bob's resources resolve through
-- the visibility function for his profile)
INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id) VALUES
    ('00000000-0000-0000-00bc-000000000099', 'bob-context', 'kb_profiles', '00000000-0000-0000-0088-000000000002')
ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name;

-- ─── Doc-type ID convenience (from migrations) ─────────────────────────────
-- 00000000-0000-0000-0001-000000000002 — session
-- 00000000-0000-0000-0001-000000000004 — research
-- 00000000-0000-0000-0001-000000000006 — concept
-- 00000000-0000-0000-0001-000000000008 — task

-- ─── Resources ─────────────────────────────────────────────────────────────
-- Helper macro-like INSERT for brevity. Each row: alice-owned unless noted.

-- Concepts (aggregators) in primary context
INSERT INTO kb_resources (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
    originator_profile_id, owner_profile_id, is_active, created, updated) VALUES

    -- c1: happy-path concept with 3 members
    ('00000000-0000-0000-00c1-000000000001', '00000000-0000-0000-00bc-000000000001',
     '00000000-0000-0000-0001-000000000006', 'test://concept-idempotency',
     'Idempotency Keys', 'idempotency-keys',
     '00000000-0000-0000-0088-000000000001', '00000000-0000-0000-0088-000000000001',
     true, now(), now()),

    -- c2: concept sharing one member with c1 (diamond overlap)
    ('00000000-0000-0000-00c1-000000000002', '00000000-0000-0000-00bc-000000000001',
     '00000000-0000-0000-0001-000000000006', 'test://concept-circuit-breakers',
     'Circuit Breakers', 'circuit-breakers',
     '00000000-0000-0000-0088-000000000001', '00000000-0000-0000-0088-000000000001',
     true, now(), now()),

    -- c3: singleton concept — no member edges
    ('00000000-0000-0000-00c1-000000000003', '00000000-0000-0000-00bc-000000000001',
     '00000000-0000-0000-0001-000000000006', 'test://concept-zero-copy',
     'Zero-Copy Patterns', 'zero-copy-patterns',
     '00000000-0000-0000-0088-000000000001', '00000000-0000-0000-0088-000000000001',
     true, now(), now()),

    -- c4: concept with a member that chains out to tier-3 and tier-4
    ('00000000-0000-0000-00c1-000000000004', '00000000-0000-0000-00bc-000000000001',
     '00000000-0000-0000-0001-000000000006', 'test://concept-auth-patterns',
     'Auth Patterns', 'auth-patterns',
     '00000000-0000-0000-0088-000000000001', '00000000-0000-0000-0088-000000000001',
     true, now(), now()),

    -- c5: soft-deleted concept — must NOT appear in results
    ('00000000-0000-0000-00c1-000000000005', '00000000-0000-0000-00bc-000000000001',
     '00000000-0000-0000-0001-000000000006', 'test://concept-deleted',
     'Deleted Concept', 'deleted-concept',
     '00000000-0000-0000-0088-000000000001', '00000000-0000-0000-0088-000000000001',
     false, now(), now())

ON CONFLICT (id) DO UPDATE SET is_active = EXCLUDED.is_active, title = EXCLUDED.title;

-- Direct members (tier 2)
INSERT INTO kb_resources (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
    originator_profile_id, owner_profile_id, is_active, created, updated) VALUES

    -- m1: research — member of c1 AND c4 (shared → diamond overlap target)
    ('00000000-0000-0000-00c2-000000000001', '00000000-0000-0000-00bc-000000000001',
     '00000000-0000-0000-0001-000000000004', 'test://m1-oauth-comparison',
     'OAuth Comparison', 'oauth-comparison',
     '00000000-0000-0000-0088-000000000001', '00000000-0000-0000-0088-000000000001',
     true, now(), now()),

    -- m2: task — member of c1 only
    ('00000000-0000-0000-00c2-000000000002', '00000000-0000-0000-00bc-000000000001',
     '00000000-0000-0000-0001-000000000008', 'test://m2-auth-middleware',
     'Auth Middleware', 'auth-middleware',
     '00000000-0000-0000-0088-000000000001', '00000000-0000-0000-0088-000000000001',
     true, now(), now()),

    -- m3: session — member of c1 only
    ('00000000-0000-0000-00c2-000000000003', '00000000-0000-0000-00bc-000000000001',
     '00000000-0000-0000-0001-000000000002', 'test://m3-auth-debug',
     'Auth Debug Session', 'auth-debug-session',
     '00000000-0000-0000-0088-000000000001', '00000000-0000-0000-0088-000000000001',
     true, now(), now()),

    -- m4: research — member of c2 only
    ('00000000-0000-0000-00c2-000000000004', '00000000-0000-0000-00bc-000000000001',
     '00000000-0000-0000-0001-000000000004', 'test://m4-circuit-design',
     'Circuit Breaker Design', 'circuit-breaker-design',
     '00000000-0000-0000-0088-000000000001', '00000000-0000-0000-0088-000000000001',
     true, now(), now()),

    -- m5: task — member of c2 only
    ('00000000-0000-0000-00c2-000000000005', '00000000-0000-0000-00bc-000000000001',
     '00000000-0000-0000-0001-000000000008', 'test://m5-circuit-impl',
     'Circuit Breaker Implementation', 'circuit-breaker-implementation',
     '00000000-0000-0000-0088-000000000001', '00000000-0000-0000-0088-000000000001',
     true, now(), now()),

    -- m6: research — member of c4; also chain link to tier-3 and tier-4
    ('00000000-0000-0000-00c2-000000000006', '00000000-0000-0000-00bc-000000000001',
     '00000000-0000-0000-0001-000000000004', 'test://m6-jwt-strategies',
     'JWT Strategies', 'jwt-strategies',
     '00000000-0000-0000-0088-000000000001', '00000000-0000-0000-0088-000000000001',
     true, now(), now())

ON CONFLICT (id) DO UPDATE SET is_active = EXCLUDED.is_active;

-- Tier-3 / tier-4 nodes
INSERT INTO kb_resources (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
    originator_profile_id, owner_profile_id, is_active, created, updated) VALUES

    -- t1: tier-3 — reachable from c4 via m6 → t1 (depth 2)
    ('00000000-0000-0000-00c3-000000000001', '00000000-0000-0000-00bc-000000000001',
     '00000000-0000-0000-0001-000000000004', 'test://t1-token-refresh',
     'Token Refresh Patterns', 'token-refresh-patterns',
     '00000000-0000-0000-0088-000000000001', '00000000-0000-0000-0088-000000000001',
     true, now(), now()),

    -- t2: tier-4 — reachable only at depth 3 (c4 → m6 → t1 → t2)
    -- Must NOT appear in a depth-2 result.
    ('00000000-0000-0000-00c3-000000000002', '00000000-0000-0000-00bc-000000000001',
     '00000000-0000-0000-0001-000000000004', 'test://t2-session-mgmt',
     'Session Management Research', 'session-management-research',
     '00000000-0000-0000-0088-000000000001', '00000000-0000-0000-0088-000000000001',
     true, now(), now())

ON CONFLICT (id) DO UPDATE SET is_active = EXCLUDED.is_active;

-- Bob-owned (cross-owner leak targets)
INSERT INTO kb_resources (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
    originator_profile_id, owner_profile_id, is_active, created, updated) VALUES

    -- b1: bob's concept, in bob's own context — must not appear for alice
    ('00000000-0000-0000-00c4-000000000001', '00000000-0000-0000-00bc-000000000099',
     '00000000-0000-0000-0001-000000000006', 'test://b1-bob-concept',
     'Bob Secret Concept', 'bob-secret-concept',
     '00000000-0000-0000-0088-000000000002', '00000000-0000-0000-0088-000000000002',
     true, now(), now()),

    -- b2: bob-owned resource referenced by an alice edge (leak attempt)
    ('00000000-0000-0000-00c4-000000000002', '00000000-0000-0000-00bc-000000000099',
     '00000000-0000-0000-0001-000000000004', 'test://b2-bob-research',
     'Bob Private Research', 'bob-private-research',
     '00000000-0000-0000-0088-000000000002', '00000000-0000-0000-0088-000000000002',
     true, now(), now())

ON CONFLICT (id) DO UPDATE SET is_active = EXCLUDED.is_active;

-- Secondary-context resource (multi-context isolation)
INSERT INTO kb_resources (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
    originator_profile_id, owner_profile_id, is_active, created, updated) VALUES

    -- s1: alice-owned concept in secondary context — must NOT appear when
    -- querying primary context
    ('00000000-0000-0000-00c5-000000000001', '00000000-0000-0000-00bc-000000000002',
     '00000000-0000-0000-0001-000000000006', 'test://s1-secondary-concept',
     'Secondary Context Concept', 'secondary-context-concept',
     '00000000-0000-0000-0088-000000000001', '00000000-0000-0000-0088-000000000001',
     true, now(), now())

ON CONFLICT (id) DO UPDATE SET is_active = EXCLUDED.is_active;

-- ─── Edges ─────────────────────────────────────────────────────────────────
-- Edges are projections of `relationship_asserted` events. We synthesize
-- the assertion event then insert the projected row, mirroring the
-- migration's genesis-event pattern. Each (source, target, kind, label,
-- polarity) is unique by the table constraint.
--
-- 8->4 mapping for the labels used here:
--   relates_to   → near    / forward
--   depends_on   → leads_to / inverse
--   references   → near    / forward
--   parent_of    → contains / forward

DO $$
DECLARE
    v_asserted_type uuid := (SELECT id FROM kb_event_types WHERE name = 'relationship_asserted');
    v_pid uuid := '00000000-0000-0000-0088-000000000001';
    v_topic uuid := '019e3d6f-2300-7000-8000-000000000050';
    v_scope uuid := '019e3d6f-2300-7000-8000-000000000010';

    edge_ids   uuid[] := ARRAY[
        '00000000-0000-0000-00e0-000000000001'::uuid,
        '00000000-0000-0000-00e0-000000000002'::uuid,
        '00000000-0000-0000-00e0-000000000003'::uuid,
        '00000000-0000-0000-00e0-000000000004'::uuid,
        '00000000-0000-0000-00e0-000000000005'::uuid,
        '00000000-0000-0000-00e0-000000000006'::uuid,
        '00000000-0000-0000-00e0-000000000007'::uuid,
        '00000000-0000-0000-00e0-000000000008'::uuid,
        '00000000-0000-0000-00e0-000000000009'::uuid,
        '00000000-0000-0000-00e1-000000000001'::uuid
    ];
    source_ids uuid[] := ARRAY[
        '00000000-0000-0000-00c1-000000000001'::uuid,
        '00000000-0000-0000-00c1-000000000001'::uuid,
        '00000000-0000-0000-00c1-000000000001'::uuid,
        '00000000-0000-0000-00c1-000000000002'::uuid,
        '00000000-0000-0000-00c1-000000000002'::uuid,
        '00000000-0000-0000-00c1-000000000002'::uuid,
        '00000000-0000-0000-00c1-000000000004'::uuid,
        '00000000-0000-0000-00c2-000000000006'::uuid,
        '00000000-0000-0000-00c3-000000000001'::uuid,
        '00000000-0000-0000-00c2-000000000002'::uuid
    ];
    target_ids uuid[] := ARRAY[
        '00000000-0000-0000-00c2-000000000001'::uuid,
        '00000000-0000-0000-00c2-000000000002'::uuid,
        '00000000-0000-0000-00c2-000000000003'::uuid,
        '00000000-0000-0000-00c2-000000000004'::uuid,
        '00000000-0000-0000-00c2-000000000005'::uuid,
        '00000000-0000-0000-00c2-000000000001'::uuid,
        '00000000-0000-0000-00c2-000000000006'::uuid,
        '00000000-0000-0000-00c3-000000000001'::uuid,
        '00000000-0000-0000-00c3-000000000002'::uuid,
        '00000000-0000-0000-00c4-000000000002'::uuid
    ];
    kinds      text[] := ARRAY['near','near','near','near','near','near','near','leads_to','leads_to','near'];
    polarities text[] := ARRAY['forward','forward','forward','forward','forward','forward','forward','inverse','inverse','forward'];
    labels     text[] := ARRAY['relates_to','relates_to','relates_to','relates_to','relates_to','relates_to','relates_to','depends_on','depends_on','references'];

    v_event_id uuid;
BEGIN
    FOR i IN 1 .. array_length(edge_ids, 1) LOOP
        v_event_id := public.uuid_generate_v7();

        INSERT INTO kb_events (
            id, event_type_id, profile_id, device_id, topic_id, scope_id,
            payload, metadata, "references", correlation_id, occurred_at, created
        ) VALUES (
            v_event_id, v_asserted_type, v_pid, 'fixture', v_topic, v_scope,
            jsonb_build_object(
                'source_resource_id', source_ids[i],
                'target', jsonb_build_object('kind', 'resource', 'value', target_ids[i]),
                'edge_kind', kinds[i],
                'polarity',  polarities[i],
                'label',     labels[i],
                'weight',    1.0
            ),
            jsonb_build_object('source', 'fixture'),
            '[]'::jsonb, v_event_id, now(), now()
        )
        ON CONFLICT (id) DO NOTHING;

        INSERT INTO kb_resource_edges (
            id, source_resource_id, target_resource_id,
            edge_kind, polarity, label, weight,
            asserted_by_event_id, last_event_id, is_folded
        ) VALUES (
            edge_ids[i], source_ids[i], target_ids[i],
            kinds[i]::edge_kind, polarities[i]::edge_polarity, labels[i], 1.0,
            v_event_id, v_event_id, false
        )
        ON CONFLICT DO NOTHING;
    END LOOP;
END $$;

-- ─── Body chunks for excerpt derivation ────────────────────────────────────
-- Only a handful of resources get seeded content — enough to exercise the
-- peek-panel excerpt path without blowing up unrelated edge-count assertions.
-- Embedding is a zero vector(768); semantic search is out of scope here.
--
-- kb_chunks.first_revision_id is NOT NULL; seed one revision per resource
-- (audit_id = NULL since this is synthetic fixture data, not an actual write).
INSERT INTO kb_resource_revisions (id, resource_id, audit_id, body_hash, chunk_count)
VALUES
    ('00000000-0000-0000-00cd-000000000001', '00000000-0000-0000-00c1-000000000001',
     NULL, md5('c1-body'), 1),
    ('00000000-0000-0000-00cd-000000000002', '00000000-0000-0000-00c2-000000000001',
     NULL, md5('m1-body'), 1)
ON CONFLICT (id) DO NOTHING;

INSERT INTO kb_chunks
    (id, resource_id, chunk_index, version, header_path, heading_depth,
     content_hash, embedding, is_current, first_revision_id)
VALUES
    -- c1 body: a multi-paragraph preamble. Excerpt = first paragraph only.
    ('00000000-0000-0000-00cc-000000000001', '00000000-0000-0000-00c1-000000000001',
     0, 1, '', 0,
     md5('c1-body'),
     ('[' || repeat('0,', 767) || '0]')::vector,
     true,
     '00000000-0000-0000-00cd-000000000001'),
    -- m1 body: a single paragraph longer than 280 chars — exercises truncation.
    ('00000000-0000-0000-00cc-000000000002', '00000000-0000-0000-00c2-000000000001',
     0, 1, '', 0,
     md5('m1-body'),
     ('[' || repeat('0,', 767) || '0]')::vector,
     true,
     '00000000-0000-0000-00cd-000000000002')
ON CONFLICT (id) DO NOTHING;

INSERT INTO kb_chunk_content (chunk_id, content) VALUES
    ('00000000-0000-0000-00cc-000000000001',
     'Idempotency keys let retries be safe.' || E'\n\n' ||
     'Further discussion of retry semantics lives in the follow-up chunk and is intentionally not part of the excerpt.'),
    ('00000000-0000-0000-00cc-000000000002',
     repeat('OAuth comparison notes repeating for word-boundary truncation coverage. ', 8))
ON CONFLICT (chunk_id) DO UPDATE SET content = EXCLUDED.content;

-- ─── Manifest rows (task stage lives here) ─────────────────────────────────
-- m2 is a task carrying temper-stage=in-progress so the detail-tier stage
-- tag path is covered. m1 (research) also gets a manifest but with no stage,
-- proving that non-task doctypes never surface stage even if it were set.
INSERT INTO kb_resource_manifests (resource_id, managed_meta) VALUES
    ('00000000-0000-0000-00c2-000000000002',
     '{"temper-type":"task","temper-stage":"in-progress"}'::jsonb),
    ('00000000-0000-0000-00c2-000000000001',
     '{"temper-type":"research"}'::jsonb)
ON CONFLICT (resource_id) DO UPDATE SET managed_meta = EXCLUDED.managed_meta;

COMMIT;
