# CLI-Native Ingest with Context CRUD — Design Spec

**Date:** 2026-04-01
**Scope:** Combined I5f (context CRUD) + I5g (CLI-native ingest), unblocking I5e validation
**Branch:** `jcoletaylor/temper-cloud`

---

## Problem

I5e (vault restructure + config unification) is functionally complete but cannot be validated end-to-end. The `temper import` flow routes through the TypeScript `/api/ingest` endpoint, which has an auth issue (profile auto-provisioning only exists in the Axum path). Rather than fix the TypeScript path that I5g deprecates, we move the entire ingest pipeline to Rust/Axum and add the context CRUD endpoints needed to support it.

## Approach

Bottom-up pipeline build (Approach A): build each layer with independent testability, then connect them.

## Design

### 1. Crate Rename: `temper-embed` → `temper-ingest`

The crate handles the full content ingestion lifecycle (extract → chunk → embed), not just embedding. Rename while the project is pre-alpha and the surface area is small.

**Changes:**
- Rename `crates/temper-embed/` → `crates/temper-ingest/`
- Update `Cargo.toml` package name
- Update workspace `Cargo.toml` member path
- Update all `use temper_embed::` → `use temper_ingest::` imports
- Existing features (`extract`, `embed`) unchanged

**New module: `temper_ingest::chunk`**

Port of `packages/temper-cloud/src/processing/chunk.ts`. Pure function, no feature gate needed.

```rust
pub struct ChunkData {
    pub chunk_index: u32,
    pub header_path: String,    // "Design > API > Auth"
    pub content: String,
    pub content_hash: String,   // "sha256:..."
}

pub fn chunk_markdown(text: &str) -> Vec<ChunkData>;
```

Algorithm:
- Split input on newlines
- Match headings via `^#{1,6}\s+(.+)$`
- Maintain a header stack: on new heading, pop headers at same or deeper level, push new header
- Build `header_path` by joining stack texts with `" > "`
- Flush accumulated content as a chunk on heading or EOF
- Skip empty chunks (trimmed content is empty)
- SHA-256 hash each chunk's content for `content_hash`
- Unit tests with parity to TypeScript `chunk.test.ts`

### 2. Context CRUD in Axum

**Handler:** `crates/temper-api/src/handlers/contexts.rs`

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/contexts` | List contexts visible to authenticated profile |
| `POST` | `/api/contexts` | Create context owned by profile |
| `GET` | `/api/contexts/:id` | Get single context (visibility-checked) |

**Service:** `crates/temper-api/src/services/context_service.rs`

- `list_visible(pool, profile_id)` — uses `contexts_visible_to(p_profile_id)` SQL function
- `create(pool, profile_id, name)` — INSERT with `kb_owner_table='kb_profiles'`, `kb_owner_id=profile_id`. Unique constraint on `(kb_owner_table, kb_owner_id, name)` prevents duplicates.
- `get_visible(pool, profile_id, context_id)` — single lookup through `contexts_visible_to()`
- `resolve_by_name(pool, profile_id, name)` — lookup by name within visible contexts (used by ingest endpoint)

**Core types:** `ContextRow` in `temper-core` (id, name, kb_owner_table, kb_owner_id, created, updated).

**Client:** New `ContextClient` sub-client in `temper-client` following existing pattern.

**CLI:** `temper context list` and `temper context create <name>`. Existing `temper context` (set active context in config) unchanged.

**Future scope (I5h):** Context rename, delete (only if zero resources), and resource relocation across contexts. See task `2026-04-01-i5h-context-crud-lifecycle-rename-delete-relocate.md`. TODO comments in the handler/service reference this task.

### 3. New Axum `POST /api/ingest` Endpoint

**Handler:** `crates/temper-api/src/handlers/ingest.rs`

Two endpoints:

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/ingest` | Create new resource with chunks + embeddings |
| `PUT` | `/api/ingest/:id` | Update existing resource content (re-chunk, re-embed) |

Both accept JSON body with gzip `Content-Encoding` (via `tower-http` `DecompressionLayer`):

```rust
// In temper-core — shared between client and server
#[derive(Debug, Serialize, Deserialize)]
pub struct IngestPayload {
    pub title: String,
    pub origin_uri: String,
    pub context_name: String,
    pub doc_type_name: String,
    pub resource_mode: String,        // "added" | "imported"
    pub content_hash: String,         // "sha256:..."
    pub slug: String,
    pub mimetype: String,
    pub content: String,              // full extracted markdown
    pub metadata: Option<serde_json::Value>,
    pub chunks_packed: String,        // base64-encoded MessagePack
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackedChunk {
    pub chunk_index: u32,
    pub header_path: String,
    pub content: String,
    pub content_hash: String,
    pub embedding: Vec<f32>,          // 768 dims
}
```

**Service:** `crates/temper-api/src/services/ingest_service.rs`

