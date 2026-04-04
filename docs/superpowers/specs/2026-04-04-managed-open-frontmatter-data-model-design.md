# Managed & Open Frontmatter Data Model — Design Spec

**Task:** `2026-04-03-enhance-and-fix-temper-init` (prerequisites workstream)
**Date:** 2026-04-04
**Mode:** build / large (multi-session)
**Depends on:** Workstreams A–E (completed), frontmatter schemas spec
**Blocks:** Vault-wide `doctor fix` migration, knowledge graph edge extraction, sync protocol v2

## Problem Statement

Today, frontmatter is local-only. The sync pipeline strips frontmatter before push, hashes only the body, and regenerates minimal frontmatter on pull (just id, title, context, doc_type). Managed fields like `stage`, `mode`, `effort`, `goal`, `branch`, `pr`, `status`, `seq` are invisible to the server — lost on pull and absent from any server-side query or merge logic.

Without a server-side model for frontmatter, we cannot:
- Detect whether a sync should update metadata vs. re-chunk/re-embed content
- Merge frontmatter changes from multiple devices
- Reconstruct a complete file from server data
- Build knowledge graph edges from relationship fields
- Query resources by managed fields server-side (e.g., "all in-progress tasks")

Migrating field names locally (unprefixed → `temper-*`) without this model would be premature — the naming is downstream of data model decisions about persistence, hashing, and merge strategy.

## Design Principles

1. **Three-tier hashing enables surgical sync.** Body changes are expensive (re-chunk + re-embed). Metadata changes are cheap (JSONB update). The hash scheme tells sync which tier changed so it does only the necessary work.
2. **Managed meta is the transport format; relational columns are the queryable truth.** `temper-type` and `temper-context` live in managed_meta for round-trip fidelity but cascade to `kb_resources` FK columns on write.
3. **No backward compatibility tax.** This is a pre-alpha system with one user. Move fast, get the model right, don't shim.
4. **JSONB for doctype-varying fields; relational for universal identity.** Tasks have stage/mode/effort/goal/branch/pr, goals have status, sessions have date. JSONB avoids a wide sparse table while still supporting GIN indexing on hot paths.
5. **Conflict resolution: merge what you can, last-write-wins for scalars, warn on irreconcilable.** Set-valued fields (tags, relations) merge additively. Scalar fields (stage, status) use last-write-wins. Structural conflicts (context move, type change) require the second writer to accept upstream first.

## Architecture

### Table: `kb_resource_manifests`

New table, 1:1 with `kb_resources`. Holds the current metadata state and three-tier hashes.

```sql
CREATE TABLE kb_resource_manifests (
    resource_id    UUID PRIMARY KEY REFERENCES kb_resources(id) ON DELETE CASCADE,
    body_hash      VARCHAR(64) NOT NULL,
    managed_meta   JSONB NOT NULL DEFAULT '{}',
    open_meta      JSONB NOT NULL DEFAULT '{}',
    managed_hash   VARCHAR(64) NOT NULL,
    open_hash      VARCHAR(64) NOT NULL,
    updated        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Index managed fields we query frequently
CREATE INDEX idx_resource_manifests_stage
    ON kb_resource_manifests ((managed_meta->>'temper-stage'));
CREATE INDEX idx_resource_manifests_status
    ON kb_resource_manifests ((managed_meta->>'temper-status'));
CREATE INDEX idx_resource_manifests_goal
    ON kb_resource_manifests ((managed_meta->>'temper-goal'));

-- GIN index for open_meta array fields (tags, relates_to, etc.)
CREATE INDEX idx_resource_manifests_open_meta
    ON kb_resource_manifests USING GIN (open_meta jsonb_path_ops);
```

### `kb_resources` Cleanup

Remove columns that are either moved to `kb_resource_manifests` or no longer needed:

