-- Event ledger unification — limb 0 of the event-primary reorientation.
-- Drops the test-only `event_substrate` schema and evolves `kb_events`
-- into one disciplined, append-only, registry-backed ledger in `public`.
-- Spec: docs/superpowers/specs/2026-05-21-event-ledger-unification-design.md

-- ─── 1. Drop the test-only event_substrate schema ───────────────────────────
-- Never emitted into in production; a clean drop, not a data migration.
DROP SCHEMA IF EXISTS event_substrate CASCADE;

-- ─── 2. Ledger taxonomy tables ──────────────────────────────────────────────

CREATE TABLE kb_event_types (
    id            UUID PRIMARY KEY DEFAULT public.uuid_generate_v7(),
    name          VARCHAR(128) NOT NULL UNIQUE,
    description   TEXT,
    is_deprecated BOOLEAN NOT NULL DEFAULT false,
    created       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE kb_topics (
    id        UUID PRIMARY KEY DEFAULT public.uuid_generate_v7(),
    fqdn      TEXT NOT NULL UNIQUE,
    parent_id UUID REFERENCES kb_topics(id),
    created   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TYPE porosity AS ENUM ('access', 'attention');

CREATE TABLE kb_scopes (
    id       UUID PRIMARY KEY DEFAULT public.uuid_generate_v7(),
    name     TEXT NOT NULL UNIQUE,
    porosity porosity NOT NULL,
    created  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ─── 3. Seed the event-type registry ────────────────────────────────────────
-- The event-type names emitted today, plus ConceptCreated/ConceptMutated
-- carried over from event_substrate as harmless registry rows (the concept
-- projection itself is limb 2).
INSERT INTO kb_event_types (name, description) VALUES
    ('resource_created',       'A resource was created.'),
    ('body_updated',           'A resource body was updated.'),
    ('managed_meta_updated',   'A resource''s managed/open frontmatter was updated.'),
    ('resource_deleted',       'A resource was soft-deleted.'),
    ('context_created',        'A context was created.'),
    ('join_request.submitted', 'A team join request was submitted.'),
    ('join_request.approved',  'A team join request was approved.'),
    ('ConceptCreated',         'Genesis event for a concept (projection is limb 2).'),
    ('ConceptMutated',         'Refinement of an existing concept (projection is limb 2).')
ON CONFLICT (name) DO NOTHING;

-- Catch any event-type string present in live data but not enumerated above.
INSERT INTO kb_event_types (name)
SELECT DISTINCT event_type FROM kb_events
ON CONFLICT (name) DO NOTHING;

-- ─── 4. Seed a root topic and a default scope ───────────────────────────────
-- Deterministic UUIDv7 ids so test fixtures can reference them by constant.
INSERT INTO kb_topics (id, fqdn) VALUES
    ('019e3d6f-2300-7000-8000-000000000040', 'temper.bootstrap');
INSERT INTO kb_scopes (id, name, porosity) VALUES
    ('019e3d6f-2300-7000-8000-000000000010', 'public', 'access');

-- ─── 5. Evolve kb_events into the disciplined ledger ────────────────────────

ALTER TABLE kb_events
    ADD COLUMN event_type_id  UUID REFERENCES kb_event_types(id),
    ADD COLUMN topic_id       UUID REFERENCES kb_topics(id),
    ADD COLUMN scope_id       UUID REFERENCES kb_scopes(id),
    ADD COLUMN metadata       JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN "references"   JSONB NOT NULL DEFAULT '[]'::jsonb,
    ADD COLUMN correlation_id UUID,
    ADD COLUMN occurred_at    TIMESTAMPTZ NOT NULL DEFAULT now();

-- Backfill: map the legacy varchar type to its registry id; occurred_at
-- mirrors the recorded time for historical rows.
UPDATE kb_events e
   SET event_type_id = et.id
  FROM kb_event_types et
 WHERE et.name = e.event_type;
UPDATE kb_events SET occurred_at = created;

ALTER TABLE kb_events ALTER COLUMN event_type_id SET NOT NULL;
ALTER TABLE kb_events DROP COLUMN event_type;

-- `created` is the recorded-at (commit) time. Give it a default so the
-- ledger-appended write path (temper-events `append_event`) need not
-- supply it — only the legacy `insert_event_and_audit` path passed it
-- explicitly, and that still works unchanged.
ALTER TABLE kb_events ALTER COLUMN created SET DEFAULT now();

CREATE INDEX idx_kb_events_event_type  ON kb_events(event_type_id);
CREATE INDEX idx_kb_events_topic       ON kb_events(topic_id) WHERE topic_id IS NOT NULL;
CREATE INDEX idx_kb_events_correlation ON kb_events(correlation_id) WHERE correlation_id IS NOT NULL;
CREATE INDEX idx_kb_events_references  ON kb_events USING GIN ("references" jsonb_path_ops);

-- ─── 6. Append-only enforcement ─────────────────────────────────────────────
-- Supersession and correction are themselves events; the ledger row is final.
CREATE FUNCTION kb_events_append_only() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'event ledger is append-only';
END;
$$;

CREATE TRIGGER kb_events_append_only
    BEFORE UPDATE OR DELETE ON kb_events
    FOR EACH ROW EXECUTE FUNCTION kb_events_append_only();

-- ─── 7. resolve_event_type — insert-or-get against the registry ─────────────
-- Used by insert_event_and_audit and by the direct-INSERT sites. Insert-or-get
-- keeps the resource-write path non-breaking: an unrecognised type name
-- auto-registers rather than erroring. Strict rejection of unknown types is
-- the temper-events `append_event` path's job, not this one's.
CREATE FUNCTION resolve_event_type(p_name VARCHAR) RETURNS UUID
LANGUAGE plpgsql AS $$
DECLARE
    v_id UUID;
BEGIN
    INSERT INTO kb_event_types (name) VALUES (p_name)
    ON CONFLICT (name) DO NOTHING;
    SELECT id INTO v_id FROM kb_event_types WHERE name = p_name;
    RETURN v_id;
END;
$$;

-- ─── 8. Evolve insert_event_and_audit to write event_type_id ────────────────
-- Signature is IDENTICAL to migration 20260521000001 — every Rust and
-- TypeScript caller is untouched. Only the body changes: resolve the type
-- name to an id, and insert event_type_id instead of the dropped column.
DROP FUNCTION IF EXISTS insert_event_and_audit(
    UUID, UUID, VARCHAR, UUID, UUID, VARCHAR, VARCHAR, TEXT, TEXT, TEXT, JSONB
);

CREATE FUNCTION insert_event_and_audit(
    p_event_id       UUID,
    p_profile_id     UUID,
    p_device_id      VARCHAR(64),
    p_context_id     UUID,
    p_resource_id    UUID,
    p_event_type     VARCHAR(64),
    p_action         VARCHAR(64),
    p_body_hash      TEXT,
    p_managed_hash   TEXT,
    p_open_hash      TEXT,
    p_payload_extra  JSONB DEFAULT '{}'::jsonb
) RETURNS TABLE(event_id UUID, audit_id UUID)
LANGUAGE plpgsql AS $$
DECLARE
    v_audit_id      UUID;
    v_event_type_id UUID;
BEGIN
    v_event_type_id := resolve_event_type(p_event_type);

    INSERT INTO kb_events (
        id, profile_id, device_id, kb_context_id, resource_id,
        event_type_id, payload, created
    )
    VALUES (
        p_event_id, p_profile_id, p_device_id, p_context_id, p_resource_id,
        v_event_type_id,
        jsonb_build_object(
            'body_hash', p_body_hash,
            'managed_hash', p_managed_hash,
            'open_hash', p_open_hash
        ) || COALESCE(p_payload_extra, '{}'::jsonb),
        now()
    );

    INSERT INTO kb_resource_audits (
        resource_id, event_id, profile_id, device_id,
        body_hash, managed_hash, open_hash, action
    )
    VALUES (
        p_resource_id, p_event_id, p_profile_id, p_device_id,
        p_body_hash, p_managed_hash, p_open_hash, p_action
    )
    RETURNING id INTO v_audit_id;

    RETURN QUERY SELECT p_event_id, v_audit_id;
END;
$$;
