# I6: Sync Protocol Design

**Date:** 2026-03-30
**Task:** I6: Sync Protocol & Vault Management
**Scope:** I6a (core sync), with guidance for I6b (auto-merge & workflow integration) and I6c (team sync & manual resolution)
**Depends on:** I6-pre (data model audit), I5c (two-tier resource model), I5b (temper-client auth), I4 (temper-cloud deployment)

## Overview

Bidirectional sync protocol that reconciles local vault files with the temper-cloud server. Built on the two-tier resource model from I5c: **imported** resources (in vault, in manifest) are synced; **added** resources (cloud-indexed only) are not.

## Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Diff computation | Server-computed via new SQL function | Single round-trip, leverages existing access control functions |
| Diff algorithm | Three-hash stateless (local_hash, remote_hash, server_hash) | Precise conflict detection without server-side manifest storage |
| Payload structure | Per-context batching | Keeps payloads bounded, maps to server query scope |
| Added vs imported tracking | `resource_mode` column on resources table | First-class concept, first-class column, simple index |
| Conflict resolution (I6b) | Auto-merge, never remove content, merge notice with event UUID | Safety over convenience; idempotent via event UUID |
| Workflow auto-sync | `--auto-sync` flag (default false), marks manifest dirty | Fast offline-first commands, explicit opt-in for network |
| Push completion | Fire-and-forget | Content hash updated synchronously by server, embedding pipeline async |
| Resource UUID origin | Client-generated UUIDv7, embedded in kb:// URI | `kb://context/doc_type/resource-uuid` maps directly to vault path and resource_id |
| Subscription config | `profile.vault_config` (defaults) + `profile.vault_config.devices[client_id]` (per-device) | Not a separate preferences path; local config.toml can override |
| Runtime/client init | Shared `with_client()` abstraction | Eliminates repeated tokio runtime + client setup across all commands |

## I6-pre: Data Model Audit (Blocks I6a)

Before implementing sync, the database schema needs consolidation and review:

1. **Migration consolidation** — collapse all migrations into a clean foundational set reflecting intended state
2. **Naming normalization** — `resources` → `kb_resources`, `blob_files` → `kb_blob_files`
3. **Add `resource_mode`** — `VARCHAR(16) NOT NULL DEFAULT 'added'` with index, part of consolidated migration
4. **Add `sync_diff_for_device` SQL function** — single-query diff computation leveraging `resources_visible_to()`
5. **Schema review against I6a-c flows** — verify all planned operations have the columns, indexes, and functions they need
6. **Apply to local dev** (`postgresql://temper:temper@127.0.0.1:5437/temper_development`), dump schema, verify
7. **Reconcile with Neon prod** — ensure prod can reach the same state cleanly
8. **Update all Rust `sqlx` types** that reference renamed tables

---

## I6a: Sync Infrastructure & Core Protocol

### Database Changes

**New column on `kb_resources` (post-rename):**

```sql
ALTER TABLE kb_resources ADD COLUMN resource_mode VARCHAR(16) NOT NULL DEFAULT 'added';
CREATE INDEX idx_kb_resources_mode ON kb_resources(resource_mode);
```

Values: `'added'` | `'imported'`

**New SQL function — `sync_diff_for_device`:**

```sql
CREATE FUNCTION sync_diff_for_device(
  p_profile_id UUID,
  p_context_names TEXT[],
  p_manifest JSONB  -- [{uri, local_hash, remote_hash}, ...]
) RETURNS TABLE (
  resource_id UUID,
  uri TEXT,
  content_hash VARCHAR(64),
  updated TIMESTAMPTZ,
  diff_type VARCHAR(16)  -- 'to_push', 'to_pull', 'conflict', 'removed'
)
```