| Column | Action | Reason |
|--------|--------|--------|
| `content_hash` | Move to `kb_resource_manifests.body_hash` | Part of three-tier hash model |
| `mimetype` | Drop | Always `text/markdown` |
| `resource_mode` | Drop | `added`/`imported` distinction eliminated by unified add/import |
| `origin_uri` UNIQUE constraint | Drop (keep column) | Provenance only; uniqueness checked by body_hash |

Remaining `kb_resources` columns (the identity table):

```
id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
originator_profile_id, owner_profile_id, is_active, created, updated
```

### Field Tier Assignment

Every frontmatter field belongs to exactly one tier:

**Identity tier** — relational columns on `kb_resources`, immutable or cascade-on-change:

| Field | Column | Mutability |
|-------|--------|-----------|
| `temper-id` | `kb_resources.id` | Immutable |
| `temper-type` | `kb_resources.kb_doc_type_id` (FK) | Mutable — cascaded from managed_meta |
| `temper-context` | `kb_resources.kb_context_id` (FK) | Mutable — cascaded from managed_meta |
| `temper-created` | `kb_resources.created` | Immutable |
| `title` | `kb_resources.title` | Mutable — cascaded from managed_meta |
| `slug` | `kb_resources.slug` | Mutable — cascaded from managed_meta |

**Managed meta tier** — `kb_resource_manifests.managed_meta` JSONB, temper-governed:

| Field | Doctypes | Notes |
|-------|----------|-------|
| `temper-type` | all | Duplicated from identity for round-trip; cascades to `kb_doc_type_id` |
| `temper-context` | all | Duplicated from identity for round-trip; cascades to `kb_context_id` |
| `temper-updated` | all | Mutable timestamp |
| `temper-source` | all | Ingestion provenance |
| `temper-legacy-id` | all | Migration tracking |
| `temper-stage` | task | Workflow stage |
| `temper-mode` | task | plan/build |
| `temper-effort` | task | small/medium/large |
| `temper-goal` | task | Parent goal slug |
| `temper-seq` | task, goal | Ordering integer |
| `temper-branch` | task | Git branch |
| `temper-pr` | task | PR URL |
| `temper-status` | goal | Lifecycle status |

Note: `title` and `slug` are transported in managed_meta for round-trip completeness but their source of truth is the relational column. On push, the API cascades changes. On pull, the CLI emits them from the relational data.

**Open meta tier** — `kb_resource_manifests.open_meta` JSONB, user-owned:

| Field | Type | Notes |
|-------|------|-------|
| `tags` | array | Obsidian-compatible |
| `aliases` | array | Obsidian-compatible |
| `date` | string | Session date (YYYY-MM-DD), also used by research/decision |
| `relates_to` | array | Knowledge graph edges |
| `depends_on` | array | Knowledge graph edges |
| `extends` | string/array | Knowledge graph edges |
| `references` | array | Knowledge graph edges (may include external URIs) |
| `preceded_by` | string/array | Knowledge graph edges |
| `derived_from` | string/array | Knowledge graph edges |
| `sessions` | array | Task→session linkage (UUIDs) |
| *(any other key)* | any | Preserved verbatim — user's own fields |

### temper-core Shared Types

```rust
/// Managed frontmatter fields — temper-governed, varies by doctype.
/// Serialized to/from JSONB in kb_resource_manifests.managed_meta
/// and to/from YAML frontmatter in vault files.
///
/// Uses serde(rename) for the temper-* YAML/JSON representation
/// and serde(skip_serializing_if) to omit None values.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ManagedMeta {
    #[serde(rename = "temper-type", skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    #[serde(rename = "temper-context", skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(rename = "temper-updated", skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    #[serde(rename = "temper-source", skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(rename = "temper-legacy-id", skip_serializing_if = "Option::is_none")]
    pub legacy_id: Option<String>,
    // task fields
    #[serde(rename = "temper-stage", skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(rename = "temper-mode", skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(rename = "temper-effort", skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(rename = "temper-goal", skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    #[serde(rename = "temper-seq", skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
    #[serde(rename = "temper-branch", skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(rename = "temper-pr", skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
    // goal fields
    #[serde(rename = "temper-status", skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    // identity-tier fields transported in managed_meta for round-trip fidelity
    // On push, API cascades to kb_resources columns. On pull, CLI emits from relational data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
}
```

