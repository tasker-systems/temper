-- R2: Seed data — behaviors, doc types, compositions, lifecycle stages, contexts, profiles
-- Deterministic UUIDs for R2 validation only — production uses Rust-based seeder with UUIDv7

INSERT INTO kb_behaviors (id, name) VALUES
    ('00000000-0000-0000-0000-000000000001', 'workflowable'),
    ('00000000-0000-0000-0000-000000000002', 'sequenceable'),
    ('00000000-0000-0000-0000-000000000003', 'assignable'),
    ('00000000-0000-0000-0000-000000000004', 'taggable');

INSERT INTO kb_doc_types (id, name) VALUES
    ('00000000-0000-0000-0001-000000000001', 'ticket'),
    ('00000000-0000-0000-0001-000000000002', 'session'),
    ('00000000-0000-0000-0001-000000000003', 'milestone'),
    ('00000000-0000-0000-0001-000000000004', 'research'),
    ('00000000-0000-0000-0001-000000000005', 'board'),
    ('00000000-0000-0000-0001-000000000006', 'concept'),
    ('00000000-0000-0000-0001-000000000007', 'source');

-- ticket: workflowable, sequenceable, assignable, taggable
INSERT INTO kb_doc_type_behaviors (kb_doc_type_id, kb_behavior_id) VALUES
    ('00000000-0000-0000-0001-000000000001', '00000000-0000-0000-0000-000000000001'),
    ('00000000-0000-0000-0001-000000000001', '00000000-0000-0000-0000-000000000002'),
    ('00000000-0000-0000-0001-000000000001', '00000000-0000-0000-0000-000000000003'),
    ('00000000-0000-0000-0001-000000000001', '00000000-0000-0000-0000-000000000004');
-- session: taggable
INSERT INTO kb_doc_type_behaviors (kb_doc_type_id, kb_behavior_id) VALUES
    ('00000000-0000-0000-0001-000000000002', '00000000-0000-0000-0000-000000000004');
-- milestone: sequenceable
INSERT INTO kb_doc_type_behaviors (kb_doc_type_id, kb_behavior_id) VALUES
    ('00000000-0000-0000-0001-000000000003', '00000000-0000-0000-0000-000000000002');
-- research: taggable
INSERT INTO kb_doc_type_behaviors (kb_doc_type_id, kb_behavior_id) VALUES
    ('00000000-0000-0000-0001-000000000004', '00000000-0000-0000-0000-000000000004');
-- concept: taggable
INSERT INTO kb_doc_type_behaviors (kb_doc_type_id, kb_behavior_id) VALUES
    ('00000000-0000-0000-0001-000000000006', '00000000-0000-0000-0000-000000000004');
-- source: taggable
INSERT INTO kb_doc_type_behaviors (kb_doc_type_id, kb_behavior_id) VALUES
    ('00000000-0000-0000-0001-000000000007', '00000000-0000-0000-0000-000000000004');

INSERT INTO kb_lifecycle_stages (id, kb_doc_type_id, name, seq) VALUES
    ('00000000-0000-0000-0002-000000000001', '00000000-0000-0000-0001-000000000001', 'backlog', 10),
    ('00000000-0000-0000-0002-000000000002', '00000000-0000-0000-0001-000000000001', 'design', 20),
    ('00000000-0000-0000-0002-000000000003', '00000000-0000-0000-0001-000000000001', 'in-progress', 30),
    ('00000000-0000-0000-0002-000000000004', '00000000-0000-0000-0001-000000000001', 'done', 40),
    ('00000000-0000-0000-0002-000000000005', '00000000-0000-0000-0001-000000000001', 'cancelled', 50);

INSERT INTO kb_lifecycle_stages (id, kb_doc_type_id, name, seq) VALUES
    ('00000000-0000-0000-0002-000000000006', '00000000-0000-0000-0001-000000000003', 'active', 10),
    ('00000000-0000-0000-0002-000000000007', '00000000-0000-0000-0001-000000000003', 'complete', 20);

INSERT INTO kb_contexts (id, name) VALUES
    ('00000000-0000-0000-0003-000000000001', 'temper'),
    ('00000000-0000-0000-0003-000000000002', 'storyteller'),
    ('00000000-0000-0000-0003-000000000003', 'tasker'),
    ('00000000-0000-0000-0003-000000000004', 'knowledge'),
    ('00000000-0000-0000-0003-000000000005', 'writing');

INSERT INTO kb_profiles (id, provider, external_id, display_name) VALUES
    ('00000000-0000-0000-0004-000000000001', 'system', NULL, 'System'),
    ('00000000-0000-0000-0004-000000000002', 'anonymous', NULL, 'Anonymous');
