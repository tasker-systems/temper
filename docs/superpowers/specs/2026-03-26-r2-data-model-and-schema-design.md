# R2: Data Model & Schema Design — Design Spec

## Architectural Pivot: Postgres as Single Source of Truth

R1 established a dual-authority model (git = content, Postgres = metadata). During R2 design, a fundamental insight emerged: since all documents are recomposable from their versioned chunks in Postgres, the knowledge base doesn't require git as a content authority. **Postgres becomes the single source of truth for everything** — content, metadata, vectors, and events.

Git and the local filesystem become an optional **materialization layer**: a convenient way to have the knowledge base on disk as markdown files for agents, editors, and Obsidian. But the canonical state lives in Postgres. `temper sync pull` materializes files from Postgres. `temper sync push` sends local edits to Postgres. Postgres always wins ties.

### What This Changes

| R1 Concept | R2 Revision |
|-----------|-------------|
| Git = content authority | Postgres = content authority (via versioned chunks) |
| Postgres = metadata authority | Postgres = everything authority |
| Dual-authority reconciliation (6 drift types) | Single-authority sync (push local changes up, pull current state down) |
| Local HNSW as offline fallback | Local HNSW as optional offline cache |
| events.jsonl as local audit trail | events.jsonl as local buffer → drains to Postgres on sync |

### What This Enables

- **Multi-tenancy and team collaboration** via scoping and ownership — no git-level access control needed
- **Apache AGE knowledge graph** alongside pg_vector, since Postgres holds all content
- **Cloud-first without git**: `temper auth login` gives full access; `git clone` becomes optional for local editing convenience
- **Simpler architecture**: one authority eliminates most reconciliation complexity

## Schema Design

### Approach: Flat Resources + Per-Behavior Join Tables

Single `resources` table for all three resource kinds. Behavior composition via `kb_doc_type_behaviors` join table linking types to behaviors, with per-behavior state tables holding actual field values. Chosen over JSONB (no type safety, weaker constraints) and EAV (query complexity for ~4 behaviors is not justified).

### Core Tables

```sql
-- Scoping mechanism for resources (replaces "project" concept)
-- CLI continues using --project as user-facing term, maps to kb_context
CREATE TABLE kb_contexts (
    id          UUID PRIMARY KEY,              -- UUIDv7
    name        VARCHAR(128) NOT NULL UNIQUE,  -- "temper", "storyteller", etc.
    created     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Document type definitions (ticket, session, milestone, research, etc.)
-- New types are data inserts, not schema changes
CREATE TABLE kb_doc_types (
    id          UUID PRIMARY KEY,              -- UUIDv7
    name        VARCHAR(64) NOT NULL UNIQUE,
    created     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The single resource table for all three kinds
CREATE TYPE resource_kind AS ENUM ('indexable', 'ingested', 'knowledge_base');

CREATE TABLE resources (
    id              UUID PRIMARY KEY,              -- UUIDv7, time-ordered
    kind            resource_kind NOT NULL,
    kb_context_id   UUID NOT NULL REFERENCES kb_contexts(id),
    kb_doc_type_id  UUID NOT NULL REFERENCES kb_doc_types(id),
    uri             TEXT NOT NULL,
    title           TEXT NOT NULL,
    slug            VARCHAR(256),
    content_hash    VARCHAR(64),                   -- SHA-256 hex, null for indexable
    mimetype        VARCHAR(128),
    created         TIMESTAMPTZ NOT NULL,
    updated         TIMESTAMPTZ NOT NULL,

    UNIQUE(slug, kb_context_id),
    UNIQUE(uri)
);

CREATE INDEX idx_resources_context ON resources(kb_context_id);
CREATE INDEX idx_resources_doc_type ON resources(kb_doc_type_id);
CREATE INDEX idx_resources_kind ON resources(kind);
CREATE INDEX idx_resources_updated ON resources(updated);
```

**Key decisions:**
- `resource_kind` is one of the few enums — we have conviction these three kinds are stable and want the constraint as a forcing function
- `doc_type` is a FK to `kb_doc_types`, not free text — enforces type registration while keeping types extensible via data
- `slug` unique within `kb_context_id` — same slug can exist in different contexts
- `uri` globally unique — each resource has one canonical URI. KB/ingested resources use vault-relative paths (`file://tickets/temper/2026-03-26-fix-search.md`), indexable resources use external URIs (`https://...`, `s3://...`). Vault-relative paths ensure URIs are stable across machines.

### Behavior Composition

