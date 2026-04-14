# `temper graph build` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `temper graph build` — a deterministic, local, additive CLI command that walks the vault, scans markdown bodies for explicit references (markdown links, wikilinks, bare UUIDs), and writes resolved references back into each file's `open_meta.references` using `pulldown-cmark` for precise body parsing. Plus a small server-side companion change so `managed_meta.temper_goal` emits a `ParentOf` edge during `reconcile_edges`.

**Architecture:** New `graph` clap subcommand group in `temper-cli` with `build` as the only subcommand today. Pipeline is a three-pass `actions/graph_build.rs` module: (1) vault walk builds per-owner, per-context slug/UUID maps via `Vault::doc_type_dir` + `Frontmatter::parse_file`, (2) body scan via `pulldown-cmark` event stream (links, text, code exclusion) resolves same-owner references, (3) merge + write-back via `Frontmatter::value_mut()` and canonical `serialize()`. Owner boundary is enforced by map partitioning — the resolution function can only look inside the scanning file's owner scope. Server-side companion renames `edge_service::extract_declarations_from_open_meta` to `extract_declarations_from_resource` with a broadened signature that also checks `managed_meta.temper_goal` on tasks.

**Tech Stack:**
- Rust workspace (`crates/temper-cli`, `crates/temper-api`, `crates/temper-core`)
- `clap` 4 for subcommand group
- `pulldown-cmark` (new dependency, temper-cli only) for markdown parsing
- `regex` (already in temper-cli) for wikilink/UUID scanning inside `Event::Text`
- `Frontmatter` aggregate type from `temper-core::frontmatter::document`
- `Vault` helper from `temper-core::vault`
- `serde_yaml::Value` for frontmatter mutation
- `cargo-nextest` + TDD pattern as established in Session 3 of frontmatter consolidation

**Spec:** `docs/superpowers/specs/2026-04-14-temper-graph-build-refined-design.md`

---

## File Structure

### New files

| File | Responsibility |
|------|----------------|
| `crates/temper-cli/src/commands/graph.rs` | Clap `graph` subcommand group dispatch; thin wrapper that unpacks flags and calls `actions::graph_build::run` |
| `crates/temper-cli/src/actions/graph_build.rs` | Three-pass pipeline: vault walk (`discover_vault`), body scanning (`scan_body_for_refs`), merge + write-back (`merge_and_write`). All public API is `run(config, params) -> Result<GraphBuildReport>`. |
| `crates/temper-cli/tests/graph_build_test.rs` | Integration test driving the end-to-end pipeline against a fixture vault directory created in a `tempfile::TempDir`. |
| `crates/temper-e2e/tests/graph_build_e2e_test.rs` | E2E test gated on `test-db`: seed vault, run `graph build` (via action call), run sync, verify `kb_resource_edges` contents including `temper-goal → ParentOf` from the server-side companion. |

### Modified files

| File | Change |
|------|--------|
| `crates/temper-cli/Cargo.toml` | Add `pulldown-cmark = "0.10"` dependency (verified not present) |
| `crates/temper-cli/src/cli.rs` | Add `Commands::Graph { action: GraphAction }` variant and `GraphAction::Build { context, dry_run, verbose }` enum |
| `crates/temper-cli/src/commands/mod.rs` | Add `pub mod graph;` |
| `crates/temper-cli/src/actions/mod.rs` | Add `pub mod graph_build;` |
| `crates/temper-cli/src/main.rs` | Add `Commands::Graph { action }` match arm routing to `commands::graph::run` |
| `crates/temper-api/src/services/edge_service.rs` | Rename `extract_declarations_from_open_meta` → `extract_declarations_from_resource`, broaden signature to take `(doc_type, managed_meta, open_meta)`, add `temper-goal → ParentOf` extraction for tasks. Update internal callers (`reconcile_edges`, `extract_and_upsert_edges`). Update 7 in-file tests. |
| `crates/temper-api/src/services/meta_service.rs` | Update 1 call site at `line 246` to pass `doc_type` and `managed_meta` to `reconcile_edges` |
| `crates/temper-api/src/services/ingest_service.rs` | Update 2 call sites at `line 464` and `line 699` to pass `doc_type` and `managed_meta` |
| `crates/temper-api/tests/edge_ingest_test.rs` | Update 8 test call sites for new signature |

### Structure notes

