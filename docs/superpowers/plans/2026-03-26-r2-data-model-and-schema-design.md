# R2: Data Model & Schema Design — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Validate the R2 schema blueprint against a real Postgres instance using sqlx migrations, produce the research deliverable, and close the R2 ticket.

**Architecture:** Set up sqlx with proper migration infrastructure following the storyteller pattern (forward-only, timestamp-named, `sqlx::migrate!()` macro). Run migrations against local pgvector-enabled Postgres (docker-compose, port 5437). Validate the full resource lifecycle end-to-end.

**Tech Stack:** PostgreSQL 18 + pgvector 0.8.2 (local Docker), sqlx 0.8 + sqlx-cli, temper CLI for research note management.

**References:**
- R2 design spec: `docs/superpowers/specs/2026-03-26-r2-data-model-and-schema-design.md`
- Storyteller migration pattern: `../storyteller/crates/storyteller-storykeeper/migrations/`
- Storyteller migrator: `../storyteller/crates/storyteller-storykeeper/src/database/migrator.rs`

---

### Task 1: Add sqlx Dependency and Migration Infrastructure

**Files:**
- Modify: `Cargo.toml`
- Create: `.env`
- Create: `migrations/` directory (sqlx-cli creates this)

- [ ] **Step 1: Add sqlx to Cargo.toml**

Add sqlx dependency with postgres, uuid, chrono, json, migrate features:

```toml
sqlx = { version = "0.8", features = ["chrono", "json", "macros", "migrate", "postgres", "runtime-tokio-rustls", "uuid"] }
```

Add to `[dependencies]` section of `Cargo.toml`.

- [ ] **Step 2: Create .env with DATABASE_URL**

```bash
echo 'DATABASE_URL=postgres://temper:temper@localhost:5437/temper_development' > .env
```

- [ ] **Step 3: Verify Postgres is running and accessible**

```bash
docker compose -f docker-compose.yml up -d
psql -h localhost -p 5437 -U temper -d temper_development -c "SELECT version();"
```

Expected: PostgreSQL 18.x with pgvector.

- [ ] **Step 4: Install sqlx-cli if not present**

```bash
cargo install sqlx-cli --no-default-features --features postgres
```

- [ ] **Step 5: Verify sqlx-cli works**

```bash
sqlx database create --database-url postgres://temper:temper@localhost:5437/temper_development
```

