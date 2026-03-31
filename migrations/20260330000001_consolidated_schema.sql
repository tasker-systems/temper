-- =============================================================================
-- Consolidated Temper Schema — I6-pre
-- =============================================================================
-- Replaces migrations: r2_schema, audit_fixes, contexts_ownership, blob_files
-- Naming: all tables use kb_ prefix
-- Audit: Category E elements (behaviors, lifecycle stages, state machines) removed

-- Extension
CREATE EXTENSION IF NOT EXISTS vector;

-- Enums
CREATE TYPE team_role AS ENUM ('owner', 'maintainer', 'member', 'watcher');
CREATE TYPE access_level AS ENUM ('vault', 'mutable', 'immutable');
CREATE TYPE invitation_status AS ENUM ('pending', 'accepted', 'declined', 'expired');
CREATE TYPE transfer_status AS ENUM ('pending', 'accepted', 'declined', 'cancelled');

-- ─── Taxonomy ────────────────────────────────────────────────────────────────

CREATE TABLE kb_contexts (
    id          UUID PRIMARY KEY,
    name        VARCHAR(128) NOT NULL,
    kb_owner_table VARCHAR(64) NOT NULL DEFAULT 'kb_profiles'
        CHECK (kb_owner_table IN ('kb_profiles', 'kb_teams')),
    kb_owner_id UUID NOT NULL,
    created     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT kb_contexts_owner_name_unique UNIQUE (kb_owner_table, kb_owner_id, name)
);
CREATE INDEX idx_contexts_owner ON kb_contexts(kb_owner_table, kb_owner_id);