```sql
-- Behavior definitions (workflowable, sequenceable, assignable, taggable)
CREATE TABLE kb_behaviors (
    id          UUID PRIMARY KEY,
    name        VARCHAR(64) NOT NULL UNIQUE,
    created     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Which behaviors each doc type composes
CREATE TABLE kb_doc_type_behaviors (
    kb_doc_type_id  UUID NOT NULL REFERENCES kb_doc_types(id),
    kb_behavior_id  UUID NOT NULL REFERENCES kb_behaviors(id),
    PRIMARY KEY (kb_doc_type_id, kb_behavior_id)
);
```

Adding a new document type that composes existing behaviors requires only:
1. INSERT into `kb_doc_types`
2. INSERT rows into `kb_doc_type_behaviors`
3. Create a template file and directory convention

No schema migration. No code changes. New behaviors themselves are a code change (per R1 WF5).

### Per-Behavior State Tables

```sql
-- Lifecycle stages defined per doc type (data-driven FSM)
CREATE TABLE kb_lifecycle_stages (
    id              UUID PRIMARY KEY,
    kb_doc_type_id  UUID NOT NULL REFERENCES kb_doc_types(id),
    name            VARCHAR(64) NOT NULL,
    seq             INT NOT NULL,                  -- ordering within lifecycle
    UNIQUE(kb_doc_type_id, name)
);

-- Workflowable: lifecycle state per resource
CREATE TABLE kb_workflowable_states (
    resource_id     UUID PRIMARY KEY REFERENCES resources(id) ON DELETE CASCADE,
    stage_id        UUID NOT NULL REFERENCES kb_lifecycle_stages(id),
    updated         TIMESTAMPTZ NOT NULL
);

-- Sequenceable: ordering within a parent resource
CREATE TABLE kb_sequenceable_states (
    resource_id     UUID PRIMARY KEY REFERENCES resources(id) ON DELETE CASCADE,
    seq             INT NOT NULL,
    parent_id       UUID REFERENCES resources(id), -- milestone for tickets, null otherwise
    updated         TIMESTAMPTZ NOT NULL
);

-- Assignable: ownership and contextual metadata
CREATE TABLE kb_assignable_states (
    resource_id     UUID PRIMARY KEY REFERENCES resources(id) ON DELETE CASCADE,
    author          VARCHAR(128),
    assignee        VARCHAR(128),
    metadata        JSONB NOT NULL DEFAULT '{}',   -- branch, pr, system, repo, etc.
    updated         TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_assignable_metadata ON kb_assignable_states USING GIN(metadata);

-- Taggable: free-form tags
CREATE TABLE kb_taggable_states (
    resource_id     UUID PRIMARY KEY REFERENCES resources(id) ON DELETE CASCADE,
    tags            TEXT[] NOT NULL DEFAULT '{}',
    updated         TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_taggable_tags ON kb_taggable_states USING GIN(tags);
```

**Key decisions:**
- Lifecycle stages are **per-doc-type**, not global. Tickets: `backlog/in-progress/done/cancelled`. Decisions: `proposed/accepted/superseded/withdrawn`. Any-state-to-any-state transitions — no rigid FSM enforcement. The stage definitions are data that could support guards later (via JSONB FSM declarations on `kb_lifecycle_stages`) but for now any transition is valid.
- `parent_id` on sequenceable captures "ticket belongs to milestone" as a proper FK relationship, replacing the frontmatter `milestone` field.
- Assignable carries `author`/`assignee` as real columns, everything else (`branch`, `pr`, `system`, `repo`) in a JSONB `metadata` column. CLI accepts `--metadata '{...}'` or convenience flags like `--branch`/`--pr` that build JSON.
- Tags as Postgres arrays with GIN index — simpler than a join table for free-form string tags.
- Each behavior table has its own `updated` timestamp for granular last-write-wins reconciliation.

### Versioned Chunks with Content Addressing

Documents are stored as versioned chunks. The full chunk text is stored alongside its embedding, making documents **recomposable** from Postgres without filesystem access.

```sql
CREATE TABLE kb_chunks (
    id              UUID PRIMARY KEY,              -- UUIDv7
    resource_id     UUID NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    chunk_index     INT NOT NULL,                  -- position in document (0 = frontmatter)
    version         INT NOT NULL DEFAULT 1,        -- bumped when content changes
    header_path     TEXT NOT NULL DEFAULT '',       -- "## Section > ### Subsection"
    content         TEXT NOT NULL,                  -- full chunk text (recomposable)
    content_hash    VARCHAR(64) NOT NULL,           -- SHA-256 of this chunk's content
    embedding       vector(768) NOT NULL,           -- kreuzberg balanced (bge-base-en-v1.5)
    is_current      BOOLEAN NOT NULL DEFAULT true,  -- false for superseded versions
    created         TIMESTAMPTZ NOT NULL DEFAULT now(),

    UNIQUE(resource_id, chunk_index, version)
);

CREATE INDEX idx_chunks_resource ON kb_chunks(resource_id);
CREATE INDEX idx_chunks_content_hash ON kb_chunks(content_hash);

-- HNSW index only over current chunks — stale versions excluded from search
CREATE INDEX idx_chunks_current_embedding ON kb_chunks
    USING hnsw(embedding vector_cosine_ops)
    WITH (m = 16, ef_construction = 200)
    WHERE is_current = true;

-- Convenience view for current document state
CREATE VIEW kb_current_chunks AS
SELECT id, resource_id, chunk_index, version, header_path,
       content, content_hash, embedding, created
FROM kb_chunks
WHERE is_current = true
ORDER BY resource_id, chunk_index;
```