- `ENTITY_DOC_TYPES` is duplicated in several places today (see Session 3 plan's deferred follow-up). This plan uses a local `const ENTITY_DOC_TYPES` in `graph_build.rs` matching `doctor.rs:31`. Extracting a shared helper is a separate task, not in scope here.
- `graph_build.rs` is self-contained: no other action module imports from it. Keeps the blast radius zero for anything that breaks.

---

## Phase A — CLI scaffold (no logic)

Establish the command shape first so every subsequent task has a place to hook into. At end of phase, `temper graph build --help` works and errors out cleanly with "not yet implemented" when run.

### Task A1: Add `pulldown-cmark` dependency

**Files:**
- Modify: `crates/temper-cli/Cargo.toml:14-43`

- [ ] **Step 1: Add dependency line**

Edit `crates/temper-cli/Cargo.toml` and add the following line in alphabetical position after `indicatif`:

```toml
pulldown-cmark = { version = "0.10", default-features = false }
```

(The `default-features = false` disables unused command-line formatting features; we only need the parser.)

- [ ] **Step 2: Verify it resolves**

Run: `cargo check -p temper-cli`
Expected: Clean compile with zero new warnings. The crate downloads and links.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-cli/Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
feat(deps): add pulldown-cmark to temper-cli

Needed by temper graph build Pass 2 body scanning to correctly
exclude code blocks and extract markdown link destinations without
hand-rolled regex fragility.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task A2: Define `GraphAction` clap subcommand enum

**Files:**
- Modify: `crates/temper-cli/src/cli.rs:19` (add to `Commands`), `crates/temper-cli/src/cli.rs:456` (append new enum at end)

- [ ] **Step 1: Add `Commands::Graph` variant**

In `crates/temper-cli/src/cli.rs`, add this variant to the `Commands` enum immediately after `Search { .. }` (the current last variant, around line 178):

```rust
/// Build, inspect, or manage the knowledge graph from vault frontmatter
Graph {
    #[command(subcommand)]
    action: GraphAction,
},
```

- [ ] **Step 2: Add `GraphAction` enum at end of file**

At the bottom of `crates/temper-cli/src/cli.rs` (after `DoctorAction` around line 456), append:

```rust
#[derive(Subcommand)]
pub enum GraphAction {
    /// Seed the vault with graph relationships discovered from markdown bodies
    Build {
        /// Scope to a single context (default: all contexts)
        #[arg(long)]
        context: Option<String>,
        /// Preview changes without writing files
        #[arg(long)]
        dry_run: bool,
        /// Include per-file edge detail in the report
        #[arg(short, long)]
        verbose: bool,
    },
}
```

- [ ] **Step 3: Export from lib**

`GraphAction` needs to be importable from `main.rs`. Verify `crates/temper-cli/src/lib.rs` re-exports `cli::*` (the existing `ResourceAction`, `SyncAction`, etc. pattern). If it uses named exports, add `GraphAction` to the list.

Run: `grep -n "pub use.*cli" crates/temper-cli/src/lib.rs` — inspect the export pattern and match it.

- [ ] **Step 4: Verify compile**

Run: `cargo check -p temper-cli`
Expected: Fails with an error like `non-exhaustive patterns: &Commands::Graph { .. } not covered` in `main.rs`. This confirms the enum variant is wired in and `main.rs` now requires a match arm.

- [ ] **Step 5: Do NOT commit yet** — the match arm lands in Task A3.

### Task A3: Add `commands/graph.rs` skeleton and wire dispatch

**Files:**
- Create: `crates/temper-cli/src/commands/graph.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs:21`
- Modify: `crates/temper-cli/src/main.rs:385` (add match arm before the closing brace of `run`)

- [ ] **Step 1: Create `commands/graph.rs` skeleton**

```rust
//! `temper graph` command dispatch.
//!
//! Thin wrapper over `actions::graph_build`. Unpacks clap flags, loads
//! the vault config, and delegates to the action.

use crate::cli::GraphAction;
use crate::config::Config;
use crate::error::Result;

pub fn run(config: &Config, action: GraphAction) -> Result<()> {
    match action {
        GraphAction::Build {
            context,
            dry_run,
            verbose,
        } => {
            let _ = (config, context, dry_run, verbose);
            Err(crate::error::TemperError::Project(
                "temper graph build: not yet implemented".into(),
            ))
        }
    }
}
```

- [ ] **Step 2: Register the module**

Add `pub mod graph;` to `crates/temper-cli/src/commands/mod.rs` in alphabetical position (between `goal` and `init`, around line 8):

```rust
pub mod goal;
pub mod graph;
pub mod init;
```

- [ ] **Step 3: Add match arm to `main.rs`**

In `crates/temper-cli/src/main.rs`, in the `run(cli: Cli)` match block, add this arm before the closing brace of the match (immediately after `Commands::Search { .. } => { ... }`, around line 385):

```rust
Commands::Graph { action } => {
    let config = temper_cli::config::load(cli.vault.as_deref())?;
    temper_cli::commands::graph::run(&config, action)
}
```

Also add `GraphAction` to the use statement at top of `main.rs` (line 3-5):

```rust
use temper_cli::cli::{
    AuthAction, Cli, Commands, ConfigAction, ContextAction, DoctorAction, GraphAction,
    ResourceAction, SkillAction, SyncAction, TeamAction,
};
```

Note: even though `main.rs` doesn't name `GraphAction` directly inside the arm (it unpacks via `Graph { action }`), having it exported keeps future handwritten dispatch consistent with the other subcommands.

Actually — re-examining: the existing pattern for `Doctor { action, .. }` in `main.rs:253-268` does NOT need `DoctorAction` in the use list because it matches `Some(DoctorAction::Fix { .. })` inline. The other groups (`SyncAction`, `TeamAction`, etc.) DO appear in the use list because they're named in outer match patterns. Our `Commands::Graph { action }` pattern doesn't need `GraphAction` at match time — the inner match happens inside `commands::graph::run`. So the use-list addition is not strictly required for compilation. Add it anyway for consistency and to keep the dispatch layer's import surface visible.

- [ ] **Step 4: Verify compile**

Run: `cargo check -p temper-cli`
Expected: Clean compile, zero warnings.

- [ ] **Step 5: Verify clap surface**

Run: `cargo run -p temper-cli --bin temper -- graph --help`
Expected: output includes `Usage: temper graph <COMMAND>` and lists `build`.

Run: `cargo run -p temper-cli --bin temper -- graph build --help`
Expected: output lists `--context`, `--dry-run`, `-v/--verbose`.

Run: `cargo run -p temper-cli --bin temper -- graph build`
Expected: exits with `temper: temper graph build: not yet implemented` and exit code 1.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/mod.rs crates/temper-cli/src/commands/graph.rs crates/temper-cli/src/main.rs
git commit -m "$(cat <<'EOF'
feat(cli): add graph subcommand group skeleton

Empty scaffold for temper graph build; dispatch returns
"not yet implemented" until the action module lands.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase B — Pass 1: Vault walk + slug/UUID maps

Build the deterministic per-owner, per-context map construction from a vault walk. No body scanning yet. This phase is 100% table-driven TDD.

### Task B1: Create `actions/graph_build.rs` with types and empty `run`

**Files:**
- Create: `crates/temper-cli/src/actions/graph_build.rs`
- Modify: `crates/temper-cli/src/actions/mod.rs`

- [ ] **Step 1: Register the module**

Append `pub mod graph_build;` to `crates/temper-cli/src/actions/mod.rs` in alphabetical position (between `goal` and `ingest`, around line 4):

```rust
pub mod goal;
pub mod graph_build;
pub mod ingest;
```

- [ ] **Step 2: Create `graph_build.rs` with public types**

```rust
//! `temper graph build` pipeline implementation.
//!
//! Three-pass additive seeder that walks the vault, scans markdown
//! bodies for explicit references (markdown links, wikilinks, bare
//! UUIDs), resolves them within-owner, and writes the resolved set
//! back into each file's `open_meta.references`.
//!
//! Owner boundaries are enforced by map partitioning: every resolution
//! map is keyed by owner, and a scanning file can only look inside
//! the map for its own owner. Cross-owner references are structurally
//! impossible.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use uuid::Uuid;

use crate::config::Config;
use crate::error::Result;

/// Doc types that live at `{vault}/{owner}/{context}/{doc_type}/`.
/// Matches `actions::doctor::ENTITY_DOC_TYPES`.
const ENTITY_DOC_TYPES: &[&str] = &["task", "goal", "session", "decision", "concept", "research"];

/// Parameters for a graph build run.
#[derive(Debug, Clone)]
pub struct GraphBuildParams {
    /// Optional single-context filter. None means all configured contexts.
    pub context_filter: Option<String>,
    /// If true, do not write any files; report what would change.
    pub dry_run: bool,
    /// If true, emit per-file detail in the report.
    pub verbose: bool,
}

/// Final report from a graph build run.
#[derive(Debug, Default, Clone)]
pub struct GraphBuildReport {
    /// Total files encountered during the vault walk.
    pub files_walked: usize,
    /// Number of resolved references discovered in Pass 2.
    pub references_found: usize,
    /// Number of files whose `open_meta.references` grew.
    pub files_modified: usize,
    /// Total new reference entries added across all files.
    pub references_added: usize,
    /// Entries already present in target files (no-op adds).
    pub already_present: usize,
    /// Per-file change details, sorted by vault-relative path.
    pub modified_files: Vec<ModifiedFile>,
}

/// Per-file change record for the report.
#[derive(Debug, Clone)]
pub struct ModifiedFile {
    /// Vault-relative path, e.g. `@me/temper/task/foo.md`.
    pub rel_path: String,
    /// Number of new references added to this file.
    pub added: usize,
    /// The actual ref strings added (populated only when verbose=true).
    pub added_refs: Vec<String>,
}

/// Owner sigil-prefixed identifier, e.g. "@me" or "+platform-eng".
pub type Owner = String;
/// Context name, e.g. "temper", "tasker".
pub type Context = String;

/// Slug resolution maps, partitioned by owner AND context for
/// same-context-first resolution. A slug lookup walks "same context
/// first, then cross-context if unique" — never crossing the owner
/// boundary.
#[derive(Debug, Default)]
pub(crate) struct SlugMap {
    /// owner → context → slug → absolute file path
    inner: HashMap<Owner, HashMap<Context, HashMap<String, PathBuf>>>,
}

/// UUID resolution map, partitioned by owner only (UUIDs are globally
/// unique within the vault and do not need context partitioning).
#[derive(Debug, Default)]
pub(crate) struct UuidMap {
    inner: HashMap<Owner, HashMap<Uuid, PathBuf>>,
}

/// Top-level entry point. Walks the vault, scans bodies, merges
/// references into open_meta, writes files back.
pub fn run(config: &Config, params: GraphBuildParams) -> Result<GraphBuildReport> {
    let _ = (config, params);
    Err(crate::error::TemperError::Project(
        "graph_build::run: not yet implemented".into(),
    ))
}
```

- [ ] **Step 3: Verify compile**

Run: `cargo check -p temper-cli`
Expected: Clean compile. Note that `SlugMap`/`UuidMap` and their inner maps produce "dead code" warnings until we implement insertion methods — that's expected and will clear in Task B2.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/actions/graph_build.rs crates/temper-cli/src/actions/mod.rs
git commit -m "$(cat <<'EOF'
feat(graph-build): scaffold actions::graph_build module

Types for params, report, and owner-partitioned slug/UUID maps.
Empty run() returns not-yet-implemented. Pipeline passes land next.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task B2: `SlugMap` insert and resolve (same-context-first)

**Files:**
- Modify: `crates/temper-cli/src/actions/graph_build.rs` — add methods to `SlugMap`
- Add tests: `#[cfg(test)] mod tests` at end of same file

- [ ] **Step 1: Write failing tests**

Append to the bottom of `graph_build.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn path(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn slug_map_resolves_same_context_first() {
        let mut map = SlugMap::default();
        map.insert("@me", "temper", "foo", path("/vault/@me/temper/task/foo.md"));
        map.insert(
            "@me",
            "tasker",
            "foo",
            path("/vault/@me/tasker/task/foo.md"),
        );

        // Scanning file is in "temper" context; "foo" should resolve to temper's foo.
        let resolved = map.resolve("@me", "temper", "foo");
        assert_eq!(
            resolved,
            Some(path("/vault/@me/temper/task/foo.md").as_path())
        );
    }

    #[test]
    fn slug_map_falls_back_cross_context_when_unique() {
        let mut map = SlugMap::default();
        map.insert(
            "@me",
            "tasker",
            "only-there",
            path("/vault/@me/tasker/task/only-there.md"),
        );

        // Scanning from "temper"; slug doesn't exist there but unique cross-context.
        let resolved = map.resolve("@me", "temper", "only-there");
        assert_eq!(
            resolved,
            Some(path("/vault/@me/tasker/task/only-there.md").as_path())
        );
    }

    #[test]
    fn slug_map_skips_ambiguous_cross_context() {
        let mut map = SlugMap::default();
        map.insert(
            "@me",
            "tasker",
            "ambiguous",
            path("/vault/@me/tasker/task/ambiguous.md"),
        );
        map.insert(
            "@me",
            "general",
            "ambiguous",
            path("/vault/@me/general/task/ambiguous.md"),
        );

        // Scanning from "temper"; slug exists in two other contexts → skip.
        let resolved = map.resolve("@me", "temper", "ambiguous");
        assert_eq!(resolved, None);
    }

    #[test]
    fn slug_map_rejects_cross_owner() {
        let mut map = SlugMap::default();
        map.insert(
            "+team-x",
            "shared",
            "leaked",
            path("/vault/+team-x/shared/task/leaked.md"),
        );

        // Scanning from @me — must not resolve to +team-x even though
        // the slug is unique there.
        let resolved = map.resolve("@me", "temper", "leaked");
        assert_eq!(resolved, None);
    }

    #[test]
    fn slug_map_returns_none_for_unknown_slug() {
        let map = SlugMap::default();
        assert_eq!(map.resolve("@me", "temper", "nonexistent"), None);
    }
}
```

- [ ] **Step 2: Run tests — expected fail**

Run: `cargo nextest run -p temper-cli slug_map_`
Expected: FAIL with "no method named `insert`/`resolve` found for struct `SlugMap`".

- [ ] **Step 3: Implement `SlugMap::insert` and `SlugMap::resolve`**

Add these methods to `SlugMap` (just above the `#[cfg(test)]` block):

```rust
impl SlugMap {
    /// Register a file at `(owner, context, slug)`.
    pub(crate) fn insert(&mut self, owner: &str, context: &str, slug: &str, path: PathBuf) {
        self.inner
            .entry(owner.to_string())
            .or_default()
            .entry(context.to_string())
            .or_default()
            .insert(slug.to_string(), path);
    }

    /// Resolve a slug for a scanning file.
    ///
    /// - Same-owner same-context: direct match wins.
    /// - Same-owner cross-context: falls back ONLY if exactly one
    ///   other context in the owner contains the slug. Ambiguous
    ///   matches return `None` with a debug trace.
    /// - Cross-owner: never resolves.
    pub(crate) fn resolve(
        &self,
        scanning_owner: &str,
        scanning_context: &str,
        slug: &str,
    ) -> Option<&std::path::Path> {
        let owner_map = self.inner.get(scanning_owner)?;

        // 1. Same-context first
        if let Some(ctx_map) = owner_map.get(scanning_context) {
            if let Some(path) = ctx_map.get(slug) {
                return Some(path.as_path());
            }
        }

        // 2. Cross-context fallback — only if exactly one match exists
        let matches: Vec<&std::path::Path> = owner_map
            .iter()
            .filter(|(ctx, _)| ctx.as_str() != scanning_context)
            .filter_map(|(_, ctx_map)| ctx_map.get(slug))
            .map(|p| p.as_path())
            .collect();

        match matches.len() {
            0 => None,
            1 => Some(matches[0]),
            n => {
                tracing::debug!(
                    owner = %scanning_owner,
                    slug = %slug,
                    n_matches = n,
                    "ambiguous cross-context slug — skipping"
                );
                None
            }
        }
    }
}
```

- [ ] **Step 4: Run tests — expected pass**

Run: `cargo nextest run -p temper-cli slug_map_`
Expected: PASS 5 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/actions/graph_build.rs
git commit -m "$(cat <<'EOF'
feat(graph-build): SlugMap with owner-partitioned resolution

Implements same-context-first resolution that mirrors server-side
edge_service::resolve_target, plus ambiguous-cross-context rejection
and the hard owner boundary that cannot be crossed by construction.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task B3: `UuidMap` insert and resolve (owner-partitioned, no context fallback)

**Files:**
- Modify: `crates/temper-cli/src/actions/graph_build.rs`

- [ ] **Step 1: Write failing tests**

Append inside the existing `#[cfg(test)] mod tests` block:

```rust
    fn uuid(s: &str) -> Uuid {
        Uuid::parse_str(s).unwrap()
    }

    #[test]
    fn uuid_map_resolves_within_owner() {
        let mut map = UuidMap::default();
        let id = uuid("019d1d24-2000-7379-8f26-ae4ae87bc5c6");
        map.insert("@me", id, path("/vault/@me/temper/task/foo.md"));

        let resolved = map.resolve("@me", id);
        assert_eq!(
            resolved,
            Some(path("/vault/@me/temper/task/foo.md").as_path())
        );
    }

    #[test]
    fn uuid_map_rejects_cross_owner() {
        let mut map = UuidMap::default();
        let id = uuid("019d1d24-2000-7379-8f26-ae4ae87bc5c6");
        map.insert("+team-x", id, path("/vault/+team-x/shared/task/leaked.md"));

        let resolved = map.resolve("@me", id);
        assert_eq!(resolved, None);
    }

    #[test]
    fn uuid_map_returns_none_for_unknown_uuid() {
        let map = UuidMap::default();
        let id = uuid("019d1d24-2000-7379-8f26-ae4ae87bc5c6");
        assert_eq!(map.resolve("@me", id), None);
    }
```

- [ ] **Step 2: Run tests — expected fail**

Run: `cargo nextest run -p temper-cli uuid_map_`
Expected: FAIL with "no method named `insert`/`resolve` found for struct `UuidMap`".

- [ ] **Step 3: Implement**

Add below `impl SlugMap`:

```rust
impl UuidMap {
    pub(crate) fn insert(&mut self, owner: &str, id: Uuid, path: PathBuf) {
        self.inner
            .entry(owner.to_string())
            .or_default()
            .insert(id, path);
    }

    pub(crate) fn resolve(&self, scanning_owner: &str, id: Uuid) -> Option<&std::path::Path> {
        self.inner
            .get(scanning_owner)?
            .get(&id)
            .map(|p| p.as_path())
    }
}
```

- [ ] **Step 4: Run tests — expected pass**

Run: `cargo nextest run -p temper-cli uuid_map_`
Expected: PASS 3 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/actions/graph_build.rs
git commit -m "$(cat <<'EOF'
feat(graph-build): UuidMap with owner-partitioned resolution

Owner boundary enforced by construction — cross-owner UUID lookups
return None.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task B4: `discover_vault` — vault walk builds maps

**Files:**
- Modify: `crates/temper-cli/src/actions/graph_build.rs`

- [ ] **Step 1: Write the failing test**

Add to the tests block. This test builds a fixture vault in a `tempfile::TempDir`, walks it, and asserts maps are populated correctly.

```rust
    use std::fs;
    use tempfile::TempDir;

    /// Create a minimal vault structure under `tmp` and write a file
    /// with valid frontmatter. Returns the absolute file path.
    fn write_vault_file(
        tmp: &TempDir,
        owner: &str,
        context: &str,
        doc_type: &str,
        slug: &str,
        temper_id: Option<&str>,
        body: &str,
    ) -> PathBuf {
        let dir = tmp.path().join(owner).join(context).join(doc_type);
        fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join(format!("{slug}.md"));
        let id_line = temper_id
            .map(|id| format!("temper-id: {id}\n"))
            .unwrap_or_default();
        // Use a doc_type the Frontmatter type knows about; give it a
        // minimal valid set of fields.
        let content = format!(
            "---\n\
             temper-context: {context}\n\
             temper-type: {doc_type}\n\
             temper-owner: '{owner}'\n\
             {id_line}\
             title: {slug}\n\
             slug: {slug}\n\
             ---\n\
             {body}\n"
        );
        fs::write(&file_path, content).unwrap();
        file_path
    }

    /// Build a minimal Config pointing at a TempDir vault.
    fn fixture_config(tmp: &TempDir, contexts: &[&str]) -> Config {
        Config {
            vault_root: tmp.path().to_path_buf(),
            state_dir: tmp.path().join(".temper"),
            contexts: contexts.iter().map(|s| s.to_string()).collect(),
            subscriptions: Vec::new(),
            skill_output: tmp.path().join(".skill"),
        }
    }

    #[test]
    fn discover_vault_builds_slug_and_uuid_maps() {
        let tmp = TempDir::new().unwrap();
        write_vault_file(
            &tmp,
            "@me",
            "temper",
            "task",
            "alpha",
            Some("019d1d24-2000-7379-8f26-ae4ae87bc5c6"),
            "body of alpha",
        );
        write_vault_file(
            &tmp,
            "@me",
            "tasker",
            "task",
            "beta",
            None,
            "body of beta",
        );
        let config = fixture_config(&tmp, &["temper", "tasker"]);

        let (slugs, uuids, files) = discover_vault(&config, None).unwrap();

        assert_eq!(files.len(), 2, "expected 2 files in walk");

        // Slug map: alpha in temper, beta in tasker, both under @me
        let alpha = slugs.resolve("@me", "temper", "alpha");
        assert!(alpha.is_some(), "alpha should resolve in same context");
        let beta = slugs.resolve("@me", "tasker", "beta");
        assert!(beta.is_some(), "beta should resolve in same context");

        // UUID map: alpha has temper-id, beta does not
        let alpha_uuid = uuid("019d1d24-2000-7379-8f26-ae4ae87bc5c6");
        assert!(uuids.resolve("@me", alpha_uuid).is_some());
    }

    #[test]
    fn discover_vault_skips_unparseable_files_silently() {
        let tmp = TempDir::new().unwrap();
        // One valid file
        write_vault_file(
            &tmp,
            "@me",
            "temper",
            "task",
            "good",
            None,
            "",
        );
        // One corrupt file — raw garbage that isn't a valid frontmatter
        let bad_dir = tmp.path().join("@me").join("temper").join("task");
        fs::write(bad_dir.join("bad.md"), "not a real frontmatter\nno yaml here\n").unwrap();

        let config = fixture_config(&tmp, &["temper"]);
        let result = discover_vault(&config, None);

        // Must not error on unparseable files
        assert!(result.is_ok(), "unparseable files should not fail the walk");
        let (slugs, _uuids, files) = result.unwrap();

        // Good file is in the walk result; bad file is skipped
        assert_eq!(files.len(), 1);
        assert!(slugs.resolve("@me", "temper", "good").is_some());
        assert!(slugs.resolve("@me", "temper", "bad").is_none());
    }

    #[test]
    fn discover_vault_respects_context_filter() {
        let tmp = TempDir::new().unwrap();
        write_vault_file(&tmp, "@me", "temper", "task", "in-temper", None, "");
        write_vault_file(&tmp, "@me", "tasker", "task", "in-tasker", None, "");
        let config = fixture_config(&tmp, &["temper", "tasker"]);

        let (_slugs, _uuids, files) = discover_vault(&config, Some("temper")).unwrap();

        assert_eq!(files.len(), 1, "context filter should restrict walk");
        assert!(files[0].path.ends_with("in-temper.md"));
    }
```

Also add to Cargo.toml `[dev-dependencies]` if tempfile isn't already available to test scope. Verify:

Run: `grep -n "tempfile" crates/temper-cli/Cargo.toml`
Expected: `tempfile = "3"` is already a production dep (verified earlier at line 37). It's available to tests through the normal dep graph — no `dev-dependencies` change needed.

- [ ] **Step 2: Define the `DiscoveredFile` type and `discover_vault` signature**

Add above the `run()` function (below the types block, above `impl SlugMap`):

```rust
/// A file captured by the vault walk. Keeps the parsed frontmatter
/// so Pass 2 doesn't re-read it.
pub(crate) struct DiscoveredFile {
    pub(crate) path: PathBuf,
    pub(crate) rel_path: String,
    pub(crate) owner: String,
    pub(crate) context: String,
    pub(crate) frontmatter: temper_core::frontmatter::Frontmatter,
}

/// Walk the vault and build per-owner slug/UUID resolution maps.
///
/// `context_filter` restricts which files appear in the returned
/// `DiscoveredFile` list (Pass 2 only scans filtered files), but the
/// maps always include all same-owner files across all contexts so
/// cross-context same-owner references can still resolve.
pub(crate) fn discover_vault(
    config: &Config,
    context_filter: Option<&str>,
) -> Result<(SlugMap, UuidMap, Vec<DiscoveredFile>)> {
    unimplemented!("Task B4")
}
```

- [ ] **Step 3: Run tests — expected fail**

Run: `cargo nextest run -p temper-cli discover_vault_`
Expected: FAIL at `unimplemented!` or earlier.

- [ ] **Step 4: Implement `discover_vault`**

Replace the `unimplemented!` body:

```rust
pub(crate) fn discover_vault(
    config: &Config,
    context_filter: Option<&str>,
) -> Result<(SlugMap, UuidMap, Vec<DiscoveredFile>)> {
    use std::fs;
    use temper_core::frontmatter::Frontmatter;
    use temper_core::vault::Vault;

    let mut slugs = SlugMap::default();
    let mut uuids = UuidMap::default();
    let mut filtered_files: Vec<DiscoveredFile> = Vec::new();

    let vault_layout = Vault::new(&config.vault_root);

    // Always walk every configured context to build the full maps;
    // context_filter only affects which files end up in the returned
    // DiscoveredFile list.
    for ctx in &config.contexts {
        let owner = config.owner_for_context(ctx);
        let include_in_scan = context_filter.map_or(true, |f| f == ctx);

        for doc_type in ENTITY_DOC_TYPES {
            let dir = vault_layout.doc_type_dir(&owner, ctx, doc_type);
            if !dir.is_dir() {
                continue;
            }

            let entries = match fs::read_dir(&dir) {
                Ok(r) => r,
                Err(e) => {
                    tracing::debug!(
                        dir = %dir.display(),
                        error = %e,
                        "could not read doc_type dir, skipping"
                    );
                    continue;
                }
            };

            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("md") {
                    continue;
                }

                // Parse frontmatter; silently skip files we can't parse.
                let frontmatter = match Frontmatter::parse_file(&path) {
                    Ok(fm) => fm,
                    Err(e) => {
                        tracing::debug!(
                            path = %path.display(),
                            error = %e,
                            "unparseable frontmatter, skipping"
                        );
                        continue;
                    }
                };

                let slug = match path.file_stem().and_then(|s| s.to_str()) {
                    Some(s) => s.to_string(),
                    None => continue,
                };

                // Populate the slug map (always, regardless of filter)
                slugs.insert(&owner, ctx, &slug, path.clone());

                // Populate UUID map if temper-id is present
                if let Some(id_str) = frontmatter
                    .value()
                    .get("temper-id")
                    .and_then(|v| v.as_str())
                {
                    if let Ok(id) = Uuid::parse_str(id_str) {
                        uuids.insert(&owner, id, path.clone());
                    }
                }

                // Add to filtered file list only if context matches
                if include_in_scan {
                    let rel_path = path
                        .strip_prefix(&config.vault_root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .to_string();

                    filtered_files.push(DiscoveredFile {
                        path: path.clone(),
                        rel_path,
                        owner: owner.clone(),
                        context: ctx.clone(),
                        frontmatter,
                    });
                }
            }
        }
    }

    // Deterministic ordering for reproducible reports
    filtered_files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    Ok((slugs, uuids, filtered_files))
}
```

- [ ] **Step 5: Run tests — expected pass**

Run: `cargo nextest run -p temper-cli discover_vault_`
Expected: PASS 3 tests.

- [ ] **Step 6: Sanity clippy**

Run: `cargo clippy -p temper-cli -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/actions/graph_build.rs
git commit -m "$(cat <<'EOF'
feat(graph-build): Pass 1 vault walk builds slug/UUID maps

Walks every doc_type directory in every configured context, parses
frontmatter via Frontmatter::parse_file, and populates per-owner
resolution maps. Unparseable files are silently skipped (additive-
not-validator). Context filter restricts the returned file list
but never the resolution maps.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase C — Pass 2: Body scanning with pulldown-cmark

Pure function: takes a markdown body string, returns a `Vec<RawRef>` where each entry is either a resolved wikilink/link/UUID candidate. Separate file-resolution from extraction so tests are cheap.

### Task C1: Define `RawRef` and `scan_body` signature with empty stub + failing test

**Files:**
- Modify: `crates/temper-cli/src/actions/graph_build.rs`

- [ ] **Step 1: Define the types**

Add near the top of `graph_build.rs` (below `DiscoveredFile`):

```rust
/// A raw reference candidate extracted from markdown body text.
/// Not yet resolved against any owner map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RawRef {
    /// A slug appearing in a wikilink `[[slug]]` (with any variants stripped).
    WikiSlug(String),
    /// A bare UUID appearing in body text.
    BareUuid(Uuid),
    /// A markdown link `[text](path)` pointing at a `.md` file.
    /// The path is the raw `dest_url` from pulldown-cmark.
    MarkdownLink(String),
}
```

- [ ] **Step 2: Write failing tests for `scan_body`**

Add to the tests block:

```rust
    // ── Pass 2: body scanning ───────────────────────────────────────

    #[test]
    fn scan_body_extracts_markdown_link() {
        let refs = scan_body("See [alpha](alpha.md) for details.");
        assert_eq!(refs, vec![RawRef::MarkdownLink("alpha.md".to_string())]);
    }

    #[test]
    fn scan_body_extracts_wikilink_bare() {
        let refs = scan_body("See [[alpha]] for details.");
        assert_eq!(refs, vec![RawRef::WikiSlug("alpha".to_string())]);
    }

    #[test]
    fn scan_body_extracts_wikilink_with_pipe_display() {
        let refs = scan_body("See [[alpha|Alpha Doc]] for details.");
        assert_eq!(refs, vec![RawRef::WikiSlug("alpha".to_string())]);
    }

    #[test]
    fn scan_body_extracts_wikilink_with_anchor() {
        let refs = scan_body("See [[alpha#section]] for details.");
        assert_eq!(refs, vec![RawRef::WikiSlug("alpha".to_string())]);
    }

    #[test]
    fn scan_body_extracts_wikilink_with_anchor_and_pipe() {
        let refs = scan_body("See [[alpha#section|display]] for details.");
        assert_eq!(refs, vec![RawRef::WikiSlug("alpha".to_string())]);
    }

    #[test]
    fn scan_body_extracts_wikilink_with_md_suffix() {
        let refs = scan_body("See [[alpha.md]] for details.");
        assert_eq!(refs, vec![RawRef::WikiSlug("alpha".to_string())]);
    }

    #[test]
    fn scan_body_extracts_bare_uuid() {
        let refs = scan_body("See 019d1d24-2000-7379-8f26-ae4ae87bc5c6 for details.");
        assert_eq!(
            refs,
            vec![RawRef::BareUuid(uuid("019d1d24-2000-7379-8f26-ae4ae87bc5c6"))]
        );
    }

    #[test]
    fn scan_body_rejects_external_urls() {
        let refs = scan_body("See [example](https://example.com).");
        assert!(refs.is_empty());
    }

    #[test]
    fn scan_body_rejects_mailto() {
        let refs = scan_body("See [contact](mailto:foo@bar.com).");
        assert!(refs.is_empty());
    }

    #[test]
    fn scan_body_rejects_intra_doc_anchors() {
        let refs = scan_body("See [jump](#section).");
        assert!(refs.is_empty());
    }

    #[test]
    fn scan_body_rejects_non_md_extensions() {
        let refs = scan_body("See [data](data.json).");
        assert!(refs.is_empty());
    }

    #[test]
    fn scan_body_rejects_extensionless_paths() {
        let refs = scan_body("See [bare](foo).");
        assert!(refs.is_empty());
    }

    #[test]
    fn scan_body_skips_code_blocks() {
        let body = "\
Regular text [[real-ref]].

```
Inside code [[fake-ref]] and `[[also-fake]]`.
```

Back to prose [[another-real]].
";
        let refs = scan_body(body);
        assert_eq!(
            refs,
            vec![
                RawRef::WikiSlug("real-ref".to_string()),
                RawRef::WikiSlug("another-real".to_string())
            ]
        );
    }

    #[test]
    fn scan_body_skips_inline_code() {
        let refs = scan_body("The token `[[not-a-ref]]` is inline code.");
        assert!(refs.is_empty());
    }

    #[test]
    fn scan_body_skips_indented_code_block() {
        let body = "Prose line.\n\n    [[fake-ref-in-indented-block]]\n\nMore prose.";
        let refs = scan_body(body);
        assert!(refs.is_empty());
    }

    #[test]
    fn scan_body_multiple_references_in_reading_order() {
        let body = "First [[alpha]], then [beta](beta.md), then [[gamma]].";
        let refs = scan_body(body);
        assert_eq!(
            refs,
            vec![
                RawRef::WikiSlug("alpha".to_string()),
                RawRef::MarkdownLink("beta.md".to_string()),
                RawRef::WikiSlug("gamma".to_string()),
            ]
        );
    }