`ManagedMeta` is used in four places:
1. **CLI vault operations** — parse from YAML, compute managed_hash
2. **CLI sync push** — send as JSON in the sync payload
3. **API ingest/sync handler** — deserialize from request, persist to JSONB, cascade identity fields
4. **API sync pull response** — serialize from JSONB, include in pull response

Open meta is `serde_json::Value` (a JSON object) — no struct because the shape is user-defined.

### Hash Computation (Shared Logic)

The `compute_frontmatter_hashes` function in `temper_core::schema` already separates `temper-*` fields from others and hashes each tier as canonical sorted JSON. This becomes the shared implementation for both CLI and API.

The body hash continues to be SHA-256 of the markdown body with frontmatter stripped.

All three hashes use the format `sha256:{lowercase_hex}`. Note: the current `compute_content_hash` in ingest.rs returns plain hex without prefix. This migration standardizes all hashes on the `sha256:` prefixed format for consistency with `compute_frontmatter_hashes`.

### Local Manifest Evolution

`ManifestEntry` expands from one hash pair to three:

```rust
pub struct ManifestEntry {
    pub path: String,
    // Three-tier local hashes (computed from vault file)
    pub body_hash: String,
    pub managed_hash: String,
    pub open_hash: String,
    // Three-tier remote hashes (from last sync)
    pub remote_body_hash: String,
    pub remote_managed_hash: String,
    pub remote_open_hash: String,
    // Sync state
    pub synced_at: DateTime<Utc>,
    pub state: ManifestEntryState,
    pub mtime_secs: Option<i64>,
}
```

**Migration of existing manifest.json**: On first load with the new format, if the entry has `content_hash`/`remote_hash` (old format), the loader maps them to `body_hash`/`remote_body_hash` and sets managed/open hashes to empty strings (forcing a full sync on next run).

### Sync Protocol Changes

#### Push Flow (CLI → API)

1. CLI reads vault file, separates: body, managed meta (temper-* fields), open meta (everything else)
2. Computes three hashes locally
3. Compares against manifest entry — determines which tiers changed
4. Sends appropriate payload:
   - **Body changed**: full `IngestPayload` (re-chunk + re-embed + managed_meta + open_meta)
   - **Meta only changed**: new lightweight `MetaUpdatePayload { resource_id, managed_meta, open_meta, managed_hash, open_hash }`
5. API handler:
   - For body push: existing ingest flow + persist managed/open meta to `kb_resource_manifests`
   - For meta push: update `kb_resource_manifests` JSONB + hashes, cascade identity fields to `kb_resources`
   - In both cases: insert `kb_events` entry

#### Pull Flow (API → CLI)

1. CLI sends manifest with three-tier hashes per resource
2. `sync_diff_for_device` compares all three tiers
3. Response indicates which tiers need pulling: `to_pull_body`, `to_pull_meta`, `to_pull_all`
4. CLI fetches:
   - **Body pull**: chunks (for content) + managed_meta + open_meta from `kb_resource_manifests`
   - **Meta pull**: just managed_meta + open_meta
5. CLI reconstructs file:
   ```
   ---
   temper-id: "{id}"
   temper-type: {doc_type}
   temper-context: {context}
   temper-created: {created}
   title: "{title}"
   slug: "{slug}"
   {managed_meta fields}
   {open_meta fields}
   ---
   {body from chunks}
   ```

#### `sync_diff_for_device` SQL Function Update

The function's manifest JSONB format expands:

```json
{
  "uri": "kb://ctx/task/uuid",
  "body_hash": "sha256:...",
  "managed_hash": "sha256:...",
  "open_hash": "sha256:..."
}
```

