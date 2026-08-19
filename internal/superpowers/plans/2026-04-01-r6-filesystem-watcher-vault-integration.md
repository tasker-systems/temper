# R6: Filesystem Watcher for Managed Vault — Research & Proposal

**Date:** 2026-04-01
**Type:** Research (R-phase) + Implementation Proposal
**Scope:** Vault filesystem watching via `notify-rs/notify`, manifest state tracking, auto-sync triggering
**Depends on:** I6a (sync infrastructure — done), I6b (auto-merge & workflow integration — backlog), I5c (two-tier resource model — done), I5e (local KB restructure — done)
**Blocks:** I6b `--auto-sync` implementation, future MCP, future `temper watch` daemon

---

## Table of Contents

1. [Problem Statement](#problem-statement)
2. [Current Architecture Analysis](#current-architecture-analysis)
3. [Notify-rs Evaluation](#notify-rs-evaluation)
4. [Proposed Architecture](#proposed-architecture)
5. [Integration with Existing Sync Protocol](#integration-with-existing-sync-protocol)
6. [Configuration Model](#configuration-model)
7. [Event Processing Pipeline](#event-processing-pipeline)
8. [Crate Placement & Module Design](#crate-placement--module-design)
9. [Implementation Plan](#implementation-plan)
10. [Risk Analysis & Mitigations](#risk-analysis--mitigations)
11. [Alternatives Considered](#alternatives-considered)
12. [Open Questions](#open-questions)
13. [Decision Log](#decision-log)

---

## Problem Statement

Temper's vault sync is **entirely pull-based**: the user runs `temper sync run`, which re-hashes every manifest entry, diffs against the server, and reconciles. This has three structural limitations:

1. **Stale manifest state** — Between syncs, the manifest doesn't reflect local edits. `temper sync status` must re-hash all files to show accurate state. For vaults with hundreds of files, this is O(n) on every invocation even when only one file changed.

2. **No auto-sync pathway** — I6b designs an `--auto-sync` flag for workflow commands (`temper task create`, `temper session save`, etc.), but this only covers *temper-originated* writes. External edits (vim, VS Code, Obsidian) are invisible until the next explicit sync. The `auto_sync: true` field on `Subscription` in `VaultConfig` is defined but has no implementation path beyond "run manifest pre-flight on every temper command."

3. **No live status for MCP** — The future MCP server (`temper-mcp`) has no mechanism to show real-time vault health. It would need to poll-and-rehash, which is expensive and laggy.

A filesystem watcher solves all three: it keeps the manifest's `LocalModified` state current in near-real-time, enables push-on-change auto-sync, and provides an event stream for MCP and CLI status.

---

## Current Architecture Analysis

### Vault Structure

The vault is a directory tree of markdown files with YAML frontmatter, tracked by a manifest:

```
{vault-root}/
├── .temper/
│   ├── manifest.json      ← HashMap<Uuid, ManifestEntry> + device_id + last_sync
│   └── events.jsonl       ← append-only local audit trail
├── {context}/
│   └── {doc_type}/
│       └── {slug}.md      ← ResourceFrontmatter + markdown body
```

### Manifest State Machine

Each `ManifestEntry` tracks:
- `path` — relative vault path (e.g., `"temper/tickets/some-task.md"`)
- `content_hash` — SHA-256 of file content at last manifest update
- `remote_hash` — SHA-256 of remote content at last sync
- `synced_at` — timestamp of last sync
- `state` — `Clean | LocalModified | RemoteModified | Conflict | Pending`

State transitions relevant to watching:
- **External edit detected** → `Clean` → `LocalModified` (content_hash updated)
- **New file appears** → no entry → potential `Pending` (if it has valid frontmatter)
- **File deleted** → entry exists but file gone → needs removal tracking
- **File renamed/moved** → old path gone, new path appears → needs re-association via `temper-id`

### Change Detection Today

The `rehash_manifest()` function in `actions/sync.rs` implements the current approach:
1. Iterate all manifest entries
2. Read each file from disk
3. Compute SHA-256
4. Compare against stored `content_hash`
5. If different, mark `LocalModified`

This is O(n) in manifest size regardless of how many files actually changed. I6b proposes an mtime optimization (`skip if mtime ≤ synced_at`) but this is still poll-based.

### Long-Running Process Precedent

**There are none.** Every CLI command creates a fresh tokio runtime, executes, and exits. The OAuth login flow uses a bounded `tokio::time::timeout_at` loop, but it's not a persistent background process. A filesystem watcher would be the **first long-running process pattern** in the codebase.

---

## Notify-rs Evaluation

### Crate Overview

| Dimension | Value |
|-----------|-------|
| **Crate** | [`notify`](https://crates.io/crates/notify) |
| **Stable version** | 8.2.0 |
| **In-development** | 9.0.0-rc.2 (adds `EventKindMask`, native tokio/futures features) |
| **MSRV** | 1.85 |
| **License** | CC0-1.0 (notify core), MIT/Apache-2.0 (sub-crates) |
| **Notable users** | alacritty, cargo-watch, deno, rust-analyzer, watchexec, zed |

### Platform Backends

| Platform | Backend | Mechanism | Notes |
|----------|---------|-----------|-------|
| **macOS** | `FsEventWatcher` (default) | FSEvents | Coalesces events. Security model may drop events for unowned files. |
| **macOS** | `KqueueWatcher` (opt-in) | kqueue | More precise but opens an fd per watched file. |
| **Linux/Android** | `INotifyWatcher` | inotify | Subject to `max_user_watches` kernel limit (~65K default). |
| **Windows** | `ReadDirectoryChangesWatcher` | ReadDirectoryChangesW | Good coverage. |
| **BSD/iOS** | `KqueueWatcher` | kqueue + mio | FreeBSD, OpenBSD, NetBSD, DragonflyBSD. |
| **All** | `PollWatcher` | Manual stat() polling | Fallback. Works on network FS, Docker, pseudo-FS. |

`RecommendedWatcher` resolves to the best native backend at compile time. `RecommendedWatcher::kind()` returns `WatcherKind` for runtime branching.

### EventKind Hierarchy

```
EventKind
├── Any
├── Access(AccessKind)        ← Read, Open, Close (noisy, rarely useful)
├── Create(CreateKind)        ← File, Folder
├── Modify(ModifyKind)
│   ├── Data(DataChange)      ← Size, Content
│   ├── Metadata(MetadataKind) ← WriteTime, Permissions, etc.
│   └── Name(RenameMode)      ← From, To, Both
├── Remove(RemoveKind)        ← File, Folder
└── Other
```

For vault watching, we care about: `Create(File)`, `Modify(Data(*))`, `Modify(Name(*))`, `Remove(File)`. We explicitly do **not** care about `Access` events.

### Debouncer Options

| Feature | `debouncer-mini` | `debouncer-full` |
|---------|-------------------|-------------------|
| Time-based coalescing | ✅ | ✅ |
| Rename stitching (From+To → single) | ❌ | ✅ |
| File ID tracking | ❌ | ✅ (`FileIdCache`) |
| Dedup creates | ❌ | ✅ |
| Suppress Modify-after-Create | ❌ | ✅ |
| Directory remove dedup (inotify) | ❌ | ✅ |
| Memory overhead | Low | Moderate (file ID map) |
| Dependencies | Minimal | +`file-id`, +`walkdir` |

**Recommendation: `notify-debouncer-full`** — Vault files are frequently edited by external editors that produce wildly different event patterns (vim: truncate+write; VS Code: create-new+rename; Obsidian: write-in-place). The full debouncer's rename stitching and event deduplication are essential for producing clean, actionable events.

### v8 vs v9

v9.0.0-rc.2 adds `EventKindMask` (kernel-level event filtering on Linux) and native `tokio`/`futures` features. However:
- v9 is still RC, not stable
- v8.2.0 is production-proven
- `EventKindMask::CORE` filtering can be added later when v9 stabilizes

**Recommendation: Start with v8.2.0**, plan migration to v9 when stable. The API surface we use (debouncer-full, RecursiveMode, Event filtering in userspace) is stable across both versions.

### Key Gotchas

| Gotcha | Impact on Temper | Mitigation |
|--------|------------------|------------|
| Editor save behaviors differ wildly | Vault files edited by vim, VS Code, Obsidian, etc. | Use `debouncer-full` for event normalization |
| macOS FSEvents may coalesce heavily | Rapid edits during a session may appear as one event | Acceptable — we only need "file changed", not granularity |
| Linux inotify `max_user_watches` | Large vaults could exhaust default limit | Document requirement; fall back to `PollWatcher` |
| Network filesystems emit no events | Vault on NFS/SMB | Detect and fall back to `PollWatcher` |
| Watcher dropped = watching stops | Async code can accidentally drop | Bind watcher to long-lived struct, not temporary |
| `.temper/manifest.json` edits trigger events | Our own writes trigger self-notification | Filter `.temper/` directory from watched events |

---

## Proposed Architecture

### Design Principles

1. **Vault watcher is a library, not just a CLI command** — Expose from `temper-core` (or a new `temper-watch` crate) so MCP and CLI can both use it.
2. **Manifest is the single state authority** — The watcher updates manifest state; it does not maintain separate state.
3. **Debounced, coalesced events** — Raw filesystem events are noisy. The watcher produces a clean stream of `VaultEvent` values.
4. **Sync is a separate concern** — The watcher detects changes and updates manifest. Auto-sync (actually pushing to the server) is a policy layer on top.
5. **Graceful degradation** — If the native backend fails, fall back to `PollWatcher`. If the watcher can't start at all, temper continues working in poll-on-demand mode.

### Component Overview

```
┌──────────────────────────────────────────────────────────────────┐
│                        temper-watch crate                        │
│                                                                  │
│  ┌──────────────┐     ┌──────────────┐     ┌─────────────────┐  │
│  │ notify-rs    │────▶│ EventFilter  │────▶│ ManifestUpdater │  │
│  │ debouncer-   │     │              │     │                 │  │
│  │ full         │     │ • ignore     │     │ • rehash file   │  │
│  │              │     │   .temper/   │     │ • update entry  │  │
│  │ Watches:     │     │ • ignore     │     │ • detect new    │  │
│  │  {vault}/    │     │   non-.md    │     │ • detect delete │  │
│  │  recursive   │     │ • match to   │     │ • emit          │  │
│  │              │     │   manifest   │     │   VaultEvent    │  │
│  └──────────────┘     └──────────────┘     └────────┬────────┘  │
│                                                      │          │
│                                              ┌───────▼────────┐ │
│                                              │ VaultEvent     │ │
│                                              │ channel (mpsc) │ │
│                                              └───────┬────────┘ │
└──────────────────────────────────────────────────────┼──────────┘
                                                       │
                     ┌─────────────────────────────────┼─────────┐
                     │                                 │         │
              ┌──────▼──────┐                      ┌───▼───┐
              │ temper watch │                      │ MCP   │
              │ (CLI daemon) │                      │server │
              │              │                      └───────┘
              │ • log events │
              │ • auto-sync  │
              │   (optional) │
              │ • PID file   │
              │ • signal     │
              │   handling   │
              └──────────────┘
```

### VaultEvent Type

```rust
/// A coalesced, vault-meaningful event derived from raw filesystem notifications.
#[derive(Debug, Clone)]
pub enum VaultEvent {
    /// A tracked resource's content changed. Manifest updated to LocalModified.
    Modified {
        resource_id: Uuid,
        path: PathBuf,
        new_hash: String,
    },
    /// A new markdown file appeared with valid frontmatter.
    /// Not yet in manifest — caller decides whether to register.
    NewFile {
        path: PathBuf,
        frontmatter: Option<ResourceFrontmatter>,
    },
    /// A tracked resource's file was deleted from disk.
    Deleted {
        resource_id: Uuid,
        path: PathBuf,
    },
    /// A tracked resource's file was renamed/moved.
    Renamed {
        resource_id: Uuid,
        old_path: PathBuf,
        new_path: PathBuf,
    },
    /// The watcher encountered an error (backend failure, permission denied, etc.)
    Error {
        message: String,
        path: Option<PathBuf>,
    },
}
```

### VaultWatcher API

```rust
pub struct VaultWatcher {
    /// The debouncer owns the underlying notify watcher
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
    /// Receiver for processed vault events
    event_rx: tokio::sync::mpsc::Receiver<VaultEvent>,
    /// Shared manifest handle for state updates
    manifest: Arc<RwLock<Manifest>>,
    /// Vault root path
    vault_root: PathBuf,
}

impl VaultWatcher {
    /// Create a new watcher on the given vault directory.
    /// Returns the watcher (must be held alive) and the manifest is loaded from disk.
    pub fn new(
        vault_root: PathBuf,
        manifest: Arc<RwLock<Manifest>>,
        config: WatcherConfig,
    ) -> Result<Self>;

    /// Receive the next vault event. Returns None if the watcher was stopped.
    pub async fn next_event(&mut self) -> Option<VaultEvent>;

    /// Get a snapshot of current manifest state (for status display).
    pub async fn manifest_snapshot(&self) -> Manifest;

    /// Trigger a full re-scan (equivalent to rehash_manifest but using the
    /// watcher's shared manifest). Useful after watcher restart or on-demand.
    pub async fn full_rescan(&self) -> Result<Vec<VaultEvent>>;
}
```

---

## Integration with Existing Sync Protocol

### Manifest Update Flow

The watcher integrates with the existing manifest by performing the same operations as `rehash_manifest()`, but incrementally and in response to filesystem events rather than as a full scan:

```
Filesystem event (debounced)
    │
    ▼
EventFilter: is this a .md file inside {context}/{doc_type}/? Is it not .temper/?
    │ yes
    ▼
Read file, parse frontmatter, extract temper-id
    │
    ▼
Look up temper-id in manifest.entries
    │
    ├── Found: compute SHA-256, compare with entry.content_hash
    │   │
    │   ├── Different → update content_hash, set state = LocalModified, emit Modified
    │   └── Same → no-op (editor touched file without changing content)
    │
    ├── Not found + valid frontmatter → emit NewFile (caller decides registration)
    │
    └── Not found + no frontmatter → ignore (not a temper-managed file)
```

For deletions:
```
Remove event for path P
    │
    ▼
Reverse-lookup: find manifest entry where entry.path == P
    │
    ├── Found → emit Deleted { resource_id, path }
    │   (do NOT remove from manifest — sync needs to know about the deletion)
    │
    └── Not found → ignore (not tracked)
```

For renames (debouncer-full stitches From+To):
```
Rename event (old_path, new_path)
    │
    ▼
Reverse-lookup: find manifest entry where entry.path == old_path
    │
    ├── Found → update entry.path = new_path, emit Renamed
    │
    └── Not found → treat new_path as potential NewFile
```

### Interaction with `temper sync run`

The watcher and explicit sync are complementary:

1. **Watcher running + `temper sync run`**: Sync reads the manifest (already up-to-date thanks to watcher). The `rehash_manifest()` step becomes a fast validation pass rather than a full recomputation. After sync completes and updates manifest entries to `Clean`, the watcher sees the manifest write but ignores `.temper/` paths.

2. **Watcher not running + `temper sync run`**: Behaves exactly as today — `rehash_manifest()` does the full O(n) scan. Zero regression.

3. **Watcher running + auto-sync policy**: When a `Modified` event fires, the auto-sync layer can immediately push the single changed resource without a full sync round. This is the fast path for the common case (edit one file, push immediately).

### Manifest Locking

The manifest needs concurrent access protection:

- **Watcher thread**: reads manifest to look up entries, writes to update state
- **CLI sync command**: reads and writes manifest during sync orchestration
- **MCP server**: reads manifest for status display

**Approach**: `Arc<RwLock<Manifest>>` with `tokio::sync::RwLock`. The watcher holds a write lock briefly per event. Sync commands acquire a write lock for the duration of the sync round (this is acceptable since sync is user-initiated and takes seconds). MCP uses read locks for display.

**Manifest persistence**: The watcher periodically flushes dirty manifest state to disk (e.g., every 5 seconds if changed, or immediately on graceful shutdown). This avoids writing `manifest.json` on every keystroke during active editing.

---

## Configuration Model

### Config Additions

Extend `~/.config/temper/config.toml`:

```toml
[watch]
# Enable/disable the filesystem watcher (default: true when running `temper watch`)
enabled = true

# Debounce timeout in milliseconds (default: 2000)
# Higher values = fewer events, more coalescing. Lower = more responsive.
debounce_ms = 2000

# Manifest flush interval in seconds (default: 5)
# How often dirty manifest state is written to disk.
flush_interval_secs = 5

# Auto-sync mode: "off", "immediate", "batched"
# - off: watcher updates manifest only, no network calls
# - immediate: push each changed file as soon as debounce fires
# - batched: collect changes, push every N seconds (see batch_interval_secs)
auto_sync_mode = "off"

# Batch interval for "batched" auto-sync mode (default: 30)
batch_interval_secs = 30

# Paths to ignore (in addition to .temper/)
# Glob patterns relative to vault root
ignore_patterns = ["*.tmp", "*.swp", ".obsidian/"]

# Force poll-based watching (useful for network filesystems)
force_poll = false

# Poll interval in seconds when using PollWatcher (default: 10)
poll_interval_secs = 10
```

### VaultConfig Integration

The existing `auto_sync` field on `Subscription` already signals intent per-context:

```rust
pub struct Subscription {
    pub context: String,
    pub auto_sync: bool,      // ← already exists, currently unused
    pub merge_policy: MergePolicy,
    // ...
}
```

The watcher respects this: when processing a `Modified` event, it checks whether the resource's context subscription has `auto_sync: true` before triggering network pushes.

### WatcherConfig Type

```rust
/// Configuration for the vault filesystem watcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatcherConfig {
    pub debounce_ms: u64,
    pub flush_interval_secs: u64,
    pub auto_sync_mode: AutoSyncMode,
    pub batch_interval_secs: u64,
    pub ignore_patterns: Vec<String>,
    pub force_poll: bool,
    pub poll_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoSyncMode {
    #[default]
    Off,
    Immediate,
    Batched,
}

impl Default for WatcherConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 2000,
            flush_interval_secs: 5,
            auto_sync_mode: AutoSyncMode::Off,
            batch_interval_secs: 30,
            ignore_patterns: vec![
                "*.tmp".into(),
                "*.swp".into(),
                ".obsidian/".into(),
            ],
            force_poll: false,
            poll_interval_secs: 10,
        }
    }
}
```

---

## Event Processing Pipeline

### Stage 1: Raw Filesystem Events → Debounced Events

`notify-debouncer-full` handles:
- Coalescing rapid writes into single events (2-second window)
- Stitching rename From+To into single Rename events
- Deduplicating Create+Modify into single Create events
- File ID tracking for reliable rename detection

### Stage 2: Debounced Events → Filtered Events

The `EventFilter` applies:

1. **Path filtering**: Ignore anything under `.temper/` (our own manifest/event writes)
2. **Extension filtering**: Only process `.md` files (vault is markdown-only)
3. **Ignore pattern matching**: Apply user-configured `ignore_patterns` (glob)
4. **Symlink handling**: Follow symlinks for content but don't double-report (use canonical paths)

Implementation sketch:

```rust
fn should_process(path: &Path, vault_root: &Path, ignore_globs: &[Pattern]) -> bool {
    let relative = path.strip_prefix(vault_root).ok();
    let relative = match relative {
        Some(r) => r,
        None => return false, // outside vault
    };

    // Always ignore .temper/ directory
    if relative.starts_with(".temper") {
        return false;
    }

    // Only markdown files
    if path.extension().map_or(true, |ext| ext != "md") {
        return false;
    }

    // User-configured ignore patterns
    let rel_str = relative.to_string_lossy();
    for glob in ignore_globs {
        if glob.matches(&rel_str) {
            return false;
        }
    }

    true
}
```

### Stage 3: Filtered Events → VaultEvents

The `ManifestUpdater` translates filtered filesystem events into semantic vault events:

1. **For `Create(File)` / `Modify(Data(_))`**:
   - Read the file
   - Parse YAML frontmatter (using existing `parse_frontmatter()`)
   - Extract `temper-id` from frontmatter
   - If `temper-id` found and in manifest → compute hash, compare, emit `Modified` or no-op
   - If `temper-id` found but not in manifest → emit `NewFile` with frontmatter
   - If no valid frontmatter → ignore (not a temper-managed file)

2. **For `Remove(File)`**:
   - Reverse-lookup path in manifest entries
   - If found → emit `Deleted`
   - If not found → ignore

3. **For `Modify(Name(Both))` (debouncer-stitched rename)**:
   - Reverse-lookup old path in manifest entries
   - If found → update manifest entry path, emit `Renamed`
   - If not found → treat new path as potential Create

### Stage 4: VaultEvents → Side Effects

Consumers decide what to do with events:

| Consumer | On Modified | On NewFile | On Deleted | On Renamed |
|----------|-------------|------------|------------|------------|
| **Manifest** | Update `content_hash`, set `LocalModified` | No-op (not auto-registered) | Retain entry, mark for sync deletion | Update `path` field |
| **Auto-sync (immediate)** | Push if `auto_sync: true` for context | Ignore | Queue deletion for next sync | Push path update |
| **Auto-sync (batched)** | Add to batch queue | Ignore | Add to batch queue | Add to batch queue |
| **MCP server** | Notify connected agents | Notify connected agents | Notify connected agents | Notify connected agents |
| **Events log** | Append to `events.jsonl` | Append | Append | Append |

---

## Crate Placement & Module Design

### Option A: New `temper-watch` Crate (Recommended)

```
crates/
├── temper-core/          ← shared types (VaultEvent, WatcherConfig added here)
├── temper-watch/         ← NEW: filesystem watcher library
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs        ← pub API: VaultWatcher, WatcherConfig
│       ├── filter.rs     ← EventFilter logic
│       ├── updater.rs    ← ManifestUpdater (fs event → VaultEvent)
│       ├── watcher.rs    ← notify-rs integration, debouncer setup
│       └── autosync.rs   ← Auto-sync policy layer (immediate/batched)
├── temper-cli/           ← depends on temper-watch for `temper watch` command
└── temper-mcp/           ← depends on temper-watch for agent notifications (future)
```

**Rationale:**
- `notify` + `notify-debouncer-full` are heavyweight dependencies (~15 transitive crates including `walkdir`, `file-id`, platform-specific backends). Isolating them prevents bloating `temper-core` or `temper-cli` for users who don't need watching.
- Clean dependency boundary: `temper-watch` depends on `temper-core` (for types), `temper-cli` depends on `temper-watch` (for the `watch` command).
- Other consumers (MCP) can depend on `temper-watch` without pulling in CLI code.

### Option B: Module in `temper-cli` Behind Feature Flag

```
crates/temper-cli/
├── Cargo.toml            ← notify, notify-debouncer-full under [features] watch = [...]
└── src/
    ├── watch/
    │   ├── mod.rs
    │   ├── filter.rs
    │   ├── updater.rs
    │   └── watcher.rs
    └── commands/
        └── watch_cmd.rs
```

**Trade-off:** Simpler workspace, but locks watching to CLI. MCP would need to duplicate or depend on CLI, which is architecturally backwards.

### Recommendation: Option A

A new `temper-watch` crate follows the same pattern as the existing workspace decomposition (`temper-core` for types, `temper-client` for HTTP, `temper-embed` for extraction/embedding). Each crate owns one concern.

### Dependency Graph Addition

```
                    temper-core (types)
                   /       |        \
                  /        |         \
          temper-client  temper-watch  temper-embed
              |         /    |
              |        /     |
          temper-cli ◄──────┘
              |
          temper (binary)
```

### Cargo.toml for `temper-watch`

```toml
[package]
name = "temper-watch"
version = "0.1.0"
edition = "2021"

[dependencies]
temper-core = { path = "../temper-core" }

# Filesystem watching
notify = "8.2"
notify-debouncer-full = "0.7"

# Async runtime
tokio = { version = "1", features = ["sync", "time", "macros"] }

# Hashing (consistent with temper-cli's sync hashing)
sha2 = "0.10"

# Glob pattern matching for ignore rules
glob = "0.3"

# Error handling
thiserror = "2"
anyhow = "1"

# Logging
tracing = "0.1"

# Serialization (for WatcherConfig, VaultEvent if persisted)
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# UUID (for resource_id in events)
uuid = { version = "1", features = ["v7", "serde"] }

# Timestamps
chrono = { version = "0.4", features = ["serde"] }
```

---

## Implementation Plan

### Phase 1: Core Watcher Library (I7a — new ticket)

**Goal:** `temper-watch` crate with `VaultWatcher` that produces `VaultEvent` values and updates manifest state. No networking, no auto-sync — pure local watching.

| # | Task | Description |
|---|------|-------------|
| 1 | **Types in temper-core** | Add `VaultEvent`, `WatcherConfig`, `AutoSyncMode` to `temper-core/src/types/`. Add `[watch]` section to `TemperConfig`. |
| 2 | **Manifest path index** | Add `path_index: HashMap<PathBuf, Uuid>` to `Manifest` for O(1) reverse lookups (path → resource_id). Rebuild on load. |
| 3 | **Create temper-watch crate** | Scaffold `crates/temper-watch/` with `Cargo.toml`, `src/lib.rs`. Add to workspace members in root `Cargo.toml`. |
| 4 | **EventFilter** | Implement `filter.rs` — path-based filtering (`.temper/`, non-`.md`, user ignore patterns). |
| 5 | **ManifestUpdater** | Implement `updater.rs` — translate filtered events to `VaultEvent`, update manifest state with SHA-256 rehash. |
| 6 | **VaultWatcher** | Implement `watcher.rs` — setup `notify-debouncer-full`, wire to EventFilter → ManifestUpdater → mpsc channel. Handle PollWatcher fallback. |
| 7 | **Manifest flush** | Background task that writes dirty manifest to disk on interval + graceful shutdown. |
| 8 | **Tests** | Unit tests for EventFilter, ManifestUpdater. Integration test using `tempdir` + file manipulation + event assertion. |

### Phase 2: CLI `temper watch` Command (I7b)

**Goal:** A long-running `temper watch` command that starts the watcher, logs events, and optionally auto-syncs.

| # | Task | Description |
|---|------|-------------|
| 1 | **`watch_cmd.rs`** | New CLI command: `temper watch [--auto-sync] [--verbose]`. Starts `VaultWatcher`, loops on events, prints status. |
| 2 | **PID file** | Write PID to `.temper/watch.pid` on start. Check for stale PID on startup. Remove on clean exit. |
| 3 | **Signal handling** | Catch SIGINT/SIGTERM → flush manifest → graceful shutdown. Use `tokio::signal`. |
| 4 | **Auto-sync layer** | When `--auto-sync` (or config `auto_sync_mode != off`): on `Modified` events, push via `temper-client`. Respect per-context `auto_sync` subscription flag. |
| 5 | **Batched sync** | For `auto_sync_mode = batched`: accumulate changed resource IDs, push batch every `batch_interval_secs`. Dedup by resource_id. |
| 6 | **Status output** | Periodic status line showing: watched files, pending changes, last sync, errors. |

### Phase 3: Integration & Polish (I7c)

**Goal:** Integrate watcher with MCP, improve `temper sync` when watcher is running, add to system health checks.

| # | Task | Description |
|---|------|-------------|
| 1 | **`temper sync` optimization** | When `.temper/watch.pid` exists and watcher is alive, `rehash_manifest()` skips the full O(n) scan and trusts the manifest's current state. Add a `--force-rehash` flag to override. |
| 2 | **`temper status` live mode** | `temper status --watch` starts a watcher and continuously updates the status display. |
| 3 | **MCP events** | `temper-mcp` uses `VaultWatcher` to notify connected agents of vault changes. |
| 4 | **Health check** | `temper check` reports watcher status (running/stopped, backend type, event backlog). |
| 5 | **Docs** | User guide for `temper watch`, configuration reference, troubleshooting (inotify limits, network FS). |

---

## Risk Analysis & Mitigations

### Technical Risks

| Risk | Severity | Likelihood | Mitigation |
|------|----------|------------|------------|
| **inotify limit exhaustion** on large vaults (Linux) | Medium | Medium | Document `sysctl` tuning. Auto-detect limit and warn. Fall back to PollWatcher if `MaxFilesWatch` error. |
| **macOS FSEvents coalescing** loses fine-grained events | Low | High | Acceptable — we only need "file changed" granularity, not per-byte diffs. Debouncer already coalesces. |
| **Manifest corruption** from concurrent watcher + CLI writes | High | Medium | `Arc<RwLock<Manifest>>` for in-process sharing. File-level locking (`.temper/manifest.lock`) for cross-process safety when daemon is running and CLI sync is invoked. |
| **Self-notification loop** — watcher triggers on own manifest writes | Medium | High | Filter `.temper/` directory absolutely. Already designed into EventFilter. |
| **Editor `.swp`/`.tmp` file churn** | Low | High | Default `ignore_patterns` include common editor temp files. Debouncer coalesces rapid changes. |
| **Large file reads** during hash computation | Medium | Low | The vault is markdown; files rarely exceed a few KB. If a user imports large extracted documents, hash computation is still fast (SHA-256 of <1MB). |
| **Race condition**: file modified between event and read | Low | Medium | If read fails (file deleted between event and read), emit `Deleted` instead. If content changed between event and hash, the next event catches it. |

### Architectural Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| **First daemon pattern** — no precedent for long-running processes | Medium | Establish clear conventions: PID file, signal handling, graceful shutdown, log rotation. Document the pattern for future use. |
| **Auto-sync token expiry** — OAuth token may expire during long watch | Medium | Refresh token before each push. `temper-client` already handles token refresh. Add retry-with-refresh to auto-sync layer. |
| **Manifest version drift** — daemon holds old manifest while CLI mutates on disk | Medium | File-level lock (advisory `flock`) on `manifest.json` for writes. Daemon periodically re-reads if lock was held by another process. |

---

## Alternatives Considered

### 1. Polling-only (Enhanced `rehash_manifest`)

**Approach:** Keep the current pull-based model. Optimize `rehash_manifest()` with mtime-based skip (I6b) and parallel hashing.

**Pros:** No new dependencies, no daemon process, no concurrency concerns.
**Cons:** Still O(n) on every sync/status invocation. No real-time events for MCP. Auto-sync requires wrapping every temper command with a pre-flight, which is slow and doesn't catch external edits.

**Verdict:** This is the I6b approach. It's necessary as a fallback but insufficient as a primary strategy for the real-time use cases.

### 2. `watchexec` Crate Instead of `notify`

**Approach:** Use [`watchexec`](https://crates.io/crates/watchexec) which wraps `notify` with higher-level filtering, debouncing, and process management.

**Pros:** Batteries-included filtering (gitignore support built-in), process supervision, proven in `cargo-watch`.
**Cons:** Much heavier dependency (pulls in `command-group`, `process-group`, `clearscreen`, etc.). Designed for running shell commands on file change, not for programmatic event streams. We don't need process supervision — we need a typed event stream.

**Verdict:** Overkill. `notify-debouncer-full` gives us exactly the abstraction level we need.

### 3. OS-specific Watcher (Direct `inotify`/`FSEvents`)

**Approach:** Use `inotify` crate on Linux, `fsevent` crate on macOS directly.

**Pros:** Maximum control, minimal dependencies per platform.
**Cons:** Significant per-platform code. No debouncing — we'd reimplement what `notify-debouncer-full` already provides. Maintenance burden across platforms.

**Verdict:** `notify` exists precisely to abstract this. No reason to go lower-level.

### 4. Git-based Change Detection

**Approach:** If the vault is a git repo, use `git status` / `git diff` to detect changes.

**Pros:** Leverages existing tooling, handles `.gitignore` natively.
**Cons:** Vault is not necessarily a git repo. Git status is a subprocess call (slow). No real-time events. Doesn't detect unsaved changes.

**Verdict:** Not applicable — the vault is a managed directory, not necessarily version-controlled.

---

## Open Questions

### Q1: Should `temper watch` be a foreground daemon or a background service?

**Options:**
- **Foreground** (recommended for v1): User runs `temper watch` in a terminal tab. Simple, visible, easy to debug. Ctrl+C to stop.
- **Background**: `temper watch --daemon` forks to background. More convenient but harder to debug, needs log file management, PID file lifecycle.
- **Launchd/systemd service**: Platform-native service management. Best UX but significant platform-specific setup.

**Recommendation:** Start with foreground-only. Add `--daemon` in a follow-up. Defer platform service integration to when there's user demand.

### Q2: Should the watcher auto-register new files?

When a new `.md` file with valid `temper-id` frontmatter appears in the vault (e.g., synced from another tool, or manually created), should the watcher automatically add it to the manifest?

**Options:**
- **No** (recommended): Emit `NewFile` event, let the user decide via `temper import` or next sync. Prevents accidental registration of scratch files.
- **Yes, if frontmatter matches a subscription**: Auto-register only if the file's context matches a configured subscription with `auto_sync: true`.

**Recommendation:** Start with no auto-registration. The `NewFile` event surfaces the information; the user retains control.

### Q3: Cross-process manifest coordination

When `temper watch` daemon is running and the user runs `temper sync run` in another terminal, how do they coordinate manifest access?

**Options:**
- **Advisory file lock** (`flock` on `.temper/manifest.lock`): Daemon acquires shared (read) or exclusive (write) lock. CLI sync acquires exclusive lock, blocking the daemon's writes during sync.
- **Unix domain socket**: Daemon listens on `.temper/watch.sock`. CLI sync sends "pause" command, runs sync, sends "resume". More complex but enables richer communication.
- **Manifest generation counter**: Both processes read a monotonic counter. If counter changed since last read, reload from disk before writing. Optimistic concurrency.

**Recommendation:** Start with advisory `flock`. It's simple, well-understood, and sufficient. Upgrade to UDS if inter-process communication needs grow (e.g., MCP server coordinating with daemon).

### Q4: What notify version to target?

- **v8.2.0**: Stable, production-proven. No `EventKindMask` (filter in userspace). Async requires manual channel bridging.
- **v9.0.0-rc.2**: Has `EventKindMask::CORE` for kernel-level filtering. Native `tokio` feature. But still RC.

**Recommendation:** Target v8.2.0 for initial implementation. Structure the code so upgrading to v9 is a Cargo.toml change + minor API adjustments (adding `EventKindMask` to config, switching to native tokio channel).

### Q5: Should the watcher monitor config file changes?

If `~/.config/temper/config.toml` changes (e.g., user adds a context subscription), should the watcher detect this and reconfigure?

**Recommendation:** Not in v1. Require `temper watch` restart for config changes. Emit a warning if config file mtime changes.

---

## Decision Log

| # | Decision | Choice | Rationale |
|---|----------|--------|-----------|
| D1 | Watcher crate | `notify-debouncer-full` v8.2.0 | Production-proven, rename stitching, editor-agnostic event normalization |
| D2 | Crate placement | New `temper-watch` crate | Isolates heavy dependency, enables reuse by MCP |
| D3 | Concurrency model | `Arc<RwLock<Manifest>>` + `tokio::sync::mpsc` | Standard Rust async patterns, minimal complexity |
| D4 | Event model | `VaultEvent` enum (Modified/NewFile/Deleted/Renamed/Error) | Maps to manifest state transitions; higher-level than raw fs events |
| D5 | Self-notification prevention | Filter `.temper/` directory in EventFilter | Simple, reliable, no special coordination needed |
| D6 | Auto-sync scope | Separate layer on top of watcher, not embedded in watcher | Watcher is pure local observation; sync policy is configurable |
| D7 | Fallback strategy | Auto-detect PollWatcher failure, fall back gracefully | `RecommendedWatcher::kind()` for detection, clear user messaging |
| D8 | Manifest persistence | Periodic flush (5s default) + flush-on-shutdown | Avoids disk thrashing on rapid edits; no data loss on clean exit |
| D9 | New file handling | Emit event, don't auto-register | User retains control over what enters the manifest |
| D10 | CLI command model | Foreground `temper watch` (v1), daemon mode deferred | Simple to implement, debug, and kill. Daemon adds complexity. |
| D11 | Cross-process locking | Advisory `flock` on `.temper/manifest.lock` | Well-understood POSIX pattern, sufficient for watcher+CLI coordination |
| D12 | Reverse path lookup | `HashMap<PathBuf, Uuid>` index on Manifest | O(1) lookups for filesystem events; rebuilt on manifest load |

---

## Appendix A: notify-debouncer-full Integration Pattern

Reference implementation for setting up the debouncer with temper's async architecture:

```rust
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{new_debouncer_opt, RecommendedCache, DebounceEventResult};
use tokio::sync::{mpsc, RwLock};

use temper_core::types::manifest::Manifest;

pub fn start_watcher(
    vault_root: &Path,
    manifest: Arc<RwLock<Manifest>>,
    config: &WatcherConfig,
) -> Result<(
    // The debouncer — must be held alive for watching to continue
    notify_debouncer_full::Debouncer<RecommendedWatcher, RecommendedCache>,
    // Receiver for processed VaultEvents
    mpsc::Receiver<VaultEvent>,
)> {
    let (vault_tx, vault_rx) = mpsc::channel::<VaultEvent>(256);
    let vault_root_owned = vault_root.to_path_buf();
    let manifest_clone = manifest.clone();
    let ignore_patterns = compile_ignore_patterns(&config.ignore_patterns);

    // Bridge: notify's sync callback → tokio mpsc channel
    let (notify_tx, notify_rx) = std::sync::mpsc::channel::<DebounceEventResult>();

    let notify_config = notify::Config::default();

    let debounce_timeout = Duration::from_millis(config.debounce_ms);

    let mut debouncer = new_debouncer_opt::<_, RecommendedWatcher, RecommendedCache>(
        debounce_timeout,
        None, // tick rate — auto
        notify_tx,
        RecommendedCache::new(),
        notify_config,
    )?;

    debouncer.watch(vault_root, RecursiveMode::Recursive)?;

    // Spawn processing task: reads from notify's sync channel,
    // filters, translates to VaultEvents, updates manifest
    tokio::spawn(async move {
        // Process in a blocking thread since notify_rx is std::sync::mpsc
        let handle = tokio::task::spawn_blocking(move || {
            for result in notify_rx {
                match result {
                    Ok(events) => {
                        for event in events {
                            // Stage 2: Filter
                            let dominated_paths: Vec<_> = event.paths.iter()
                                .filter(|p| should_process(p, &vault_root_owned, &ignore_patterns))
                                .cloned()
                                .collect();

                            if dominated_paths.is_empty() {
                                continue;
                            }

                            // Stage 3: Translate to VaultEvent
                            // (simplified — full implementation in updater.rs)
                            // Uses manifest_clone for lookups
                        }
                    }
                    Err(errors) => {
                        for error in errors {
                            let _ = vault_tx.blocking_send(VaultEvent::Error {
                                message: format!("{:?}", error),
                                path: None,
                            });
                        }
                    }
                }
            }
        });
        let _ = handle.await;
    });

    Ok((debouncer, vault_rx))
}
```

## Appendix B: Manifest Path Index Extension

```rust
// Addition to temper-core/src/types/manifest.rs

use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

impl Manifest {
    /// Rebuild the path → resource_id reverse index.
    /// Called after loading from disk or after bulk mutations.
    pub fn rebuild_path_index(&mut self) {
        self.path_index = self.entries.iter()
            .map(|(id, entry)| (PathBuf::from(&entry.path), *id))
            .collect();
    }

    /// Look up a resource_id by its vault file path.
    /// O(1) when path_index is current.
    pub fn lookup_by_path(&self, path: &Path) -> Option<Uuid> {
        self.path_index.get(path).copied()
    }

    /// Update a manifest entry and keep the path index in sync.
    pub fn update_entry_path(&mut self, resource_id: Uuid, new_path: PathBuf) {
        if let Some(entry) = self.entries.get_mut(&resource_id) {
            let old_path = PathBuf::from(&entry.path);
            self.path_index.remove(&old_path);
            entry.path = new_path.to_string_lossy().to_string();
            self.path_index.insert(new_path, resource_id);
        }
    }
}
```

## Appendix C: Related Tickets & Dependencies

| Ticket | Relationship | Status |
|--------|-------------|--------|
| **I6a** — Sync Infrastructure | Foundation — provides manifest types, sync orchestration, `rehash_manifest()` | ✅ Done |
| **I6b** — Auto-Merge & Workflow Integration | Parallel — `--auto-sync` on workflow commands. Watcher provides the "external edit" auto-sync path that I6b doesn't cover. | Backlog |
| **I6c** — Team Sync & Manual Resolution | Future consumer — team sync benefits from real-time local state tracking. | Backlog |
| **I5e** — Local KB Restructure | Foundation — vault directory layout, manifest initialization. | ✅ Done |
| **I5c** — Two-Tier Resource Model | Foundation — only `imported` resources are in the manifest and thus watchable. | ✅ Done |
| **MCP** — Agent workflow server | Future consumer — notify agents of vault changes. | Backlog |

### New Tickets to Create

| Ticket | Scope | Phase |
|--------|-------|-------|
| **I7a** — Vault Watcher Core Library | `temper-watch` crate, `VaultWatcher`, EventFilter, ManifestUpdater, tests | Phase 1 |
| **I7b** — `temper watch` CLI Command | Foreground daemon, PID file, signal handling, auto-sync layer | Phase 2 |
| **I7c** — Watcher Integration & Polish | Sync optimization, MCP integration, health checks, docs | Phase 3 |