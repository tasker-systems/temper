# Sync Metadata-Only PATCH — Design Spec

**Date:** 2026-04-11
**Task:** `2026-04-11-knowledge-graph-ui-and-seeding-the-vault-for-relationships`
**Related specs:**
- `2026-04-11-open-meta-intentionality-and-graph-build-design.md`
- `2026-04-11-knowledge-graph-ui-design.md`
**Mode:** build
**Effort:** medium

---

## Problem

Any hash change — body, managed_meta, or open_meta — triggers a full re-ingest:
the client builds an `IngestPayload` with body + chunks, the server calls
`replace_chunks()`, and every chunk gets version-bumped, re-inserted, and
re-embedded. When `temper graph build` writes relationship frontmatter across
hundreds of vault files, only `open_hash` changes. The body is identical.
Re-chunking and re-embedding is wasted work — and at ~1000 vault files, it
makes metadata-only operations impractically slow.

## Current State — What's Already Built

The three-tier hash model is fully implemented at the diff layer:

| Layer | Status |
|-------|--------|
| `rehash_manifest()` | Detects body, managed, and open hash changes independently |
| `sync_diff_for_device()` SQL | Returns `to_push_body`, `to_push_meta`, `to_pull_body`, `to_pull_meta` as distinct diff types |
| `SyncManifestEntry` | Carries all six hashes (body/managed/open x local/remote) |
| `sync refresh` / `sync reset` | Track managed and open hashes independently |
| `categorize_diff_rows()` | Matches `to_push_body`, `to_push_meta`, etc. — but collapses them into undifferentiated `SyncPushItem` |
| `DiffRow` struct | Has `managed_hash` and `open_hash` fields marked `#[expect(dead_code)]` with reason "will be used for three-tier sync" |

The hard design work (three-tier hashing, diff categorization SQL) is done.
This spec wires up the last mile.

## Design

### 1. Wire Types — Preserve Push/Pull Kind

Add a `kind` field to `SyncPushItem` and `SyncPullItem` in
`temper-core/src/types/sync.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncItemKind {
    Body,
    MetaOnly,
}

pub struct SyncPushItem {
    pub uri: String,
    pub resource_id: Option<ResourceId>,
    #[serde(default = "default_body")]
    pub kind: SyncItemKind,
}

pub struct SyncPullItem {
    pub uri: String,
    pub resource_id: ResourceId,
    pub content_hash: String,
    #[serde(default = "default_body")]
    pub kind: SyncItemKind,
}
```

Default is `Body` for backward compatibility with servers that don't yet
send the field.

`categorize_diff_rows()` maps `to_push_body` → `SyncItemKind::Body`,
`to_push_meta` → `SyncItemKind::MetaOnly` (same for pull). Remove the
`#[expect(dead_code)]` annotations from `DiffRow`.

### 2. Server Endpoint — `PATCH /api/resources/{id}/metadata`

New handler in `temper-api/src/handlers/` (or added to the existing
`ingest` handler module):

**Request payload:**

```rust
pub struct MetadataPatchPayload {
    pub managed_meta: Option<serde_json::Value>,
    pub open_meta: Option<serde_json::Value>,
    pub doc_type_name: String,
}
```

**Server-side behavior:**

1. Auth + `can_modify_resource()` check (same as `update()`)
2. Strip tier-1 system fields from managed_meta (same as `update()`)
3. Apply doc-type defaults (same as `update()`)
4. Update `kb_resource_manifests`: set `managed_meta`, `open_meta`,
   recompute `managed_hash` and `open_hash`, bump `updated`
5. Insert `metadata_update` event in `kb_events` + audit trail
6. Call `reconcile_edges()` — relationship fields live in `open_meta`,
   so edge changes must propagate even without a body change
7. Return the updated `ResourceRow`

**Does NOT:**
- Touch `kb_resource_chunks` — no re-chunking, no re-embedding
- Modify `body_hash` or `content_hash`
- Accept or process `content` or `chunks_packed` fields

### 3. Client Push Branching

In `temper-cli/src/actions/sync.rs`, `push_resource()` branches on
`item.kind`:

**`SyncItemKind::Body`** — existing path:
- `build_ingest_payload()` with body + chunks
- `PUT /api/ingest/{id}` (or `POST` for new resources)
- Full re-chunk and re-embed

**`SyncItemKind::MetaOnly`** — new path:
- Parse frontmatter, `split_frontmatter_tiers()`
- Send only `MetadataPatchPayload` via `PATCH /api/resources/{id}/metadata`
- No body, no chunks, no embedding pipeline
- Update manifest remote hashes on success

This means the CLI doesn't need to run the embedding pipeline for
metadata-only changes, saving local compute as well as network transfer.

### 4. Client Pull Branching

`pull_resource()` branches on `item.kind`:

**`SyncItemKind::Body`** — existing path:
- Download full resource content
- Overwrite vault file
- Rebuild local hashes

**`SyncItemKind::MetaOnly`** — new path:
- Fetch resource via existing `GET /api/resources/{id}` (already returns
  `managed_meta` and `open_meta` on the manifest join)
- Read the local vault file, parse frontmatter
- Merge server-side managed_meta and open_meta into the local frontmatter
  (last-write-wins per field, same as current metadata conflict strategy)
- Write the updated file preserving body content untouched
- Update manifest remote hashes to match server values

### 5. temper-client Changes

Add `metadata_patch()` to the ingest client (or a new metadata client):

```rust
pub async fn metadata_patch(
    &self,
    resource_id: Uuid,
    payload: &MetadataPatchPayload,
) -> Result<ResourceRow> { ... }
```

## Scope Boundary

This spec does **not** cover:

- Conflict resolution changes — existing last-write-wins for metadata
  tiers is sufficient
- Batch metadata PATCH — single-resource is enough; callers loop
- `sync status` display changes — showing "(meta only)" annotations is
  polish, not required for correctness
- Changes to `sync_diff_for_device()` SQL — it already works correctly

## Testing Strategy

- **Unit tests**: `categorize_diff_rows()` preserves `SyncItemKind`
  correctly for all diff types
- **Integration tests**: metadata PATCH endpoint updates manifests without
  touching chunks; verify chunk versions are unchanged after PATCH
- **E2E test**: modify frontmatter only (add a `relates_to` field),
  `temper sync run`, verify only PATCH was called (not full ingest),
  verify edges were reconciled, verify chunks unchanged

## Dependencies

- None — this is foundational infrastructure
- Blocks: `temper graph build` (Spec 2), any future metadata editing

## Estimated Effort

Medium — the SQL and hash infrastructure is done. The work is:
1. Wire types (~30 min)
2. Server endpoint (~1-2 hours)
3. Client push/pull branching (~1-2 hours)
4. Tests (~1-2 hours)
