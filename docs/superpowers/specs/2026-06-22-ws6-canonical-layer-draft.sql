-- ============================================================================
-- DRAFT — WS6 endgame: canonical identity/auth/infra layer
-- ----------------------------------------------------------------------------
-- NOT a live migration. A review artifact. Do NOT place in migrations/ or load
-- via the schema-artifact harness until the collapse is authored for execution.
--
-- WHAT THIS IS. The already-live `temper_next` substrate (schema-artifact/
-- 01_schema.sql + 02_functions.sql) is the canonical resource/content/cogmap/
-- event model. It deliberately OMITS the operational Domain-A identity/auth/infra
-- tables (01_schema header "Out of scope for this artifact"). This file is the
-- additive GRAFT + RECONCILE + CARRY-OVER layer that, applied on top of the live
-- substrate at collapse, completes the single canonical schema.
--
-- It mirrors how the collapse actually executes: the substrate is already live;
-- we (a) reconcile the shared `kb_profiles`, (b) graft the 7 substrate-absent
-- infra tables + their enums, (c) carry the identity/seed data from the
-- renamed-aside legacy schema. It is NOT a from-scratch full schema.
--
-- Grounding:
--   docs/superpowers/specs/2026-06-22-ws6-migration-endgame-design.md  (sequencing)
--   docs/superpowers/specs/2026-06-22-ws6-endgame-schema-diff.md       (Flag 1 + correction)
--   migrations/20260330000001_consolidated_schema.sql                  (verbatim infra DDL)
--   migrations/20260407000001_system_access_gate.sql                   (system_settings, join_requests)
--
-- LEGACY = the renamed-aside stale `public` schema at collapse time (e.g.
-- `public_legacy`). Set the real name when the collapse step is authored.
-- ============================================================================

SET search_path TO temper_next, public;   -- canonical namespace at collapse (pre-rename to `public`)

-- ============================================================================
-- 1. SHARED TABLE RECONCILIATION — kb_profiles
-- ----------------------------------------------------------------------------
-- Substrate kb_profiles = (id, handle, display_name, system_access, created).
-- Public dropped in substrate: email, avatar_url, preferences, vault_config,
-- is_active, updated; slug → handle (already native). Decisions (Flag 1):
--   re-add  : email, preferences
--   drop    : avatar_url, vault_config (vestigial under cloud-only),
--             is_active (subsumed by system_access='none'), updated
--   handle  : public.slug maps to substrate.handle (1:1, both NOT NULL UNIQUE)
-- ============================================================================

ALTER TABLE kb_profiles ADD COLUMN email       VARCHAR(256);
ALTER TABLE kb_profiles ADD COLUMN preferences JSONB NOT NULL DEFAULT '{}'::jsonb;

-- ============================================================================
-- 2. ENUMS for the grafted infra layer (absent from substrate)
-- ----------------------------------------------------------------------------
-- team_role already exists in the substrate (01_schema §ENUMS). porosity is
-- intentionally NOT re-added — it is RETIRED (visibility is teams:RBAC), and
-- kb_scopes is superseded (renamed → kb_cogmaps), see Flag-1 correction.
-- ============================================================================

CREATE TYPE join_request_status AS ENUM ('pending', 'approved', 'rejected', 'withdrawn');
CREATE TYPE invitation_status   AS ENUM ('pending', 'accepted', 'declined', 'expired');
CREATE TYPE transfer_status     AS ENUM ('pending', 'accepted', 'declined', 'cancelled');

-- ============================================================================
-- 3. GRAFTED INFRA TABLES (7) — verbatim from migrations, FKs target
--    substrate-present ids (kb_profiles / kb_teams / kb_resources).
-- ----------------------------------------------------------------------------
-- NOTE [harmonization, follow-up]: public PK id-defaults are inconsistent
-- (bare UUID = app-supplied; kb_blob_files = gen_random_uuid()). The substrate
-- convention is uuid_generate_v7(). Left verbatim here to stay faithful to the
-- source DDL; unify to uuid_generate_v7() when this becomes a real migration.
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
-- 4. IDENTITY DATA CARRY-OVER (from the renamed-aside LEGACY schema)
-- ----------------------------------------------------------------------------
-- Substrate synthesized ONLY the corpus owner (1 kb_profiles row). The other
-- 4 profiles + all 5 auth_links + the system_settings seed must carry over.
-- Done as INSERT...SELECT from LEGACY (NOT hardcoded): correct, idempotent, and
-- keeps third-party PII out of the repo. Confirmed (Flag 1): only the owner owns
-- resources, so no resource-data carry-over is needed.
--
-- system_access is set explicitly (the synth default 'none' is wrong for the
-- owner). Owner → admin; the 2 registered humans → approved (preserves their
-- access_mode='open' + is_active=true status quo); sentinels (system/anonymous)
-- → none. Adjustable.
-- ============================================================================

-- Profiles: owner already present (ON CONFLICT updates it); 4 others inserted.
INSERT INTO kb_profiles (id, handle, display_name, system_access, email, preferences, created)
SELECT p.id, p.slug, p.display_name,
       CASE WHEN p.slug = 'j-cole-taylor'              THEN 'admin'::system_access
            WHEN p.slug IN ('gm-anirudh', 'lohjishan') THEN 'approved'::system_access
            ELSE 'none'::system_access END,
       p.email, p.preferences, p.created
FROM LEGACY.kb_profiles p
ON CONFLICT (id) DO UPDATE
    SET system_access = EXCLUDED.system_access,
        email         = EXCLUDED.email,
        preferences   = EXCLUDED.preferences;

-- Auth links: all 5 (substrate-absent table; every profile_id now resolves).
INSERT INTO kb_profile_auth_links
    (id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at)
SELECT id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at
FROM LEGACY.kb_profile_auth_links;

-- Instance settings: the single live row (access_mode='open').
INSERT INTO kb_system_settings
    (id, access_mode, gating_team_slug, terms_version, terms_resource_uri, instance_name, updated)
SELECT id, access_mode, gating_team_slug, terms_version, terms_resource_uri, instance_name, updated
FROM LEGACY.kb_system_settings
ON CONFLICT (id) DO UPDATE SET access_mode = EXCLUDED.access_mode;

-- ============================================================================
-- OPEN — deferred to later endgame beats (NOT resolved by this draft):
--   * 11 shared-table column reconciliation BEYOND kb_profiles (kb_events,
--     kb_resource_audits, kb_teams, kb_contexts, kb_topics, kb_chunks, ...):
--     canonical = temper_next shape; needs a per-table column diff to confirm
--     no public-only column carries live data. (schema-diff §C)
--   * temper-events scope/porosity disentanglement: ledger.rs/types/scope.rs
--     target the retired scopes model — substrate vs scaffolding call (step 2).
--   * Pre-drop: spot-check the 2 Flag-2 content-hash mismatches before dropping
--     kb_resource_revisions. (gated, execution-phase)
-- ============================================================================
