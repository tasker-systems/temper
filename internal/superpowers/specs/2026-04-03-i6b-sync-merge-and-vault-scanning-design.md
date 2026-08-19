# I6b: Sync Merge, Vault Scanning, and Progress Reporting

**Mode:** plan | **Effort:** large | **Context:** temper
**Depends on:** I6a (sync infrastructure — implemented), frontmatter hash fix + mtime optimization (this branch)
**De-scoped:** Change ledger (`kb_resource_change_ledger`) — no concrete use case yet; revisit if needed.

## Problem Statement

`temper sync run` can push and pull resources between the local vault and the cloud API, but it has significant gaps relative to the designed sync protocol:

1. **No vault scanning** — new markdown files placed in the vault are invisible to sync until manually imported
2. **No merge capability** — when both local and remote have changed, resources are marked as conflicts and skipped indefinitely
3. **No post-merge processing** — merged content needs to be re-chunked, re-embedded, and pushed, but no pipeline exists for this
4. **No progress feedback** — sync runs silently and only prints a summary at the end

These gaps mean that collaborative editing produces permanent conflicts, the vault can't be used as a natural ingestion surface, and users have no visibility into what sync is doing.

## Approach

Extend the existing sequential `sync_orchestration` pipeline with new phases inserted at the right points. No new API endpoints are required — all server interaction uses the existing ingest and sync APIs. Merge logic lives in `temper-ingest` as a shared crate so future consumers (web, MCP) can reuse it.

## 1. Vault Scanning

### When It Runs

New first phase of `sync_orchestration`, before rehash. Walks the vault directory tree looking for `.md` files not present in the manifest.

### Discovery Logic

