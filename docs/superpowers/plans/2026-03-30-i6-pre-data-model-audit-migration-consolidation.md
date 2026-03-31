# I6-pre: Data Model Audit & Migration Consolidation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consolidate all database migrations into a single clean foundation, rename tables for consistency, remove unused schema elements, and add I6a prerequisites (resource_mode column, sync_diff_for_device function).

**Architecture:** Replace 5 incremental migrations with 1 consolidated migration. Rename `resources` → `kb_resources` and `blob_files` → `kb_blob_files`. Remove Category E tables (behavior/state-machine/lifecycle). Add `resource_mode` column and `sync_diff_for_device()` SQL function. Update all Rust and TypeScript SQL strings.

**Tech Stack:** PostgreSQL 18 + pgvector 0.8.2, sqlx (Rust), @neondatabase/serverless (TypeScript), Vercel

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `migrations/20260330000001_consolidated_schema.sql` | Create | Single consolidated schema migration |
| `migrations/20260330000002_seed.sql` | Create | Seed data (contexts, doc_types, profiles — no behaviors/lifecycle) |
| `migrations/20260326000001_r2_schema.sql` | Delete | Replaced by consolidated |
| `migrations/20260326000002_r2_seed.sql` | Delete | Replaced by consolidated |
| `migrations/20260328000001_audit_fixes.sql` | Delete | Rolled into consolidated |
| `migrations/20260328000002_contexts_ownership.sql` | Delete | Rolled into consolidated |
| `migrations/20260329000001_blob_files.sql` | Delete | Rolled into consolidated |
| `crates/temper-api/src/services/resource_service.rs` | Modify | `resources` → `kb_resources` in SQL strings |
| `crates/temper-api/src/services/event_service.rs` | Modify | No table name changes needed (only uses `kb_events` and `resources_visible_to`) |
| `crates/temper-api/tests/common/fixtures.rs` | Modify | `resources` → `kb_resources`, remove cleanup of deleted tables |
| `packages/temper-cloud/src/ingest.ts` | Modify | `resources` → `kb_resources` in SQL strings |
| `packages/temper-cloud/src/upload.ts` | Modify | `blob_files` → `kb_blob_files` in SQL strings |
| `packages/temper-cloud/src/processing/store.ts` | Modify | `blob_files` → `kb_blob_files` in SQL strings |
| `api/upload.ts` | Modify | No direct table name changes (uses `kb_profiles`, `resources_visible_to`, and `buildInsertBlobFileQuery`) |
| `api/ingest/[id].ts` | Modify | `resources` → `kb_resources` in SQL strings |

---

### Task 1: Write Consolidated Schema Migration

**Files:**
- Create: `migrations/20260330000001_consolidated_schema.sql`
- Delete: `migrations/20260326000001_r2_schema.sql`
- Delete: `migrations/20260328000001_audit_fixes.sql`
- Delete: `migrations/20260328000002_contexts_ownership.sql`
- Delete: `migrations/20260329000001_blob_files.sql`

This task produces the single consolidated migration containing all tables, functions, views, and indexes.

- [ ] **Step 1: Delete old migration files**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
rm migrations/20260326000001_r2_schema.sql
rm migrations/20260328000001_audit_fixes.sql
rm migrations/20260328000002_contexts_ownership.sql
rm migrations/20260329000001_blob_files.sql
```

- [ ] **Step 2: Write the consolidated schema migration**

Create `migrations/20260330000001_consolidated_schema.sql` with the complete schema. Key changes from the original:

**Renames:**
- `resources` → `kb_resources`
- `blob_files` → `kb_blob_files`

**New elements:**
- `resource_mode VARCHAR(16) NOT NULL DEFAULT 'added'` column on `kb_resources` with CHECK constraint and index
- `sync_diff_for_device()` SQL function

**Removed (Category E):**
- `kb_behaviors` table
- `kb_doc_type_behaviors` table
- `kb_lifecycle_stages` table
- `kb_workflowable_states` table
- `kb_sequenceable_states` table
- `kb_assignable_states` table
- `kb_taggable_states` table

**Retained (Category D):**
- `kb_ingestion_records` — future re-ingestion provenance
- `kb_transfers` — future ownership transfer

**Note on kb_events:** Retained as-is. Open design question for I6b: how to bridge local events.jsonl ↔ cloud kb_events, and whether event IDs should annotate document changes for merge reconciliation.

```sql
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
    id           UUID PRIMARY KEY, -- UUIDv7
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
    id                    UUID PRIMARY KEY, -- UUIDv7
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
    id            UUID PRIMARY KEY, -- UUIDv7
    profile_id    UUID NOT NULL REFERENCES kb_profiles(id),
    client_id     VARCHAR(64) NOT NULL,
    last_sync_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    manifest_hash VARCHAR(64),
    UNIQUE(profile_id, client_id)
);
CREATE INDEX idx_device_sync_profile ON kb_device_sync_state(profile_id);