Internally:
1. Joins against `resources_visible_to(p_profile_id)` for access control
2. Filters to `resource_mode = 'imported'` and requested contexts
3. LEFT JOINs manifest entries (parsed from JSONB) on URI
4. Applies three-hash comparison:
   - `local_hash == remote_hash == server_hash` → Clean (omitted from results)
   - `local_hash != remote_hash, server_hash == remote_hash` → `to_push` (only local changed)
   - `local_hash == remote_hash, server_hash != remote_hash` → `to_pull` (only remote changed)
   - `local_hash != remote_hash, server_hash != remote_hash` → `conflict` (both changed)
   - CLI has entry, server resource `is_active = false` → `removed`
   - Server has imported resource not in CLI manifest → `to_pull` (new remote resource)
5. Also handles new local resources (in manifest but no server record) → `to_push` with `resource_id = NULL`

**`kb_device_sync_state`** — no schema change. Existing `last_sync_at` and `manifest_hash` columns updated at sync completion.

### API Endpoints

#### `POST /api/sync/status`

**Location:** `api/sync/status.ts`

**Auth:** Bearer JWT (existing pattern, profile resolved via `kb_profile_auth_links`)

**Request:**
```json
{
  "contexts": [
    {
      "name": "temper",
      "entries": [
        {
          "uri": "kb://temper/task/abc-def-123",
          "local_hash": "sha256...",
          "remote_hash": "sha256..."
        }
      ]
    }
  ]
}
```

**Response:**
```json
{
  "to_push": [
    { "uri": "kb://temper/task/abc-def-123", "resource_id": null }
  ],
  "to_pull": [
    { "uri": "kb://temper/session/def-456", "resource_id": "uuid", "content_hash": "sha256..." }
  ],
  "conflicts": [
    { "uri": "kb://temper/goal/ghi-789", "resource_id": "uuid", "server_hash": "sha256..." }
  ],
  "removed": [
    { "uri": "kb://temper/note/jkl-012", "resource_id": "uuid" }
  ]
}
```

**Implementation:** Calls `sync_diff_for_device()` SQL function. The function performs a FULL OUTER JOIN between server resources and manifest entries (matched on URI), so it can detect both server-only resources (to_pull) and manifest-only entries (to_push with `resource_id` null). For new local resources not yet on the server, the CLI creates them via `POST /api/ingest` using its locally-generated UUIDv7 from the kb:// URI.

**UUID collision handling:** If the CLI-provided UUIDv7 collides with a resource outside the user's visible scope, the ingest endpoint generates a new UUID and returns it in the response. The CLI updates its local manifest and vault filename accordingly.

#### `POST /api/sync/complete`

**Location:** `api/sync/complete.ts`

**Request:**
```json
{
  "client_id": "device-uuid",
  "merged_resources": [
    { "resource_id": "uuid", "content_hash": "sha256..." }
  ]
}
```

**Response:**
```json
{
  "last_sync_at": "2026-03-30T...",
  "updated_count": 3
}
```

**Implementation:**
- Updates `kb_device_sync_state.last_sync_at` for this device (upsert on profile_id + client_id)
- For each merged resource: updates `kb_resources.content_hash` and triggers `processIngest` workflow (fire-and-forget) so embeddings reflect merged content

#### Update existing ingest endpoints

- `POST /api/ingest` — set `resource_mode` based on request metadata (default `'added'`, explicit `'imported'` when called from sync/import flow)
- `PUT /api/ingest/:id` — no change needed (mode already set at creation)

### Rust Client Extensions

**New sub-client:**
```rust
impl TemperClient {
    pub fn sync(&self) -> sync::SyncClient<'_>
}
```

**File:** `crates/temper-client/src/sync.rs`

```rust
impl<'a> SyncClient<'a> {
    pub async fn status(&self, request: &SyncStatusRequest) -> Result<SyncStatusResponse>
    pub async fn complete(&self, request: &SyncCompleteRequest) -> Result<SyncCompleteResponse>
}
```

### New Types

**File:** `crates/temper-core/src/types/sync.rs`

