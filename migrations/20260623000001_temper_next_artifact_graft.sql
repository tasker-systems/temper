-- WS6 collapse — forward-migrate temper_next to match the artifact's two un-migrated deltas:
--   (1) the T6 GRAPH PORT (graph_traverse + graph_subgraph_nodes), added to
--       schema-artifact/02_functions.sql in f2f8247 with no forward migration; and
--   (2) the IDENTITY/INFRA GRAFT, added to schema-artifact/01_schema.sql + 02_functions.sql in
--       f3664c7 with no forward migration: kb_profiles.email + .preferences, the three status enums
--       (join_request_status / invitation_status / transfer_status), the seven operational tables
--       (kb_profile_auth_links / kb_system_settings / kb_join_requests / kb_team_invitations /
--       kb_transfers / kb_blob_files / kb_ingestion_records), and the has_system_access /
--       is_system_admin access-gate predicates.
--
-- Append-only to the frozen temper_next lineage: the install migration (20260613000001) stays
-- untouched and every prior forward delta (4c mutations 20260616000001, can_modify 20260617000001,
-- invocation envelope 20260618000001) precedes this. The artifact (schema-artifact/01_schema.sql +
-- 02_functions.sql) is the design-master; this is its faithful append. The semantic drift guard
-- (crates/temper-next/tests/schema_drift.rs) proves the lineage reconstructs the artifact, so the
-- table/column/index/constraint DDL is byte-faithful to the artifact and the function BODIES here are
-- byte-identical to the artifact (unqualified names that resolve against the `SET search_path` below
-- — exactly as the prior forward deltas do); the body is what pg_get_functiondef fingerprints, so it
-- must not schema-qualify.
SET search_path TO temper_next, public;

-- ── kb_profiles re-grafted columns ───────────────────────────────────────────
ALTER TABLE kb_profiles ADD COLUMN email       VARCHAR(256);
ALTER TABLE kb_profiles ADD COLUMN preferences JSONB NOT NULL DEFAULT '{}'::jsonb;

-- ── Status enums for the grafted operational/identity infra layer ─────────────
CREATE TYPE join_request_status AS ENUM ('pending', 'approved', 'rejected', 'withdrawn');
CREATE TYPE invitation_status   AS ENUM ('pending', 'accepted', 'declined', 'expired');
CREATE TYPE transfer_status     AS ENUM ('pending', 'accepted', 'declined', 'cancelled');

-- ============================================================================
-- GRAFTED IDENTITY / INFRA LAYER (WS6) — operational tables the kernel omitted.
-- Byte-faithful to schema-artifact/01_schema.sql.
-- ============================================================================

-- ── Identity / auth ──────────────────────────────────────────────────────────
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
CREATE INDEX idx_auth_links_email   ON kb_profile_auth_links(email);

-- ── Instance access gate ───────────────────────────────────────────────────────
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
CREATE UNIQUE INDEX idx_join_requests_one_pending
    ON kb_join_requests (team_id, requesting_profile_id) WHERE status = 'pending';
CREATE INDEX idx_join_requests_status_created
    ON kb_join_requests (status, created DESC);

-- ── Team invitations ───────────────────────────────────────────────────────────
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

-- ── Ownership transfers ────────────────────────────────────────────────────────
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
CREATE INDEX idx_transfers_to_profile   ON kb_transfers(to_profile_id)   WHERE status = 'pending';
CREATE INDEX idx_transfers_from_profile ON kb_transfers(from_profile_id) WHERE status = 'pending';
CREATE INDEX idx_transfers_resource     ON kb_transfers(resource_id);

-- ── Blob/upload refs ──────────────────────────────────────────────────────────
CREATE TABLE kb_blob_files (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),   -- see harmonization NOTE
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
CREATE INDEX idx_kb_blob_files_profile  ON kb_blob_files(profile_id);
CREATE INDEX idx_kb_blob_files_resource ON kb_blob_files(resource_id);
CREATE INDEX idx_kb_blob_files_status   ON kb_blob_files(status);

-- ── Ingestion idempotency ─────────────────────────────────────────────────────
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

-- ============================================================================
-- GRAPH READ FUNCTIONS (T6 graph port) — bodies byte-identical to the artifact.
-- ============================================================================

CREATE OR REPLACE FUNCTION graph_traverse(p_profile uuid, p_seed_ids uuid[], p_depth int)
RETURNS TABLE (resource_id uuid, source_id uuid, target_id uuid,
               edge_kind edge_kind, polarity edge_polarity, label text, depth int)