The diff_type column becomes more granular:

| diff_type | Meaning |
|-----------|---------|
| `to_push_body` | Local body changed, remote unchanged |
| `to_push_meta` | Local meta changed (managed and/or open), remote unchanged |
| `to_pull_body` | Remote body changed, local unchanged |
| `to_pull_meta` | Remote meta changed, local unchanged |
| `conflict_body` | Both sides changed body |
| `conflict_meta` | Both sides changed meta |
| `to_push` | New local resource (not on server) |
| `to_pull` | New remote resource (not in manifest) |
| `removed` | Resource deactivated server-side |

#### Conflict Resolution

**Scalar managed fields** (stage, mode, effort, goal, seq, branch, pr, status, source): **last-write-wins** based on `kb_events.created` timestamp. The later event's value is the truth.

**Set-valued open fields** (tags, aliases, relates_to, depends_on, references, etc.): **additive merge**. Union of both sides. Removals are not synced — if you remove a tag locally, the remote copy may re-add it. This is a deliberate simplicity choice; explicit removal semantics can come later via event tombstones.

**Identity-cascading fields** (temper-type, temper-context): **first-write-wins at the relational level**. If two devices both move a resource to a different context, the first push succeeds and the second must sync (accept upstream) before retrying. The CLI warns: "Resource {slug} was moved to context {ctx} by another device. Syncing that change. Re-run your move if you still want a different context."

**Body conflicts**: existing merge strategy (paragraph-level three-way merge via `similar` crate) is unchanged.

**Irreconcilable conflicts**: CLI warns and skips the file with a message indicating what needs resolution. No `.conflict.md` files for meta — only for body content.

### kb_events Integration

The API creates events for meaningful state changes:

```sql
-- Event types and when they fire:
-- 'resource_created'       — new resource via ingest
-- 'body_updated'           — body hash changed via sync push
-- 'managed_meta_updated'   — managed_meta changed via sync push or meta update
-- 'open_meta_updated'      — open_meta changed via sync push or meta update
-- 'resource_deactivated'   — soft delete
-- 'resource_moved'         — context or type change (cascaded from managed_meta)
```

The `payload` JSONB captures what changed:

```json
{
  "tier": "managed",
  "fields_changed": ["temper-stage", "temper-mode"],
  "previous_values": {"temper-stage": "backlog", "temper-mode": null},
  "new_values": {"temper-stage": "in-progress", "temper-mode": "build"}
}
```

Events provide chronological ordering for merge sequencing. The `device_id` field identifies which device made the change.

**Scope boundary**: local `events.jsonl` reconciliation with `kb_events` is deferred. This spec only covers server-side event creation when the API is interacted with.

### IngestPayload Evolution

The existing `IngestPayload` gains two fields:

```rust
pub struct IngestPayload {
    // ... existing fields ...
    pub managed_meta: Option<serde_json::Value>,  // NEW — managed frontmatter as JSON
    pub open_meta: Option<serde_json::Value>,      // NEW — open frontmatter as JSON
}
```

The existing `metadata` field is deprecated in favor of the explicit tier split.

A new lightweight payload for meta-only updates:

```rust
pub struct MetaUpdatePayload {
    pub resource_id: Uuid,
    pub managed_meta: serde_json::Value,
    pub open_meta: serde_json::Value,
    pub managed_hash: String,
    pub open_hash: String,
}
```

### API Endpoint Changes

| Endpoint | Change |
|----------|--------|
| `POST /api/ingest` | Accept `managed_meta` + `open_meta` in payload; persist to `kb_resource_manifests`; cascade identity fields; create `kb_events` |
| `PUT /api/resources/:id/meta` | **NEW** — meta-only update; accepts `MetaUpdatePayload`; cascades identity fields; creates `kb_events` |
| `GET /api/resources/:id` | Response includes `managed_meta`, `open_meta`, `body_hash`, `managed_hash`, `open_hash` from manifests join |
| `GET /api/resources/:id/content` | Unchanged — returns reconstructed markdown body |
| `POST /api/sync/status` | Manifest entries now include three hashes; response diff_types are tier-aware |
| `POST /api/sync/push` | Routes to ingest (body changed) or meta update (meta only changed) |
| `POST /api/sync/pull` | Response includes managed_meta + open_meta alongside body content |