**On document update:**
1. Compute full document content hash → compare against `resources.content_hash`
2. If unchanged, no-op
3. If changed, re-chunk the document
4. For each chunk position: hash new chunk content, compare against existing current chunk at that position
5. Only INSERT new version rows for chunks whose `content_hash` actually changed
6. Set `is_current = false` on superseded chunks at those positions
7. New chunks (document grew) get version 1 at their new positions
8. If document shrank, set `is_current = false` on removed positions
9. Update `resources.content_hash` and `resources.updated`

**What this gives you:**
- Append-only edits create new chunk rows only for new/changed sections
- The partial HNSW index (`WHERE is_current = true`) keeps search clean without scanning stale vectors
- Historical chunk versions retained for auditing and diffing
- `SELECT content FROM kb_current_chunks WHERE resource_id = $1 ORDER BY chunk_index` reconstructs the current document

**Embedding strategy:**
- 768 dimensions from day one — kreuzberg `balanced` preset (bge-base-en-v1.5)
- HNSW index over IVFFlat: better recall, no training data requirement, handles incremental inserts
- Cosine distance (`vector_cosine_ops`) consistent with L2-normalized vectors
- Current corpus (~4K files) re-embedded on migration — trivial one-time cost

### IngestedResource Provenance

```sql
CREATE TABLE kb_ingestion_records (
    resource_id         UUID PRIMARY KEY REFERENCES resources(id) ON DELETE CASCADE,
    source_uri          TEXT NOT NULL,
    source_mimetype     VARCHAR(128),
    conversion_tool     VARCHAR(64) NOT NULL,      -- "kreuzberg-v4"
    conversion_version  VARCHAR(32) NOT NULL,
    fetched_at          TIMESTAMPTZ NOT NULL,
    converted_at        TIMESTAMPTZ NOT NULL,
    source_hash         VARCHAR(64)                -- SHA-256 of original, for re-fetch detection
);
```

Only `ingested` kind resources have provenance records. `source_hash` enables detecting when original content has changed upstream.

### Events Table

Local `events.jsonl` drains to Postgres on `temper sync`, then prunes confirmed events locally.