1. Walk `{vault_root}/{context}/{doc_type}/` directories for `.md` files
2. For each file, check if any manifest entry has a matching `path` — if not, it's untracked
3. Parse existing frontmatter via `ingest::parse_source_frontmatter()` (reuse, don't duplicate)
4. Determine context and doc_type:
   - If frontmatter has `context` and `doc_type` fields, honor those (override)
   - Otherwise infer from directory position: `temper/research/foo.md` → context=temper, doc_type=research
5. Files that can't be mapped (e.g., at vault root, unknown doc_type directory) → warn and skip
6. For files needing frontmatter: generate via `ingest::build_frontmatter()` with new UUIDv7, write back to file
7. Add new entry to manifest as `Pending` state with body-only content hash and current mtime

### What It Reuses

`ingest::parse_source_frontmatter`, `ingest::build_frontmatter`, `ingest::compute_content_hash`, `ingest::strip_frontmatter`. The directory-to-context/doc_type inference logic should be extracted from the existing import command into a shared function in `temper-cli/src/actions/ingest.rs`.

### New Code

- `temper-cli/src/actions/sync.rs`: `scan_vault_for_untracked()` function
- Shared helper in `ingest.rs`: `infer_context_and_doctype(vault_root, file_path, frontmatter)` → `Result<(String, String)>`

## 2. Semantic Merge Strategy

### When It Runs

After the server diff returns results, resources classified as `conflict` (both sides changed) go through the merge pipeline instead of being skipped.

### Merge Pipeline

`similar` is the diff engine throughout — it drives both the initial change-region analysis and any deeper semantic resolution. The pipeline progressively escalates only when simpler strategies can't fully resolve.

**Step 1 — Diff with `similar`:**
- Fetch remote content via `client.resources().content(id)`
- Diff local body (frontmatter-stripped) against remote body using `similar` at paragraph granularity (split on `\n\n` boundaries)
- This produces a sequence of `Equal`, `Insert`, `Delete`, and `Replace` operations over paragraph-level chunks

**Step 2 — Scan change regions:**
- Walk the diff operations and classify each change region:
  - **Equal**: unchanged — accept as-is
  - **Insert** (content in one side only): non-conflicting addition — accept
  - **Delete** (content removed from one side only): non-conflicting removal — accept
  - **Replace** (different content in both sides at the same position): potential conflict — escalate to step 3
- If no `Replace` regions exist, all changes are non-conflicting. Reassemble and return — no annotation needed. This covers append-only changes, disjoint edits, and all other non-overlapping modifications in a single pass.

**Step 3 — Semantic boundary resolution (only for Replace regions):**
- For each `Replace` region, run both the local and remote variants through `temper-ingest`'s semantic chunker to understand finer-grained structure
- Re-diff at the semantic chunk level using `similar` — this may reveal that changes within the region are actually at non-overlapping sub-boundaries (e.g., one side changed a paragraph header, the other changed body text in a different paragraph within the same coarse region)
- Accept any sub-chunks that resolve cleanly
- Any sub-chunks that remain in true conflict (both sides changed the same semantic unit) → annotate

**Step 4 — Conflict annotation (only for unresolved sub-chunks):**
- For each truly conflicting region, embed both versions inline:

```markdown
<!-- sync-conflict: local (a1b2c3d4) vs remote (e5f6g7h8) at 2026-04-03T12:00:00Z -->
<!-- local version -->
[local content]
<!-- remote version -->
[remote content]
<!-- end conflict -->
```

- HTML comments don't break markdown rendering but are searchable via grep or `temper sync status`
- The file is treated as successfully merged — conflict-annotated content is valid content that gets re-processed and pushed like any other merge result. This ensures both sides converge on the same version and prevents conflict cascades on subsequent syncs.

### Key Insight

Most real-world edits to knowledge-base documents are non-conflicting: one person appends notes, another edits an earlier section, or edits happen at different times to different parts of the file. By using `similar` as the primary diff engine and scanning the change regions first, the vast majority of merges resolve at step 2 without ever needing chunk-level analysis or conflict annotation. The semantic chunker is a targeted escalation tool, not the default path.

### Where It Lives

New `merge` module in `temper-ingest`:
- `diff_paragraphs(local: &str, remote: &str) -> Vec<DiffOp>` — step 1, `similar`-based paragraph diff
- `classify_change_regions(ops: &[DiffOp]) -> ChangeClassification` — step 2, scan for conflicts vs. clean merges
- `resolve_replace_region(local_region: &str, remote_region: &str) -> RegionResult` — step 3, semantic chunk escalation
- `attempt_merge(local: &str, remote: &str) -> MergeResult` — orchestrates the full pipeline
- `annotate_conflict(local: &str, remote: &str, local_hash: &str, remote_hash: &str) -> String` — step 4 formatting

The `MergeResult` enum:
```
AutoMerged { content: String, strategy: MergeStrategy }
ConflictAnnotated { content: String, conflict_count: usize }
```

`MergeStrategy`: `NonConflicting` (resolved at step 2), `SemanticResolution` (required step 3), `Mixed` (some regions resolved, some annotated).

### New Dependency

`similar` crate (workspace dependency) — the diff engine for both paragraph-level and chunk-level analysis.

## 3. Post-Merge Re-chunk/Re-embed and Push

After merge (whether clean or conflict-annotated), the merged content goes through the existing ingest pipeline:

1. Write merged content back to vault file (preserve frontmatter)
2. Compute body-only content hash
3. Run through `temper-ingest` pipeline: `chunk_markdown()` → `embed_texts()` → `pack_chunks()`
4. Build `IngestPayload` via `ingest::build_ingest_payload()`
5. `PUT /api/ingest/{resource_id}` — server updates content_hash, replaces chunks/embeddings
6. Update manifest: `content_hash` = new body hash, `remote_hash` = server's returned hash, `state` = Clean, `mtime_secs` = current file mtime

This is identical to the existing push path for dirty resources. The only difference is the content has been through the merge pipeline first. No new API endpoints needed.

## 4. Progress Reporting

### Design

A callback trait that `sync_orchestration` accepts, with methods for each event type:

```rust
pub trait SyncProgress {
    fn scan_found(&self, path: &str, context: &str, doc_type: &str);
    fn scan_skipped(&self, path: &str, reason: &str);
    fn rehash_progress(&self, processed: usize, total: usize, skipped_by_mtime: usize);
    fn push_start(&self, path: &str, kind: PushKind); // New, Modified, Merged, ConflictAnnotated
    fn push_done(&self, path: &str);
    fn pull_start(&self, path: &str);
    fn pull_done(&self, path: &str);
    fn merge_result(&self, path: &str, outcome: &MergeResult);
    fn phase_summary(&self, phase: &str, count: usize);
}
```

### Implementations

- `TerminalProgress` — the CLI implementation, uses `indicatif` for progress bars during rehash and the existing `output` module for styled/colored status lines. Respects TTY detection.
- `NoopProgress` — for tests and non-interactive contexts
- `CollectingProgress` — captures events into a `Vec` for test assertions

### Example Output

```
Scanning vault...
  + New: temper/research/my-notes.md (context: temper, doc_type: research)
  ⚠ Skipped: orphan-file.md (cannot infer context/doc_type)
  Found 1 new, 1 skipped

Checking for changes...
  Rehashed 12/588 files (576 unchanged by mtime)

Syncing with server...
  ↑ Push: temper/research/my-notes.md (new)
  ↑ Push: temper/task/some-task.md (modified)
  ↓ Pull: temper/session/2026-04-02.md
  ⟳ Merge: temper/research/collab-notes.md — auto-merged (disjoint)
  ↑ Push: temper/research/collab-notes.md (merged)
  ⟳ Merge: temper/task/some-task.md — 1 conflict region (annotated)
  ↑ Push: temper/task/some-task.md (conflict-annotated)

Done: 3 pushed, 1 pulled, 2 merged (1 with conflicts)
```

## 5. Updated Orchestration Flow

The revised `sync_orchestration` steps:

1. **Scan vault** — discover untracked files, write frontmatter, add to manifest
2. **Rehash manifest** — mtime-optimized, body-only hashing (already implemented)
3. **Request diff** from server
4. **Push new resources** — vault-scanned `Pending` entries first
5. **Push dirty resources** — `LocalModified` entries
6. **Pull** remote-only or remote-modified resources
7. **Merge conflicts** — three-phase semantic merge via `temper-ingest`
8. **Re-process and push merged** — chunk/embed/push merged and conflict-annotated content
9. **Handle removed** — delete local files, remove from manifest
10. **Complete sync round** — `POST /api/sync/complete`, update manifest timestamp
11. **Save manifest**

Progress callbacks fire throughout. The `SyncResult` struct expands to include merge counts.

## 6. Crate Responsibilities

| Crate | New Responsibilities |
|-------|---------------------|
| `temper-ingest` | `merge` module: change pattern analysis, semantic chunk diff, auto-merge, conflict annotation |
| `temper-cli` | Vault scanning, progress trait + terminal implementation, updated orchestration |
| `temper-core` | `MergeResult`, `MergeStrategy`, `PushKind` types; expanded `SyncResult` |
| `temper-client` | No changes — existing `resources().content()` and `ingest().update()` suffice |
| `temper-api` | No changes — existing endpoints handle all operations |

## 7. Testing Strategy

### Unit Tests (temper-ingest)
- Append-only detection (local appended, remote appended, neither)
- Disjoint block detection with various change patterns
- Semantic chunk diff with known chunk boundaries
- Conflict annotation formatting and round-trip parseability
- Edge cases: empty files, files with only frontmatter, single-chunk files

### Unit Tests (temper-cli)
- Vault scan discovers files in correct directory structure
- Vault scan respects frontmatter overrides
- Vault scan skips unmappable files with warnings
- Rehash + mtime integration (already done in this branch)
- Progress trait captures expected events

### Integration Tests
- Full sync cycle with vault-scanned file → push → verify on server
- Merge scenario: modify local, modify remote differently, sync → auto-merged
- Conflict scenario: modify same chunk on both sides → conflict-annotated → pushed
- Second sync after conflict annotation → clean (no cascade)

## 8. Future Architecture: Event-Driven State Machine

> This section documents a target architecture refinement for a future ticket. It is NOT part of this implementation.

The current sequential pipeline works well for single-user vault sync. As sync scales to team collaboration (I6c), concurrent operations, and partial retry, the architecture should evolve toward modeling each resource as an independent state machine:

```
Untracked → Pending → Pushing → Clean
Clean → LocalModified → Pushing → Clean
Clean → RemoteModified → Pulling → Clean
Clean → BothModified → Merging → Pushing → Clean
Clean → Removed → (deleted)
```

A `SyncEngine` would process resources independently, enabling:
- Per-resource error recovery (one failed push doesn't block others)
- Natural parallelism (independent resources can sync concurrently)
- Pure function state transitions (easier to test)
- Richer progress reporting (each resource has a clear lifecycle position)

The `SyncProgress` trait introduced in this spec is designed to be compatible with this future architecture — it already reports per-resource events rather than batch summaries.

## De-scoped

- **Change ledger** (`kb_resource_change_ledger`): No concrete use case identified. The three-hash comparison model is stateless and sufficient. Revisit if audit trail or historical diff queries become needed.
- **Manual conflict resolution command** (`temper sync resolve`): Conflict annotations are valid content that syncs normally. A resolution UX can be added later without protocol changes.
- **Team subscription scoping** (I6c): Separate design exercise.

## New Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `similar` | latest | Chunk-level sequence diffing for merge phase 2 |