```

- [ ] **Step 3: Add stub `scan_body`**

Add above `impl SlugMap`:

```rust
/// Scan a markdown body for raw reference candidates.
///
/// Walks the pulldown-cmark event stream and collects:
/// - `Event::Start(Tag::Link { dest_url, .. })` → `RawRef::MarkdownLink`
///   when `dest_url` ends in `.md` and is not an external URL
/// - Wikilinks `[[...]]` and bare UUIDs inside `Event::Text` events
///   (which are emitted only outside code contexts by pulldown-cmark)
///
/// Does NOT resolve candidates against any owner map — that's the
/// caller's job in Pass 3.
pub(crate) fn scan_body(body: &str) -> Vec<RawRef> {
    unimplemented!("Task C1")
}
```

- [ ] **Step 4: Run tests — expected fail**

Run: `cargo nextest run -p temper-cli scan_body_`
Expected: FAIL at `unimplemented!`.

- [ ] **Step 5: Implement `scan_body`**

Replace the body with:

```rust
pub(crate) fn scan_body(body: &str) -> Vec<RawRef> {
    use pulldown_cmark::{Event, LinkType, Parser, Tag};

    // Wikilink regex: `[[slug]]`, `[[slug|display]]`, `[[slug#section]]`,
    // `[[slug#section|display]]`, `[[slug.md]]`. Rejects folder/ prefixes
    // by disallowing `/` in the slug.
    static WIKILINK_RE: once_cell::sync::Lazy<regex::Regex> =
        once_cell::sync::Lazy::new(|| {
            regex::Regex::new(
                r"\[\[([^\]\|#/]+?)(?:\.md)?(?:#[^\]\|]*)?(?:\|[^\]]*)?\]\]",
            )
            .unwrap()
        });

    // UUID regex: 8-4-4-4-12 hex in standard form.
    static UUID_RE: once_cell::sync::Lazy<regex::Regex> =
        once_cell::sync::Lazy::new(|| {
            regex::Regex::new(
                r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b",
            )
            .unwrap()
        });

    let mut out: Vec<RawRef> = Vec::new();
    let parser = Parser::new(body);

    for event in parser {
        match event {
            Event::Start(Tag::Link(_link_type, dest_url, _title)) => {
                let url = dest_url.as_ref();
                if is_external_or_anchor(url) {
                    continue;
                }
                if url.ends_with(".md") {
                    out.push(RawRef::MarkdownLink(url.to_string()));
                }
                // Non-`.md` internal links are ignored: too ambiguous.
                let _ = LinkType::Inline; // silence unused import if LinkType matching not needed
            }
            Event::Text(text) => {
                // Scan text OUTSIDE code contexts (pulldown-cmark gives
                // us code spans as Event::Code and code blocks as
                // Start(Tag::CodeBlock) .. End, not as Text events).
                scan_text_for_wikilinks(&text, &WIKILINK_RE, &mut out);
                scan_text_for_uuids(&text, &UUID_RE, &mut out);
            }
            // Event::Code and Start(Tag::CodeBlock) events are explicitly
            // NOT scanned — that's the guarantee pulldown-cmark gives us
            // for free.
            _ => {}
        }
    }

    out
}

