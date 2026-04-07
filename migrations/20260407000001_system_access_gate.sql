-- migrations/20260407000001_system_access_gate.sql
-- System access gate: join_request_status enum, kb_system_settings singleton,
-- kb_join_requests table, has_system_access/is_system_admin SQL functions,
-- and temper-system team bootstrap.

-- 1. Join request status enum
CREATE TYPE join_request_status AS ENUM ('pending', 'approved', 'rejected', 'withdrawn');

-- 2. System settings singleton
CREATE TABLE kb_system_settings (
    id                  INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    access_mode         VARCHAR(16) NOT NULL DEFAULT 'open'
        CHECK (access_mode IN ('open', 'invite_only')),
    gating_team_slug    VARCHAR(128),
    terms_version       VARCHAR(32),
    terms_resource_uri  TEXT,
    instance_name       VARCHAR(128),
    updated             TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO kb_system_settings (id, access_mode) VALUES (1, 'open');

-- 3. Join requests table
CREATE TABLE kb_join_requests (
    id                       UUID PRIMARY KEY,
    team_id                  UUID NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    requesting_profile_id    UUID NOT NULL REFERENCES kb_profiles(id) ON DELETE CASCADE,
    status                   join_request_status NOT NULL DEFAULT 'pending',
    message                  TEXT,
    source                   VARCHAR(16) NOT NULL,
    accepted_terms_version   VARCHAR(32),
    accepted_terms_at        TIMESTAMPTZ,
    reviewed_by_profile_id   UUID REFERENCES kb_profiles(id),
    reviewed_at              TIMESTAMPTZ,
    decision_note            TEXT,
    created                  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated                  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One pending request per profile per team
CREATE UNIQUE INDEX idx_join_requests_one_pending
    ON kb_join_requests (team_id, requesting_profile_id)
    WHERE status = 'pending';

-- Admin queue ordering
CREATE INDEX idx_join_requests_status_created
    ON kb_join_requests (status, created DESC);

-- 4. System access check function
CREATE FUNCTION has_system_access(p_profile_id UUID) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    WITH settings AS (
        SELECT access_mode, gating_team_slug
          FROM kb_system_settings
         LIMIT 1
    )
    SELECT CASE
        WHEN settings.access_mode = 'open' THEN true
        WHEN settings.access_mode = 'invite_only' THEN EXISTS (
            SELECT 1
              FROM kb_team_members tm
              JOIN kb_teams t ON t.id = tm.team_id
             WHERE tm.profile_id = p_profile_id
               AND t.slug = settings.gating_team_slug
               AND t.is_active = true
        )
        ELSE false
    END
      FROM settings
$$;

-- 5. System admin check function
CREATE FUNCTION is_system_admin(p_profile_id UUID) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    WITH settings AS (
        SELECT gating_team_slug
          FROM kb_system_settings
         LIMIT 1
    )
    SELECT EXISTS (
        SELECT 1
          FROM kb_team_members tm
          JOIN kb_teams t ON t.id = tm.team_id
         WHERE tm.profile_id = p_profile_id
           AND t.slug = settings.gating_team_slug
           AND t.is_active = true
           AND tm.role = 'owner'
    )
      FROM settings
$$;

-- 6. Bootstrap: temper-system team + general context
-- System profile (00000000-0000-0000-0004-000000000001) exists from seed migration.
INSERT INTO kb_teams (id, name, slug, description, created_by_profile_id, is_active, created, updated)
VALUES (
    '00000000-0000-0000-0000-000000000002',
    'temper-system',
    'temper-system',
    'System team for instance-wide access control and shared content',
    '00000000-0000-0000-0004-000000000001',
    true,
    now(),
    now()
);

INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id, created)
VALUES (
    '00000000-0000-0000-0000-000000000003',
    'general',
    'kb_teams',
    '00000000-0000-0000-0000-000000000002',
    now()
);
