-- migrations/20260406000002_insert_event_and_audit.sql

-- ─── 1. Fix action column type + add index ──────────────────────────────────

ALTER TABLE kb_resource_audits
    ALTER COLUMN action TYPE VARCHAR(64);

CREATE INDEX idx_resource_audits_action
    ON kb_resource_audits(action);

-- ─── 2. Atomic event + audit insertion function ─────────────────────────────

CREATE OR REPLACE FUNCTION insert_event_and_audit(
    p_event_id       UUID,
    p_profile_id     UUID,
    p_device_id      VARCHAR(64),
    p_context_id     UUID,
    p_resource_id    UUID,
    p_event_type     VARCHAR(64),
    p_action         VARCHAR(64),
    p_body_hash      TEXT,
    p_managed_hash   TEXT,
    p_open_hash      TEXT
) RETURNS TABLE(event_id UUID, audit_id UUID)
LANGUAGE plpgsql AS $$
DECLARE
    v_audit_id UUID;
BEGIN
    -- Insert event row
    INSERT INTO kb_events (id, profile_id, device_id, kb_context_id, resource_id, event_type, payload, created)
    VALUES (
        p_event_id,
        p_profile_id,
        p_device_id,
        p_context_id,
        p_resource_id,
        p_event_type,
        jsonb_build_object(
            'body_hash', p_body_hash,
            'managed_hash', p_managed_hash,
            'open_hash', p_open_hash
        ),
        now()
    );

    -- Insert audit row
    INSERT INTO kb_resource_audits (resource_id, event_id, profile_id, device_id, body_hash, managed_hash, open_hash, action)
    VALUES (p_resource_id, p_event_id, p_profile_id, p_device_id, p_body_hash, p_managed_hash, p_open_hash, p_action)
    RETURNING id INTO v_audit_id;

    RETURN QUERY SELECT p_event_id, v_audit_id;
END;
$$;
