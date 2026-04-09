-- migrations/20260409000001_sync_diff_for_device_owner_scoped_uris.sql
--
-- Fix sync_diff_for_device URI parsing for owner-scoped URIs.
--
-- Background: the existing `sync_diff_for_device` implementation (last
-- updated in migrations/20260404000002_resource_manifests.sql lines 72-191)
-- parses the resource id out of each manifest entry's URI via
-- `split_part(entry->>'uri', '/', 5)::UUID`. That index is correct for the
-- legacy 3-segment URI shape `kb://<ctx>/<type>/<ident>` where splitting on
-- '/' yields:
--
--     parts: ['kb:', '', '<ctx>', '<type>', '<ident>']
--     index: [ 1  , 2 ,    3    ,    4    ,    5      ]
--
-- After Phase 2 of the system-access-gate work, the CLI builds URIs via
-- `Vault::canonical_uri` which emits the 4-segment owner-scoped form
-- `kb://@<owner>/<ctx>/<type>/<ident>` (or `kb://+<team>/<ctx>/<type>/<ident>`
-- for team contexts). Splitting that on '/' yields:
--
--     parts: ['kb:', '', '@<owner>', '<ctx>', '<type>', '<ident>']
--     index: [ 1  , 2 ,      3     ,    4   ,    5    ,     6     ]
--
-- Index 5 is now the doc_type string, and casting it to UUID fails with
-- `invalid input syntax for type uuid`, which surfaces as a 500 Internal
-- Error on `POST /api/sync/status` any time the client sends a non-empty
-- manifest. The existing e2e test at tests/e2e/tests/sync_test.rs
-- (`sync_status_matching_hash_no_diff`) missed this because it hand-built
-- URIs in the legacy 3-segment format, bypassing `Vault::canonical_uri`.
--
-- The fix replaces `split_part(..., '/', 5)` with
-- `regexp_replace(uri, '^.*/', '')::UUID`, which grabs everything after the
-- final slash regardless of how many segments precede it. This is
-- intentionally format-agnostic with respect to the segment count.
--
-- LOAD-BEARING ASSUMPTION: the URI tail is expected to be a UUID. That
-- assumption holds for every current call site of this function because
-- `build_status_request` in `crates/temper-cli/src/actions/sync.rs:188`
-- re-derives the tail via `id.to_string()` on every entry, regardless of
-- whether the source manifest path was originally stored with a slug or
-- UUID. Any FUTURE code path that feeds server-emitted URIs straight into
-- `sync_diff_for_device` must canonicalize the tail to a UUID first —
-- `kb_resource_uri()` (migrations/20260407000002_owner_scoped_uris.sql:49)
-- emits `COALESCE(r.slug, r.id::text)`, so slug-tailed URIs are legal in
-- other parts of the system and would fail the ::UUID cast here.
--
-- The rest of the function body is copied verbatim from the Session 3 /
-- resource_manifests version. Only the extraction expression changes.

CREATE OR REPLACE FUNCTION sync_diff_for_device(
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
            -- Extract the trailing identifier segment regardless of whether
            -- the URI is in the legacy 3-segment or the owner-scoped
            -- 4-segment form. The last segment is always the resource
            -- identifier that build_status_request emits via id.to_string().
            (regexp_replace(entry->>'uri', '^.*/', ''))::UUID AS extracted_resource_id,
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
                 AND sr.body_hash != me.remote_body_hash THEN 'to_pull_body'
            -- Meta changed locally only (skip when client sends empty hashes — old format)
            WHEN me.local_managed_hash != '' AND me.local_managed_hash != me.remote_managed_hash
                 AND sr.managed_hash = me.remote_managed_hash THEN 'to_push_meta'
            WHEN me.local_open_hash != '' AND me.local_open_hash != me.remote_open_hash
                 AND sr.open_hash = me.remote_open_hash THEN 'to_push_meta'
            -- Meta changed on server only (skip when client sends empty hashes)
            WHEN me.local_managed_hash != '' AND sr.managed_hash != me.remote_managed_hash
                 AND me.local_managed_hash = me.remote_managed_hash THEN 'to_pull_meta'
            WHEN me.local_open_hash != '' AND sr.open_hash != me.remote_open_hash
                 AND me.local_open_hash = me.remote_open_hash THEN 'to_pull_meta'
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