CREATE TABLE kb_doc_types (
    id      UUID PRIMARY KEY,
    name    VARCHAR(64) NOT NULL UNIQUE,
    created TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ─── Identity ────────────────────────────────────────────────────────────────

CREATE TABLE kb_profiles (
    id           UUID PRIMARY KEY,
    display_name VARCHAR(128) NOT NULL,
    email        VARCHAR(256),
    avatar_url   TEXT,
    preferences  JSONB NOT NULL DEFAULT '{}',
    vault_config JSONB NOT NULL DEFAULT '{}',
    is_active    BOOLEAN NOT NULL DEFAULT true,
    created      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE kb_profile_auth_links (
    id                    UUID PRIMARY KEY,
    profile_id            UUID NOT NULL REFERENCES kb_profiles(id) ON DELETE CASCADE,
    auth_provider         VARCHAR(32) NOT NULL,
    auth_provider_user_id VARCHAR(128) NOT NULL,
    email                 VARCHAR(256),
    is_default            BOOLEAN NOT NULL DEFAULT false,
    linked_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(auth_provider, auth_provider_user_id)
);
CREATE INDEX idx_auth_links_profile ON kb_profile_auth_links(profile_id);
CREATE INDEX idx_auth_links_email ON kb_profile_auth_links(email);

-- ─── Resources ───────────────────────────────────────────────────────────────

CREATE TABLE kb_resources (
    id                     UUID PRIMARY KEY,
    kb_context_id          UUID NOT NULL REFERENCES kb_contexts(id),
    kb_doc_type_id         UUID NOT NULL REFERENCES kb_doc_types(id),
    uri                    TEXT NOT NULL UNIQUE,
    title                  TEXT NOT NULL,
    slug                   VARCHAR(256),
    content_hash           VARCHAR(64),
    mimetype               VARCHAR(128),
    resource_mode          VARCHAR(16) NOT NULL DEFAULT 'added'
        CHECK (resource_mode IN ('added', 'imported')),
    originator_profile_id  UUID NOT NULL REFERENCES kb_profiles(id),
    owner_profile_id       UUID NOT NULL REFERENCES kb_profiles(id),
    is_active              BOOLEAN NOT NULL DEFAULT true,
    created                TIMESTAMPTZ NOT NULL,
    updated                TIMESTAMPTZ NOT NULL,
    UNIQUE(slug, kb_context_id)
);
CREATE INDEX idx_kb_resources_context ON kb_resources(kb_context_id);
CREATE INDEX idx_kb_resources_doc_type ON kb_resources(kb_doc_type_id);
CREATE INDEX idx_kb_resources_updated ON kb_resources(updated);
CREATE INDEX idx_kb_resources_owner ON kb_resources(owner_profile_id);
CREATE INDEX idx_kb_resources_originator ON kb_resources(originator_profile_id);
CREATE INDEX idx_kb_resources_mode ON kb_resources(resource_mode);

-- ─── Chunks & Search ─────────────────────────────────────────────────────────

CREATE TABLE kb_chunks (
    id           UUID PRIMARY KEY,
    resource_id  UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    chunk_index  INT NOT NULL,
    version      INT NOT NULL DEFAULT 1,
    header_path  TEXT NOT NULL DEFAULT '',
    content      TEXT NOT NULL,
    content_hash VARCHAR(64) NOT NULL,
    embedding    vector(768) NOT NULL,
    is_current   BOOLEAN NOT NULL DEFAULT true,
    created      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(resource_id, chunk_index, version)
);
CREATE INDEX idx_chunks_resource ON kb_chunks(resource_id);
CREATE INDEX idx_chunks_content_hash ON kb_chunks(content_hash);
CREATE INDEX idx_chunks_current_embedding ON kb_chunks
    USING hnsw (embedding vector_cosine_ops)
    WITH (m = 16, ef_construction = 200)
    WHERE is_current = true;

CREATE VIEW kb_current_chunks AS
SELECT id, resource_id, chunk_index, version, header_path, content,
       content_hash, embedding, created
  FROM kb_chunks
 WHERE is_current = true
 ORDER BY resource_id, chunk_index;

-- ─── Blob Files ──────────────────────────────────────────────────────────────

CREATE TABLE kb_blob_files (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    profile_id      UUID NOT NULL REFERENCES kb_profiles(id),
    resource_id     UUID REFERENCES kb_resources(id),
    blob_url        TEXT NOT NULL,
    pathname        TEXT NOT NULL,
    content_type    TEXT,
    file_size_bytes BIGINT,
    status          TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'processing', 'processed', 'failed')),
    error_message   TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_kb_blob_files_profile ON kb_blob_files(profile_id);
CREATE INDEX idx_kb_blob_files_resource ON kb_blob_files(resource_id);
CREATE INDEX idx_kb_blob_files_status ON kb_blob_files(status);

-- ─── Ingestion Records ───────────────────────────────────────────────────────

CREATE TABLE kb_ingestion_records (
    resource_id        UUID PRIMARY KEY REFERENCES kb_resources(id) ON DELETE CASCADE,
    source_uri         TEXT NOT NULL,
    source_mimetype    VARCHAR(128),
    conversion_tool    VARCHAR(64) NOT NULL,
    conversion_version VARCHAR(32) NOT NULL,
    fetched_at         TIMESTAMPTZ NOT NULL,
    converted_at       TIMESTAMPTZ NOT NULL,
    source_hash        VARCHAR(64)
);

-- ─── Events ──────────────────────────────────────────────────────────────────
-- NOTE: Open design question for I6b — how to bridge local events.jsonl
-- with cloud kb_events, and whether event IDs should annotate document
-- changes for merge reconciliation.

CREATE TABLE kb_events (
    id            UUID PRIMARY KEY,
    profile_id    UUID NOT NULL REFERENCES kb_profiles(id),
    client_id     VARCHAR(64) NOT NULL,
    kb_context_id UUID REFERENCES kb_contexts(id),
    resource_id   UUID REFERENCES kb_resources(id),
    event_type    VARCHAR(64) NOT NULL,
    payload       JSONB NOT NULL DEFAULT '{}',
    created       TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_events_resource ON kb_events(resource_id);
CREATE INDEX idx_events_type ON kb_events(event_type);
CREATE INDEX idx_events_created ON kb_events(created);
CREATE INDEX idx_events_client ON kb_events(client_id);
CREATE INDEX idx_events_profile ON kb_events(profile_id);

-- ─── Sync ────────────────────────────────────────────────────────────────────

CREATE TABLE kb_device_sync_state (
    id            UUID PRIMARY KEY,
    profile_id    UUID NOT NULL REFERENCES kb_profiles(id),
    client_id     VARCHAR(64) NOT NULL,
    last_sync_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    manifest_hash VARCHAR(64),
    UNIQUE(profile_id, client_id)
);
CREATE INDEX idx_device_sync_profile ON kb_device_sync_state(profile_id);

-- ─── Teams ───────────────────────────────────────────────────────────────────

CREATE TABLE kb_teams (
    id                   UUID PRIMARY KEY,
    name                 VARCHAR(128) NOT NULL,
    slug                 VARCHAR(128) NOT NULL UNIQUE,
    description          VARCHAR(512),
    metadata             JSONB NOT NULL DEFAULT '{}',
    created_by_profile_id UUID NOT NULL REFERENCES kb_profiles(id),
    is_active            BOOLEAN NOT NULL DEFAULT true,
    created              TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated              TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE kb_team_members (
    id                    UUID PRIMARY KEY,
    team_id               UUID NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    profile_id            UUID NOT NULL REFERENCES kb_profiles(id),
    role                  team_role NOT NULL,
    joined_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    invited_by_profile_id UUID REFERENCES kb_profiles(id),
    UNIQUE(team_id, profile_id)
);
CREATE INDEX idx_team_members_profile ON kb_team_members(profile_id);
CREATE INDEX idx_team_members_team ON kb_team_members(team_id);

CREATE TABLE kb_team_resources (
    id                  UUID PRIMARY KEY,
    team_id             UUID NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    resource_id         UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    access_level        access_level NOT NULL,
    added_by_profile_id UUID NOT NULL REFERENCES kb_profiles(id),
    added_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(team_id, resource_id)
);
CREATE INDEX idx_team_resources_resource ON kb_team_resources(resource_id);
CREATE INDEX idx_team_resources_team ON kb_team_resources(team_id);

CREATE TABLE kb_team_invitations (
    id                    UUID PRIMARY KEY,
    team_id               UUID NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    invited_email         VARCHAR(256) NOT NULL,
    invited_by_profile_id UUID NOT NULL REFERENCES kb_profiles(id),
    role                  team_role NOT NULL,
    token                 VARCHAR(128) NOT NULL UNIQUE,
    status                invitation_status NOT NULL DEFAULT 'pending',
    expires_at            TIMESTAMPTZ NOT NULL DEFAULT now() + INTERVAL '7 days',
    created               TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(team_id, invited_email)
);
CREATE INDEX idx_invitations_token ON kb_team_invitations(token);
CREATE INDEX idx_invitations_email ON kb_team_invitations(invited_email);

-- ─── Transfers ───────────────────────────────────────────────────────────────

CREATE TABLE kb_transfers (
    id              UUID PRIMARY KEY,
    resource_id     UUID NOT NULL REFERENCES kb_resources(id),
    from_profile_id UUID NOT NULL REFERENCES kb_profiles(id),
    to_profile_id   UUID NOT NULL REFERENCES kb_profiles(id),
    status          transfer_status NOT NULL DEFAULT 'pending',
    created         TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at     TIMESTAMPTZ,
    UNIQUE(resource_id, from_profile_id, to_profile_id, status)
);
CREATE INDEX idx_transfers_to_profile ON kb_transfers(to_profile_id) WHERE status = 'pending';
CREATE INDEX idx_transfers_from_profile ON kb_transfers(from_profile_id) WHERE status = 'pending';
CREATE INDEX idx_transfers_resource ON kb_transfers(resource_id);

-- ─── SQL Functions ───────────────────────────────────────────────────────────

CREATE FUNCTION resources_visible_to(
    p_profile_id UUID,
    p_team_id    UUID DEFAULT NULL
) RETURNS TABLE (resource_id UUID, access_level VARCHAR(32), via VARCHAR(256))
LANGUAGE SQL STABLE AS $$
    SELECT r.id AS resource_id,
           'vault'::VARCHAR(32) AS access_level,
           ('owner:' || p_profile_id)::VARCHAR(256) AS via
      FROM kb_resources r
     WHERE r.owner_profile_id = p_profile_id
       AND r.is_active = true

    UNION ALL

    SELECT tr.resource_id,
           tr.access_level::VARCHAR(32),
           ('team:' || t.slug)::VARCHAR(256) AS via
      FROM kb_team_resources tr
      JOIN kb_teams t ON t.id = tr.team_id AND t.is_active = true
      JOIN kb_team_members tm ON tm.team_id = t.id AND tm.profile_id = p_profile_id
      JOIN kb_resources r ON r.id = tr.resource_id AND r.is_active = true
     WHERE (p_team_id IS NULL OR t.id = p_team_id)
$$;

CREATE FUNCTION can_modify_resource(
    p_profile_id  UUID,
    p_resource_id UUID
) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM kb_resources
         WHERE id = p_resource_id
           AND owner_profile_id = p_profile_id
           AND is_active = true

        UNION ALL

        SELECT 1
          FROM kb_team_resources tr
          JOIN kb_teams t ON t.id = tr.team_id AND t.is_active = true
          JOIN kb_team_members tm ON tm.team_id = t.id AND tm.profile_id = p_profile_id
          JOIN kb_resources r ON r.id = tr.resource_id AND r.is_active = true
         WHERE tr.resource_id = p_resource_id
           AND tr.access_level IN ('vault', 'mutable')
           AND tm.role != 'watcher'
    )
$$;

CREATE FUNCTION can_manage_team(
    p_profile_id UUID,
    p_team_id    UUID,
    p_action     VARCHAR(32)
) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    SELECT EXISTS (
        SELECT 1
          FROM kb_team_members tm
         WHERE tm.team_id = p_team_id
           AND tm.profile_id = p_profile_id
           AND (
               (p_action = 'delete' AND tm.role = 'owner')
               OR (p_action IN ('invite', 'remove', 'change_role')
                   AND tm.role IN ('owner', 'maintainer'))
           )
    )
$$;

CREATE FUNCTION contexts_visible_to(
    p_profile_id UUID,
    p_team_id    UUID DEFAULT NULL
) RETURNS TABLE (id UUID, name VARCHAR(128), kb_owner_table VARCHAR(64), kb_owner_id UUID)
LANGUAGE SQL STABLE AS $$
    SELECT c.id, c.name, c.kb_owner_table, c.kb_owner_id
      FROM kb_contexts c
     WHERE c.kb_owner_table = 'kb_profiles'
       AND c.kb_owner_id = p_profile_id

    UNION ALL

    SELECT c.id, c.name, c.kb_owner_table, c.kb_owner_id
      FROM kb_contexts c
      JOIN kb_teams t ON t.id = c.kb_owner_id AND c.kb_owner_table = 'kb_teams' AND t.is_active = true
      JOIN kb_team_members tm ON tm.team_id = t.id AND tm.profile_id = p_profile_id
     WHERE (p_team_id IS NULL OR t.id = p_team_id)
$$;

CREATE FUNCTION sync_diff_for_device(
    p_profile_id    UUID,
    p_context_names TEXT[],
    p_manifest      JSONB
) RETURNS TABLE (
    resource_id UUID,
    uri         TEXT,
    content_hash VARCHAR(64),
    updated     TIMESTAMPTZ,
    diff_type   VARCHAR(16)
)
LANGUAGE SQL STABLE AS $$
    WITH
    visible AS (
        SELECT v.resource_id FROM resources_visible_to(p_profile_id) v
    ),
    manifest_entries AS (
        SELECT
            (entry->>'uri')::TEXT AS uri,
            (entry->>'local_hash')::VARCHAR(64) AS local_hash,
            (entry->>'remote_hash')::VARCHAR(64) AS remote_hash
        FROM jsonb_array_elements(p_manifest) AS entry
    ),
    server_resources AS (
        SELECT r.id, r.uri, r.content_hash, r.updated, r.is_active
          FROM kb_resources r
          JOIN visible v ON v.resource_id = r.id
          JOIN kb_contexts c ON c.id = r.kb_context_id
         WHERE r.resource_mode = 'imported'
           AND c.name = ANY(p_context_names)
    )
    SELECT
        sr.id AS resource_id,
        sr.uri,
        sr.content_hash,
        sr.updated,
        CASE
            WHEN NOT sr.is_active THEN 'removed'::VARCHAR(16)
            WHEN me.local_hash != me.remote_hash AND sr.content_hash != me.remote_hash THEN 'conflict'
            WHEN me.local_hash != me.remote_hash AND sr.content_hash = me.remote_hash THEN 'to_push'
            WHEN me.local_hash = me.remote_hash AND sr.content_hash != me.remote_hash THEN 'to_pull'
        END AS diff_type
      FROM server_resources sr
      JOIN manifest_entries me ON me.uri = sr.uri
     WHERE sr.is_active = false
        OR me.local_hash != sr.content_hash

    UNION ALL

    SELECT
        sr.id AS resource_id,
        sr.uri,
        sr.content_hash,
        sr.updated,
        'to_pull'::VARCHAR(16) AS diff_type
      FROM server_resources sr
      LEFT JOIN manifest_entries me ON me.uri = sr.uri
     WHERE me.uri IS NULL
       AND sr.is_active = true

    UNION ALL

    SELECT
        NULL::UUID AS resource_id,
        me.uri,
        me.local_hash AS content_hash,
        NULL::TIMESTAMPTZ AS updated,
        'to_push'::VARCHAR(16) AS diff_type
      FROM manifest_entries me
      LEFT JOIN server_resources sr ON sr.uri = me.uri
     WHERE sr.id IS NULL
$$;