```rust
pub struct SyncContextEntries {
    pub name: String,
    pub entries: Vec<SyncManifestEntry>,
}

pub struct SyncManifestEntry {
    pub uri: String,
    pub local_hash: String,
    pub remote_hash: String,
}

pub struct SyncStatusRequest {
    pub contexts: Vec<SyncContextEntries>,
}

pub struct SyncStatusResponse {
    pub to_push: Vec<SyncPushItem>,
    pub to_pull: Vec<SyncPullItem>,
    pub conflicts: Vec<SyncConflictItem>,
    pub removed: Vec<SyncRemovedItem>,
}

pub struct SyncPushItem {
    pub uri: String,
    pub resource_id: Option<Uuid>,
}

pub struct SyncPullItem {
    pub uri: String,
    pub resource_id: Uuid,
    pub content_hash: String,
}

pub struct SyncConflictItem {
    pub uri: String,
    pub resource_id: Uuid,
    pub server_hash: String,
}

pub struct SyncRemovedItem {
    pub uri: String,
    pub resource_id: Uuid,
}

pub struct SyncCompleteRequest {
    pub client_id: String,
    pub merged_resources: Vec<MergedResource>,
}

pub struct MergedResource {
    pub resource_id: Uuid,
    pub content_hash: String,
}

pub struct SyncCompleteResponse {
    pub last_sync_at: DateTime<Utc>,
    pub updated_count: u32,
}
```

### CLI Command Flow

**Commands:**
```
temper sync [--context <name>...] [--format text|json]
temper sync status [--context <name>...] [--format text|json]
```

Added as subcommands under a `Sync` variant in `Commands` enum (`cli.rs`).

**10-step orchestration for `temper sync`:**

1. **Load config** — resolve vault root, load config.toml, determine sync contexts (from `--context` flag, or `sync.contexts` in config, or all contexts if neither)
2. **Authenticate** — ensure valid token via `client.token()`, fail early if not logged in
3. **Rehash manifest** — for each entry in target contexts:
   - Stat local file, compute SHA-256
   - If hash differs from `manifest.content_hash`: update hash, set state `LocalModified`
   - If file missing: flag for removal consideration
   - Mtime optimization: skip rehash if mtime < `manifest.synced_at`
4. **Request diff** — `POST /api/sync/status` per context (uri, local_hash, remote_hash)
5. **Push** — for each `to_push`:
   - Read local file
   - Resource exists: `PUT /api/ingest/:id`
   - New resource: `POST /api/ingest` with client-generated UUIDv7
   - Update manifest: `remote_hash` = response `content_hash`, state = `Clean`
6. **Pull** — for each `to_pull`:
   - `GET /api/resources/:id/content`
   - Write via `actions::ingest::write_vault_file_and_register()`
   - Update manifest: both hashes = content hash, state = `Clean`
7. **Handle conflicts** — I6a: mark as `Conflict` in manifest, log, skip. I6b: auto-merge.
8. **Handle removed** — delete local file, remove from manifest
9. **Complete** — `POST /api/sync/complete` with client_id + merged hashes
10. **Save manifest** — persist to disk

**`temper sync status`** — runs steps 1-4 only, displays diff without acting.

**Progress output (text):**
```
Syncing temper...
  ↑ Push  3 resources
  ↓ Pull  2 resources
  ✗ Conflict  1 resource (skipped)
  − Removed  0 resources
  ✓ Sync complete (5 resources, 1 conflict)
```

### CLI Actions Layer

**New files:**
- `crates/temper-cli/src/actions/sync.rs` — sync business logic
- `crates/temper-cli/src/commands/sync_cmd.rs` — CLI argument parsing and output

**`actions/sync.rs`:**