Single-transaction pipeline:
1. Auth via existing middleware (profile auto-provisioned)
2. `context_service::resolve_by_name()` → context UUID (404 if not found)
3. Resolve `doc_type_name` → doc_type UUID from `kb_doc_types` (400 if unknown)
4. Content-hash dedup: check `kb_resources` for matching hash + profile → return existing resource if found (200, idempotent)
5. INSERT `kb_resources` with `resource_mode` from request
6. Decode `chunks_packed`: base64 → bytes → `rmp_serde::from_slice` → `Vec<PackedChunk>`
7. Version-bump: `UPDATE kb_chunks SET is_current = false WHERE resource_id = $1 AND is_current = true`
8. Batch INSERT new `kb_chunks` rows with embeddings cast to `::vector`
9. Return created `ResourceRow`

**New dependencies:**
- `rmp-serde` — both `temper-api` (decode) and `temper-client` (encode)
- `base64` — already in workspace
- `tower-http` decompression — enable on the ingest route

**Error responses:**
- Context not found → 404
- Doc type not found → 400
- Duplicate content hash → 200 with existing resource
- Invalid MessagePack → 400

### 4. Update `temper-client` IngestClient

Replace multipart form approach with JSON POST:

```rust
impl IngestClient<'_> {
    pub async fn create(&self, payload: &IngestPayload) -> Result<ResourceRow>;
    pub async fn update(&self, id: Uuid, payload: &IngestPayload) -> Result<ResourceRow>;
}
```

- Serialize to JSON, gzip compress via `reqwest` gzip feature
- `Content-Type: application/json`, `Content-Encoding: gzip`
- Remove old multipart handling entirely

### 5. CLI Pipeline Update

**New flow in `actions/ingest.rs`:**

```
temper_ingest::extract::extract_to_markdown()
  → temper_ingest::chunk::chunk_markdown()
  → temper_ingest::embed::embed_texts()     // batch embed all chunks
  → pack_chunks()                            // ChunkData + embeddings → MessagePack → base64
  → build_ingest_payload()                   // construct IngestPayload
  → client.ingest().create()
  → write_vault_file_and_register()          // local vault write (unchanged)
```

- `pack_chunks()` combines `ChunkData` with `Vec<Vec<f32>>` into `Vec<PackedChunk>`, serializes via `rmp_serde::to_vec`, then base64 encodes
- Remove old `build_ingest_request()` and old `IngestRequest` type — the TypeScript ingest path is dead for CLI users
- `temper import` sets `resource_mode = "imported"`
- `temper add` sets `resource_mode = "added"`
- Local vault write (`write_vault_file_and_register`) unchanged

### 6. Migrate Sync Endpoints to Axum

The TypeScript sync endpoints are thin wrappers around SQL functions. Migrating them eliminates the last split-routing surface for CLI users — after this, CLI traffic is 100% Axum.

**Handler:** `crates/temper-api/src/handlers/sync.rs`

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/sync/status` | Compute sync diff for device |
| `POST` | `/api/sync/complete` | Finalize sync round |

**Service:** `crates/temper-api/src/services/sync_service.rs`

`compute_sync_diff(pool, profile_id, body)`:
- Flatten `body.contexts` into `context_names: Vec<String>` and `manifest_entries: Vec<ManifestEntry>` (uri, local_hash, remote_hash)
- Call `sync_diff_for_device(profile_id, context_names, manifest_jsonb)` SQL function
- Categorize rows into `SyncDiffResult { to_push, to_pull, conflicts, removed }` — port of the TypeScript `categorizeDiffRows()` pure function

`complete_sync_round(pool, profile_id, body)`:
- Batch UPDATE `kb_resources` content hashes for merged resources (single query using `unnest()` instead of the TypeScript per-row loop — addresses code review audit item 5e)
- UPSERT `kb_device_sync_state` with `last_sync_at = now()`
- Return `SyncCompleteResult { last_sync_at, updated_count }`

**Core types:** The sync request/response types (`SyncStatusRequest`, `SyncDiffResult`, `SyncCompleteRequest`, `SyncCompleteResult`) already exist in `temper-core`. The `SyncPushItem`, `SyncPullItem`, `SyncConflictItem`, `SyncRemovedItem` types may need adding.

**Client:** Update `temper-client` `SyncClient` to point at the Axum endpoints (same paths, no client changes needed if the request/response shapes match).

**Delete:** Remove `api/sync/status.ts` and `api/sync/complete.ts`.

### 7. Vercel Routing Cleanup

- Delete `api/ingest.ts`, `api/ingest/[id].ts`, `api/sync/status.ts`, `api/sync/complete.ts`
- The `vercel.json` catch-all routes all these paths to Axum naturally
- No `vercel.json` changes needed

**What stays in TypeScript:**
- `api/upload.ts` — blob upload for web UI / MCP
- `api/auth/cli-callback.ts` — OAuth PKCE relay

### 8. End-to-End Validation

Proves I5e is complete:

1. `temper context create temper` — create context server-side
2. `temper import docs/2026-04-01-i5e-handoff.md --context temper --doc-type task` — full pipeline
3. Verify in DB: resource with `resource_mode='imported'`, chunks with embeddings in `kb_current_chunks`
4. `temper search "config unification"` — search hits against imported content
5. `temper sync run --context temper` — verify sync through Axum (no longer optional — sync is now fully on Axum)

## Out of Scope

- Local HNSW index update (I5g originally included this — defer to separate work)
- Context rename/delete/resource relocation (I5h)
- Web UI ingest path (stays on TypeScript blob upload)
- Batch ingest optimization for `temper import --dir`