```sql
CREATE TABLE kb_events (
    id              UUID PRIMARY KEY,              -- UUIDv7
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

**Key decisions:**
- `profile_id` references an auth profile derived from the auth flow — covers logged-in users, system processes, and anonymous/generic profiles for non-auth scenarios
- `event_type` as VARCHAR, not enum — new event types added without migration
- `client_id` distinguishes originating machine for the cross-machine ledger
- `payload` JSONB holds event-specific fields (from/to stage, old/new hash, fields changed, etc.)

### Auth Profiles (Lightweight)

```sql
CREATE TABLE kb_profiles (
    id              UUID PRIMARY KEY,              -- UUIDv7
    provider        VARCHAR(32) NOT NULL,          -- "github", "system", "anonymous"
    external_id     VARCHAR(128),                  -- GitHub user ID, null for system/anon
    display_name    VARCHAR(128) NOT NULL,
    email           VARCHAR(256),
    created         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated         TIMESTAMPTZ NOT NULL DEFAULT now(),

    UNIQUE(provider, external_id)
);
```

Seeded with system and anonymous profiles on bootstrap. Full auth flow design is R4 scope — this table provides the FK target that events and future ownership tracking need.

## Frontmatter-to-Postgres Field Mapping

With Postgres as authority, frontmatter in local markdown files is a **materialized view** of Postgres state.

| Frontmatter field | Postgres location |
|-------------------|-------------------|
| `id` | `resources.id` |
| `type` | `kb_doc_types.name` (via `resources.kb_doc_type_id`) |
| `title` | `resources.title` |
| `slug` | `resources.slug` |
| `project` | `kb_contexts.name` (via `resources.kb_context_id`) |
| `milestone` | parent resource title (via `kb_sequenceable_states.parent_id`) |
| `stage` | `kb_lifecycle_stages.name` (via `kb_workflowable_states.stage_id`) |
| `scope` | `kb_assignable_states.metadata->>'scope'` |
| `seq` | `kb_sequenceable_states.seq` |
| `created` | `resources.created` |
| `updated` | `resources.updated` |
| `branch` | `kb_assignable_states.metadata->>'branch'` |
| `pr` | `kb_assignable_states.metadata->>'pr'` |
| `date` | `resources.created` (sessions/research) |
| `cluster` | `kb_assignable_states.metadata->>'cluster'` |
| `tags` | `kb_taggable_states.tags` |

On `temper sync push`: parse frontmatter changes, update appropriate Postgres tables.
On `temper sync pull`: reconstruct frontmatter from structured Postgres data, write to local files.

## Reconciliation Model (Simplified from R1)

With Postgres as single authority, the six R1 drift types collapse:

| R1 Drift Type | R2 Resolution |
|---------------|---------------|
| Metadata drift | **Postgres wins.** Local frontmatter re-materialized from Postgres on pull. |
| Content drift | **Push resolves.** Local edit → `temper sync push` → re-chunk, hash, version, embed → Postgres updated. |
| Index staleness | **Eliminated.** pg_vector IS the index. Local HNSW is optional offline cache. |
| Partial sync failure | **Unchanged.** Local `.temper/sync_queue.jsonl` buffers failed operations for retry. Client-pull reconciliation on reconnect. |
| Orphaned references | **Unchanged.** URI validation remains opt-in via `temper check --validate-uris`. |
| Out-of-band mutation | **Postgres wins.** It's the authority. Local files re-materialize on pull. |

**Sync queue stays local.** It exists to bridge connectivity gaps — Postgres can't hold a queue for operations that failed because Postgres was unreachable. Multi-machine coordination happens through Postgres as the shared authority, not through sync queue visibility.

## Migration Plan

One-time bootstrap from existing vault to Postgres:

1. **Seed reference data**: Create `kb_contexts` from `temper.toml` projects, `kb_doc_types` from known types (ticket, session, milestone, research, board, concept, source), `kb_behaviors` and `kb_doc_type_behaviors` from known compositions
2. **Seed lifecycle stages**: Ticket stages (backlog, in-progress, done, cancelled), milestone statuses (active, complete)
3. **Seed profiles**: System and anonymous profiles
4. **Import resources**: Read all markdown files, parse frontmatter → populate `resources` and behavior state tables
5. **Chunk and embed**: Re-embed all content with kreuzberg `balanced` (768-dim) → populate `kb_chunks`
6. **Import events**: Read `events.jsonl` → populate `kb_events`
7. **Verify**: Round-trip check — materialize files from Postgres, compare against originals

Local vault remains as-is after migration. It becomes the first materialized client.

## Seed Data

### Behaviors

| Name | Description |
|------|-------------|
| workflowable | Has lifecycle stages |
| sequenceable | Has ordering within a parent |
| assignable | Has author, assignee, and contextual metadata |
| taggable | Has free-form tags |

### Doc Type Compositions

| Doc Type | Behaviors |
|----------|-----------|
| ticket | workflowable, sequenceable, assignable, taggable |
| session | taggable |
| milestone | sequenceable |
| research | taggable |
| board | (none) |
| concept | taggable |
| source | taggable |

### Lifecycle Stages

| Doc Type | Stages (in order) |
|----------|-------------------|
| ticket | backlog, design, in-progress, done, cancelled |
| milestone | active, complete |

## Open Questions Resolved

| Question (from R2 ticket) | Resolution |
|---------------------------|------------|
| Behavior composition approach | Per-behavior join tables with FK constraints |
| Sync queue placement | Local `.temper/sync_queue.jsonl` — inherently a local concern |
| Event schema placement | Local buffer → drains to Postgres `kb_events` on sync, prunes confirmed |
| pg_vector dimensions | 768-dim from day one (kreuzberg balanced) |
| pg_vector index type | HNSW with partial index on `is_current = true` |
| Distance function | Cosine (`vector_cosine_ops`) |

## Open Questions for Downstream Workstreams

1. **R3**: Does the deployment platform support pg_vector and Apache AGE extensions? This is now a hard requirement.
2. **R4**: The `kb_profiles` table is a placeholder — full auth flow design (GitHub OAuth, token management) is R4 scope.
3. **R4**: `kb_contexts` may need ownership/permission columns for multi-tenancy — deferred to auth design.
4. **R5**: Should the local HNSW offline cache use 768-dim (matching cloud) or stay at 384-dim (lighter weight)? The cloud uses 768 regardless.
5. **R5**: Chunk versioning history retention policy — prune after N versions? Time-based? Keep forever?
6. **Future**: Apache AGE knowledge graph integration is now trivially possible since Postgres holds all content. Design deferred but architecturally unblocked.
