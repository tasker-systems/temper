-- Add missing doc_types for knowledge base migration.
-- These align with temper's preferred nomenclature: task, goal, resource.
INSERT INTO kb_doc_types (id, name) VALUES
    ('00000000-0000-0000-0001-000000000008', 'task'),
    ('00000000-0000-0000-0001-000000000009', 'goal'),
    ('00000000-0000-0000-0001-00000000000a', 'resource')
ON CONFLICT (name) DO NOTHING;
