-- Event substrate v1 bootstrap rows.
-- Deterministic UUIDv7 values: timestamp 2026-05-18T00:00:00Z (ms = 1779062400000
-- = 0x019E3D6F2300) followed by version nibble 7 and zero-padded random bytes.
-- Each id increments the last 4 hex chars so they remain v7-sortable.

INSERT INTO event_substrate.event_types (id, name, description) VALUES
    ('019e3d6f-2300-7000-8000-000000000001', 'ConceptCreated',
     'Genesis event for a concept; emits initial characterization, scope, topic, and optional derived-from references.'),
    ('019e3d6f-2300-7000-8000-000000000002', 'ConceptMutated',
     'Refinement or correction of an existing concept; must reference the prior event via a single Supersedes reference.');

INSERT INTO event_substrate.scopes (id, name, porosity) VALUES
    ('019e3d6f-2300-7000-8000-000000000010', 'public', 'access');

INSERT INTO event_substrate.profiles (id, name) VALUES
    ('019e3d6f-2300-7000-8000-000000000020', 'system');

INSERT INTO event_substrate.entities (id, profile_id, name) VALUES
    ('019e3d6f-2300-7000-8000-000000000030',
     '019e3d6f-2300-7000-8000-000000000020',
     'system-bootstrap');

INSERT INTO event_substrate.topics (id, fqdn) VALUES
    ('019e3d6f-2300-7000-8000-000000000040', 'event_substrate.bootstrap');