-- ─── Teams ───────────────────────────────────────────────────────────────────

CREATE TABLE kb_teams (
    id                   UUID PRIMARY KEY, -- UUIDv7
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
    id                    UUID PRIMARY KEY, -- UUIDv7
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
    id                  UUID PRIMARY KEY, -- UUIDv7
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
    id                    UUID PRIMARY KEY, -- UUIDv7
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
    id              UUID PRIMARY KEY, -- UUIDv7
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

-- Access control: which resources can a profile see?
CREATE FUNCTION resources_visible_to(
    p_profile_id UUID,
    p_team_id    UUID DEFAULT NULL
) RETURNS TABLE (resource_id UUID, access_level VARCHAR(32), via VARCHAR(256))
LANGUAGE SQL STABLE AS $$
    -- Resources owned by this profile
    SELECT r.id AS resource_id,
           'vault'::VARCHAR(32) AS access_level,
           ('owner:' || p_profile_id)::VARCHAR(256) AS via
      FROM kb_resources r
     WHERE r.owner_profile_id = p_profile_id
       AND r.is_active = true

    UNION ALL

    -- Resources shared via team membership
    SELECT tr.resource_id,
           tr.access_level::VARCHAR(32),
           ('team:' || t.slug)::VARCHAR(256) AS via
      FROM kb_team_resources tr
      JOIN kb_teams t ON t.id = tr.team_id AND t.is_active = true
      JOIN kb_team_members tm ON tm.team_id = t.id AND tm.profile_id = p_profile_id
      JOIN kb_resources r ON r.id = tr.resource_id AND r.is_active = true
     WHERE (p_team_id IS NULL OR t.id = p_team_id)
$$;

-- Can a profile modify a specific resource?
CREATE FUNCTION can_modify_resource(
    p_profile_id  UUID,
    p_resource_id UUID
) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    SELECT EXISTS (
        -- Owner can always modify
        SELECT 1 FROM kb_resources
         WHERE id = p_resource_id
           AND owner_profile_id = p_profile_id
           AND is_active = true

        UNION ALL

        -- Team member with sufficient access
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

-- Team admin authorization
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

-- Context visibility scoping
CREATE FUNCTION contexts_visible_to(
    p_profile_id UUID,
    p_team_id    UUID DEFAULT NULL
) RETURNS TABLE (id UUID, name VARCHAR(128), kb_owner_table VARCHAR(64), kb_owner_id UUID)
LANGUAGE SQL STABLE AS $$
    -- Contexts owned by this profile
    SELECT c.id, c.name, c.kb_owner_table, c.kb_owner_id
      FROM kb_contexts c
     WHERE c.kb_owner_table = 'kb_profiles'
       AND c.kb_owner_id = p_profile_id

    UNION ALL

    -- Contexts owned by teams this profile belongs to
    SELECT c.id, c.name, c.kb_owner_table, c.kb_owner_id
      FROM kb_contexts c
      JOIN kb_teams t ON t.id = c.kb_owner_id AND c.kb_owner_table = 'kb_teams' AND t.is_active = true
      JOIN kb_team_members tm ON tm.team_id = t.id AND tm.profile_id = p_profile_id
     WHERE (p_team_id IS NULL OR t.id = p_team_id)
$$;

-- Sync diff computation
-- Returns the set of resources that differ between client manifest and server state.
-- Three-hash comparison: local_hash (current file), remote_hash (last known server hash),
-- server_hash (current server content_hash).
CREATE FUNCTION sync_diff_for_device(
    p_profile_id    UUID,
    p_context_names TEXT[],
    p_manifest      JSONB  -- [{"uri": "...", "local_hash": "...", "remote_hash": "..."}, ...]
) RETURNS TABLE (
    resource_id UUID,
    uri         TEXT,
    content_hash VARCHAR(64),
    updated     TIMESTAMPTZ,
    diff_type   VARCHAR(16)  -- 'to_push', 'to_pull', 'conflict', 'removed'
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
    -- Server resource exists, manifest entry exists: compare hashes
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
            -- local_hash == remote_hash == server_hash → clean, omitted
        END AS diff_type
      FROM server_resources sr
      JOIN manifest_entries me ON me.uri = sr.uri
     WHERE sr.is_active = false
        OR me.local_hash != sr.content_hash  -- skip clean entries

    UNION ALL

    -- Server resource exists but not in manifest → new remote resource to pull
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

    -- Manifest entry exists but no server resource → new local resource to push
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
```

- [ ] **Step 3: Verify the SQL file is syntactically valid (visual review)**

Open the file and scan for:
- All FKs point to `kb_resources` (not `resources`)
- All FKs point to `kb_blob_files` (not `blob_files`)
- No references to removed tables (`kb_behaviors`, `kb_doc_type_behaviors`, `kb_lifecycle_stages`, `kb_workflowable_states`, `kb_sequenceable_states`, `kb_assignable_states`, `kb_taggable_states`)
- `resource_mode` column exists on `kb_resources` with CHECK and index

---

### Task 2: Write Consolidated Seed Migration

**Files:**
- Create: `migrations/20260330000002_seed.sql`
- Delete: `migrations/20260326000002_r2_seed.sql`

- [ ] **Step 1: Delete old seed migration**

```bash
rm migrations/20260326000002_r2_seed.sql
```

- [ ] **Step 2: Write the consolidated seed**

Create `migrations/20260330000002_seed.sql`. Removes behavior seeds, lifecycle stage seeds, and doc_type_behavior junction data. Retains doc_types, contexts, and profiles.

```sql
-- =============================================================================
-- Consolidated Seed Data — I6-pre
-- =============================================================================
-- Replaces: r2_seed.sql
-- Removed: kb_behaviors, kb_doc_type_behaviors, kb_lifecycle_stages seeds

-- ─── Doc Types (system-level, not tenant-scoped) ─────────────────────────────
INSERT INTO kb_doc_types (id, name) VALUES
    ('00000000-0000-0000-0001-000000000001', 'ticket'),
    ('00000000-0000-0000-0001-000000000002', 'session'),
    ('00000000-0000-0000-0001-000000000003', 'milestone'),
    ('00000000-0000-0000-0001-000000000004', 'research'),
    ('00000000-0000-0000-0001-000000000005', 'board'),
    ('00000000-0000-0000-0001-000000000006', 'concept'),
    ('00000000-0000-0000-0001-000000000007', 'source');

-- ─── Seed Profiles ───────────────────────────────────────────────────────────
INSERT INTO kb_profiles (id, display_name) VALUES
    ('00000000-0000-0000-0004-000000000001', 'System'),
    ('00000000-0000-0000-0004-000000000002', 'Anonymous');

INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id, is_default) VALUES
    ('00000000-0000-0000-0005-000000000001', '00000000-0000-0000-0004-000000000001', 'system', 'system', true),
    ('00000000-0000-0000-0005-000000000002', '00000000-0000-0000-0004-000000000002', 'anonymous', 'anonymous', true);

-- ─── Contexts (owned by System profile) ──────────────────────────────────────
INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id) VALUES
    ('00000000-0000-0000-0003-000000000001', 'temper', 'kb_profiles', '00000000-0000-0000-0004-000000000001'),
    ('00000000-0000-0000-0003-000000000002', 'storyteller', 'kb_profiles', '00000000-0000-0000-0004-000000000001'),
    ('00000000-0000-0000-0003-000000000003', 'tasker', 'kb_profiles', '00000000-0000-0000-0004-000000000001'),
    ('00000000-0000-0000-0003-000000000004', 'knowledge', 'kb_profiles', '00000000-0000-0000-0004-000000000001'),
    ('00000000-0000-0000-0003-000000000005', 'writing', 'kb_profiles', '00000000-0000-0000-0004-000000000001');
```

- [ ] **Step 3: Commit migration files**

```bash
git add migrations/
git commit -m "refactor: consolidate migrations into clean I6-pre foundation

Remove 5 incremental migrations, replace with single consolidated schema.
Renames: resources → kb_resources, blob_files → kb_blob_files.
Adds: resource_mode column, sync_diff_for_device() function.
Removes: kb_behaviors, kb_doc_type_behaviors, kb_lifecycle_stages,
kb_workflowable_states, kb_sequenceable_states, kb_assignable_states,
kb_taggable_states and their seed data.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Reset Local Dev Database and Apply Migration

**Files:**
- No file changes — database operations only

- [ ] **Step 1: Verify Docker is running**

```bash
docker compose -f /Users/petetaylor/projects/tasker-systems/temper/docker-compose.yml ps
```

Expected: `temper-postgres` container is running and healthy. If not:

```bash
docker compose -f /Users/petetaylor/projects/tasker-systems/temper/docker-compose.yml up -d
```

- [ ] **Step 2: Drop and recreate the database**

```bash
docker exec temper-postgres psql -U temper -d postgres -c "DROP DATABASE IF EXISTS temper_development;"
docker exec temper-postgres psql -U temper -d postgres -c "CREATE DATABASE temper_development OWNER temper;"
```

- [ ] **Step 3: Run sqlx migrations**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development sqlx migrate run
```

Expected: Both migrations applied successfully.

- [ ] **Step 4: Verify schema — check key tables exist with correct names**

```bash
docker exec temper-postgres psql -U temper -d temper_development -c "\dt kb_*"
```

Expected output should list: `kb_blob_files`, `kb_chunks`, `kb_contexts`, `kb_device_sync_state`, `kb_doc_types`, `kb_events`, `kb_ingestion_records`, `kb_profiles`, `kb_profile_auth_links`, `kb_resources`, `kb_teams`, `kb_team_members`, `kb_team_resources`, `kb_team_invitations`, `kb_transfers`.

Should NOT list: `kb_behaviors`, `kb_doc_type_behaviors`, `kb_lifecycle_stages`, `kb_workflowable_states`, `kb_sequenceable_states`, `kb_assignable_states`, `kb_taggable_states`.

Should NOT list the old names: `resources`, `blob_files`.

- [ ] **Step 5: Verify resource_mode column exists**

```bash
docker exec temper-postgres psql -U temper -d temper_development -c "\d kb_resources" | grep resource_mode
```

Expected: `resource_mode | character varying(16) | not null default 'added'::character varying`

- [ ] **Step 6: Verify sync_diff_for_device function exists**

```bash
docker exec temper-postgres psql -U temper -d temper_development -c "\df sync_diff_for_device"
```

Expected: Function listed with correct signature.

- [ ] **Step 7: Verify seed data**

```bash
docker exec temper-postgres psql -U temper -d temper_development -c "SELECT name FROM kb_doc_types ORDER BY name;"
docker exec temper-postgres psql -U temper -d temper_development -c "SELECT name FROM kb_contexts ORDER BY name;"
docker exec temper-postgres psql -U temper -d temper_development -c "SELECT display_name FROM kb_profiles ORDER BY display_name;"
```

Expected: 7 doc types, 5 contexts, 2 profiles.

---

### Task 4: Update Rust Code for Table Renames

**Files:**
- Modify: `crates/temper-api/src/services/resource_service.rs:22-224`
- Modify: `crates/temper-api/tests/common/fixtures.rs:1-118`

No changes needed in:
- `event_service.rs` — only references `kb_events` (already prefixed) and `resources_visible_to` (function, not table)
- `search_service.rs` — stub with no SQL
- `profile_service.rs` — only references `kb_profiles` and `kb_profile_auth_links` (already prefixed)

- [ ] **Step 1: Update resource_service.rs — rename `resources` → `kb_resources`**

In `crates/temper-api/src/services/resource_service.rs`, replace every occurrence of the table name `resources` in SQL strings with `kb_resources`. The table name appears in these patterns:
- `FROM resources r` → `FROM kb_resources r`
- `INSERT INTO resources` → `INSERT INTO kb_resources`
- `UPDATE resources` → `UPDATE kb_resources`

There are 7 occurrences across the file:
- Line 29: `FROM resources r` (list with context filter)
- Line 53: `FROM resources r` (list all visible)
- Line 81: `FROM resources r` (get single)
- Line 137: `INSERT INTO resources` (create)
- Line 182: `UPDATE resources` (update)
- Line 215: `UPDATE resources` (soft delete)

Note: `resources_visible_to` is a function name, NOT a table reference — do NOT rename it.

- [ ] **Step 2: Update fixtures.rs — rename `resources` → `kb_resources`, remove deleted table cleanup**

In `crates/temper-api/tests/common/fixtures.rs`:

Replace `DELETE FROM resources` with `DELETE FROM kb_resources` (line 63).
Replace `INSERT INTO resources` with `INSERT INTO kb_resources` (line 97-98).

Update the comment on line 19 to remove references to deleted tables:
```rust
    // Delete in reverse FK order. Leave kb_doc_types, kb_contexts,
    // and the two seed profiles intact.
```

(Remove `kb_behaviors` and `kb_lifecycle_stages` from the comment since those tables no longer exist.)

- [ ] **Step 3: Verify Rust compiles**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo check --all-features 2>&1 | tail -20
```

Expected: Compiles without errors. sqlx will verify queries against the local database.

- [ ] **Step 4: Run clippy**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo clippy --all-features -- -D warnings 2>&1 | tail -20
```

Expected: No warnings.

- [ ] **Step 5: Run Rust tests**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo test --all-features 2>&1 | tail -30
```

Expected: All tests pass. The test database is the same as dev (the fixture `clean_and_seed` manages test isolation).

Note: if temper-api tests use a separate test database (`temper_test`), that database also needs to be recreated and migrated. Check `crates/temper-api/.env.template` — it shows `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_test`. If tests fail because the test DB doesn't exist:

```bash
docker exec temper-postgres psql -U temper -d postgres -c "DROP DATABASE IF EXISTS temper_test;"
docker exec temper-postgres psql -U temper -d postgres -c "CREATE DATABASE temper_test OWNER temper;"
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_test sqlx migrate run
```

- [ ] **Step 6: Commit Rust changes**

```bash
git add crates/
git commit -m "refactor: update Rust SQL strings for kb_resources table rename

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Update TypeScript Code for Table Renames

**Files:**
- Modify: `packages/temper-cloud/src/ingest.ts:73-214`
- Modify: `packages/temper-cloud/src/upload.ts:21`
- Modify: `packages/temper-cloud/src/processing/store.ts:67`
- Modify: `api/ingest/[id].ts:63-114`

No changes needed in:
- `api/upload.ts` — references `kb_profiles` (already prefixed), `resources_visible_to` (function), and uses `buildInsertBlobFileQuery` (changes in upload.ts package)
- `api/ingest.ts` — uses functions from `ingest.ts` package (changes there)
- `api/workflows/process-ingest.ts` — only references `kb_chunks` (already prefixed)
- `api/workflows/process-upload.ts` — only references `kb_chunks` (already prefixed)

- [ ] **Step 1: Update packages/temper-cloud/src/ingest.ts**

Replace `FROM resources` with `FROM kb_resources` (line 76).
Replace `INSERT INTO resources` with `INSERT INTO kb_resources` (lines 173-176).
Replace `UPDATE resources` with `UPDATE kb_resources` (line 212).

There are 3 SQL statements to update. Do NOT change `resources_visible_to` (function name, not table).

- [ ] **Step 2: Update packages/temper-cloud/src/upload.ts**

Replace `INSERT INTO blob_files` with `INSERT INTO kb_blob_files` (line 21).

- [ ] **Step 3: Update packages/temper-cloud/src/processing/store.ts**

Replace `UPDATE blob_files` with `UPDATE kb_blob_files` (line 67).

- [ ] **Step 4: Update api/ingest/[id].ts**

Replace `FROM resources` with `FROM kb_resources` (lines 66 and 113).

There are 2 SQL statements to update.

- [ ] **Step 5: Run TypeScript type check**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
npx tsc --noEmit 2>&1 | tail -20
```

Expected: No type errors.

- [ ] **Step 6: Run biome lint**

```bash
npx @biomejs/biome check api/ packages/ 2>&1 | tail -20
```

Expected: No lint errors.

- [ ] **Step 7: Commit TypeScript changes**

```bash
git add packages/ api/
git commit -m "refactor: update TypeScript SQL strings for table renames

kb_resources (was resources), kb_blob_files (was blob_files)

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Final Verification and Neon Prod Reconciliation Plan

**Files:**
- No code changes

- [ ] **Step 1: Run full pre-commit verification**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo check --all-features
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo clippy --all-features -- -D warnings
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo test --all-features
npx tsc --noEmit
npx @biomejs/biome check api/ packages/
```

Expected: All pass.

- [ ] **Step 2: Document Neon prod reconciliation plan**

The Neon production database has never been deployed with real data (per task spec: "nothing is deployed to prod, we can wipe it and begin again"). Reconciliation plan:

1. Drop the Neon production database (or create a fresh branch)
2. Run `sqlx migrate run` against the Neon connection string
3. Verify with `\dt kb_*` and `\df sync_diff_for_device`

This does NOT need to happen in this task — it's documented here for when I6a deployment occurs.

- [ ] **Step 3: Verify git status is clean**

```bash
git status
```

Expected: No untracked or modified files (all committed in previous tasks).
