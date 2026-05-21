-- migrations/20260521000001_event_payload_extra.sql
--
-- Add a `p_payload_extra` JSONB argument to `insert_event_and_audit` so
-- event-type-specific enrichment can be merged into `kb_events.payload`
-- alongside the base `{body_hash, managed_hash, open_hash}` rollup.
--
-- Motivating case: `managed_meta_updated` events now carry the set of
-- managed/open keys that changed, so `list_events` can answer *what*
-- changed without re-deriving it by diffing snapshots. Body changes stay
-- hash-only by design — there is no key set to enumerate for a blob.
--
-- `p_payload_extra` carries a `DEFAULT '{}'::jsonb`: it is genuinely
-- optional enrichment, so callers emitting hash-only events may omit it.
-- That keeps the function callable from both the Rust services (which
-- pass it explicitly) and the temper-cloud serverless functions in
-- TypeScript (which emit only `resource_created` / `body_updated` and
-- pass 10 positional arguments).

DROP FUNCTION IF EXISTS insert_event_and_audit(
    UUID, UUID, VARCHAR, UUID, UUID, VARCHAR, VARCHAR, TEXT, TEXT, TEXT
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
    v_audit_id UUID;
BEGIN
    -- Insert event row. The base payload is the hash rollup; any
    -- event-type-specific enrichment is merged on top via `||`.
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
        ) || COALESCE(p_payload_extra, '{}'::jsonb),
        now()
    );

    -- Insert audit row
    INSERT INTO kb_resource_audits (resource_id, event_id, profile_id, device_id, body_hash, managed_hash, open_hash, action)
    VALUES (p_resource_id, p_event_id, p_profile_id, p_device_id, p_body_hash, p_managed_hash, p_open_hash, p_action)
    RETURNING id INTO v_audit_id;

    RETURN QUERY SELECT p_event_id, v_audit_id;
END;
$$;