Expected: Database already exists (created by docker-compose), no error.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml .env
git commit -m "chore: add sqlx dependency and database configuration"
```

### Task 2: Create Schema Migration

**Files:**
- Create: `migrations/20260326000001_r2_schema.sql`

- [ ] **Step 1: Create the migration via sqlx-cli**

```bash
sqlx migrate add r2_schema
```

This creates `migrations/<timestamp>_r2_schema.sql`. Rename to use the deterministic timestamp format:

```bash
mv migrations/*_r2_schema.sql migrations/20260326000001_r2_schema.sql
```

- [ ] **Step 2: Write the schema DDL**

Write the full R2 schema into `migrations/20260326000001_r2_schema.sql`:

```sql
-- R2: Data Model & Schema Design — Temper Cloud Schema
-- Postgres 18 + pgvector 0.8.2
-- Design spec: docs/superpowers/specs/2026-03-26-r2-data-model-and-schema-design.md

-- Extensions
CREATE EXTENSION IF NOT EXISTS vector;

-- Scoping mechanism (replaces "project" concept at persistence layer)
CREATE TABLE kb_contexts (
    id          UUID PRIMARY KEY,
    name        VARCHAR(128) NOT NULL UNIQUE,
    created     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Document type definitions
CREATE TABLE kb_doc_types (
    id          UUID PRIMARY KEY,
    name        VARCHAR(64) NOT NULL UNIQUE,
    created     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Behavior definitions
CREATE TABLE kb_behaviors (
    id          UUID PRIMARY KEY,
    name        VARCHAR(64) NOT NULL UNIQUE,
    created     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Doc type → behavior composition
CREATE TABLE kb_doc_type_behaviors (
    kb_doc_type_id  UUID NOT NULL REFERENCES kb_doc_types(id),
    kb_behavior_id  UUID NOT NULL REFERENCES kb_behaviors(id),
    PRIMARY KEY (kb_doc_type_id, kb_behavior_id)
);

-- Auth profiles (lightweight — full auth design is R4 scope)
CREATE TABLE kb_profiles (
    id              UUID PRIMARY KEY,
    provider        VARCHAR(32) NOT NULL,
    external_id     VARCHAR(128),
    display_name    VARCHAR(128) NOT NULL,
    email           VARCHAR(256),
    created         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(provider, external_id)
);

-- Core resource table
CREATE TABLE resources (
    id              UUID PRIMARY KEY,
    kb_context_id   UUID NOT NULL REFERENCES kb_contexts(id),
    kb_doc_type_id  UUID NOT NULL REFERENCES kb_doc_types(id),
    uri             TEXT NOT NULL,
    title           TEXT NOT NULL,
    slug            VARCHAR(256),
    content_hash    VARCHAR(64),
    mimetype        VARCHAR(128),
    created         TIMESTAMPTZ NOT NULL,
    updated         TIMESTAMPTZ NOT NULL,
    UNIQUE(slug, kb_context_id),
    UNIQUE(uri)
);

CREATE INDEX idx_resources_context ON resources(kb_context_id);
CREATE INDEX idx_resources_doc_type ON resources(kb_doc_type_id);
CREATE INDEX idx_resources_updated ON resources(updated);

-- Lifecycle stages per doc type (data-driven)
CREATE TABLE kb_lifecycle_stages (
    id              UUID PRIMARY KEY,
    kb_doc_type_id  UUID NOT NULL REFERENCES kb_doc_types(id),
    name            VARCHAR(64) NOT NULL,
    seq             INT NOT NULL,
    UNIQUE(kb_doc_type_id, name)
);

-- Workflowable state
CREATE TABLE kb_workflowable_states (
    resource_id     UUID PRIMARY KEY REFERENCES resources(id) ON DELETE CASCADE,
    stage_id        UUID NOT NULL REFERENCES kb_lifecycle_stages(id),
    updated         TIMESTAMPTZ NOT NULL
);

-- Sequenceable state
CREATE TABLE kb_sequenceable_states (
    resource_id     UUID PRIMARY KEY REFERENCES resources(id) ON DELETE CASCADE,
    seq             INT NOT NULL,
    parent_id       UUID REFERENCES resources(id),
    updated         TIMESTAMPTZ NOT NULL
);

-- Assignable state
CREATE TABLE kb_assignable_states (
    resource_id     UUID PRIMARY KEY REFERENCES resources(id) ON DELETE CASCADE,
    author          VARCHAR(128),
    assignee        VARCHAR(128),
    metadata        JSONB NOT NULL DEFAULT '{}',
    updated         TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_assignable_metadata ON kb_assignable_states USING GIN(metadata);

-- Taggable state
CREATE TABLE kb_taggable_states (
    resource_id     UUID PRIMARY KEY REFERENCES resources(id) ON DELETE CASCADE,
    tags            TEXT[] NOT NULL DEFAULT '{}',
    updated         TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_taggable_tags ON kb_taggable_states USING GIN(tags);

-- Versioned chunks with content addressing
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

-- HNSW index only over current chunks
CREATE INDEX idx_chunks_current_embedding ON kb_chunks
    USING hnsw(embedding vector_cosine_ops)
    WITH (m = 16, ef_construction = 200)
    WHERE is_current = true;

-- Current document state view
CREATE VIEW kb_current_chunks AS
SELECT id, resource_id, chunk_index, version, header_path,
       content, content_hash, embedding, created
FROM kb_chunks
WHERE is_current = true
ORDER BY resource_id, chunk_index;

-- Ingestion provenance
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

-- Events (local jsonl drains here on sync)
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
```

- [ ] **Step 3: Run the migration**

```bash
sqlx migrate run --database-url postgres://temper:temper@localhost:5437/temper_development
```

Expected: Migration applied successfully. `_sqlx_migrations` table created with one entry.

- [ ] **Step 4: Verify tables exist**

```bash
psql -h localhost -p 5437 -U temper -d temper_development -c "\dt kb_* resources"
```

Expected: All 14 tables listed.

- [ ] **Step 5: Verify indexes and view**

```bash
psql -h localhost -p 5437 -U temper -d temper_development -c "\di idx_*"
psql -h localhost -p 5437 -U temper -d temper_development -c "\dv kb_*"
```

Expected: All 11 indexes (including partial HNSW `idx_chunks_current_embedding`) and `kb_current_chunks` view.

- [ ] **Step 6: Commit**

```bash
git add migrations/20260326000001_r2_schema.sql
git commit -m "feat: add R2 schema migration — temper cloud tables, indexes, views"
```

### Task 3: Create Seed Data Migration

**Files:**
- Create: `migrations/20260326000002_r2_seed.sql`

- [ ] **Step 1: Create the seed migration**

```bash
sqlx migrate add r2_seed
mv migrations/*_r2_seed.sql migrations/20260326000002_r2_seed.sql
```

- [ ] **Step 2: Write seed data**

Write into `migrations/20260326000002_r2_seed.sql`:

```sql
-- R2: Seed data — behaviors, doc types, compositions, lifecycle stages, contexts, profiles
-- Deterministic UUIDs for reproducibility (production uses UUIDv7)

-- Behaviors
INSERT INTO kb_behaviors (id, name) VALUES
    ('00000000-0000-0000-0000-000000000001', 'workflowable'),
    ('00000000-0000-0000-0000-000000000002', 'sequenceable'),
    ('00000000-0000-0000-0000-000000000003', 'assignable'),
    ('00000000-0000-0000-0000-000000000004', 'taggable');

-- Doc types
INSERT INTO kb_doc_types (id, name) VALUES
    ('00000000-0000-0000-0001-000000000001', 'ticket'),
    ('00000000-0000-0000-0001-000000000002', 'session'),
    ('00000000-0000-0000-0001-000000000003', 'milestone'),
    ('00000000-0000-0000-0001-000000000004', 'research'),
    ('00000000-0000-0000-0001-000000000005', 'board'),
    ('00000000-0000-0000-0001-000000000006', 'concept'),
    ('00000000-0000-0000-0001-000000000007', 'source');

-- ticket: workflowable, sequenceable, assignable, taggable
INSERT INTO kb_doc_type_behaviors (kb_doc_type_id, kb_behavior_id) VALUES
    ('00000000-0000-0000-0001-000000000001', '00000000-0000-0000-0000-000000000001'),
    ('00000000-0000-0000-0001-000000000001', '00000000-0000-0000-0000-000000000002'),
    ('00000000-0000-0000-0001-000000000001', '00000000-0000-0000-0000-000000000003'),
    ('00000000-0000-0000-0001-000000000001', '00000000-0000-0000-0000-000000000004');
-- session: taggable
INSERT INTO kb_doc_type_behaviors (kb_doc_type_id, kb_behavior_id) VALUES
    ('00000000-0000-0000-0001-000000000002', '00000000-0000-0000-0000-000000000004');
-- milestone: sequenceable
INSERT INTO kb_doc_type_behaviors (kb_doc_type_id, kb_behavior_id) VALUES
    ('00000000-0000-0000-0001-000000000003', '00000000-0000-0000-0000-000000000002');
-- research: taggable
INSERT INTO kb_doc_type_behaviors (kb_doc_type_id, kb_behavior_id) VALUES
    ('00000000-0000-0000-0001-000000000004', '00000000-0000-0000-0000-000000000004');
-- concept: taggable
INSERT INTO kb_doc_type_behaviors (kb_doc_type_id, kb_behavior_id) VALUES
    ('00000000-0000-0000-0001-000000000006', '00000000-0000-0000-0000-000000000004');
-- source: taggable
INSERT INTO kb_doc_type_behaviors (kb_doc_type_id, kb_behavior_id) VALUES
    ('00000000-0000-0000-0001-000000000007', '00000000-0000-0000-0000-000000000004');

-- Ticket lifecycle stages
INSERT INTO kb_lifecycle_stages (id, kb_doc_type_id, name, seq) VALUES
    ('00000000-0000-0000-0002-000000000001', '00000000-0000-0000-0001-000000000001', 'backlog', 10),
    ('00000000-0000-0000-0002-000000000002', '00000000-0000-0000-0001-000000000001', 'design', 20),
    ('00000000-0000-0000-0002-000000000003', '00000000-0000-0000-0001-000000000001', 'in-progress', 30),
    ('00000000-0000-0000-0002-000000000004', '00000000-0000-0000-0001-000000000001', 'done', 40),
    ('00000000-0000-0000-0002-000000000005', '00000000-0000-0000-0001-000000000001', 'cancelled', 50);

-- Milestone lifecycle stages
INSERT INTO kb_lifecycle_stages (id, kb_doc_type_id, name, seq) VALUES
    ('00000000-0000-0000-0002-000000000006', '00000000-0000-0000-0001-000000000003', 'active', 10),
    ('00000000-0000-0000-0002-000000000007', '00000000-0000-0000-0001-000000000003', 'complete', 20);

-- Contexts (from temper.toml projects)
INSERT INTO kb_contexts (id, name) VALUES
    ('00000000-0000-0000-0003-000000000001', 'temper'),
    ('00000000-0000-0000-0003-000000000002', 'storyteller'),
    ('00000000-0000-0000-0003-000000000003', 'tasker'),
    ('00000000-0000-0000-0003-000000000004', 'knowledge'),
    ('00000000-0000-0000-0003-000000000005', 'writing');

-- System and anonymous profiles
INSERT INTO kb_profiles (id, provider, external_id, display_name) VALUES
    ('00000000-0000-0000-0004-000000000001', 'system', NULL, 'System'),
    ('00000000-0000-0000-0004-000000000002', 'anonymous', NULL, 'Anonymous');
```

- [ ] **Step 3: Run the seed migration**

```bash
sqlx migrate run --database-url postgres://temper:temper@localhost:5437/temper_development
```

Expected: Second migration applied.

- [ ] **Step 4: Validate behavior compositions**

```bash
psql -h localhost -p 5437 -U temper -d temper_development -c "
SELECT dt.name AS doc_type, b.name AS behavior
FROM kb_doc_type_behaviors dtb
JOIN kb_doc_types dt ON dt.id = dtb.kb_doc_type_id
JOIN kb_behaviors b ON b.id = dtb.kb_behavior_id
ORDER BY dt.name, b.name;
"
```

Expected: 9 rows — ticket has 4 behaviors, concept/research/session/source each have 1, milestone has 1.

- [ ] **Step 5: Validate lifecycle stages**

```bash
psql -h localhost -p 5437 -U temper -d temper_development -c "
SELECT dt.name AS doc_type, ls.name AS stage, ls.seq
FROM kb_lifecycle_stages ls
JOIN kb_doc_types dt ON dt.id = ls.kb_doc_type_id
ORDER BY dt.name, ls.seq;
"
```

Expected: 7 rows — 5 ticket stages + 2 milestone stages.

- [ ] **Step 6: Commit**

```bash
git add migrations/20260326000002_r2_seed.sql
git commit -m "feat: add R2 seed data — behaviors, doc types, stages, contexts, profiles"
```

### Task 3: Validate End-to-End Resource Lifecycle

**Files:**
- No files created — this is a validation task run interactively via psql

- [ ] **Step 1: Create a test resource with full behavior composition**

```bash
psql -h localhost -p 5437 -U temper -d temper_development -c "
-- Create a ticket resource
INSERT INTO resources (id, kb_context_id, kb_doc_type_id, uri, title, slug, content_hash, mimetype, created, updated)
VALUES (
    '019d2f00-0000-7000-8000-000000000001',
    '00000000-0000-0000-0003-000000000001',
    '00000000-0000-0000-0001-000000000001',
    'kb://tickets/temper/2026-03-26-test-ticket.md',
    'Test Ticket',
    '2026-03-26-test-ticket',
    'abc123def456',
    'text/markdown',
    now(), now()
);

-- Attach all four behavior states
INSERT INTO kb_workflowable_states (resource_id, stage_id, updated)
VALUES ('019d2f00-0000-7000-8000-000000000001', '00000000-0000-0000-0002-000000000001', now());

INSERT INTO kb_sequenceable_states (resource_id, seq, parent_id, updated)
VALUES ('019d2f00-0000-7000-8000-000000000001', 10, NULL, now());

INSERT INTO kb_assignable_states (resource_id, author, assignee, metadata, updated)
VALUES ('019d2f00-0000-7000-8000-000000000001', 'petetaylor', NULL,
    '{\"branch\": \"jc/test-ticket\", \"system\": \"github.com\", \"repo\": \"tasker-systems/temper\"}', now());

INSERT INTO kb_taggable_states (resource_id, tags, updated)
VALUES ('019d2f00-0000-7000-8000-000000000001', ARRAY['testing', 'r2-validation'], now());
"
```

Expected: All 5 inserts succeed.

- [ ] **Step 2: Create versioned chunks and verify content addressing**

```bash
psql -h localhost -p 5437 -U temper -d temper_development -c "
-- Create initial chunks (version 1)
INSERT INTO kb_chunks (id, resource_id, chunk_index, version, header_path, content, content_hash, embedding, is_current)
VALUES
    ('019d2f00-0000-7000-8000-100000000001',
     '019d2f00-0000-7000-8000-000000000001', 0, 1, '',
     'title: Test Ticket | type: ticket | tags: testing, r2-validation',
     'hash_chunk_0_v1',
     (SELECT array_agg(0)::vector(768) FROM generate_series(1, 768)),
     true),
    ('019d2f00-0000-7000-8000-100000000002',
     '019d2f00-0000-7000-8000-000000000001', 1, 1, '## Description',
     'This is the description of the test ticket.',
     'hash_chunk_1_v1',
     (SELECT array_agg(0.1)::vector(768) FROM generate_series(1, 768)),
     true);

-- Simulate content edit: supersede chunk 1, add version 2
UPDATE kb_chunks SET is_current = false
WHERE resource_id = '019d2f00-0000-7000-8000-000000000001' AND chunk_index = 1 AND version = 1;

INSERT INTO kb_chunks (id, resource_id, chunk_index, version, header_path, content, content_hash, embedding, is_current)
VALUES (
    '019d2f00-0000-7000-8000-100000000003',
    '019d2f00-0000-7000-8000-000000000001', 1, 2, '## Description',
    'This is the UPDATED description with more detail.',
    'hash_chunk_1_v2',
    (SELECT array_agg(0.2)::vector(768) FROM generate_series(1, 768)),
    true
);
"
```

Expected: 2 inserts, 1 update, 1 insert — all succeed.

- [ ] **Step 3: Verify the kb_current_chunks view returns only current versions**

```bash
psql -h localhost -p 5437 -U temper -d temper_development -c "
SELECT chunk_index, version, left(content, 60) AS content_preview
FROM kb_current_chunks
WHERE resource_id = '019d2f00-0000-7000-8000-000000000001';
"
```

Expected: 2 rows — chunk 0 version 1, chunk 1 version 2. Superseded chunk 1 version 1 is excluded.

- [ ] **Step 4: Verify tag GIN index query**

```bash
psql -h localhost -p 5437 -U temper -d temper_development -c "
SELECT r.title, t.tags FROM kb_taggable_states t
JOIN resources r ON r.id = t.resource_id WHERE t.tags @> ARRAY['testing'];
"
```

Expected: "Test Ticket" with `{testing,r2-validation}`.

- [ ] **Step 5: Verify JSONB metadata query**

```bash
psql -h localhost -p 5437 -U temper -d temper_development -c "
SELECT r.title, a.author, a.metadata->>'branch' AS branch
FROM kb_assignable_states a JOIN resources r ON r.id = a.resource_id
WHERE a.metadata @> '{\"system\": \"github.com\"}';
"
```

Expected: "Test Ticket", "petetaylor", "jc/test-ticket".

- [ ] **Step 6: Verify CASCADE delete cleans up all related rows**

```bash
psql -h localhost -p 5437 -U temper -d temper_development -c "
DELETE FROM resources WHERE id = '019d2f00-0000-7000-8000-000000000001';

SELECT 'chunks' AS table_name, count(*) FROM kb_chunks WHERE resource_id = '019d2f00-0000-7000-8000-000000000001'
UNION ALL SELECT 'workflowable', count(*) FROM kb_workflowable_states WHERE resource_id = '019d2f00-0000-7000-8000-000000000001'
UNION ALL SELECT 'sequenceable', count(*) FROM kb_sequenceable_states WHERE resource_id = '019d2f00-0000-7000-8000-000000000001'
UNION ALL SELECT 'assignable', count(*) FROM kb_assignable_states WHERE resource_id = '019d2f00-0000-7000-8000-000000000001'
UNION ALL SELECT 'taggable', count(*) FROM kb_taggable_states WHERE resource_id = '019d2f00-0000-7000-8000-000000000001';
"
```

Expected: All counts are 0. CASCADE cleaned everything.

- [ ] **Step 7: Record validation results**

If any step failed, fix the schema migration (`20260326000001_r2_schema.sql`), reset and re-run:

```bash
sqlx database drop --database-url postgres://temper:temper@localhost:5437/temper_development -y
sqlx database create --database-url postgres://temper:temper@localhost:5437/temper_development
sqlx migrate run --database-url postgres://temper:temper@localhost:5437/temper_development
```

Then re-run validation steps.

### Task 4: Save Research Note and Close Ticket

**Files:**
- Create: Research note via `temper research save`

- [ ] **Step 1: Save the R2 research note to the knowledge base**

Pipe the research summary to `temper research save`:

```bash
temper research save "R2 Data Model and Schema Design" --project temper
```

The research note content should cover:
- The Postgres-as-authority pivot (what changed from R1's dual-authority model)
- Retirement of `resource_kind` discriminator — all resources are peers
- Schema architecture: single `resources` table + per-behavior join tables
- URI scheme design (`kb://` provenance, resolution strategy)
- Versioned chunks with `is_current` flag and partial HNSW index
- `kb_contexts` replacing project at persistence layer
- 768-dim embeddings from day one (kreuzberg balanced)
- Events as local buffer draining to Postgres on sync
- Schema validated against local Postgres 18 + pgvector 0.8.2 via sqlx migrations
- Open questions deferred to R3/R4/R5

- [ ] **Step 2: Update the milestone to mark R2 done**

Edit `/Users/petetaylor/projects/knowledge/milestones/temper/temper-cloud.md`:
- Change R2 status from `**in-progress**` to `**done**`
- Add session log entry for R2 completion

- [ ] **Step 3: Close the R2 ticket**

```bash
temper ticket done 2026-03-26-r2-data-model-and-schema-design --project temper
```

- [ ] **Step 4: Save session note**

```bash
temper session save "R2 Data Model and Schema Design — Postgres as Authority" --ticket 2026-03-26-r2-data-model-and-schema-design --state done --project temper
```

- [ ] **Step 5: Commit knowledge base changes**

```bash
cd /Users/petetaylor/projects/knowledge && git add -A && git commit -m "docs: R2 Data Model & Schema Design research complete"
```

- [ ] **Step 6: Final commit in temper repo**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper && git add -A && git commit -m "docs: R2 research complete — validated schema, implementation plan"
```
