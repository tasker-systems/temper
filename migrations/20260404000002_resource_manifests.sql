-- =============================================================================
-- Resource Manifests — three-tier hash tracking for sync
-- =============================================================================
-- Moves body_hash, managed_meta, and open_meta out of kb_resources into a
-- dedicated manifests table. Updates sync_diff_for_device and resource_for_uri
-- to read from the new table.

-- ─── 1. Create kb_resource_manifests ────────────────────────────────────────

CREATE TABLE kb_resource_manifests (
    resource_id   UUID PRIMARY KEY REFERENCES kb_resources(id) ON DELETE CASCADE,
    body_hash     VARCHAR(128) NOT NULL DEFAULT '',
    managed_meta  JSONB NOT NULL DEFAULT '{}',
    open_meta     JSONB NOT NULL DEFAULT '{}',
    managed_hash  VARCHAR(128) NOT NULL DEFAULT '',
    open_hash     VARCHAR(128) NOT NULL DEFAULT '',
    updated       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ─── 2. Backfill from existing kb_resources ─────────────────────────────────

INSERT INTO kb_resource_manifests (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
SELECT
    r.id,
    COALESCE(r.content_hash, ''),
    '{}'::JSONB,
    '{}'::JSONB,
    'sha256:' || encode(sha256(convert_to('{}', 'UTF8')), 'hex'),
    'sha256:' || encode(sha256(convert_to('{}', 'UTF8')), 'hex'),
    r.updated
FROM kb_resources r;

-- ─── 3. Indexes on kb_resource_manifests ────────────────────────────────────

-- Expression indexes for stage/status queries on managed_meta
CREATE INDEX idx_resource_manifests_stage
    ON kb_resource_manifests ((managed_meta->>'temper-stage'))
    WHERE managed_meta->>'temper-stage' IS NOT NULL;

CREATE INDEX idx_resource_manifests_status
    ON kb_resource_manifests ((managed_meta->>'temper-status'))
    WHERE managed_meta->>'temper-status' IS NOT NULL;

CREATE INDEX idx_resource_manifests_goal
    ON kb_resource_manifests ((managed_meta->>'temper-goal'))
    WHERE managed_meta->>'temper-goal' IS NOT NULL;

-- GIN index on open_meta for flexible queries
CREATE INDEX idx_resource_manifests_open_meta
    ON kb_resource_manifests USING GIN (open_meta);

-- ─── 4. Drop columns and indexes from kb_resources ──────────────────────────

ALTER TABLE kb_resources DROP COLUMN IF EXISTS content_hash;
ALTER TABLE kb_resources DROP COLUMN IF EXISTS mimetype;
ALTER TABLE kb_resources DROP COLUMN IF EXISTS resource_mode;

-- Drop the UNIQUE constraint on origin_uri (keep the column)
ALTER TABLE kb_resources DROP CONSTRAINT IF EXISTS kb_resources_origin_uri_key;

-- Drop the resource_mode index
DROP INDEX IF EXISTS idx_kb_resources_mode;

-- ─── 5. Updated sync_diff_for_device ────────────────────────────────────────
-- Now reads from kb_resource_manifests for three-tier hashes.
-- Returns body_hash, managed_hash, open_hash, and diff_type.
-- Backward-compatible: accepts old manifest entries with local_hash/remote_hash.
-- Must DROP first because the return type changed (added managed_hash, open_hash columns).

DROP FUNCTION IF EXISTS sync_diff_for_device(UUID, TEXT[], JSONB);

CREATE FUNCTION sync_diff_for_device(
    p_profile_id    UUID,
    p_context_names TEXT[],
    p_manifest      JSONB
) RETURNS TABLE (
    resource_id   UUID,
    kb_uri        TEXT,
    body_hash     VARCHAR(64),
    managed_hash  VARCHAR(64),
    open_hash     VARCHAR(64),
    updated       TIMESTAMPTZ,
    diff_type     VARCHAR(32)
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
            -- Support both old field names (local_hash) and new (body_hash)
            COALESCE(entry->>'body_hash', entry->>'local_hash', '')::VARCHAR(128) AS local_body_hash,
            COALESCE(entry->>'remote_body_hash', entry->>'remote_hash', '')::VARCHAR(128) AS remote_body_hash,
            COALESCE(entry->>'managed_hash', '')::VARCHAR(128) AS local_managed_hash,
            COALESCE(entry->>'remote_managed_hash', '')::VARCHAR(128) AS remote_managed_hash,
            COALESCE(entry->>'open_hash', '')::VARCHAR(128) AS local_open_hash,
            COALESCE(entry->>'remote_open_hash', '')::VARCHAR(128) AS remote_open_hash
        FROM jsonb_array_elements(p_manifest) AS entry
    ),
    server_resources AS (
        SELECT r.id,
               kb_resource_uri(r.id) AS kb_uri,
               COALESCE(m.body_hash, '') AS body_hash,
               COALESCE(m.managed_hash, '') AS managed_hash,
               COALESCE(m.open_hash, '') AS open_hash,
               r.updated,
               r.is_active
          FROM kb_resources r
          JOIN visible v ON v.resource_id = r.id
          JOIN kb_contexts c ON c.id = r.kb_context_id
          LEFT JOIN kb_resource_manifests m ON m.resource_id = r.id
         WHERE c.name = ANY(p_context_names)
    )
    -- Server resource exists, manifest entry exists: compare hashes
    SELECT
        sr.id AS resource_id,
        sr.kb_uri,
        sr.body_hash::VARCHAR(128),
        sr.managed_hash::VARCHAR(128),
        sr.open_hash::VARCHAR(128),
        sr.updated,
        CASE
            WHEN NOT sr.is_active THEN 'removed'::VARCHAR(32)
            -- Body changed on both sides
            WHEN me.local_body_hash != me.remote_body_hash
                 AND sr.body_hash != me.remote_body_hash THEN 'conflict'
            -- Body changed locally only
            WHEN me.local_body_hash != me.remote_body_hash
                 AND sr.body_hash = me.remote_body_hash THEN 'to_push_body'
            -- Body changed on server only
            WHEN me.local_body_hash = me.remote_body_hash
                 AND sr.body_hash != me.remote_body_hash THEN 'to_pull'
            -- Meta changed locally only (skip when client sends empty hashes — old format)
            WHEN me.local_managed_hash != '' AND me.local_managed_hash != me.remote_managed_hash
                 AND sr.managed_hash = me.remote_managed_hash THEN 'to_push_meta'
            WHEN me.local_open_hash != '' AND me.local_open_hash != me.remote_open_hash
                 AND sr.open_hash = me.remote_open_hash THEN 'to_push_meta'
            -- Meta changed on server only (skip when client sends empty hashes)
            WHEN me.local_managed_hash != '' AND sr.managed_hash != me.remote_managed_hash
                 AND me.local_managed_hash = me.remote_managed_hash THEN 'to_pull'
            WHEN me.local_open_hash != '' AND sr.open_hash != me.remote_open_hash
                 AND me.local_open_hash = me.remote_open_hash THEN 'to_pull'
            -- Meta changed on both sides (skip when client sends empty hashes)
            WHEN me.local_managed_hash != '' AND me.local_managed_hash != me.remote_managed_hash
                 AND sr.managed_hash != me.remote_managed_hash THEN 'conflict'
            WHEN me.local_open_hash != '' AND me.local_open_hash != me.remote_open_hash
                 AND sr.open_hash != me.remote_open_hash THEN 'conflict'
        END AS diff_type
      FROM server_resources sr
      JOIN manifest_entries me ON me.extracted_resource_id = sr.id
     WHERE sr.is_active = false
        OR me.local_body_hash != sr.body_hash
        -- Only compare meta hashes when the client provides them (non-empty).
        -- Old-format clients send empty strings for managed/open hashes.
        OR (me.local_managed_hash != '' AND me.local_managed_hash != sr.managed_hash)
        OR (me.local_open_hash != '' AND me.local_open_hash != sr.open_hash)

    UNION ALL

    -- Server resource exists but not in manifest -> pull
    SELECT
        sr.id AS resource_id,
        sr.kb_uri,
        sr.body_hash::VARCHAR(128),
        sr.managed_hash::VARCHAR(128),
        sr.open_hash::VARCHAR(128),
        sr.updated,
        'to_pull'::VARCHAR(32) AS diff_type
      FROM server_resources sr
      LEFT JOIN manifest_entries me ON me.extracted_resource_id = sr.id
     WHERE me.uri IS NULL
       AND sr.is_active = true

    UNION ALL

    -- Manifest entry exists but no server resource -> push
    SELECT
        NULL::UUID AS resource_id,
        me.uri AS kb_uri,
        me.local_body_hash::VARCHAR(128) AS body_hash,
        me.local_managed_hash::VARCHAR(128) AS managed_hash,
        me.local_open_hash::VARCHAR(128) AS open_hash,
        NULL::TIMESTAMPTZ AS updated,
        'to_push_body'::VARCHAR(32) AS diff_type
      FROM manifest_entries me
      LEFT JOIN server_resources sr ON sr.id = me.extracted_resource_id
     WHERE sr.id IS NULL
$$;

-- ─── 6. Updated resource_for_uri ────────────────────────────────────────────
-- Now joins kb_resource_manifests for body_hash.
-- Must DROP first because the return type changed (content_hash → body_hash).

DROP FUNCTION IF EXISTS resource_for_uri(UUID, TEXT);

CREATE FUNCTION resource_for_uri(p_profile_id UUID, p_kb_uri TEXT)
RETURNS TABLE (
    resource_id  UUID,
    origin_uri   TEXT,
    body_hash    VARCHAR(64),
    updated      TIMESTAMPTZ,
    is_active    BOOLEAN,
    access_level VARCHAR(32),
    team_role    team_role
)
LANGUAGE SQL STABLE AS $$
    WITH parsed AS (
        SELECT (split_part(p_kb_uri, '/', 5))::UUID AS extracted_id
    )
    SELECT r.id AS resource_id,
           r.origin_uri,
           COALESCE(m.body_hash, '')::VARCHAR(128) AS body_hash,
           r.updated,
           r.is_active,
           v.access_level,
           v.team_role
      FROM parsed p
      JOIN kb_resources r ON r.id = p.extracted_id
      LEFT JOIN kb_resource_manifests m ON m.resource_id = r.id
      JOIN resources_visible_to(p_profile_id, NULL, ARRAY[p.extracted_id]) v
        ON v.resource_id = r.id
$$;