fn is_external_or_anchor(url: &str) -> bool {
    url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("mailto:")
        || url.starts_with('#')
}

fn scan_text_for_wikilinks(text: &str, re: &regex::Regex, out: &mut Vec<RawRef>) {
    for caps in re.captures_iter(text) {
        if let Some(m) = caps.get(1) {
            let slug = m.as_str().trim();
            if !slug.is_empty() {
                out.push(RawRef::WikiSlug(slug.to_string()));
            }
        }
    }
}

fn scan_text_for_uuids(text: &str, re: &regex::Regex, out: &mut Vec<RawRef>) {
    for m in re.find_iter(text) {
        if let Ok(id) = Uuid::parse_str(m.as_str()) {
            out.push(RawRef::BareUuid(id));
        }
    }
}
```

- [ ] **Step 6: Add `once_cell` dependency if not already present**

Run: `grep -n "once_cell" crates/temper-cli/Cargo.toml`

If not present, add to `[dependencies]` in alphabetical position:

```toml
once_cell = "1"
```

Run: `cargo check -p temper-cli`
Expected: clean compile.

- [ ] **Step 7: Run tests — expected pass**

Run: `cargo nextest run -p temper-cli scan_body_`
Expected: PASS 15 tests.

Pay special attention to `scan_body_skips_code_blocks` and `scan_body_skips_inline_code` — these validate the pulldown-cmark-provided guarantee is actually kicking in.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/actions/graph_build.rs crates/temper-cli/Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
feat(graph-build): Pass 2 body scanning via pulldown-cmark

Walks the markdown event stream extracting markdown link
destinations, wikilinks, and bare UUIDs. Code blocks and inline
code are skipped by construction (pulldown-cmark emits them as
distinct events, never as Event::Text). Rejects external URLs,
mailto links, anchor-only links, and non-.md paths.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase D — Pass 2 continued: resolving raw refs against owner maps

### Task D1: `resolve_ref` — takes a `RawRef` + scanning file context + maps, returns resolved target

**Files:**
- Modify: `crates/temper-cli/src/actions/graph_build.rs`

- [ ] **Step 1: Write failing tests**

Append to the tests block:

```rust
    // ── Pass 2: resolution ──────────────────────────────────────────

    fn build_test_maps() -> (SlugMap, UuidMap) {
        let mut slugs = SlugMap::default();
        let mut uuids = UuidMap::default();
        slugs.insert("@me", "temper", "alpha", path("/v/@me/temper/task/alpha.md"));
        slugs.insert("@me", "tasker", "beta", path("/v/@me/tasker/task/beta.md"));
        uuids.insert(
            "@me",
            uuid("019d1d24-2000-7379-8f26-ae4ae87bc5c6"),
            path("/v/@me/temper/task/alpha.md"),
        );
        slugs.insert("+team-x", "shared", "leaked", path("/v/+team-x/shared/task/leaked.md"));
        (slugs, uuids)
    }

    #[test]
    fn resolve_ref_wikislug_same_owner_same_context() {
        let (slugs, uuids) = build_test_maps();
        let resolved = resolve_ref(
            &RawRef::WikiSlug("alpha".to_string()),
            "@me",
            "temper",
            Path::new("/v/@me/temper/task/other.md"),
            &slugs,
            &uuids,
        );
        assert_eq!(resolved, Some("alpha".to_string()));
    }

    #[test]
    fn resolve_ref_wikislug_cross_owner_rejected() {
        let (slugs, uuids) = build_test_maps();
        let resolved = resolve_ref(
            &RawRef::WikiSlug("leaked".to_string()),
            "@me",
            "temper",
            Path::new("/v/@me/temper/task/other.md"),
            &slugs,
            &uuids,
        );
        assert_eq!(resolved, None, "cross-owner must not resolve");
    }

    #[test]
    fn resolve_ref_bare_uuid_same_owner() {
        let (slugs, uuids) = build_test_maps();
        let resolved = resolve_ref(
            &RawRef::BareUuid(uuid("019d1d24-2000-7379-8f26-ae4ae87bc5c6")),
            "@me",
            "temper",
            Path::new("/v/@me/temper/task/other.md"),
            &slugs,
            &uuids,
        );
        assert_eq!(
            resolved,
            Some("019d1d24-2000-7379-8f26-ae4ae87bc5c6".to_string())
        );
    }

    #[test]
    fn resolve_ref_markdown_link_relative_md() {
        let (slugs, uuids) = build_test_maps();
        // Scanning from a file in /v/@me/temper/task/, linking to ./alpha.md
        let resolved = resolve_ref(
            &RawRef::MarkdownLink("alpha.md".to_string()),
            "@me",
            "temper",
            Path::new("/v/@me/temper/task/other.md"),
            &slugs,
            &uuids,
        );
        assert_eq!(resolved, Some("alpha".to_string()));
    }

    #[test]
    fn resolve_ref_markdown_link_unresolvable_returns_none() {
        let (slugs, uuids) = build_test_maps();
        let resolved = resolve_ref(
            &RawRef::MarkdownLink("nonexistent.md".to_string()),
            "@me",
            "temper",
            Path::new("/v/@me/temper/task/other.md"),
            &slugs,
            &uuids,
        );
        assert_eq!(resolved, None);
    }

    #[test]
    fn resolve_ref_self_reference_returns_none() {
        let (slugs, uuids) = build_test_maps();
        // Scanning file IS alpha.md; a wikilink to [[alpha]] from inside
        // alpha would create a self-edge, which is meaningless.
        let resolved = resolve_ref(
            &RawRef::WikiSlug("alpha".to_string()),
            "@me",
            "temper",
            Path::new("/v/@me/temper/task/alpha.md"),
            &slugs,
            &uuids,
        );
        assert_eq!(resolved, None, "self-reference should be rejected");
    }
```

- [ ] **Step 2: Define `resolve_ref` stub**

Add:

```rust
/// Resolve a raw reference candidate against the owner-partitioned
/// maps. Returns the canonical string form to store in
/// `open_meta.references` — either a slug or a UUID string.
///
/// - `scanning_file` is the absolute path of the file whose body is
///   being scanned; used to reject self-references and to resolve
///   relative markdown links.
/// - Wikilinks resolve via `SlugMap::resolve` with same-context-first.
/// - Markdown links resolve the `dest_url` relative to the scanning
///   file's parent directory, then look up the resulting stem in
///   `SlugMap`.
/// - Bare UUIDs resolve via `UuidMap::resolve` with owner scoping.
///
/// Self-edges (resolution → scanning_file itself) are rejected: they
/// would produce a source == target edge which `edge_service` already
/// rejects server-side.
pub(crate) fn resolve_ref(
    raw: &RawRef,
    scanning_owner: &str,
    scanning_context: &str,
    scanning_file: &std::path::Path,
    slugs: &SlugMap,
    uuids: &UuidMap,
) -> Option<String> {
    unimplemented!("Task D1")
}
```

- [ ] **Step 3: Run tests — expected fail**

Run: `cargo nextest run -p temper-cli resolve_ref_`
Expected: FAIL.

- [ ] **Step 4: Implement**

```rust
pub(crate) fn resolve_ref(
    raw: &RawRef,
    scanning_owner: &str,
    scanning_context: &str,
    scanning_file: &std::path::Path,
    slugs: &SlugMap,
    uuids: &UuidMap,
) -> Option<String> {
    match raw {
        RawRef::WikiSlug(slug) => {
            let target = slugs.resolve(scanning_owner, scanning_context, slug)?;
            if target == scanning_file {
                return None;
            }
            Some(slug.clone())
        }
        RawRef::BareUuid(id) => {
            let target = uuids.resolve(scanning_owner, *id)?;
            if target == scanning_file {
                return None;
            }
            Some(id.to_string())
        }
        RawRef::MarkdownLink(dest) => {
            // Resolve the dest relative to the scanning file's dir.
            let scanning_dir = scanning_file.parent()?;
            let joined = scanning_dir.join(dest);
            let canonical = joined.canonicalize().ok().unwrap_or(joined);
            // Look up the resolved path's stem in the SlugMap for the
            // scanning owner. We need to find which (context, slug)
            // pair points to this path.
            let stem = canonical.file_stem()?.to_str()?.to_string();
            let target = slugs.resolve(scanning_owner, scanning_context, &stem)?;
            // Verify the resolved slug actually corresponds to the path
            // we computed (guards against stem collisions across contexts
            // where resolve() might have picked a different match).
            if target != canonical && target != joined_path_fallback(&canonical, &stem) {
                tracing::debug!(
                    dest = %dest,
                    stem = %stem,
                    resolved = %target.display(),
                    computed = %canonical.display(),
                    "markdown link stem collision — rejecting"
                );
                return None;
            }
            if target == scanning_file {
                return None;
            }
            Some(stem)
        }
    }
}

