-- Fix sync_diff_for_device: return NULL resource_id for new local resources
-- that don't exist on the server yet. The previous behavior returned the
-- local manifest UUID, causing the client to use PUT (update) instead of
-- POST (create), which 404'd because the resource doesn't exist server-side.

CREATE OR REPLACE FUNCTION sync_diff_for_device(
    p_profile_id    UUID,
    p_context_names TEXT[],
    p_manifest      JSONB
) RETURNS TABLE (
    resource_id  UUID,
    kb_uri       TEXT,
    content_hash VARCHAR(64),
    updated      TIMESTAMPTZ,
    diff_type    VARCHAR(16)
)
LANGUAGE SQL STABLE AS $$
    WITH
    visible AS (
        SELECT v.resource_id FROM resources_visible_to(p_profile_id) v
    ),
    manifest_entries AS (
        SELECT
            (entry->>'uri')::TEXT AS uri,
            (split_part(entry->>'uri', '/', 5))::UUID AS extracted_resource_id,
            (entry->>'local_hash')::VARCHAR(64) AS local_hash,
            (entry->>'remote_hash')::VARCHAR(64) AS remote_hash
        FROM jsonb_array_elements(p_manifest) AS entry
    ),
    server_resources AS (
        SELECT r.id, kb_resource_uri(r.id) AS kb_uri, r.content_hash, r.updated, r.is_active
          FROM kb_resources r
          JOIN visible v ON v.resource_id = r.id
          JOIN kb_contexts c ON c.id = r.kb_context_id
         WHERE r.resource_mode = 'imported'
           AND c.name = ANY(p_context_names)
    )
    -- Server resource exists, manifest entry exists: compare hashes
    SELECT
        sr.id AS resource_id,
        sr.kb_uri,
        sr.content_hash,
        sr.updated,
        CASE
            WHEN NOT sr.is_active THEN 'removed'::VARCHAR(16)
            WHEN me.local_hash != me.remote_hash AND sr.content_hash != me.remote_hash THEN 'conflict'
            WHEN me.local_hash != me.remote_hash AND sr.content_hash = me.remote_hash THEN 'to_push'
            WHEN me.local_hash = me.remote_hash AND sr.content_hash != me.remote_hash THEN 'to_pull'
        END AS diff_type
      FROM server_resources sr
      JOIN manifest_entries me ON me.extracted_resource_id = sr.id
     WHERE sr.is_active = false
        OR me.local_hash != sr.content_hash

    UNION ALL

    -- Server resource exists but not in manifest → pull
    SELECT
        sr.id AS resource_id,
        sr.kb_uri,
        sr.content_hash,
        sr.updated,
        'to_pull'::VARCHAR(16) AS diff_type
      FROM server_resources sr
      LEFT JOIN manifest_entries me ON me.extracted_resource_id = sr.id
     WHERE me.uri IS NULL
       AND sr.is_active = true

    UNION ALL

    -- Manifest entry exists but no server resource → new local resource to push
    SELECT
        NULL::UUID AS resource_id,
        me.uri AS kb_uri,
        me.local_hash AS content_hash,
        NULL::TIMESTAMPTZ AS updated,
        'to_push'::VARCHAR(16) AS diff_type
      FROM manifest_entries me
      LEFT JOIN server_resources sr ON sr.id = me.extracted_resource_id
     WHERE sr.id IS NULL
$$;