LANGUAGE sql STABLE AS $$
  WITH RECURSIVE visible AS (SELECT rv.resource_id AS id FROM resources_visible_to(p_profile) rv),
  walk AS (
    SELECT e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, 1 AS depth
      FROM kb_edges e
     WHERE e.source_table='kb_resources' AND e.target_table='kb_resources'
       AND e.source_id = ANY(p_seed_ids) AND NOT e.is_folded
       AND e.source_id IN (SELECT id FROM visible) AND e.target_id IN (SELECT id FROM visible)
    UNION
    SELECT e.source_id, e.target_id, e.edge_kind, e.polarity, e.label, w.depth+1
      FROM kb_edges e JOIN walk w ON e.source_id = w.target_id
     WHERE e.source_table='kb_resources' AND e.target_table='kb_resources'
       AND NOT e.is_folded AND w.depth < p_depth
       AND e.target_id IN (SELECT id FROM visible)
  )
  SELECT w.target_id, w.source_id, w.target_id, w.edge_kind, w.polarity, w.label, w.depth FROM walk w;
$$;

CREATE OR REPLACE FUNCTION graph_subgraph_nodes(
  p_profile uuid, p_context_name varchar, p_aggregator_types text[], p_depth int)
RETURNS TABLE (resource_id uuid, slug varchar, title text, doc_type varchar,
               edge_count int, session_count int, first_chunk text, stage_raw text)
LANGUAGE sql STABLE AS $$
  WITH ctx AS (SELECT id FROM kb_contexts WHERE name = p_context_name),
  doc AS (  -- doc_type property per resource
    SELECT p.owner_id AS rid, p.property_value #>> '{}' AS dt
      FROM kb_properties p
     WHERE p.owner_table='kb_resources' AND p.property_key='doc_type' AND NOT p.is_folded),
  seeds AS (
    SELECT r.id
      FROM kb_resources r
      JOIN kb_resource_homes h ON h.resource_id=r.id AND h.anchor_table='kb_contexts'
      JOIN ctx ON ctx.id = h.anchor_id
      JOIN doc ON doc.rid = r.id
     WHERE r.is_active AND doc.dt = ANY(p_aggregator_types)),
  walked AS (
    SELECT DISTINCT t.resource_id AS id
      FROM graph_traverse(p_profile, ARRAY(SELECT id FROM seeds), p_depth) t
    UNION SELECT id FROM seeds),
  nodes AS (
    SELECT r.id, doc.dt AS doc_type, r.title FROM kb_resources r
      JOIN walked w ON w.id=r.id JOIN doc ON doc.rid=r.id
     WHERE r.is_active AND doc.dt <> 'session')  -- sessions are not nodes
  SELECT
    n.id,
    -- slug retired in substrate (§7-dissolved); derive from title to match Rust text::slugify:
    -- lowercase, non-alphanumeric runs → single dash, trim leading/trailing dashes. Presentational.
    lower(regexp_replace(regexp_replace(n.title, '[^a-zA-Z0-9]+', '-', 'g'), '(^-+|-+$)', '', 'g'))::varchar AS slug,
    n.title,
    n.doc_type::varchar,
    (SELECT count(*)::int FROM kb_edges e
       WHERE NOT e.is_folded AND e.source_table='kb_resources' AND e.target_table='kb_resources'
         AND (e.source_id=n.id OR e.target_id=n.id)) AS edge_count,
    -- session adjacency: 0 until re-modelled. The legacy session-count join depended on kb_doc_types
    -- (dropped in the substrate); re-modelling session adjacency onto properties is a follow-up.
    -- GraphNode.session_count stays valid (0) so the UI degrades gracefully.
    0::int AS session_count,
    (SELECT cc.content FROM kb_chunks ch
       JOIN kb_content_blocks b ON b.id=ch.block_id
       JOIN kb_chunk_content cc ON cc.chunk_id=ch.id
      WHERE ch.resource_id=n.id AND ch.is_current AND NOT b.is_folded
      ORDER BY b.seq, ch.chunk_index LIMIT 1) AS first_chunk,
    (SELECT sp.property_value #>> '{}' FROM kb_properties sp
      WHERE sp.owner_table='kb_resources' AND sp.owner_id=n.id
        AND sp.property_key='temper-stage' AND NOT sp.is_folded LIMIT 1) AS stage_raw
  FROM nodes n;
$$;

-- ============================================================================
-- SYSTEM ACCESS GATE (WS6 graft) — bodies byte-identical to the artifact.
-- ============================================================================

CREATE OR REPLACE FUNCTION has_system_access(p_profile_id UUID) RETURNS BOOLEAN
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
        )
        ELSE false
    END
      FROM settings
$$;

CREATE OR REPLACE FUNCTION is_system_admin(p_profile_id UUID) RETURNS BOOLEAN
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
           AND tm.role = 'owner'
    )
      FROM settings
$$;