/// On some filesystems `canonicalize` normalizes symlinks; if the
/// computed `joined` doesn't exist (dangling link), fall back to
/// the un-canonicalized form so the collision check doesn't
/// over-reject.
fn joined_path_fallback(canonical: &std::path::Path, _stem: &str) -> PathBuf {
    canonical.to_path_buf()
}
```

**Note on the collision check:** resolving a markdown link's stem via `SlugMap::resolve` works for the normal case where the stem is unique within the same context. If the link resolved via cross-context fallback to a different file than the link's literal path would point to, we reject it — this is the "markdown links should resolve literally" contract. Two-phase: the first `resolve` gives us a best-effort candidate, then we verify the candidate matches the literal path we computed.

- [ ] **Step 5: Run tests — expected pass**

Run: `cargo nextest run -p temper-cli resolve_ref_`
Expected: PASS 6 tests.

If the `resolve_ref_markdown_link_relative_md` test fails because of `canonicalize` not finding the file in the test's temp paths, adjust the implementation to skip canonicalization when the path doesn't exist (test paths like `/v/@me/...` aren't real). Replace `.canonicalize().ok().unwrap_or(joined)` with a pure-lexical cleanup:

```rust
let canonical = lexical_clean(&joined);
```

where `lexical_clean` removes `./` and `../` without touching the filesystem:

```rust
fn lexical_clean(path: &std::path::Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        use std::path::Component;
        match component {
            Component::ParentDir => { out.pop(); }
            Component::CurDir => {}
            c => out.push(c.as_os_str()),
        }
    }
    out
}
```

Replace `canonicalize` usage in `resolve_ref`'s `MarkdownLink` arm with `lexical_clean(&joined)`. Remove `joined_path_fallback` — the fallback arm isn't needed with pure-lexical cleanup.

Re-run: `cargo nextest run -p temper-cli resolve_ref_`
Expected: PASS 6 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/actions/graph_build.rs
git commit -m "$(cat <<'EOF'
feat(graph-build): resolve raw refs against owner-scoped maps

Wikilinks via SlugMap same-context-first, UUIDs via UuidMap, markdown
links via lexical path cleanup + same-owner stem lookup. Self-edges
are rejected. Cross-owner resolution is impossible by construction.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase E — Pass 3: Merge and write-back

### Task E1: `merge_references` pure helper + tests

**Files:**
- Modify: `crates/temper-cli/src/actions/graph_build.rs`

- [ ] **Step 1: Write failing tests**

```rust
    // ── Pass 3: merge ───────────────────────────────────────────────

    #[test]
    fn merge_references_union_preserves_existing_order() {
        let existing = vec!["foo".to_string(), "bar".to_string()];
        let discovered = vec!["baz".to_string()];
        let (merged, added) = merge_references(&existing, &discovered);
        assert_eq!(merged, vec!["foo", "bar", "baz"]);
        assert_eq!(added, 1);
    }

    #[test]
    fn merge_references_dedupes_across_existing_and_discovered() {
        let existing = vec!["foo".to_string(), "bar".to_string()];
        let discovered = vec!["foo".to_string(), "baz".to_string()];
        let (merged, added) = merge_references(&existing, &discovered);
        assert_eq!(merged, vec!["foo", "bar", "baz"]);
        assert_eq!(added, 1);
    }

    #[test]
    fn merge_references_no_new_entries_reports_zero_added() {
        let existing = vec!["foo".to_string(), "bar".to_string()];
        let discovered = vec!["foo".to_string()];
        let (merged, added) = merge_references(&existing, &discovered);
        assert_eq!(merged, vec!["foo", "bar"]);
        assert_eq!(added, 0);
    }

    #[test]
    fn merge_references_empty_existing() {
        let existing: Vec<String> = vec![];
        let discovered = vec!["foo".to_string(), "bar".to_string()];
        let (merged, added) = merge_references(&existing, &discovered);
        assert_eq!(merged, vec!["foo", "bar"]);
        assert_eq!(added, 2);
    }

    #[test]
    fn merge_references_discovered_duplicates_deduped() {
        let existing: Vec<String> = vec![];
        let discovered = vec!["foo".to_string(), "foo".to_string(), "bar".to_string()];
        let (merged, added) = merge_references(&existing, &discovered);
        assert_eq!(merged, vec!["foo", "bar"]);
        assert_eq!(added, 2);
    }
```

- [ ] **Step 2: Stub**

```rust
/// Merge discovered references into existing in insertion order.
/// Preserves `existing` order, appends new entries in `discovered`
/// order, deduplicates by string equality.
///
/// Returns `(merged, num_actually_added)` so the caller can
/// distinguish "wrote a new file" from "discovered-but-already-present".
pub(crate) fn merge_references(
    existing: &[String],
    discovered: &[String],
) -> (Vec<String>, usize) {
    unimplemented!("Task E1")
}
```

- [ ] **Step 3: Run tests — expected fail**

Run: `cargo nextest run -p temper-cli merge_references_`
Expected: FAIL.

- [ ] **Step 4: Implement**

```rust
pub(crate) fn merge_references(
    existing: &[String],
    discovered: &[String],
) -> (Vec<String>, usize) {
    let mut seen: HashSet<&str> = existing.iter().map(|s| s.as_str()).collect();
    let mut merged: Vec<String> = existing.to_vec();
    let mut added = 0usize;

    for d in discovered {
        if seen.insert(d.as_str()) {
            merged.push(d.clone());
            added += 1;
        }
    }

    // `seen` borrowed `existing` and `discovered`; drop before returning
    // merged (which contains clones) to avoid lifetime issues.
    drop(seen);
    (merged, added)
}
```

Note: the `drop(seen)` is defensive — depending on NLL analysis, the borrow may end earlier. If clippy flags the `drop` as unnecessary, remove it.

- [ ] **Step 5: Run tests — expected pass**

Run: `cargo nextest run -p temper-cli merge_references_`
Expected: PASS 5 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/actions/graph_build.rs
git commit -m "$(cat <<'EOF'
feat(graph-build): merge_references preserves order + dedupes

Pure helper for Pass 3 merge semantics: insertion-order union of
existing and discovered reference lists.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

### Task E2: `write_back_references` — mutates Frontmatter and writes to disk

**Files:**
- Modify: `crates/temper-cli/src/actions/graph_build.rs`

- [ ] **Step 1: Write failing tests**

```rust
    // ── Pass 3: write-back ──────────────────────────────────────────

    #[test]
    fn write_back_adds_new_references_field_when_missing() {
        let tmp = TempDir::new().unwrap();
        let file = write_vault_file(&tmp, "@me", "temper", "task", "alpha", None, "body");

        let merged = vec!["beta".to_string(), "gamma".to_string()];
        write_back_references(&file, &merged).unwrap();

        // Re-parse and check
        let fm = temper_core::frontmatter::Frontmatter::parse_file(&file).unwrap();
        let refs = fm
            .value()
            .get("references")
            .and_then(|v| v.as_sequence())
            .unwrap();
        let refs_strs: Vec<&str> = refs.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(refs_strs, vec!["beta", "gamma"]);
    }

    #[test]
    fn write_back_updates_existing_references_field() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("@me").join("temper").join("task");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("alpha.md");
        let content = "---\n\
temper-context: temper\n\
temper-type: task\n\
temper-owner: '@me'\n\
title: alpha\n\
slug: alpha\n\
references:\n  - existing1\n  - existing2\n\
---\nbody\n";
        fs::write(&file, content).unwrap();

        let merged = vec![
            "existing1".to_string(),
            "existing2".to_string(),
            "new".to_string(),
        ];
        write_back_references(&file, &merged).unwrap();

        let fm = temper_core::frontmatter::Frontmatter::parse_file(&file).unwrap();
        let refs: Vec<String> = fm
            .value()
            .get("references")
            .and_then(|v| v.as_sequence())
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        assert_eq!(refs, vec!["existing1", "existing2", "new"]);
    }

    #[test]
    fn write_back_preserves_body() {
        let tmp = TempDir::new().unwrap();
        let body_text = "# Heading\n\nSome content with [[alpha]] reference.\n";
        let file = write_vault_file(&tmp, "@me", "temper", "task", "bravo", None, body_text);

        write_back_references(&file, &["alpha".to_string()]).unwrap();

        let fm = temper_core::frontmatter::Frontmatter::parse_file(&file).unwrap();
        // Trailing newline normalization is allowed — just check the
        // meaningful body content is intact.
        assert!(fm.body().contains("# Heading"));
        assert!(fm.body().contains("[[alpha]]"));
    }
```

- [ ] **Step 2: Stub**

```rust
/// Read the frontmatter of `file`, set `open_meta.references` to
/// `merged` (as a YAML sequence of strings), and write the file back
/// via the canonical serialize path.
///
/// `merged` MUST already be the full final set — this function does
/// NOT itself merge.
pub(crate) fn write_back_references(
    file: &std::path::Path,
    merged: &[String],
) -> Result<()> {
    unimplemented!("Task E2")
}
```

- [ ] **Step 3: Run tests — expected fail**

Run: `cargo nextest run -p temper-cli write_back_`
Expected: FAIL.

- [ ] **Step 4: Implement**

```rust
pub(crate) fn write_back_references(
    file: &std::path::Path,
    merged: &[String],
) -> Result<()> {
    use serde_yaml::{Mapping, Value};
    use temper_core::frontmatter::Frontmatter;

    let mut fm = Frontmatter::parse_file(file)?;

    // Build the new references sequence
    let seq: Vec<Value> = merged.iter().map(|s| Value::String(s.clone())).collect();
    let new_value = Value::Sequence(seq);

    // Mutate the underlying mapping
    let mapping = fm
        .value_mut()
        .as_mapping_mut()
        .ok_or_else(|| crate::error::TemperError::Project(
            format!("frontmatter of {} is not a mapping", file.display()),
        ))?;
    mapping.insert(Value::String("references".to_string()), new_value);

    // Fall back to a no-op if merged is empty AND there was no
    // pre-existing references field — we should not write a spurious
    // empty key. Check by looking at the original value before mutation.
    if merged.is_empty() {
        // Re-parse would be expensive; trust caller to skip empty
        // merged sets before calling this function.
    }

    fm.write_to(file)
}
```

- [ ] **Step 5: Run tests — expected pass**

Run: `cargo nextest run -p temper-cli write_back_`
Expected: PASS 3 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/actions/graph_build.rs
git commit -m "$(cat <<'EOF'
feat(graph-build): write_back_references mutates open_meta + writes

Uses Frontmatter::value_mut() to inject/overwrite the references
field as a YAML sequence, then serializes via the canonical path.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase F — Pipeline wiring

### Task F1: Wire `run()` through all three passes + implement report

**Files:**
- Modify: `crates/temper-cli/src/actions/graph_build.rs`
- Modify: `crates/temper-cli/src/commands/graph.rs`

- [ ] **Step 1: Write failing end-to-end test**

```rust
    #[test]
    fn run_end_to_end_seeds_references_from_wikilinks() {
        let tmp = TempDir::new().unwrap();
        // Target files
        write_vault_file(&tmp, "@me", "temper", "task", "alpha", None, "");
        write_vault_file(&tmp, "@me", "temper", "task", "beta", None, "");
        // Source file with wikilinks to both
        let source = write_vault_file(
            &tmp,
            "@me",
            "temper",
            "task",
            "source",
            None,
            "This references [[alpha]] and [[beta]] explicitly.",
        );

        let config = fixture_config(&tmp, &["temper"]);
        let params = GraphBuildParams {
            context_filter: None,
            dry_run: false,
            verbose: false,
        };
        let report = run(&config, params).unwrap();

        assert_eq!(report.files_modified, 1);
        assert_eq!(report.references_added, 2);

        // The source file now has references: [alpha, beta]
        let fm = temper_core::frontmatter::Frontmatter::parse_file(&source).unwrap();
        let refs: Vec<String> = fm
            .value()
            .get("references")
            .and_then(|v| v.as_sequence())
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        assert_eq!(refs, vec!["alpha", "beta"]);
    }

    #[test]
    fn run_end_to_end_idempotent() {
        let tmp = TempDir::new().unwrap();
        write_vault_file(&tmp, "@me", "temper", "task", "alpha", None, "");
        write_vault_file(
            &tmp,
            "@me",
            "temper",
            "task",
            "source",
            None,
            "See [[alpha]].",
        );

        let config = fixture_config(&tmp, &["temper"]);

        let first = run(
            &config,
            GraphBuildParams {
                context_filter: None,
                dry_run: false,
                verbose: false,
            },
        )
        .unwrap();
        assert_eq!(first.files_modified, 1);

        let second = run(
            &config,
            GraphBuildParams {
                context_filter: None,
                dry_run: false,
                verbose: false,
            },
        )
        .unwrap();
        assert_eq!(second.files_modified, 0, "second run must be a no-op");
        assert_eq!(second.references_added, 0);
    }

    #[test]
    fn run_dry_run_does_not_write() {
        let tmp = TempDir::new().unwrap();
        write_vault_file(&tmp, "@me", "temper", "task", "alpha", None, "");
        let source = write_vault_file(
            &tmp,
            "@me",
            "temper",
            "task",
            "source",
            None,
            "See [[alpha]].",
        );
        let content_before = std::fs::read_to_string(&source).unwrap();

        let config = fixture_config(&tmp, &["temper"]);
        let report = run(
            &config,
            GraphBuildParams {
                context_filter: None,
                dry_run: true,
                verbose: false,
            },
        )
        .unwrap();
        assert_eq!(report.files_modified, 1, "report counts as if written");

        let content_after = std::fs::read_to_string(&source).unwrap();
        assert_eq!(content_before, content_after, "dry-run must not write");
    }
