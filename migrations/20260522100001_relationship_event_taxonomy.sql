-- Relationship-event taxonomy — phase 1 of limb 1 (edges as event projections).
-- Spec: docs/superpowers/specs/2026-05-22-limb1-relationship-events-edge-projection-design.md
-- This migration is ADDITIVE: new enums + registry/topic rows. The breaking
-- kb_resource_edges cutover is a separate later migration.

-- ─── Structural edge-typing enums ───────────────────────────────────────────
CREATE TYPE edge_kind     AS ENUM ('express', 'contains', 'leads_to', 'near');
CREATE TYPE edge_polarity AS ENUM ('forward', 'inverse');

-- ─── Topic rows for the three framing-schema classes ────────────────────────
-- Deterministic UUIDv7 ids so fixtures can reference them by constant.
INSERT INTO kb_topics (id, fqdn) VALUES
    ('019e3d6f-2300-7000-8000-000000000050', 'declaration'),
    ('019e3d6f-2300-7000-8000-000000000051', 'deformation'),
    ('019e3d6f-2300-7000-8000-000000000052', 'judgment')
ON CONFLICT (fqdn) DO NOTHING;

-- ─── Event-type registry rows ───────────────────────────────────────────────
INSERT INTO kb_event_types (name, description) VALUES
    ('relationship_asserted',   'A knowledge-graph relationship was asserted (genesis).'),
    ('relationship_retyped',    'A relationship''s structural kind or label changed.'),
    ('relationship_reweighted', 'A relationship''s weight changed.'),
    ('relationship_folded',     'A relationship was folded — preserved, off the default projection.'),
    ('relationship_decayed',    'A relationship decayed (phase-4 mechanics).'),
    ('relationship_corrected',  'A relationship was corrected as wrong — carries a scar (phase-4 mechanics).')
ON CONFLICT (name) DO NOTHING;
