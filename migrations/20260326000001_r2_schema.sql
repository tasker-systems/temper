-- R2: Data Model & Schema Design — Temper Cloud Schema
-- Postgres 18 + pgvector 0.8.2
-- Design spec: docs/superpowers/specs/2026-03-26-r2-data-model-and-schema-design.md

CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE kb_contexts (
    id          UUID PRIMARY KEY,
    name        VARCHAR(128) NOT NULL UNIQUE,
    created     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE kb_doc_types (
    id          UUID PRIMARY KEY,
    name        VARCHAR(64) NOT NULL UNIQUE,
    created     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE kb_behaviors (
    id          UUID PRIMARY KEY,
    name        VARCHAR(64) NOT NULL UNIQUE,
    created     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE kb_doc_type_behaviors (
    kb_doc_type_id  UUID NOT NULL REFERENCES kb_doc_types(id),
    kb_behavior_id  UUID NOT NULL REFERENCES kb_behaviors(id),
    PRIMARY KEY (kb_doc_type_id, kb_behavior_id)
);

CREATE TABLE kb_profiles (
    id              UUID PRIMARY KEY,              -- UUIDv7
    display_name    VARCHAR(128) NOT NULL,
    email           VARCHAR(256),                  -- cached from default auth provider
    avatar_url      TEXT,
    preferences     JSONB NOT NULL DEFAULT '{}',   -- theme, default project, notifications
    vault_config    JSONB NOT NULL DEFAULT '{}',   -- local vault path, sync preferences
    is_active       BOOLEAN NOT NULL DEFAULT true,
    created         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE kb_profile_auth_links (
    id                        UUID PRIMARY KEY,              -- UUIDv7
    profile_id                UUID NOT NULL REFERENCES kb_profiles(id) ON DELETE CASCADE,
    auth_provider             VARCHAR(32) NOT NULL,          -- "neon_auth", "auth0", "okta", etc.
    auth_provider_user_id     VARCHAR(128) NOT NULL,         -- external identity ID from this provider
    email                     VARCHAR(256),                  -- email from this provider at link time
    is_default                BOOLEAN NOT NULL DEFAULT false, -- which link is the primary identity
    linked_at                 TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(auth_provider, auth_provider_user_id)
);

CREATE INDEX idx_auth_links_profile ON kb_profile_auth_links(profile_id);
CREATE INDEX idx_auth_links_email ON kb_profile_auth_links(email);

CREATE TABLE resources (
    id              UUID PRIMARY KEY,
    kb_context_id   UUID NOT NULL REFERENCES kb_contexts(id),
    kb_doc_type_id  UUID NOT NULL REFERENCES kb_doc_types(id),
    uri             TEXT NOT NULL,
    title           TEXT NOT NULL,
    slug            VARCHAR(256),
    content_hash    VARCHAR(64),
    mimetype        VARCHAR(128),
    originator_profile_id UUID NOT NULL REFERENCES kb_profiles(id),
    owner_profile_id    UUID NOT NULL REFERENCES kb_profiles(id),
    is_active           BOOLEAN NOT NULL DEFAULT true,
    created         TIMESTAMPTZ NOT NULL,
    updated         TIMESTAMPTZ NOT NULL,
    UNIQUE(slug, kb_context_id),
    UNIQUE(uri)
);

CREATE INDEX idx_resources_context ON resources(kb_context_id);
CREATE INDEX idx_resources_doc_type ON resources(kb_doc_type_id);
CREATE INDEX idx_resources_updated ON resources(updated);
CREATE INDEX idx_resources_owner ON resources(owner_profile_id);
CREATE INDEX idx_resources_originator ON resources(originator_profile_id);

CREATE TABLE kb_lifecycle_stages (
    id              UUID PRIMARY KEY,
    kb_doc_type_id  UUID NOT NULL REFERENCES kb_doc_types(id),
    name            VARCHAR(64) NOT NULL,
    seq             INT NOT NULL,
    UNIQUE(kb_doc_type_id, name)
);

CREATE TABLE kb_workflowable_states (
    resource_id     UUID PRIMARY KEY REFERENCES resources(id) ON DELETE CASCADE,
    stage_id        UUID NOT NULL REFERENCES kb_lifecycle_stages(id),
    updated         TIMESTAMPTZ NOT NULL
);

CREATE TABLE kb_sequenceable_states (
    resource_id     UUID PRIMARY KEY REFERENCES resources(id) ON DELETE CASCADE,
    seq             INT NOT NULL,
    parent_id       UUID REFERENCES resources(id),
    updated         TIMESTAMPTZ NOT NULL
);

CREATE TABLE kb_assignable_states (
    resource_id     UUID PRIMARY KEY REFERENCES resources(id) ON DELETE CASCADE,
    author          VARCHAR(128),
    assignee        VARCHAR(128),
    metadata        JSONB NOT NULL DEFAULT '{}',
    updated         TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_assignable_metadata ON kb_assignable_states USING GIN(metadata);

CREATE TABLE kb_taggable_states (
    resource_id     UUID PRIMARY KEY REFERENCES resources(id) ON DELETE CASCADE,
    tags            TEXT[] NOT NULL DEFAULT '{}',
    updated         TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_taggable_tags ON kb_taggable_states USING GIN(tags);

CREATE TABLE kb_chunks (
    id              UUID PRIMARY KEY,
    resource_id     UUID NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    chunk_index     INT NOT NULL,
    version         INT NOT NULL DEFAULT 1,
    header_path     TEXT NOT NULL DEFAULT '',
    content         TEXT NOT NULL,
    content_hash    VARCHAR(64) NOT NULL,
    embedding       vector(768) NOT NULL,
    is_current      BOOLEAN NOT NULL DEFAULT true,
    created         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(resource_id, chunk_index, version)
);

CREATE INDEX idx_chunks_resource ON kb_chunks(resource_id);
CREATE INDEX idx_chunks_content_hash ON kb_chunks(content_hash);

CREATE INDEX idx_chunks_current_embedding ON kb_chunks
    USING hnsw(embedding vector_cosine_ops)
    WITH (m = 16, ef_construction = 200)
    WHERE is_current = true;

CREATE VIEW kb_current_chunks AS
SELECT id, resource_id, chunk_index, version, header_path,
       content, content_hash, embedding, created
FROM kb_chunks
WHERE is_current = true
ORDER BY resource_id, chunk_index;

CREATE TABLE kb_ingestion_records (
    resource_id         UUID PRIMARY KEY REFERENCES resources(id) ON DELETE CASCADE,
    source_uri          TEXT NOT NULL,
    source_mimetype     VARCHAR(128),
    conversion_tool     VARCHAR(64) NOT NULL,
    conversion_version  VARCHAR(32) NOT NULL,
    fetched_at          TIMESTAMPTZ NOT NULL,
    converted_at        TIMESTAMPTZ NOT NULL,
    source_hash         VARCHAR(64)
);

-- R4: Team and access control enums
CREATE TYPE team_role AS ENUM ('owner', 'maintainer', 'member', 'watcher');
CREATE TYPE access_level AS ENUM ('vault', 'mutable', 'immutable');
CREATE TYPE invitation_status AS ENUM ('pending', 'accepted', 'declined', 'expired');

CREATE TABLE kb_teams (
    id                      UUID PRIMARY KEY,              -- UUIDv7
    name                    VARCHAR(128) NOT NULL,
    slug                    VARCHAR(128) NOT NULL UNIQUE,
    description             VARCHAR(512),
    metadata                JSONB NOT NULL DEFAULT '{}',
    created_by_profile_id   UUID NOT NULL REFERENCES kb_profiles(id),
    is_active               BOOLEAN NOT NULL DEFAULT true,
    created                 TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated                 TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE kb_team_members (
    id                      UUID PRIMARY KEY,              -- UUIDv7
    team_id                 UUID NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    profile_id              UUID NOT NULL REFERENCES kb_profiles(id),
    role                    team_role NOT NULL,
    joined_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    invited_by_profile_id   UUID REFERENCES kb_profiles(id),
    UNIQUE(team_id, profile_id)
);

CREATE INDEX idx_team_members_profile ON kb_team_members(profile_id);
CREATE INDEX idx_team_members_team ON kb_team_members(team_id);

CREATE TABLE kb_team_resources (
    id                      UUID PRIMARY KEY,              -- UUIDv7
    team_id                 UUID NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    resource_id             UUID NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    access_level            access_level NOT NULL,
    added_by_profile_id     UUID NOT NULL REFERENCES kb_profiles(id),
    added_at                TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(team_id, resource_id)
);

CREATE INDEX idx_team_resources_resource ON kb_team_resources(resource_id);
CREATE INDEX idx_team_resources_team ON kb_team_resources(team_id);

CREATE TABLE kb_team_invitations (
    id                      UUID PRIMARY KEY,              -- UUIDv7
    team_id                 UUID NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    invited_email           VARCHAR(256) NOT NULL,
    invited_by_profile_id   UUID NOT NULL REFERENCES kb_profiles(id),
    role                    team_role NOT NULL,
    token                   VARCHAR(128) NOT NULL UNIQUE,
    status                  invitation_status NOT NULL DEFAULT 'pending',
    expires_at              TIMESTAMPTZ NOT NULL DEFAULT now() + INTERVAL '7 days',
    created                 TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(team_id, invited_email)
);

CREATE INDEX idx_invitations_token ON kb_team_invitations(token);
CREATE INDEX idx_invitations_email ON kb_team_invitations(invited_email);

-- R4: Composable access control functions
-- These are STABLE (no side effects) so the query planner can inline them.
-- They compose into CTEs, subqueries, and joins for vector search and graph traversal.
-- Functions: resources_visible_to, can_modify_resource, can_manage_team

CREATE FUNCTION resources_visible_to(
    p_profile_id UUID,
    p_team_id UUID DEFAULT NULL
) RETURNS TABLE(resource_id UUID, access_level VARCHAR(32), via VARCHAR(256))
LANGUAGE SQL STABLE AS $$
    -- Resources I own (always visible, full control)
    SELECT id, 'owner'::VARCHAR(32), 'ownership'::VARCHAR(256)
    FROM resources
    WHERE owner_profile_id = p_profile_id
      AND is_active = true

    UNION

    -- Resources shared with teams I belong to
    SELECT tr.resource_id, tr.access_level::VARCHAR(32), ('team:' || t.slug)::VARCHAR(256)
    FROM kb_team_resources tr
    JOIN kb_teams t ON t.id = tr.team_id
    JOIN kb_team_members tm ON tm.team_id = tr.team_id
    WHERE tm.profile_id = p_profile_id
      AND t.is_active = true
      AND (p_team_id IS NULL OR tr.team_id = p_team_id)
$$;

CREATE FUNCTION can_modify_resource(
    p_profile_id UUID,
    p_resource_id UUID
) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    SELECT EXISTS (
        -- I own it
        SELECT 1 FROM resources
        WHERE id = p_resource_id AND owner_profile_id = p_profile_id
    ) OR EXISTS (
        -- It's vault or mutable in a team I belong to, and I'm not a watcher
        SELECT 1
        FROM kb_team_resources tr
        JOIN kb_team_members tm ON tm.team_id = tr.team_id
        WHERE tr.resource_id = p_resource_id
          AND tm.profile_id = p_profile_id
          AND tr.access_level IN ('vault', 'mutable')
          AND tm.role != 'watcher'
    )
$$;

CREATE FUNCTION can_manage_team(
    p_profile_id UUID,
    p_team_id UUID,
    p_action VARCHAR(32)  -- 'invite', 'remove', 'change_role', 'delete'
) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM kb_team_members
        WHERE team_id = p_team_id
          AND profile_id = p_profile_id
          AND (
            (p_action = 'delete' AND role = 'owner')
            OR (p_action IN ('invite', 'remove', 'change_role')
                AND role IN ('owner', 'maintainer'))
          )
    )
$$;

CREATE TABLE kb_events (
    id              UUID PRIMARY KEY,
    profile_id      UUID NOT NULL REFERENCES kb_profiles(id),
    client_id       VARCHAR(64) NOT NULL,
    kb_context_id   UUID REFERENCES kb_contexts(id),
    resource_id     UUID REFERENCES resources(id),
    event_type      VARCHAR(64) NOT NULL,
    payload         JSONB NOT NULL DEFAULT '{}',
    created         TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_events_resource ON kb_events(resource_id);
CREATE INDEX idx_events_type ON kb_events(event_type);
CREATE INDEX idx_events_created ON kb_events(created);
CREATE INDEX idx_events_client ON kb_events(client_id);
CREATE INDEX idx_events_profile ON kb_events(profile_id);