```

- [ ] **Step 2: Run tests — expected fail**

Run: `cargo nextest run -p temper-cli run_end_to_end run_dry_run`
Expected: FAIL at `run_end_to_end_seeds_references_from_wikilinks` — still hits the "not yet implemented" branch of `run()`.

- [ ] **Step 3: Implement `run()`**

Replace the stub `run()` with:

```rust
pub fn run(config: &Config, params: GraphBuildParams) -> Result<GraphBuildReport> {
    // Pass 1: walk + maps
    let (slugs, uuids, files) = discover_vault(config, params.context_filter.as_deref())?;
    let files_walked = files.len();

    // Pass 2: scan + resolve, accumulating per-file discovered refs
    // keyed by absolute file path.
    let mut discovered: HashMap<PathBuf, Vec<String>> = HashMap::new();
    let mut references_found = 0usize;

    for file in &files {
        let raw_refs = scan_body(file.frontmatter.body());
        for raw in &raw_refs {
            if let Some(resolved) = resolve_ref(
                raw,
                &file.owner,
                &file.context,
                &file.path,
                &slugs,
                &uuids,
            ) {
                references_found += 1;
                discovered
                    .entry(file.path.clone())
                    .or_default()
                    .push(resolved);
            }
        }
    }

    // Pass 3: merge + write back (or simulate for dry-run)
    let mut report = GraphBuildReport {
        files_walked,
        references_found,
        ..Default::default()
    };

    // Build a quick lookup from path -> &DiscoveredFile for existing-ref access
    let file_by_path: HashMap<&std::path::Path, &DiscoveredFile> =
        files.iter().map(|f| (f.path.as_path(), f)).collect();

    // Stable ordering for the modified_files report
    let mut paths: Vec<&PathBuf> = discovered.keys().collect();
    paths.sort();

    for path in paths {
        let disc_refs = &discovered[path];
        let file = file_by_path
            .get(path.as_path())
            .expect("discovered path not in walk");

        let existing = existing_references(&file.frontmatter);
        let (merged, added) = merge_references(&existing, disc_refs);

        // Count "already present" = discovered count - added
        // (discovered may contain duplicates with existing; dedupe-on-insert
        // collapses them into `added`)
        let already = disc_refs.len().saturating_sub(added);
        report.already_present += already;

        if added == 0 {
            continue;
        }

        if !params.dry_run {
            write_back_references(path, &merged)?;
        }

        report.files_modified += 1;
        report.references_added += added;

        let added_refs = if params.verbose {
            merged.iter().skip(existing.len()).cloned().collect()
        } else {
            Vec::new()
        };

        report.modified_files.push(ModifiedFile {
            rel_path: file.rel_path.clone(),
            added,
            added_refs,
        });
    }

    Ok(report)
}

/// Read the existing `open_meta.references` field from a parsed
/// Frontmatter as a `Vec<String>`. Missing, null, or wrong-typed
/// fields yield an empty vec.
fn existing_references(fm: &temper_core::frontmatter::Frontmatter) -> Vec<String> {
    fm.value()
        .get("references")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}
```

- [ ] **Step 4: Run tests — expected pass**

Run: `cargo nextest run -p temper-cli run_end_to_end run_dry_run`
Expected: PASS 3 tests.

Also run the full `graph_build` test module to make sure nothing regressed:

Run: `cargo nextest run -p temper-cli graph_build::`
Expected: all tests pass (currently ~35 between the units).

- [ ] **Step 5: Wire the commands/graph.rs dispatch to actually call run**

Replace the body of `commands/graph.rs`:

```rust
//! `temper graph` command dispatch.

use crate::actions::graph_build::{self, GraphBuildParams};
use crate::cli::GraphAction;
use crate::config::Config;
use crate::error::Result;
use crate::output;

pub fn run(config: &Config, action: GraphAction) -> Result<()> {
    match action {
        GraphAction::Build {
            context,
            dry_run,
            verbose,
        } => {
            let params = GraphBuildParams {
                context_filter: context,
                dry_run,
                verbose,
            };
            let report = graph_build::run(config, params)?;
            render_report(&report, dry_run, verbose);
            Ok(())
        }
    }
}

fn render_report(report: &graph_build::GraphBuildReport, dry_run: bool, verbose: bool) {
    let verb = if dry_run { "Would modify" } else { "Modified" };

    output::heading(format!(
        "temper graph build — {} files walked",
        report.files_walked
    ));
    output::plain(format!(
        "  Pass 2 (scanning):    {} references found",
        report.references_found
    ));
    output::plain("  Pass 3 (merge):");
    output::plain(format!("    Files modified:     {}", report.files_modified));
    output::plain(format!(
        "    References added:   {}",
        report.references_added
    ));
    output::plain(format!(
        "    Already present:    {}",
        report.already_present
    ));

    if !report.modified_files.is_empty() {
        output::blank();
        output::plain(format!("{verb} files:"));
        for mf in &report.modified_files {
            output::plain(format!("  {}  (+{} references)", mf.rel_path, mf.added));
            if verbose {
                for r in &mf.added_refs {
                    output::plain(format!("    - {r}"));
                }
            }
        }
    }
}
```

Verify that `crate::output::heading`/`plain`/`blank` exist. If not, use whichever are present (grep the file):

Run: `grep -n "pub fn" crates/temper-cli/src/output.rs`

Adjust the `output::*` calls to match whatever the module exports. The existing commands (see `crates/temper-cli/src/commands/doctor.rs`) are the reference for idiomatic output.

- [ ] **Step 6: Verify full compile**

Run: `cargo check -p temper-cli`
Expected: clean.

Run: `cargo clippy -p temper-cli -- -D warnings`
Expected: clean.

- [ ] **Step 7: Smoke test on the real vault (dry-run only)**

Run: `cargo run -p temper-cli --bin temper -- graph build --dry-run`
Expected: Report printed, shows some number of files walked (around 750 on the real vault), references found/added/modified numbers (likely non-zero since the real vault does have some prose references to existing slugs). Zero files actually modified on disk (verify with `cd /Users/petetaylor/projects/kb-vault && git status --porcelain`).

**IMPORTANT:** Only run the dry-run against the real vault. Do NOT run without `--dry-run` until Phase I's integration test has validated the behavior end-to-end. The real vault is production data.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/actions/graph_build.rs crates/temper-cli/src/commands/graph.rs
git commit -m "$(cat <<'EOF'
feat(graph-build): wire three-pass pipeline end-to-end

run() walks the vault, scans bodies, resolves refs, merges, and
writes back (or simulates for dry-run). Command dispatch renders
the report. Verified idempotent and dry-run no-op in unit tests.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase G — Integration test with fixture vault

### Task G1: Add external integration test

**Files:**
- Create: `crates/temper-cli/tests/graph_build_test.rs`

- [ ] **Step 1: Write the integration test**

Create `crates/temper-cli/tests/graph_build_test.rs`:

```rust
//! Integration test for `temper graph build` end-to-end pipeline.
//!
//! Builds a fixture vault in a temp dir, runs graph_build::run,
//! asserts file contents, then runs again to verify idempotency.

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use temper_cli::actions::graph_build::{self, GraphBuildParams};
use temper_cli::config::Config;

fn write_file(dir: &PathBuf, name: &str, content: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, content).unwrap();
    path
}

fn fixture_config(tmp: &TempDir, contexts: &[&str]) -> Config {
    Config {
        vault_root: tmp.path().to_path_buf(),
        state_dir: tmp.path().join(".temper"),
        contexts: contexts.iter().map(|s| s.to_string()).collect(),
        subscriptions: Vec::new(),
        skill_output: tmp.path().join(".skill"),
    }
}

fn file_content(temper_ctx: &str, slug: &str, body: &str) -> String {
    format!(
        "---\n\
temper-context: {temper_ctx}\n\
temper-type: task\n\
temper-owner: '@me'\n\
title: {slug}\n\
slug: {slug}\n\
---\n\
{body}\n"
    )
}

#[test]
fn graph_build_resolves_mixed_references() {
    let tmp = TempDir::new().unwrap();

    let temper_task_dir = tmp.path().join("@me").join("temper").join("task");

    // Targets
    write_file(
        &temper_task_dir,
        "alpha.md",
        &file_content("temper", "alpha", "alpha body"),
    );
    write_file(
        &temper_task_dir,
        "beta.md",
        &file_content("temper", "beta", "beta body"),
    );

    // Source with wikilink + markdown link + code block that should be ignored
    let source_body = "\
# Source

See [[alpha]] and [beta](beta.md).

```
This is a code block with [[fake-ref]] that must be ignored.
```

Back to prose, another mention: [[alpha]].
";
    let source = write_file(
        &temper_task_dir,
        "source.md",
        &file_content("temper", "source", source_body),
    );

    let config = fixture_config(&tmp, &["temper"]);

    // First run: should write
    let report = graph_build::run(
        &config,
        GraphBuildParams {
            context_filter: None,
            dry_run: false,
            verbose: false,
        },
    )
    .unwrap();

    assert_eq!(report.files_walked, 3);
    // alpha gets added once (dedupe), beta gets added once
    assert_eq!(report.files_modified, 1);
    assert_eq!(report.references_added, 2);

    // Verify source.md has references: [alpha, beta]
    let fm = temper_core::frontmatter::Frontmatter::parse_file(&source).unwrap();
    let refs: Vec<String> = fm
        .value()
        .get("references")
        .and_then(|v| v.as_sequence())
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    assert_eq!(refs, vec!["alpha", "beta"]);

    // Fake-ref from the code block must NOT appear
    assert!(!refs.iter().any(|r| r == "fake-ref"));

    // Second run: idempotent
    let second = graph_build::run(
        &config,
        GraphBuildParams {
            context_filter: None,
            dry_run: false,
            verbose: false,
        },
    )
    .unwrap();
    assert_eq!(second.files_modified, 0);
    assert_eq!(second.references_added, 0);
}