```rust
/// Rehash manifest entries for given contexts, updating state where changed
pub fn rehash_manifest(
    manifest: &mut Manifest, vault_root: &Path, contexts: &[String]
) -> RehashResult

/// Build SyncStatusRequest from manifest entries for given contexts
pub fn build_status_request(
    manifest: &Manifest, contexts: &[String]
) -> SyncStatusRequest

/// Push a single resource — read file, call ingest create or update
pub async fn push_resource(
    client: &TemperClient, manifest: &mut Manifest, vault_root: &Path, item: &SyncPushItem
) -> Result<()>

/// Pull a single resource — fetch content, write vault file, update manifest
pub async fn pull_resource(
    client: &TemperClient, manifest: &mut Manifest, vault_root: &Path, item: &SyncPullItem
) -> Result<()>

/// Remove local file and manifest entry
pub fn remove_resource(
    manifest: &mut Manifest, vault_root: &Path, item: &SyncRemovedItem
) -> Result<()>

/// Build SyncCompleteRequest from merged resources
pub fn build_complete_request(
    device_id: &str, merged: Vec<MergedResource>
) -> SyncCompleteRequest
```

### Shared Runtime Abstraction

**New file:** `crates/temper-cli/src/actions/runtime.rs`

Extracts the repeated pattern of creating a tokio runtime + initializing an authenticated TemperClient:

```rust
/// Initialize authenticated TemperClient with device_id and OAuth config
pub fn build_client() -> Result<TemperClient>

/// Create tokio runtime and execute an async closure with an authenticated client
pub fn with_client<F, T>(f: F) -> Result<T>
where
    F: FnOnce(TemperClient) -> Pin<Box<dyn Future<Output = Result<T>>>>
```

Existing commands (`add.rs`, `import_cmd.rs`, `pull.rs`, `remove.rs`) refactored to use `with_client()` as part of I6a.

---

## I6b: Auto-Merge & Workflow Integration (Guidance)

### Auto-Merge Model

When sync detects a conflict (both local and remote changed since last sync):

1. Diff the local and remote content
2. Inject merge notice at appropriate heading level:
   ```markdown
   ### Temper Merge Notice: event <uuid> for <email> on <timestamp>
   ```
3. Concatenate both sides' changes — never remove content
4. **Idempotency:** Check for existing event UUID in file content before injecting. If the event UUID already exists, skip the merge notice (prevents replay on interrupted syncs)
5. Mark manifest entry as `Merged` (new state) so sync completion can batch-post merged hashes

### Manifest Rehash Before Sync

Step 3 of sync flow (already defined in I6a):
- Compute SHA-256 for each local file in target contexts
- Mtime-based skip: only rehash if file mtime > `manifest.synced_at`
- Update manifest state from `Clean` → `LocalModified` where hash changed

### `--auto-sync` Flag

Added to: `temper task create/move/done`, `temper session save`, `temper goal create/update`

- Default: `false`
- When `true`: after writing the local file, update manifest to `LocalModified`, then immediately push via light ingest path using `with_client()` abstraction
- Uses existing `POST /api/ingest` (new) or `PUT /api/ingest/:id` (existing) based on manifest state

### `sync.contexts` Config

New field in `config.toml`:
```toml
[sync]
contexts = ["temper", "tasker"]
```

Specifies which contexts participate in sync. Falls back to all contexts if unset. Parsed alongside existing `VaultConfig`.

### Post-Merge Sync Completion

After auto-merge, the sync completion step (`POST /api/sync/complete`) includes merged resources with new content hashes. The server updates `kb_resources.content_hash` and triggers `processIngest` to re-embed the merged content.

---

## I6c: Team Sync & Manual Resolution (Guidance)

### Manual Conflict Resolution

- `temper sync resolve <uuid>` — `--keep local|remote`, or accept current file as resolution
- `temper merge <uuid>` — parse existing merge notices, present both versions, produce clean file
- `POST /api/sync/resolve` endpoint — record resolution server-side, update resource state

### Team Subscriptions

- Subscribe to team-shared resources by team slug
- Server includes team resources in sync/status response via `resources_visible_to()` with team parameter
- Per-team sync scoping alongside per-context scoping

### Doc Type Filtering

- Subscription scoping by doc_type (e.g., only sync tasks and sessions)
- Expressed in subscription config, passed to server in sync/status request

### Remote Config

- Defaults from `profile.vault_config` JSONB on `kb_profiles`
- Per-device overrides from `profile.vault_config.devices[client_id]`
- Local `config.toml` can override remote config
- Priority: local config > per-device remote > remote defaults