### Template Updates

All Askama templates emit `temper-*` prefixed field names:

```yaml
---
temper-id: "{{ id }}"
temper-type: task
temper-context: "{{ context }}"
temper-created: {{ datetime }}
temper-updated: {{ datetime }}
title: "{{ title }}"
slug: "{{ slug }}"
temper-goal: "{{ goal }}"
temper-stage: backlog
temper-mode: {{ mode }}
temper-effort: {{ effort }}
temper-seq: {{ seq }}
temper-branch: null
temper-pr: null
---
```

### CLI Action Code Updates

All `set_frontmatter_field()` calls use `temper-*` prefixed names:

```rust
// Before:
vault::set_frontmatter_field(&content, "stage", stage);
// After:
vault::set_frontmatter_field(&content, "temper-stage", stage);
```

`TaskInfo`/`GoalInfo` structs gain `#[serde(alias)]` to read both old and new formats during transition:

```rust
pub struct TaskInfo {
    pub title: String,
    pub slug: String,
    #[serde(alias = "context", alias = "temper-context")]
    pub context: String,
    #[serde(alias = "stage", alias = "temper-stage")]
    pub stage: String,
    // ...
}
```

`parse_source_frontmatter` in ingest.rs is extended to check `temper-*` names first, falling back to legacy names.

### Migration Strategy

**Database migration** (single SQL file):

1. Create `kb_resource_manifests` table
2. Populate from existing `kb_resources`:
   ```sql
   INSERT INTO kb_resource_manifests (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash)
   SELECT id, COALESCE(content_hash, ''), '{}', '{}',
          'sha256:' || encode(sha256('{}'), 'hex'),
          'sha256:' || encode(sha256('{}'), 'hex')
   FROM kb_resources;
   ```
3. Drop `mimetype`, `resource_mode` columns from `kb_resources`
4. Drop UNIQUE constraint on `origin_uri` (keep column, keep index for lookups)
5. Drop `content_hash` column from `kb_resources` (moved to manifests)
6. Drop `idx_kb_resources_mode` index

**Local manifest migration**: On first load, detect old format (`content_hash` field present), map to new three-tier format, force full sync.

**Vault file migration**: Run `temper doctor fix` after templates and CLI are updated. This renames legacy fields to `temper-*` names. The next sync push will populate `managed_meta` and `open_meta` on the server.

## Implementation Order

1. **temper-core types**: `ManagedMeta` struct, `MetaUpdatePayload`, expand `ManifestEntry`, manifest migration logic
2. **Database migration**: `kb_resource_manifests` table, `kb_resources` cleanup, updated `sync_diff_for_device`
3. **temper-api handlers**: ingest handler accepts managed/open meta, new `PUT /resources/:id/meta`, sync endpoints handle three-tier hashes, `kb_events` insertion
4. **temper-cli templates**: all four templates emit `temper-*` fields
5. **temper-cli actions**: `set_frontmatter_field` calls, `TaskInfo`/`GoalInfo` serde aliases, `parse_source_frontmatter` update, sync push/pull three-tier logic
6. **Verification**: run `temper doctor fix` on test vault, full sync round-trip test

## Non-Goals (Explicit Scope Boundaries)

- Local `events.jsonl` reconciliation with `kb_events` — deferred
- `kb_device_sync_state` unification with `kb_events` — deferred
- Knowledge graph edge extraction from relationship fields — deferred (but open_meta persistence enables it)
- Removal semantics for set-valued fields (tombstones) — deferred
- `temper move` command — deferred (but context-change cascade in API enables it)
- MCP server updates — will inherit temper-core types automatically