#[test]
fn graph_build_respects_owner_boundary() {
    let tmp = TempDir::new().unwrap();

    // @me and +team-x both have a "shared" slug
    let me_dir = tmp.path().join("@me").join("temper").join("task");
    let team_dir = tmp.path().join("+team-x").join("temper").join("task");

    write_file(
        &me_dir,
        "shared.md",
        &file_content("temper", "shared", "@me shared body"),
    );
    write_file(
        &team_dir,
        "shared.md",
        &file_content("temper", "shared", "+team-x shared body"),
    );

    // @me source file tries to link to "shared"
    let source = write_file(
        &me_dir,
        "source.md",
        &file_content("temper", "source", "See [[shared]]."),
    );

    let config = fixture_config(&tmp, &["temper"]);
    let report = graph_build::run(
        &config,
        GraphBuildParams {
            context_filter: None,
            dry_run: false,
            verbose: false,
        },
    )
    .unwrap();

    // Source should resolve to the @me/shared (same owner), NOT +team-x
    let fm = temper_core::frontmatter::Frontmatter::parse_file(&source).unwrap();
    let refs: Vec<String> = fm
        .value()
        .get("references")
        .and_then(|v| v.as_sequence())
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    assert_eq!(refs, vec!["shared"]);
    // references_added must be exactly 1 — cross-owner target doesn't
    // double-count
    assert_eq!(report.references_added, 1);
}
```

Note: this test uses `+team-x` which requires `temper-core::validation::validate_owner_pattern` to accept it. It's already tested for in `vault.rs:206` so the pattern should work. If the test fails because `@me` isn't in `config.contexts`, add additional context configuration. The `discover_vault` walk uses `config.contexts` to iterate, and `owner_for_context` falls back to `@me` for contexts without subscriptions — but it won't walk `+team-x` directories at all unless there's a subscription. For this test to validate the owner-boundary rejection path, both owners need to be walked. Adjust by providing `subscriptions` that map one context to `+team-x`:

Actually, re-reading `owner_for_context`: it only ever returns ONE owner per context. So you can't have the same context under two owners in a vault walk. Adjust the test: use a second context like `shared-team` that lives under `+team-x`:

Replace the second test fixture setup:

```rust
#[test]
fn graph_build_respects_owner_boundary() {
    use temper_core::types::vault_config::Subscription;

    let tmp = TempDir::new().unwrap();

    // @me/temper/task/shared.md
    let me_dir = tmp.path().join("@me").join("temper").join("task");
    write_file(
        &me_dir,
        "shared.md",
        &file_content("temper", "shared", "@me shared body"),
    );

    // +team-x/team-ctx/task/shared.md (same slug, different owner)
    let team_dir = tmp.path().join("+team-x").join("team-ctx").join("task");
    write_file(
        &team_dir,
        "shared.md",
        &file_content("team-ctx", "shared", "+team-x shared body"),
    );

    // @me source file tries to link to "shared"
    let source = write_file(
        &me_dir,
        "source.md",
        &file_content("temper", "source", "See [[shared]]."),
    );

    // Config: two contexts, one per owner
    let mut config = fixture_config(&tmp, &["temper", "team-ctx"]);
    config.subscriptions = vec![
        Subscription {
            context: "team-ctx".to_string(),
            // Whatever fields the Subscription struct needs; look them up
            ..Default::default()
        },
    ];

    // If Subscription doesn't impl Default, construct it explicitly by
    // reading temper-core/src/types/vault_config.rs and filling the
    // required fields with minimal values that make owner_for_context
    // return "+team-x" for "team-ctx".

    // ... rest of test ...
}
```

**IF** the Subscription struct is non-trivial, an alternative: have the test build two separate contexts both under `@me` — that won't test cross-owner rejection but will test cross-context resolution. Owner boundary is already covered by the unit test `slug_map_rejects_cross_owner`. The integration test can focus on cross-context-within-owner.

Simpler approach: make this test exercise cross-context same-owner rejection when the slug is ambiguous. Rewrite:

```rust
#[test]
fn graph_build_skips_ambiguous_cross_context_slugs() {
    let tmp = TempDir::new().unwrap();

    // Two contexts both have a "shared" slug
    let temper_dir = tmp.path().join("@me").join("temper").join("task");
    let tasker_dir = tmp.path().join("@me").join("tasker").join("task");

    write_file(
        &temper_dir,
        "shared.md",
        &file_content("temper", "shared", "temper shared body"),
    );
    write_file(
        &tasker_dir,
        "shared.md",
        &file_content("tasker", "shared", "tasker shared body"),
    );

    // Source in a THIRD context that has no "shared" file itself
    let general_dir = tmp.path().join("@me").join("general").join("task");
    let source = write_file(
        &general_dir,
        "source.md",
        &file_content("general", "source", "See [[shared]]."),
    );

    let config = fixture_config(&tmp, &["temper", "tasker", "general"]);
    let report = graph_build::run(
        &config,
        GraphBuildParams {
            context_filter: None,
            dry_run: false,
            verbose: false,
        },
    )
    .unwrap();

    // Ambiguous cross-context → skipped, source gets no references
    let fm = temper_core::frontmatter::Frontmatter::parse_file(&source).unwrap();
    let refs = fm.value().get("references");
    assert!(
        refs.is_none() || refs.unwrap().as_sequence().map(|s| s.is_empty()).unwrap_or(true),
        "ambiguous cross-context slug must not resolve"
    );
    assert_eq!(report.references_added, 0);
}
```

Use this simpler test instead of the owner-boundary one. Owner boundary is already covered at the unit level.

- [ ] **Step 2: Run the integration test**

Run: `cargo nextest run -p temper-cli --test graph_build_test`
Expected: PASS 2 tests.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-cli/tests/graph_build_test.rs
git commit -m "$(cat <<'EOF'
test(graph-build): integration test for end-to-end pipeline

Fixture vault exercises wikilink + markdown link resolution with a
code-block false-positive trap, then re-runs to validate idempotency.
Also covers ambiguous cross-context slug rejection.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase H — Server-side companion: `temper-goal → ParentOf`

Move the responsibility for deriving a `ParentOf` edge from `managed_meta.temper_goal` into the server's edge extraction path. Rename the existing extraction function, broaden its signature, update all call sites.

### Task H1: Rename + broaden `extract_declarations_from_open_meta`

**Files:**
- Modify: `crates/temper-api/src/services/edge_service.rs`

- [ ] **Step 1: Write the failing test**

Append to the existing `#[cfg(test)] mod tests` block at the bottom of `edge_service.rs`:

```rust
    #[test]
    fn extract_task_with_temper_goal_produces_parent_edge() {
        let managed = json!({"temper-goal": "some-goal"});
        let open = json!({});
        let decls = extract_declarations_from_resource("task", &managed, &open);
        // Should contain exactly one ParentOf declaration pointing to "some-goal"
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].0, EdgeType::ParentOf);
        assert_eq!(decls[0].1, TargetRef::Slug("some-goal".to_string()));
    }

    #[test]
    fn extract_non_task_with_temper_goal_ignores_it() {
        let managed = json!({"temper-goal": "some-goal"});
        let open = json!({});
        let decls = extract_declarations_from_resource("research", &managed, &open);
        assert!(decls.is_empty(), "only tasks emit temper-goal → parent_of");
    }

    #[test]
    fn extract_task_without_temper_goal_produces_no_edge() {
        let managed = json!({});
        let open = json!({});
        let decls = extract_declarations_from_resource("task", &managed, &open);
        assert!(decls.is_empty());
    }

    #[test]
    fn extract_task_with_temper_goal_and_open_meta_refs_combines_both() {
        let managed = json!({"temper-goal": "some-goal"});
        let open = json!({"relates_to": ["other-task"]});
        let decls = extract_declarations_from_resource("task", &managed, &open);
        assert_eq!(decls.len(), 2);
        assert!(decls.iter().any(|(t, _)| *t == EdgeType::ParentOf));
        assert!(decls.iter().any(|(t, _)| *t == EdgeType::RelatesTo));
    }

    #[test]
    fn extract_task_with_empty_temper_goal_string_produces_no_edge() {
        let managed = json!({"temper-goal": ""});
        let open = json!({});
        let decls = extract_declarations_from_resource("task", &managed, &open);
        assert!(decls.is_empty(), "empty string is not a valid goal slug");
    }
```

- [ ] **Step 2: Run tests — expected fail**

Run: `cargo nextest run -p temper-api extract_task_with`
Expected: FAIL — function doesn't exist yet.

- [ ] **Step 3: Rename the existing function and broaden its signature**

In `crates/temper-api/src/services/edge_service.rs` at line 333, replace:

```rust
pub fn extract_declarations_from_open_meta(
    open_meta: &serde_json::Value,
) -> Vec<(EdgeType, TargetRef)> {
    if !open_meta.is_object() {
        return Vec::new();
    }

    let rels: ResourceRelationships = match serde_json::from_value(open_meta.clone()) {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(
                error = %e,
                "open_meta did not contain valid relationship fields"
            );
            return Vec::new();
        }
    };

    rels.to_edge_declarations()
}
```

with:

```rust
/// Extract edge declarations from a resource's full meta.
///
/// Reads relationship fields from `open_meta` (via `ResourceRelationships`)
/// and, for tasks, the `temper-goal` field from `managed_meta` which
/// yields a reversed `ParentOf` edge to the goal resource.
///
/// Pure function — no database access. Unknown fields in either
/// tier are ignored.
pub fn extract_declarations_from_resource(
    doc_type: &str,
    managed_meta: &serde_json::Value,
    open_meta: &serde_json::Value,
) -> Vec<(EdgeType, TargetRef)> {
    let mut edges = Vec::new();

    // Open-meta relationships (existing path)
    if open_meta.is_object() {
        match serde_json::from_value::<ResourceRelationships>(open_meta.clone()) {
            Ok(rels) => edges.extend(rels.to_edge_declarations()),
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "open_meta did not contain valid relationship fields"
                );
            }
        }
    }

    // Managed-meta derivations
    if doc_type == "task" {
        if let Some(goal_slug) = managed_meta
            .get("temper-goal")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            if let Some(target) = TargetRef::parse(goal_slug) {
                edges.push((EdgeType::ParentOf, target));
            }
        }
    }

    edges
}
```

Note: using `&str` for `doc_type` keeps the API call-site agnostic whether callers have a typed enum or a row-read string. Callers pass the `doc_type` column value from `kb_resources`.

- [ ] **Step 4: Update internal callers in the same file**

Find line 358 (`pub async fn extract_and_upsert_edges`) and change its signature + body to pass both meta tiers. Replace the function:

```rust
pub async fn extract_and_upsert_edges(
    pool: &PgPool,
    profile_id: &ProfileId,
    context_id: &ContextId,
    resource_id: &ResourceId,
    doc_type: &str,
    managed_meta: &serde_json::Value,
    open_meta: &serde_json::Value,
) -> ApiResult<(usize, usize)> {
    let declarations = extract_declarations_from_resource(doc_type, managed_meta, open_meta);
    if declarations.is_empty() {
        return Ok((0, 0));
    }

    let (resolved, unresolved) =
        resolve_declarations(pool, profile_id, context_id, resource_id, &declarations).await?;

    let created = upsert_edges(pool, &resolved, profile_id).await?;
    let deferred = defer_edges(pool, resource_id, context_id, profile_id, &unresolved).await?;

    tracing::info!(
        resource_id = %resource_id,
        created,
        deferred,
        "extracted and upserted edges from resource meta"
    );

    Ok((created, deferred))
}
```

Find line 393 (`pub async fn reconcile_edges`) and similarly:

```rust
pub async fn reconcile_edges(
    pool: &PgPool,
    profile_id: &ProfileId,
    context_id: &ContextId,
    resource_id: &ResourceId,
    doc_type: &str,
    managed_meta: &serde_json::Value,
    open_meta: &serde_json::Value,
) -> ApiResult<EdgeReconciliation> {
    let declarations = extract_declarations_from_resource(doc_type, managed_meta, open_meta);

    // ... rest of body unchanged ...
```

Leave the rest of `reconcile_edges` (the existing/new set diffing logic) unchanged.

- [ ] **Step 5: Update existing in-file test call sites**

Find the 7 existing tests in the `#[cfg(test)] mod tests` block that use `extract_declarations_from_open_meta` (lines ~557, 563, 570, 577, 591, 601, 609). Rename each call:

```rust
// Before
let decls = extract_declarations_from_open_meta(&json!({}));
// After
let decls = extract_declarations_from_resource("task", &json!({}), &json!({}));
```

Do this for each existing call, supplying `"task"` as the doc_type and `json!({})` as the managed_meta when the test originally had no managed meta. For tests that already exercise `parent:` in open_meta (e.g., `extract_parent_produces_parent_of` at line 607), the test should still pass with the new signature — `parent` in open_meta continues to produce a ParentOf edge via `ResourceRelationships::to_edge_declarations`, independent of `temper-goal`.

- [ ] **Step 6: Run tests — expected pass**

Run: `cargo nextest run -p temper-api edge_service::tests`
Expected: all existing tests pass + 5 new ones pass.

- [ ] **Step 7: Verify the rest of temper-api still compiles**

Run: `cargo check -p temper-api`
Expected: FAIL at the external callers (meta_service.rs:246, ingest_service.rs:464, ingest_service.rs:699). These get fixed in Task H2. Leave them broken for now.

- [ ] **Step 8: Do NOT commit yet — callers land in H2.**

### Task H2: Update external callers

**Files:**
- Modify: `crates/temper-api/src/services/meta_service.rs:246`
- Modify: `crates/temper-api/src/services/ingest_service.rs:464`
- Modify: `crates/temper-api/src/services/ingest_service.rs:699`
- Modify: `crates/temper-api/tests/edge_ingest_test.rs` (8 call sites)

- [ ] **Step 1: Read the call sites**

Run:
```
cargo check -p temper-api 2>&1 | head -60
```
The error output will show you the exact line numbers and what arguments each caller passes. Read each file around the flagged line.

For each caller, the task is to pass the resource's `doc_type` and its `managed_meta` JSON in addition to the existing `open_meta`. The resource row must have both of these available — if a caller doesn't have them in scope, it may need a small refactor to fetch them from the DB or pass them from its own caller.

Read `meta_service.rs` around line 246:

Run: `sed -n '220,260p' crates/temper-api/src/services/meta_service.rs`

This will show the function that calls `reconcile_edges` and you can determine which variables to pass. The function is `update_meta` and it already has the resource's managed/open JSON in scope from the previous PATCH body — pass them directly.

- [ ] **Step 2: Update meta_service.rs:246**

Context: `update_meta` receives a patch body, reads the resource, applies the patch, and calls `reconcile_edges`. Both `managed_meta` and `open_meta` are in scope. Change:

```rust
// Before
if let Err(e) = super::edge_service::reconcile_edges(
    pool,
    &profile_id,
    &ctx_id,
    &resource_id,
    open_meta_ref,
)
// After
if let Err(e) = super::edge_service::reconcile_edges(
    pool,
    &profile_id,
    &ctx_id,
    &resource_id,
    doc_type_str,
    managed_meta_ref,
    open_meta_ref,
)
```

Adjust variable names to whatever is actually in scope. You may need to read the resource's `doc_type` column earlier in the function — that value is typically in the resource row. If not present, add a query or pass it from the caller.

- [ ] **Step 3: Update ingest_service.rs:464**

Run: `sed -n '440,480p' crates/temper-api/src/services/ingest_service.rs`

This is the CREATE path calling `extract_and_upsert_edges`. The resource being created has its `doc_type`, `managed_meta`, and `open_meta` available in scope. Apply the same pattern.

- [ ] **Step 4: Update ingest_service.rs:699**

Run: `sed -n '680,720p' crates/temper-api/src/services/ingest_service.rs`

This is the UPDATE path calling `reconcile_edges`. Same treatment.

- [ ] **Step 5: Update edge_ingest_test.rs**

Run: `sed -n '1,40p' crates/temper-api/tests/edge_ingest_test.rs`

Read the test file fully to understand its pattern. It calls `extract_and_upsert_edges` and `reconcile_edges` from integration tests with `test-db`. Each call site needs the new `doc_type` + `managed_meta` params. The test fixtures probably create resources with known `doc_type` — pass that through. For tests that set up a plain open_meta but no managed_meta, pass `&serde_json::json!({})` for managed_meta.

- [ ] **Step 6: Verify temper-api compiles and tests pass**

Run: `cargo check -p temper-api`
Expected: clean.

Run: `cargo nextest run -p temper-api`
Expected: all existing tests pass.

- [ ] **Step 7: Run the test-db integration tests**

```bash
cargo make docker-up
cargo nextest run -p temper-api --features test-db
```

Expected: all pass including `edge_ingest_test.rs`. If `reconcile_edges` signature changes broke something subtle, the db tests surface it.

- [ ] **Step 8: Regenerate sqlx offline cache if any SQL touched**

This phase should not have touched SQL, but to be safe:

Run: `cargo sqlx prepare --workspace --check -- --all-features 2>&1 | tail -20`
Expected: "query data matches" or equivalent clean output. If drift is reported, run `cargo sqlx prepare --workspace -- --all-features` and commit the updated `.sqlx/` files.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-api/src/services/edge_service.rs \
        crates/temper-api/src/services/meta_service.rs \
        crates/temper-api/src/services/ingest_service.rs \
        crates/temper-api/tests/edge_ingest_test.rs
git commit -m "$(cat <<'EOF'
feat(edge-service): derive ParentOf from managed_meta.temper_goal

Renames extract_declarations_from_open_meta to
extract_declarations_from_resource and broadens the signature to
accept (doc_type, managed_meta, open_meta). Tasks with a non-empty
temper-goal now produce a ParentOf edge to the goal resource
server-side — the authoritative task-to-goal relationship stays in
managed_meta with no drift-prone CLI write-back. Updates call sites
in meta_service, ingest_service, and the edge_ingest test suite.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase I — E2E test: graph build → sync → reconcile

### Task I1: Verify the existing e2e harness and add an e2e test

**Files:**
- Create: `crates/temper-e2e/tests/graph_build_e2e_test.rs`

- [ ] **Step 1: Inspect the existing e2e harness**

Run: `ls crates/temper-e2e/tests/`
Run: `head -80 crates/temper-e2e/tests/[pick_one].rs`

Read at least one existing e2e test to understand the harness: how the test vault is set up, how sync is invoked, how the DB is inspected. Most likely there's a shared fixture module with a `setup_vault_and_sync` helper.

- [ ] **Step 2: Write the e2e test**

Following the harness pattern, create `crates/temper-e2e/tests/graph_build_e2e_test.rs` with one test:

```rust
//! End-to-end test: temper graph build → temper sync run → server reconcile
//!
//! Asserts that graph build's wikilink resolution propagates all the
//! way to `kb_resource_edges` after sync, AND that the server-side
//! companion change derives `ParentOf` edges from `managed_meta.temper_goal`
//! without any CLI-side parent write-back.

#![cfg(feature = "test-db")]

use std::fs;

// Adjust these imports to match the existing e2e harness in this crate
use temper_e2e::harness::{spin_up_test_server, temp_vault_dir, run_cli_graph_build, run_cli_sync};

#[tokio::test]
async fn graph_build_then_sync_materializes_edges() {
    let (pool, _server) = spin_up_test_server().await;
    let vault = temp_vault_dir();

    // Fixture: a goal, two tasks, and a third task with a wikilink to one of them
    write_frontmatter_file(
        &vault,
        "@me/temper/goal/my-goal.md",
        r#"
temper-context: temper
temper-type: goal
temper-owner: '@me'
title: my-goal
slug: my-goal
"#,
        "",
    );
    write_frontmatter_file(
        &vault,
        "@me/temper/task/task-a.md",
        r#"
temper-context: temper
temper-type: task
temper-owner: '@me'
temper-goal: my-goal
title: task-a
slug: task-a
temper-stage: in-progress
temper-mode: build
temper-effort: small
"#,
        "",
    );
    write_frontmatter_file(
        &vault,
        "@me/temper/task/task-b.md",
        r#"
temper-context: temper
temper-type: task
temper-owner: '@me'
temper-goal: my-goal
title: task-b
slug: task-b
temper-stage: in-progress
temper-mode: build
temper-effort: small
"#,
        "",
    );
    write_frontmatter_file(
        &vault,
        "@me/temper/task/source.md",
        r#"
temper-context: temper
temper-type: task
temper-owner: '@me'
title: source
slug: source
temper-stage: in-progress
temper-mode: build
temper-effort: small
"#,
        "See [[task-a]] for the background.\n",
    );

    // Step 1: initial sync so the server knows about all resources
    run_cli_sync(&vault).await;

    // Step 2: graph build writes source.md's references: [task-a]
    run_cli_graph_build(&vault).await;

    // Step 3: second sync pushes the updated source.md (metadata-only PATCH)
    run_cli_sync(&vault).await;

    // ── Assertions ─────────────────────────────────────────────────

    // 1. source → references → task-a edge
    let source_to_task_a: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_resource_edges e
         JOIN kb_resources src ON e.source_resource_id = src.id
         JOIN kb_resources tgt ON e.target_resource_id = tgt.id
         WHERE src.slug = 'source'
           AND tgt.slug = 'task-a'
           AND e.edge_type::text = 'references'
           AND e.metadata->>'provenance' = 'frontmatter'"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(source_to_task_a, 1, "source should reference task-a after graph build + sync");

    // 2. my-goal → parent_of → task-a (from server-side temper-goal extraction)
    let goal_parent_a: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_resource_edges e
         JOIN kb_resources src ON e.source_resource_id = src.id
         JOIN kb_resources tgt ON e.target_resource_id = tgt.id
         WHERE src.slug = 'my-goal'
           AND tgt.slug = 'task-a'
           AND e.edge_type::text = 'parent_of'
           AND e.metadata->>'provenance' = 'frontmatter'"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        goal_parent_a, 1,
        "my-goal should be parent_of task-a via server-side temper-goal extraction"
    );

    // 3. my-goal → parent_of → task-b (second task with same goal)
    let goal_parent_b: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_resource_edges e
         JOIN kb_resources src ON e.source_resource_id = src.id
         JOIN kb_resources tgt ON e.target_resource_id = tgt.id
         WHERE src.slug = 'my-goal'
           AND tgt.slug = 'task-b'
           AND e.edge_type::text = 'parent_of'"
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(goal_parent_b, 1);

    // 4. Verify the vault file does NOT contain a `parent:` field in
    //    open_meta — the temper-goal derivation is server-side only,
    //    not a CLI write-back.
    let task_a_content = fs::read_to_string(vault.join("@me/temper/task/task-a.md")).unwrap();
    assert!(
        !task_a_content.lines().any(|l| l.trim_start().starts_with("parent:")),
        "task-a.md should not have a parent: field — temper-goal derivation is server-side"
    );
}
```

This test's exact shape depends on the existing e2e harness conventions. Read at least one existing e2e test in the crate before finalizing — imports, setup helpers, assertion SQL style should all match the local idioms.

- [ ] **Step 3: Run the e2e test**

```bash
cargo make docker-up
cargo nextest run -p temper-e2e --features test-db graph_build_then_sync
```

Expected: PASS. If failures occur, they're most likely in the assertion SQL or the harness helper imports — adjust to match the actual harness.

- [ ] **Step 4: Run the entire test-db suite to catch regressions**

Run: `cargo nextest run --workspace --features test-db`
Expected: 902+ tests pass (baseline from Session 3 of frontmatter consolidation was 902; new tests added here should bring it up by at least the count from Tasks B-G and H1).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-e2e/tests/graph_build_e2e_test.rs
git commit -m "$(cat <<'EOF'
test(e2e): graph build end-to-end with sync and server reconcile

Seeds a fixture vault with a goal and three tasks (two linked to the
goal via temper-goal, one with a wikilink to another), runs graph
build and sync, and asserts the resulting kb_resource_edges rows.
Validates both the CLI-side wikilink write-back and the server-side
temper-goal → parent_of extraction added in Phase H.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Phase J — Final verification

### Task J1: Full verification sweep

**Files:** none (verification only)

- [ ] **Step 1: Cargo check everything**

Run: `cargo make check`
Expected: clean (fmt, clippy -D warnings, docs, machete, TS typecheck, biome).

- [ ] **Step 2: Workspace unit tests**

Run: `cargo nextest run --workspace`
Expected: all pass, count ≥ 749 (the Session 3 baseline) plus the new graph_build tests.

- [ ] **Step 3: Workspace integration tests**

Run: `cargo nextest run --workspace --features test-db`
Expected: all pass, count ≥ 902 (the Session 3 baseline) plus the graph build integration and e2e tests.

- [ ] **Step 4: Grep guards**

Verify the rename was complete — there should be zero remaining references to the old function name in production code:

Run: `grep -rn "extract_declarations_from_open_meta" crates/ 2>&1 | grep -v docs/`
Expected: empty output, or only in the Session 3 deferred task notes (acceptable).

Verify pulldown-cmark is actually used:

Run: `grep -n "pulldown_cmark\|pulldown-cmark" crates/temper-cli/`
Expected: at least one hit in `actions/graph_build.rs` and one in `Cargo.toml`.

- [ ] **Step 5: Real-vault byte-diff gate (dry-run only)**

Run the command against the real vault in dry-run mode:

Run: `cargo run -p temper-cli --bin temper -- graph build --dry-run`
Expected: Report emitted; references found and "would modify" counts are surfaced. Verify with `cd /Users/petetaylor/projects/kb-vault && git status --porcelain` that nothing was written.

- [ ] **Step 6: Real-vault actual run (OPTIONAL — gated on user approval)**

If the user approves after reviewing the dry-run output:

Run: `cargo run -p temper-cli --bin temper -- graph build`
Expected: Files modified per the dry-run preview.

Verify: `cd /Users/petetaylor/projects/kb-vault && git diff --stat` and spot-check a few modified files to confirm the changes match expectations. Follow with `temper sync run` to push the changes and verify the server materialized the edges.

**Important:** Step 6 is a production-data operation. Don't run it without explicit user approval; the dry-run in Step 5 is the load-bearing verification for CI purposes.

- [ ] **Step 7: Final commit (only if polish edits needed)**

If the verification sweep surfaces any small polish items (naming, comment updates, missed error-path handling), address them with a single follow-up commit:

```bash
git add -p
git commit -m "$(cat <<'EOF'
fixup(graph-build): polish from verification sweep

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
EOF
)"
```

If no polish is needed, there's nothing to commit and the phase is complete.

---

## Summary

- **Phases A–G land the CLI pipeline** (Pass 1 walk → Pass 2 scan → Pass 3 merge/write), covered by per-task unit tests plus a two-case integration test with a fixture vault.
- **Phase H lands the server-side companion** (rename + broaden extraction, update 3 callers + 8 test call sites) so `managed_meta.temper_goal` produces a ParentOf edge without drift-prone CLI write-back.
- **Phase I lands the e2e test** proving the full graph build → sync → reconcile loop, including both CLI-originated references and server-side temper-goal extraction.
- **Phase J is the final verification sweep** mirroring the real-vault byte-diff gate pattern that became load-bearing in Session 3 of frontmatter consolidation.

Total: ~20 tasks across 10 phases. Expected to fit in a single session following the Session 3 subagent-driven-development cadence.
