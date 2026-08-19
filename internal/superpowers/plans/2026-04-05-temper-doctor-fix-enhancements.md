# Temper Doctor Fix Enhancements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enhance `temper doctor fix` to be the single vault health tool — filename slugification, file relocation, field inference, manifest reconciliation — and remove `temper normalize`.

**Architecture:** Composable fix-action pipeline. One vault walk collects `Vec<FixAction>` from five fix functions (F1–F5), sorts actions (field fixes → file moves → manifest updates), then applies them. Dry-run prints without applying. Session and research creation commands updated to produce slugified filenames.

**Tech Stack:** Rust (temper-cli, temper-core), serde_yaml, cargo-nextest for tests

---

### Task 1: Remove `temper normalize`

**Files:**
- Delete: `crates/temper-cli/src/actions/normalize.rs`
- Delete: `crates/temper-cli/src/commands/normalize.rs`
- Delete: `crates/temper-cli/tests/normalize_test.rs`
- Modify: `crates/temper-cli/src/actions/mod.rs:4` (remove `pub mod normalize;`)
- Modify: `crates/temper-cli/src/commands/mod.rs:9` (remove `pub mod normalize;`)
- Modify: `crates/temper-cli/src/cli.rs:72-80` (remove `Normalize` enum variant)
- Modify: `crates/temper-cli/src/main.rs:268-276` (remove `Commands::Normalize` match arm)

- [ ] **Step 1: Delete normalize files**

```bash
rm crates/temper-cli/src/actions/normalize.rs
rm crates/temper-cli/src/commands/normalize.rs
rm crates/temper-cli/tests/normalize_test.rs
```

- [ ] **Step 2: Remove module declarations**

In `crates/temper-cli/src/actions/mod.rs`, delete line 4:
```rust
pub mod normalize;
```

In `crates/temper-cli/src/commands/mod.rs`, delete line 9:
```rust
pub mod normalize;
```

- [ ] **Step 3: Remove CLI enum variant**

In `crates/temper-cli/src/cli.rs`, delete lines 72-80:
```rust
    /// [Deprecated: use `temper doctor fix`] Normalize vault structure and repair drift
    Normalize {
        #[arg(long)]
        context: Option<String>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        fix_slugs: bool,
    },
```

- [ ] **Step 4: Remove match arm in main.rs**

In `crates/temper-cli/src/main.rs`, delete lines 268-276:
```rust
        Commands::Normalize {
            context,
            dry_run,
            fix_slugs,
        } => {
            let config = temper_cli::config::load(cli.vault.as_deref())?;
            temper_cli::commands::normalize::run(&config, context.as_deref(), dry_run, fix_slugs)?;
            Ok(())
        }
```

- [ ] **Step 5: Check for other references**

Search for remaining `normalize` references in `crates/temper-cli/src/`:
- `discovery.rs:72` — `#[serde(rename = "normalize")]` in event types. This is an event log enum variant for historical events. **Keep it** — removing it would break deserialization of existing event logs.
- `commands/warmup.rs:242` — warmup display string. Remove the normalize line from warmup output.

- [ ] **Step 6: Verify compilation**

```bash
cargo check --workspace --all-features
```

Expected: compiles cleanly with no normalize references.

- [ ] **Step 7: Run existing tests**

```bash
cargo nextest run --workspace
```

Expected: all tests pass (normalize tests are gone, everything else unchanged).

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor: remove deprecated temper normalize command

Subsume normalize functionality into temper doctor fix.
Delete actions/normalize.rs, commands/normalize.rs, and tests."
```

---

### Task 2: Define `FixAction` enum and `FixPlan` collector

**Files:**
- Create: `crates/temper-cli/src/actions/doctor_fix.rs`
- Modify: `crates/temper-cli/src/actions/mod.rs` (add `pub mod doctor_fix;`)
- Modify: `crates/temper-cli/src/actions/doctor.rs` (re-export `FixReport` only, delegate fix logic)

- [ ] **Step 1: Write unit tests for FixAction display and FixPlan sorting**

Create `crates/temper-cli/src/actions/doctor_fix.rs`:

```rust
//! Doctor fix pipeline — composable fix actions for vault health.

use std::path::PathBuf;
use uuid::Uuid;

/// A single mutation that doctor fix can perform.
#[derive(Debug, Clone, PartialEq)]
pub enum FixAction {
    /// Rename a legacy frontmatter field to its temper-* equivalent.
    RenameField {
        path: PathBuf,
        old_key: String,
        new_key: String,
    },
    /// Set a missing frontmatter field to an inferred value.
    SetField {
        path: PathBuf,
        key: String,
        value: String,
        reason: String,
    },
    /// Rename a file on disk (slugification).
    RenameFile {
        old_path: PathBuf,
        new_path: PathBuf,
        reason: String,
    },
    /// Move a file to the correct context/doctype directory.
    RelocateFile {
        old_path: PathBuf,
        new_path: PathBuf,
        reason: String,
    },
    /// Update a manifest entry's path after a file move/rename.
    UpdateManifest {
        temper_id: Uuid,
        old_path: String,
        new_path: String,
    },
    /// Remove a stale manifest entry (file no longer exists on disk).
    RemoveManifest {
        temper_id: Uuid,
        reason: String,
    },
}

impl FixAction {
    /// Sort key: field fixes (0) before file moves (1) before manifest updates (2).
    pub fn phase(&self) -> u8 {
        match self {
            FixAction::RenameField { .. } | FixAction::SetField { .. } => 0,
            FixAction::RenameFile { .. } | FixAction::RelocateFile { .. } => 1,
            FixAction::UpdateManifest { .. } | FixAction::RemoveManifest { .. } => 2,
        }
    }

    /// The file path this action targets (for grouping in output).
    pub fn target_path(&self) -> &PathBuf {
        match self {
            FixAction::RenameField { path, .. } | FixAction::SetField { path, .. } => path,
            FixAction::RenameFile { old_path, .. } | FixAction::RelocateFile { old_path, .. } => {
                old_path
            }
            // Manifest actions don't have a PathBuf; return a placeholder.
            // Callers should handle manifest actions separately.
            FixAction::UpdateManifest { .. } | FixAction::RemoveManifest { .. } => {
                static MANIFEST: once_cell::sync::Lazy<PathBuf> =
                    once_cell::sync::Lazy::new(|| PathBuf::from(".temper/manifest.json"));
                &MANIFEST
            }
        }
    }
}

/// Collected fix actions from a vault scan, ready to sort and apply.
#[derive(Debug, Default)]
pub struct FixPlan {
    pub actions: Vec<FixAction>,
}

impl FixPlan {
    pub fn new() -> Self {
        Self {
            actions: Vec::new(),
        }
    }

    pub fn add(&mut self, action: FixAction) {
        self.actions.push(action);
    }

    pub fn extend(&mut self, actions: Vec<FixAction>) {
        self.actions.extend(actions);
    }

    /// Sort actions by phase: field fixes → file moves → manifest updates.
    pub fn sort(&mut self) {
        self.actions.sort_by_key(|a| a.phase());
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    pub fn count_by_phase(&self) -> (usize, usize, usize) {
        let fields = self.actions.iter().filter(|a| a.phase() == 0).count();
        let files = self.actions.iter().filter(|a| a.phase() == 1).count();
        let manifest = self.actions.iter().filter(|a| a.phase() == 2).count();
        (fields, files, manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fix_action_phase_ordering() {
        assert_eq!(
            FixAction::RenameField {
                path: PathBuf::from("a.md"),
                old_key: "type".into(),
                new_key: "temper-type".into(),
            }
            .phase(),
            0
        );
        assert_eq!(
            FixAction::SetField {
                path: PathBuf::from("a.md"),
                key: "slug".into(),
                value: "foo".into(),
                reason: "inferred".into(),
            }
            .phase(),
            0
        );
        assert_eq!(
            FixAction::RenameFile {
                old_path: PathBuf::from("old.md"),
                new_path: PathBuf::from("new.md"),
                reason: "slugify".into(),
            }
            .phase(),
            1
        );
        assert_eq!(
            FixAction::RelocateFile {
                old_path: PathBuf::from("old.md"),
                new_path: PathBuf::from("new.md"),
                reason: "wrong context".into(),
            }
            .phase(),
            1
        );
        assert_eq!(
            FixAction::UpdateManifest {
                temper_id: Uuid::nil(),
                old_path: "old".into(),
                new_path: "new".into(),
            }
            .phase(),
            2
        );
        assert_eq!(
            FixAction::RemoveManifest {
                temper_id: Uuid::nil(),
                reason: "stale".into(),
            }
            .phase(),
            2
        );
    }

    #[test]
    fn fix_plan_sorts_by_phase() {
        let mut plan = FixPlan::new();
        plan.add(FixAction::UpdateManifest {
            temper_id: Uuid::nil(),
            old_path: "a".into(),
            new_path: "b".into(),
        });
        plan.add(FixAction::RenameFile {
            old_path: PathBuf::from("old.md"),
            new_path: PathBuf::from("new.md"),
            reason: "slugify".into(),
        });
        plan.add(FixAction::RenameField {
            path: PathBuf::from("a.md"),
            old_key: "type".into(),
            new_key: "temper-type".into(),
        });

        plan.sort();

        let phases: Vec<u8> = plan.actions.iter().map(|a| a.phase()).collect();
        assert_eq!(phases, vec![0, 1, 2]);
    }

    #[test]
    fn fix_plan_count_by_phase() {
        let mut plan = FixPlan::new();
        plan.add(FixAction::RenameField {
            path: PathBuf::from("a.md"),
            old_key: "type".into(),
            new_key: "temper-type".into(),
        });
        plan.add(FixAction::SetField {
            path: PathBuf::from("a.md"),
            key: "slug".into(),
            value: "foo".into(),
            reason: "inferred".into(),
        });
        plan.add(FixAction::RenameFile {
            old_path: PathBuf::from("old.md"),
            new_path: PathBuf::from("new.md"),
            reason: "slugify".into(),
        });

        let (fields, files, manifest) = plan.count_by_phase();
        assert_eq!(fields, 2);
        assert_eq!(files, 1);
        assert_eq!(manifest, 0);
    }
}
```

- [ ] **Step 2: Add module declaration**

In `crates/temper-cli/src/actions/mod.rs`, add:
```rust
pub mod doctor_fix;
```

- [ ] **Step 3: Verify tests pass**

```bash
cargo nextest run --workspace -E 'test(fix_action)' -E 'test(fix_plan)'
```

Expected: 3 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/actions/doctor_fix.rs crates/temper-cli/src/actions/mod.rs
git commit -m "feat: add FixAction enum and FixPlan collector for doctor fix pipeline"
```

---

### Task 3: Implement fix functions F1 (legacy renames) and F2 (field inference)

**Files:**
- Modify: `crates/temper-cli/src/actions/doctor_fix.rs` (add `fix_legacy_fields`, `fix_missing_fields`)
- Reference: `crates/temper-cli/src/actions/doctor.rs:175-193` (LEGACY_FIELD_MAP)
- Reference: `crates/temper-cli/src/vault.rs:134-144` (slugify)

- [ ] **Step 1: Write tests for F1 legacy field renames**

Add to the `tests` module in `doctor_fix.rs`:

```rust
    fn make_frontmatter(yaml: &str) -> serde_yaml::Value {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn f1_renames_legacy_type_field() {
        let fm = make_frontmatter("type: task\ntitle: Test");
        let path = PathBuf::from("temper/task/test.md");
        let actions = fix_legacy_fields(&path, &fm);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], FixAction::RenameField { old_key, new_key, .. }
            if old_key == "type" && new_key == "temper-type"));
    }

    #[test]
    fn f1_skips_when_new_field_exists() {
        let fm = make_frontmatter("temper-type: task\ntype: task\ntitle: Test");
        let path = PathBuf::from("temper/task/test.md");
        let actions = fix_legacy_fields(&path, &fm);
        // Still emits rename — the apply step handles "both exist" by dropping old
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn f1_no_actions_for_clean_frontmatter() {
        let fm = make_frontmatter("temper-type: task\ntemper-id: \"019...\"\ntitle: Test");
        let path = PathBuf::from("temper/task/test.md");
        let actions = fix_legacy_fields(&path, &fm);
        assert!(actions.is_empty());
    }
```

- [ ] **Step 2: Implement `fix_legacy_fields`**

Add to `doctor_fix.rs` (above tests module):

```rust
use std::path::Path;

use crate::vault;

/// Legacy field rename map: (old_name, new_name).
/// Duplicated from doctor.rs — will be the single source after refactor.
const LEGACY_FIELD_MAP: &[(&str, &str)] = &[
    ("id", "temper-id"),
    ("type", "temper-type"),
    ("doc_type", "temper-type"),
    ("context", "temper-context"),
    ("project", "temper-context"),
    ("ingestion_source", "temper-source"),
    ("created", "temper-created"),
    ("updated", "temper-updated"),
    ("stage", "temper-stage"),
    ("mode", "temper-mode"),
    ("effort", "temper-effort"),
    ("goal", "temper-goal"),
    ("seq", "temper-seq"),
    ("branch", "temper-branch"),
    ("pr", "temper-pr"),
    ("status", "temper-status"),
    ("legacy_id", "temper-legacy-id"),
];

/// F1: Detect legacy field names and emit rename actions.
pub fn fix_legacy_fields(path: &Path, fm: &serde_yaml::Value) -> Vec<FixAction> {
    let mut actions = Vec::new();
    for (old_key, new_key) in LEGACY_FIELD_MAP {
        if fm.get(*old_key).is_some() {
            actions.push(FixAction::RenameField {
                path: path.to_path_buf(),
                old_key: old_key.to_string(),
                new_key: new_key.to_string(),
            });
        }
    }
    actions
}
```

- [ ] **Step 3: Run F1 tests**

```bash
cargo nextest run --workspace -E 'test(f1_)'
```

Expected: 3 tests pass.

- [ ] **Step 4: Write tests for individual inference rules and the aggregate F2**

Add to the `tests` module. Tests target individual rule functions for isolation,
plus the aggregate `fix_missing_fields` for integration:

```rust
    fn make_infer_ctx<'a>(
        path: &'a Path,
        filename: &'a str,
        fm: &'a serde_yaml::Value,
        path_context: &str,
        path_doc_type: &str,
        effective_doc_type: &str,
    ) -> InferContext<'a> {
        InferContext {
            path,
            filename,
            fm,
            path_context: path_context.to_string(),
            path_doc_type: path_doc_type.to_string(),
            effective_doc_type: effective_doc_type.to_string(),
        }
    }

    #[test]
    fn rule_infer_temper_id_generates_when_missing() {
        let fm = make_frontmatter("temper-type: task\ntitle: Test");
        let path = PathBuf::from("/vault/temper/task/test.md");
        let ctx = make_infer_ctx(&path, "test.md", &fm, "temper", "task", "task");
        let result = infer_temper_id(&ctx);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "temper-id");
    }

    #[test]
    fn rule_infer_temper_id_skips_when_present() {
        let fm = make_frontmatter("temper-id: \"019abc\"\ntemper-type: task");
        let path = PathBuf::from("/vault/temper/task/test.md");
        let ctx = make_infer_ctx(&path, "test.md", &fm, "temper", "task", "task");
        assert!(infer_temper_id(&ctx).is_none());
    }

    #[test]
    fn rule_infer_temper_type_from_directory() {
        let fm = make_frontmatter("title: Test");
        let path = PathBuf::from("/vault/temper/task/test.md");
        let ctx = make_infer_ctx(&path, "test.md", &fm, "temper", "task", "task");
        let result = infer_temper_type(&ctx);
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, "task");
    }

    #[test]
    fn rule_infer_temper_context_from_directory() {
        let fm = make_frontmatter("temper-type: task\ntitle: Test");
        let path = PathBuf::from("/vault/temper/task/test.md");
        let ctx = make_infer_ctx(&path, "test.md", &fm, "temper", "task", "task");
        let result = infer_temper_context(&ctx);
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, "temper");
    }

    #[test]
    fn rule_infer_title_from_filename() {
        let fm = make_frontmatter("temper-type: task");
        let path = PathBuf::from("/vault/temper/task/my-feature-x.md");
        let ctx = make_infer_ctx(&path, "my-feature-x.md", &fm, "temper", "task", "task");
        let result = infer_title(&ctx);
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, "My Feature X");
    }

    #[test]
    fn rule_infer_slug_from_title() {
        let fm = make_frontmatter("temper-type: task\ntitle: My Feature X");
        let path = PathBuf::from("/vault/temper/task/my-feature-x.md");
        let ctx = make_infer_ctx(&path, "my-feature-x.md", &fm, "temper", "task", "task");
        let result = infer_slug(&ctx);
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, "my-feature-x");
    }

    #[test]
    fn rule_infer_date_from_filename_for_session() {
        let fm = make_frontmatter("temper-type: session\ntitle: Test");
        let path = PathBuf::from("/vault/temper/session/2026-04-05-my-session.md");
        let ctx = make_infer_ctx(&path, "2026-04-05-my-session.md", &fm, "temper", "session", "session");
        let result = infer_date(&ctx);
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, "2026-04-05");
    }

    #[test]
    fn rule_infer_date_skips_for_tasks() {
        let fm = make_frontmatter("temper-type: task\ntitle: Test");
        let path = PathBuf::from("/vault/temper/task/2026-04-05-test.md");
        let ctx = make_infer_ctx(&path, "2026-04-05-test.md", &fm, "temper", "task", "task");
        assert!(infer_date(&ctx).is_none());
    }

    #[test]
    fn rule_infer_temper_created_from_date() {
        let fm = make_frontmatter("temper-type: session\ntitle: Test\ndate: 2026-04-05");
        let path = PathBuf::from("/vault/temper/session/2026-04-05-test.md");
        let ctx = make_infer_ctx(&path, "2026-04-05-test.md", &fm, "temper", "session", "session");
        let result = infer_temper_created(&ctx);
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, "2026-04-05T00:00:00Z");
    }

    #[test]
    fn rule_infer_stage_backlog_for_tasks() {
        let fm = make_frontmatter("temper-type: task\ntitle: Test\nslug: test");
        let path = PathBuf::from("/vault/temper/task/test.md");
        let ctx = make_infer_ctx(&path, "test.md", &fm, "temper", "task", "task");
        let result = infer_temper_stage(&ctx);
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, "backlog");
    }

    #[test]
    fn rule_infer_stage_skips_non_tasks() {
        let fm = make_frontmatter("temper-type: session\ntitle: Test\ndate: 2026-04-05");
        let path = PathBuf::from("/vault/temper/session/2026-04-05-test.md");
        let ctx = make_infer_ctx(&path, "2026-04-05-test.md", &fm, "temper", "session", "session");
        assert!(infer_temper_stage(&ctx).is_none());
    }

    // Aggregate test: fix_missing_fields runs all rules
    #[test]
    fn f2_skips_fields_already_present() {
        let fm = make_frontmatter(
            "temper-type: task\ntemper-id: \"019abc\"\ntemper-context: temper\ntemper-created: \"2026-04-05T00:00:00Z\"\ntitle: Test\nslug: test\ntemper-stage: done"
        );
        let path = PathBuf::from("/vault/temper/task/test.md");
        let vault_root = Path::new("/vault");
        let actions = fix_missing_fields(&path, &fm, vault_root);
        assert!(actions.is_empty());
    }
```

- [ ] **Step 5: Implement `fix_missing_fields`**

Add to `doctor_fix.rs`:

```rust
use crate::ids;

/// Extract a date prefix (YYYY-MM-DD) from a filename if present.
fn extract_date_from_filename(filename: &str) -> Option<String> {
    if filename.len() >= 10 {
        let prefix = &filename[..10];
        // Validate it looks like a date
        if prefix.chars().nth(4) == Some('-')
            && prefix.chars().nth(7) == Some('-')
            && prefix[..4].chars().all(|c| c.is_ascii_digit())
            && prefix[5..7].chars().all(|c| c.is_ascii_digit())
            && prefix[8..10].chars().all(|c| c.is_ascii_digit())
        {
            return Some(prefix.to_string());
        }
    }
    None
}

/// Extract the slug portion from a filename, stripping date prefix and extension.
fn slug_from_filename(filename: &str) -> String {
    let stem = filename.strip_suffix(".md").unwrap_or(filename);
    let without_date = if extract_date_from_filename(stem).is_some() && stem.len() > 10 {
        // Strip date prefix and any separator (hyphen, space-emdash-space, etc.)
        let rest = &stem[10..];
        rest.trim_start_matches(|c: char| c == '-' || c == ' ' || c == '\u{2014}')
    } else {
        stem
    };
    vault::slugify(without_date)
}

/// Humanize a slug: "my-feature-x" → "My Feature X".
fn humanize_slug(slug: &str) -> String {
    slug.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    format!("{upper}{}", chars.as_str())
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Infer context and doc_type from the file's path relative to vault root.
/// Returns (context, doc_type) or None if path doesn't match expected structure.
fn infer_from_path(path: &Path, vault_root: &Path) -> Option<(String, String)> {
    let rel = path.strip_prefix(vault_root).ok()?;
    let parts: Vec<&str> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();

    if parts.len() != 3 {
        return None;
    }

    // Research legacy: research/{context}/{file}.md → context=parts[1], doc_type=research
    if parts[0] == "research" {
        return Some((parts[1].to_string(), "research".to_string()));
    }

    // Standard: {context}/{doc_type}/{file}.md
    Some((parts[0].to_string(), parts[1].to_string()))
}

/// Get a frontmatter string value.
fn fm_str(fm: &serde_yaml::Value, key: &str) -> Option<String> {
    fm.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

/// Context passed to each inference rule.
pub struct InferContext<'a> {
    pub path: &'a Path,
    pub filename: &'a str,
    pub fm: &'a serde_yaml::Value,
    pub path_context: String,
    pub path_doc_type: String,
    pub effective_doc_type: String,
}

/// A single field inference rule: check if the field is missing, and if so, compute a value.
/// Returns `None` if the field is already present or can't be inferred.
type InferFn = fn(&InferContext) -> Option<(String, String, String)>; // (key, value, reason)

/// Inference rule: generate temper-id if missing.
fn infer_temper_id(ctx: &InferContext) -> Option<(String, String, String)> {
    if ctx.fm.get("temper-id").is_none() && ctx.fm.get("id").is_none() {
        Some(("temper-id".into(), ids::generate_id(), "generated UUIDv7".into()))
    } else {
        None
    }
}

/// Inference rule: infer temper-type from directory path.
fn infer_temper_type(ctx: &InferContext) -> Option<(String, String, String)> {
    if ctx.fm.get("temper-type").is_none()
        && ctx.fm.get("type").is_none()
        && ctx.fm.get("doc_type").is_none()
        && !ctx.path_doc_type.is_empty()
    {
        Some((
            "temper-type".into(),
            ctx.path_doc_type.clone(),
            format!("inferred from directory: {}", ctx.path_doc_type),
        ))
    } else {
        None
    }
}

/// Inference rule: infer temper-context from directory path.
fn infer_temper_context(ctx: &InferContext) -> Option<(String, String, String)> {
    if ctx.fm.get("temper-context").is_none()
        && ctx.fm.get("context").is_none()
        && ctx.fm.get("project").is_none()
        && !ctx.path_context.is_empty()
    {
        Some((
            "temper-context".into(),
            ctx.path_context.clone(),
            format!("inferred from directory: {}", ctx.path_context),
        ))
    } else {
        None
    }
}

/// Inference rule: infer title from filename.
fn infer_title(ctx: &InferContext) -> Option<(String, String, String)> {
    if ctx.fm.get("title").is_none() {
        let slug = slug_from_filename(ctx.filename);
        if !slug.is_empty() {
            return Some(("title".into(), humanize_slug(&slug), "inferred from filename".into()));
        }
    }
    None
}

/// Inference rule: infer slug from title or filename.
fn infer_slug(ctx: &InferContext) -> Option<(String, String, String)> {
    if ctx.fm.get("slug").is_none() {
        let slug = if let Some(title) = fm_str(ctx.fm, "title") {
            vault::slugify(&title)
        } else {
            slug_from_filename(ctx.filename)
        };
        if !slug.is_empty() {
            return Some(("slug".into(), slug, "inferred from title/filename".into()));
        }
    }
    None
}

/// Inference rule: infer date for session/research from filename or temper-created.
fn infer_date(ctx: &InferContext) -> Option<(String, String, String)> {
    if (ctx.effective_doc_type != "session" && ctx.effective_doc_type != "research")
        || ctx.fm.get("date").is_some()
    {
        return None;
    }
    if let Some(date) = extract_date_from_filename(ctx.filename) {
        return Some(("date".into(), date, "extracted from filename date prefix".into()));
    }
    if let Some(created) = fm_str(ctx.fm, "temper-created") {
        if created.len() >= 10 {
            return Some(("date".into(), created[..10].to_string(), "extracted from temper-created".into()));
        }
    }
    None
}

/// Inference rule: infer temper-created from date field or filename.
fn infer_temper_created(ctx: &InferContext) -> Option<(String, String, String)> {
    if ctx.fm.get("temper-created").is_some() || ctx.fm.get("created").is_some() {
        return None;
    }
    let date_str = fm_str(ctx.fm, "date")
        .or_else(|| extract_date_from_filename(ctx.filename));
    date_str.map(|date| (
        "temper-created".into(),
        format!("{date}T00:00:00Z"),
        "derived from date".into(),
    ))
}

/// Inference rule: default temper-stage to backlog for tasks.
fn infer_temper_stage(ctx: &InferContext) -> Option<(String, String, String)> {
    if ctx.effective_doc_type == "task"
        && ctx.fm.get("temper-stage").is_none()
        && ctx.fm.get("stage").is_none()
    {
        Some(("temper-stage".into(), "backlog".into(), "default for tasks".into()))
    } else {
        None
    }
}

/// All inference rules, applied in order.
const INFER_RULES: &[InferFn] = &[
    infer_temper_id,
    infer_temper_type,
    infer_temper_context,
    infer_title,
    infer_slug,
    infer_date,
    infer_temper_created,
    infer_temper_stage,
];

/// F2: Infer missing required fields by running declarative rules.
pub fn fix_missing_fields(
    path: &Path,
    fm: &serde_yaml::Value,
    vault_root: &Path,
) -> Vec<FixAction> {
    let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
    let (path_context, path_doc_type) = infer_from_path(path, vault_root).unwrap_or_default();
    let effective_doc_type = fm_str(fm, "temper-type")
        .or_else(|| fm_str(fm, "type"))
        .or_else(|| fm_str(fm, "doc_type"))
        .unwrap_or_else(|| path_doc_type.clone());

    let ctx = InferContext {
        path,
        filename,
        fm,
        path_context,
        path_doc_type,
        effective_doc_type,
    };

    INFER_RULES
        .iter()
        .filter_map(|rule| rule(&ctx))
        .map(|(key, value, reason)| FixAction::SetField {
            path: path.to_path_buf(),
            key,
            value,
            reason,
        })
        .collect()
}
```

- [ ] **Step 6: Run F2 tests**

```bash
cargo nextest run --workspace -E 'test(f2_)'
```

Expected: 9 tests pass.

- [ ] **Step 7: Write tests for helper functions**

Add to `tests` module:

```rust
    #[test]
    fn extract_date_from_filename_valid() {
        assert_eq!(
            extract_date_from_filename("2026-04-05-my-session.md"),
            Some("2026-04-05".to_string())
        );
    }

    #[test]
    fn extract_date_from_filename_no_date() {
        assert_eq!(extract_date_from_filename("my-task.md"), None);
    }

    #[test]
    fn slug_from_filename_strips_date_and_emdash() {
        assert_eq!(
            slug_from_filename("2026-04-05 \u{2014} my-session.md"),
            "my-session"
        );
    }

    #[test]
    fn slug_from_filename_strips_date_hyphen() {
        assert_eq!(
            slug_from_filename("2026-04-05-my-session.md"),
            "my-session"
        );
    }

    #[test]
    fn slug_from_filename_no_date() {
        assert_eq!(slug_from_filename("my-task.md"), "my-task");
    }

    #[test]
    fn humanize_slug_capitalizes_words() {
        assert_eq!(humanize_slug("my-feature-x"), "My Feature X");
    }

    #[test]
    fn humanize_slug_single_word() {
        assert_eq!(humanize_slug("task"), "Task");
    }
```

- [ ] **Step 8: Run all tests**

```bash
cargo nextest run --workspace -E 'test(doctor_fix)' -E 'test(f1_)' -E 'test(f2_)' -E 'test(extract_date)' -E 'test(slug_from)' -E 'test(humanize)'
```

Expected: all pass.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-cli/src/actions/doctor_fix.rs
git commit -m "feat: implement F1 (legacy renames) and F2 (field inference) fix functions"
```

---

### Task 4: Implement fix functions F3 (relocation) and F4 (filename slugification)

**Files:**
- Modify: `crates/temper-cli/src/actions/doctor_fix.rs`
- Reference: `crates/temper-cli/src/actions/ingest.rs:405-419` (dedup_vault_slug)

- [ ] **Step 1: Write tests for F3 relocation**

Add to `tests` module in `doctor_fix.rs`:

```rust
    #[test]
    fn f3_relocates_research_from_legacy_path() {
        let fm = make_frontmatter("temper-type: research\ntemper-context: temper\ntitle: Test");
        let path = PathBuf::from("/vault/research/temper/test.md");
        let vault_root = Path::new("/vault");
        let actions = fix_relocation(&path, &fm, vault_root);
        assert_eq!(actions.len(), 1);
        if let FixAction::RelocateFile { new_path, .. } = &actions[0] {
            assert_eq!(new_path, &PathBuf::from("/vault/temper/research/test.md"));
        } else {
            panic!("expected RelocateFile");
        }
    }

    #[test]
    fn f3_relocates_wrong_context() {
        let fm = make_frontmatter("temper-type: task\ntemper-context: temper\ntitle: Test");
        let path = PathBuf::from("/vault/general/task/test.md");
        let vault_root = Path::new("/vault");
        let actions = fix_relocation(&path, &fm, vault_root);
        assert_eq!(actions.len(), 1);
        if let FixAction::RelocateFile { new_path, .. } = &actions[0] {
            assert_eq!(new_path, &PathBuf::from("/vault/temper/task/test.md"));
        } else {
            panic!("expected RelocateFile");
        }
    }

    #[test]
    fn f3_no_relocation_when_correct() {
        let fm = make_frontmatter("temper-type: task\ntemper-context: temper\ntitle: Test");
        let path = PathBuf::from("/vault/temper/task/test.md");
        let vault_root = Path::new("/vault");
        let actions = fix_relocation(&path, &fm, vault_root);
        assert!(actions.is_empty());
    }
```

- [ ] **Step 2: Implement `fix_relocation`**

```rust
/// F3: Detect files in wrong context/doctype directory and emit relocation actions.
pub fn fix_relocation(
    path: &Path,
    fm: &serde_yaml::Value,
    vault_root: &Path,
) -> Vec<FixAction> {
    let mut actions = Vec::new();

    let fm_context = fm_str(fm, "temper-context")
        .or_else(|| fm_str(fm, "context"))
        .or_else(|| fm_str(fm, "project"));
    let fm_doc_type = fm_str(fm, "temper-type")
        .or_else(|| fm_str(fm, "type"))
        .or_else(|| fm_str(fm, "doc_type"));

    // Need both context and doc_type from frontmatter to determine correct location
    let (Some(context), Some(doc_type)) = (fm_context, fm_doc_type) else {
        return actions;
    };

    let filename = path.file_name().unwrap_or_default();

    // Determine expected directory
    let expected_dir = if doc_type == "research" {
        vault_root.join(&context).join("research")
    } else {
        vault_root.join(&context).join(&doc_type)
    };

    let expected_path = expected_dir.join(filename);
    let current_dir = path.parent().unwrap_or(path);

    if current_dir != expected_dir {
        actions.push(FixAction::RelocateFile {
            old_path: path.to_path_buf(),
            new_path: expected_path,
            reason: format!(
                "frontmatter says context={context}, type={doc_type}; file is in wrong directory"
            ),
        });
    }

    actions
}
```

- [ ] **Step 3: Run F3 tests**

```bash
cargo nextest run --workspace -E 'test(f3_)'
```

Expected: 3 tests pass.

- [ ] **Step 4: Write tests for F4 filename slugification**

```rust
    #[test]
    fn f4_slugifies_emdash_session_filename() {
        let fm = make_frontmatter("temper-type: session\ntitle: My Session\ndate: 2026-04-05");
        let path = PathBuf::from("/vault/temper/session/2026-04-05 \u{2014} my-session.md");
        let vault_root = Path::new("/vault");
        let actions = fix_filename(&path, &fm, vault_root);
        assert_eq!(actions.len(), 1);
        if let FixAction::RenameFile { new_path, .. } = &actions[0] {
            assert_eq!(
                new_path,
                &PathBuf::from("/vault/temper/session/2026-04-05-my-session.md")
            );
        } else {
            panic!("expected RenameFile");
        }
    }

    #[test]
    fn f4_strips_date_from_task_filename() {
        let fm = make_frontmatter("temper-type: task\ntitle: My Task\nslug: my-task");
        let path = PathBuf::from("/vault/temper/task/2026-04-05-my-task.md");
        let vault_root = Path::new("/vault");
        let actions = fix_filename(&path, &fm, vault_root);
        assert_eq!(actions.len(), 1);
        if let FixAction::RenameFile { new_path, .. } = &actions[0] {
            assert_eq!(
                new_path,
                &PathBuf::from("/vault/temper/task/my-task.md")
            );
        } else {
            panic!("expected RenameFile");
        }
    }

    #[test]
    fn f4_no_rename_when_already_correct_task() {
        let fm = make_frontmatter("temper-type: task\ntitle: My Task\nslug: my-task");
        let path = PathBuf::from("/vault/temper/task/my-task.md");
        let vault_root = Path::new("/vault");
        let actions = fix_filename(&path, &fm, vault_root);
        assert!(actions.is_empty());
    }

    #[test]
    fn f4_no_rename_when_already_correct_session() {
        let fm = make_frontmatter("temper-type: session\ntitle: My Session\ndate: 2026-04-05");
        let path = PathBuf::from("/vault/temper/session/2026-04-05-my-session.md");
        let vault_root = Path::new("/vault");
        let actions = fix_filename(&path, &fm, vault_root);
        assert!(actions.is_empty());
    }

    #[test]
    fn f4_slugifies_non_slug_task_filename() {
        let fm = make_frontmatter("temper-type: task\ntitle: My Feature!\nslug: my-feature");
        let path = PathBuf::from("/vault/temper/task/My Feature!.md");
        let vault_root = Path::new("/vault");
        let actions = fix_filename(&path, &fm, vault_root);
        assert_eq!(actions.len(), 1);
        if let FixAction::RenameFile { new_path, .. } = &actions[0] {
            assert_eq!(
                new_path,
                &PathBuf::from("/vault/temper/task/my-feature.md")
            );
        } else {
            panic!("expected RenameFile");
        }
    }
```

- [ ] **Step 5: Implement `fix_filename`**

```rust
/// Doc types that keep a date prefix in their filename.
const DATE_PREFIX_DOC_TYPES: &[&str] = &["session", "research"];

/// F4: Ensure filename matches doc-type-specific slug conventions.
pub fn fix_filename(
    path: &Path,
    fm: &serde_yaml::Value,
    vault_root: &Path,
) -> Vec<FixAction> {
    let mut actions = Vec::new();

    let filename = match path.file_name().and_then(|f| f.to_str()) {
        Some(f) => f,
        None => return actions,
    };

    let doc_type = fm_str(fm, "temper-type")
        .or_else(|| fm_str(fm, "type"))
        .or_else(|| fm_str(fm, "doc_type"))
        .unwrap_or_default();

    // Determine the slug to use
    let slug = fm_str(fm, "slug").unwrap_or_else(|| {
        if let Some(title) = fm_str(fm, "title") {
            vault::slugify(&title)
        } else {
            slug_from_filename(filename)
        }
    });

    if slug.is_empty() {
        return actions;
    }

    // Build expected filename based on doc type
    let expected_filename = if DATE_PREFIX_DOC_TYPES.contains(&doc_type.as_str()) {
        // Session/research: {date}-{slug}.md
        let date = fm_str(fm, "date")
            .or_else(|| extract_date_from_filename(filename))
            .unwrap_or_default();
        if date.is_empty() {
            format!("{slug}.md")
        } else {
            format!("{date}-{slug}.md")
        }
    } else {
        // Task/goal/decision/concept: {slug}.md
        format!("{slug}.md")
    };

    if filename != expected_filename {
        let parent = path.parent().unwrap_or(vault_root);
        let mut new_path = parent.join(&expected_filename);

        // Dedup: if target exists and isn't the same file, append suffix
        if new_path.exists() && new_path != *path {
            let stem = expected_filename.strip_suffix(".md").unwrap_or(&expected_filename);
            for i in 2..1000 {
                let candidate = parent.join(format!("{stem}-{i}.md"));
                if !candidate.exists() {
                    new_path = candidate;
                    break;
                }
            }
        }

        // Don't emit a rename if old == new (can happen after normalization)
        if new_path != *path {
            actions.push(FixAction::RenameFile {
                old_path: path.to_path_buf(),
                new_path,
                reason: format!("slugify: expected {expected_filename}"),
            });
        }
    }

    actions
}
```

- [ ] **Step 6: Run F4 tests**

```bash
cargo nextest run --workspace -E 'test(f4_)'
```

Expected: 5 tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/actions/doctor_fix.rs
git commit -m "feat: implement F3 (relocation) and F4 (filename slugification) fix functions"
```

---

### Task 5: Implement fix function F5 (manifest reconciliation)

**Files:**
- Modify: `crates/temper-cli/src/actions/doctor_fix.rs`
- Reference: `crates/temper-cli/src/manifest_io.rs` (load_manifest, save_manifest)
- Reference: `crates/temper-core/src/types/manifest.rs` (Manifest, ManifestEntry)

- [ ] **Step 1: Write tests for F5**

Add to `tests` module:

```rust
    use temper_core::types::manifest::{Manifest, ManifestEntry, ManifestEntryState};
    use chrono::Utc;

    fn make_manifest_with_entry(id: Uuid, path: &str) -> Manifest {
        let mut manifest = Manifest {
            device_id: "test-device".into(),
            last_sync: None,
            entries: std::collections::HashMap::new(),
        };
        manifest.entries.insert(
            id,
            ManifestEntry {
                path: path.to_string(),
                body_hash: "sha256:abc".into(),
                remote_body_hash: "sha256:abc".into(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None,
            },
        );
        manifest
    }

    #[test]
    fn f5_updates_manifest_for_rename() {
        let id = Uuid::now_v7();
        let manifest = make_manifest_with_entry(id, "temper/task/old-name.md");
        let rename = FixAction::RenameFile {
            old_path: PathBuf::from("/vault/temper/task/old-name.md"),
            new_path: PathBuf::from("/vault/temper/task/new-name.md"),
            reason: "slugify".into(),
        };
        let vault_root = Path::new("/vault");
        let actions = fix_manifest_for_moves(&[rename], &manifest, vault_root);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], FixAction::UpdateManifest { temper_id, new_path, .. }
            if *temper_id == id && new_path == "temper/task/new-name.md"));
    }

    #[test]
    fn f5_removes_stale_manifest_entries() {
        let id = Uuid::now_v7();
        let manifest = make_manifest_with_entry(id, "temper/task/deleted-file.md");
        let vault_root = Path::new("/vault");
        // The file doesn't exist on disk
        let actions = fix_stale_manifest_entries(&manifest, vault_root);
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], FixAction::RemoveManifest { temper_id, .. }
            if *temper_id == id));
    }
```

- [ ] **Step 2: Implement `fix_manifest_for_moves` and `fix_stale_manifest_entries`**

```rust
use temper_core::types::manifest::Manifest;

/// F5a: For each file rename/relocate action, emit a manifest update if the entry exists.
pub fn fix_manifest_for_moves(
    move_actions: &[FixAction],
    manifest: &Manifest,
    vault_root: &Path,
) -> Vec<FixAction> {
    let mut actions = Vec::new();

    for action in move_actions {
        let (old_path, new_path) = match action {
            FixAction::RenameFile {
                old_path, new_path, ..
            }
            | FixAction::RelocateFile {
                old_path, new_path, ..
            } => (old_path, new_path),
            _ => continue,
        };

        let old_rel = old_path
            .strip_prefix(vault_root)
            .unwrap_or(old_path)
            .to_string_lossy()
            .to_string();
        let new_rel = new_path
            .strip_prefix(vault_root)
            .unwrap_or(new_path)
            .to_string_lossy()
            .to_string();

        // Find manifest entry by path
        for (id, entry) in &manifest.entries {
            if entry.path == old_rel {
                actions.push(FixAction::UpdateManifest {
                    temper_id: *id,
                    old_path: old_rel.clone(),
                    new_path: new_rel.clone(),
                });
                break;
            }
        }
    }

    actions
}

/// F5b: Find manifest entries whose files no longer exist on disk.
pub fn fix_stale_manifest_entries(manifest: &Manifest, vault_root: &Path) -> Vec<FixAction> {
    let mut actions = Vec::new();
    for (id, entry) in &manifest.entries {
        let full_path = vault_root.join(&entry.path);
        if !full_path.exists() {
            actions.push(FixAction::RemoveManifest {
                temper_id: *id,
                reason: format!("vault file not found: {}", entry.path),
            });
        }
    }
    actions
}
```

- [ ] **Step 3: Run F5 tests**

```bash
cargo nextest run --workspace -E 'test(f5_)'
```

Expected: 2 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/actions/doctor_fix.rs
git commit -m "feat: implement F5 (manifest reconciliation) fix function"
```

---

### Task 6: Implement the action applicator and wire into doctor fix

**Files:**
- Modify: `crates/temper-cli/src/actions/doctor_fix.rs` (add `apply_plan`)
- Modify: `crates/temper-cli/src/actions/doctor.rs` (rewrite `fix()` to use pipeline)
- Modify: `crates/temper-cli/src/commands/doctor.rs` (update `run_fix` output)

- [ ] **Step 1: Write test for apply_plan**

Add to `tests` module in `doctor_fix.rs`:

```rust
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn apply_plan_renames_field_in_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.md");
        fs::write(&file, "---\ntype: task\ntitle: Test\n---\nBody\n").unwrap();

        let mut plan = FixPlan::new();
        plan.add(FixAction::RenameField {
            path: file.clone(),
            old_key: "type".into(),
            new_key: "temper-type".into(),
        });

        let report = apply_plan(&mut plan, false).unwrap();
        assert_eq!(report.fields_renamed, 1);

        let content = fs::read_to_string(&file).unwrap();
        assert!(content.contains("temper-type: task"));
        assert!(!content.contains("\ntype: task"));
    }

    #[test]
    fn apply_plan_sets_missing_field() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.md");
        fs::write(&file, "---\ntemper-type: task\ntitle: Test\n---\nBody\n").unwrap();

        let mut plan = FixPlan::new();
        plan.add(FixAction::SetField {
            path: file.clone(),
            key: "slug".into(),
            value: "test".into(),
            reason: "inferred".into(),
        });

        let report = apply_plan(&mut plan, false).unwrap();
        assert_eq!(report.fields_set, 1);

        let content = fs::read_to_string(&file).unwrap();
        assert!(content.contains("slug: test"));
    }

    #[test]
    fn apply_plan_renames_file() {
        let dir = TempDir::new().unwrap();
        let old = dir.path().join("old.md");
        let new = dir.path().join("new.md");
        fs::write(&old, "---\ntitle: Test\n---\n").unwrap();

        let mut plan = FixPlan::new();
        plan.add(FixAction::RenameFile {
            old_path: old.clone(),
            new_path: new.clone(),
            reason: "slugify".into(),
        });

        apply_plan(&mut plan, false).unwrap();
        assert!(!old.exists());
        assert!(new.exists());
    }

    #[test]
    fn apply_plan_dry_run_changes_nothing() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.md");
        let original = "---\ntype: task\ntitle: Test\n---\nBody\n";
        fs::write(&file, original).unwrap();

        let mut plan = FixPlan::new();
        plan.add(FixAction::RenameField {
            path: file.clone(),
            old_key: "type".into(),
            new_key: "temper-type".into(),
        });

        apply_plan(&mut plan, true).unwrap();

        let content = fs::read_to_string(&file).unwrap();
        assert_eq!(content, original);
    }
```

- [ ] **Step 2: Implement `apply_plan` and `ApplyReport`**

Add to `doctor_fix.rs`:

```rust
use crate::error::Result;

/// Report from applying a fix plan.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ApplyReport {
    pub fields_renamed: u32,
    pub fields_set: u32,
    pub files_renamed: u32,
    pub files_relocated: u32,
    pub manifest_updated: u32,
    pub manifest_removed: u32,
}

/// Apply a sorted fix plan. In dry_run mode, counts actions without executing.
pub fn apply_plan(plan: &mut FixPlan, dry_run: bool) -> Result<ApplyReport> {
    plan.sort();
    let mut report = ApplyReport::default();

    for action in &plan.actions {
        match action {
            FixAction::RenameField {
                path,
                old_key,
                new_key,
            } => {
                if !dry_run {
                    let content = fs::read_to_string(path)?;
                    let fm = vault::parse_frontmatter(&content);
                    if fm.is_some() {
                        let has_new = fm.as_ref().unwrap().get(new_key.as_str()).is_some();
                        let updated = if has_new {
                            // Both exist — drop the old
                            vault::remove_frontmatter_field(&content, old_key)
                        } else {
                            vault::rename_frontmatter_field(&content, old_key, new_key)
                        };
                        fs::write(path, updated)?;
                    }
                }
                report.fields_renamed += 1;
            }
            FixAction::SetField {
                path, key, value, ..
            } => {
                if !dry_run {
                    let content = fs::read_to_string(path)?;
                    let needs_quotes = value.contains(' ')
                        || value.contains(':')
                        || value.contains('"')
                        || key == "temper-id"
                        || key == "temper-created";
                    let formatted = if needs_quotes {
                        format!("\"{value}\"")
                    } else {
                        value.clone()
                    };
                    let updated = vault::insert_frontmatter_field(&content, key, &formatted);
                    fs::write(path, updated)?;
                }
                report.fields_set += 1;
            }
            FixAction::RenameFile {
                old_path, new_path, ..
            } => {
                if !dry_run {
                    if let Some(parent) = new_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::rename(old_path, new_path)?;
                }
                report.files_renamed += 1;
            }
            FixAction::RelocateFile {
                old_path, new_path, ..
            } => {
                if !dry_run {
                    if let Some(parent) = new_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::rename(old_path, new_path)?;
                }
                report.files_relocated += 1;
            }
            FixAction::UpdateManifest { .. } | FixAction::RemoveManifest { .. } => {
                // Manifest actions are applied in a separate batch after all file ops.
                // Count them here; actual manifest write happens in the orchestrator.
                match action {
                    FixAction::UpdateManifest { .. } => report.manifest_updated += 1,
                    FixAction::RemoveManifest { .. } => report.manifest_removed += 1,
                    _ => {}
                }
            }
        }
    }

    Ok(report)
}

/// Apply manifest actions to the manifest in memory. Caller must save afterward.
pub fn apply_manifest_actions(plan: &FixPlan, manifest: &mut Manifest) {
    for action in &plan.actions {
        match action {
            FixAction::UpdateManifest {
                temper_id,
                new_path,
                ..
            } => {
                if let Some(entry) = manifest.entries.get_mut(temper_id) {
                    entry.path = new_path.clone();
                }
            }
            FixAction::RemoveManifest { temper_id, .. } => {
                manifest.entries.remove(temper_id);
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 3: Run apply tests**

```bash
cargo nextest run --workspace -E 'test(apply_plan)'
```

Expected: 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/actions/doctor_fix.rs
git commit -m "feat: implement apply_plan action applicator with dry-run support"
```

---

### Task 7: Wire the pipeline into `doctor fix` command

**Files:**
- Modify: `crates/temper-cli/src/actions/doctor.rs` (rewrite `fix()` to use pipeline)
- Modify: `crates/temper-cli/src/commands/doctor.rs` (update output formatting)

- [ ] **Step 1: Rewrite `doctor::fix()` to use the pipeline**

Replace the `fix()` function in `crates/temper-cli/src/actions/doctor.rs` (lines 197-234) and the supporting `fix_directory`/`fix_file` functions (lines 237-332) with:

```rust
use crate::actions::doctor_fix::{
    self, apply_manifest_actions, apply_plan, fix_filename, fix_legacy_fields,
    fix_manifest_for_moves, fix_missing_fields, fix_relocation, fix_stale_manifest_entries,
    ApplyReport, FixPlan,
};
use crate::manifest_io;

/// Auto-fix issues in the vault using the composable fix pipeline.
pub fn fix(config: &Config, context_filter: Option<&str>, dry_run: bool) -> Result<ApplyReport> {
    let mut plan = FixPlan::new();

    let contexts_to_scan: Vec<String> = if let Some(ctx) = context_filter {
        vec![ctx.to_string()]
    } else {
        config.contexts.clone()
    };

    // Walk standard entity doc type directories
    for doc_type in ENTITY_DOC_TYPES {
        for ctx in &contexts_to_scan {
            let dir = config.doc_type_dir(ctx, doc_type);
            if !dir.is_dir() {
                continue;
            }
            collect_fixes_for_directory(&dir, &config.vault_root, &mut plan)?;
        }
    }

    // Walk research directory: {vault_root}/research/{context}/
    let research_root = config.vault_root.join("research");
    if research_root.is_dir() {
        for ctx in &contexts_to_scan {
            let dir = research_root.join(ctx);
            if !dir.is_dir() {
                continue;
            }
            collect_fixes_for_directory(&dir, &config.vault_root, &mut plan)?;
        }
    }

    // F5: Manifest reconciliation
    let temper_dir = config.vault_root.join(".temper");
    let manifest_result = manifest_io::load_manifest(&temper_dir, "doctor-fix");
    if let Ok(manifest) = &manifest_result {
        // Collect move actions for manifest updates
        let move_actions: Vec<_> = plan
            .actions
            .iter()
            .filter(|a| a.phase() == 1)
            .cloned()
            .collect();
        plan.extend(fix_manifest_for_moves(&move_actions, manifest, &config.vault_root));
        plan.extend(fix_stale_manifest_entries(manifest, &config.vault_root));
    }

    // Apply
    let report = apply_plan(&mut plan, dry_run)?;

    // Apply manifest changes and save
    if !dry_run {
        if let Ok(mut manifest) = manifest_result {
            apply_manifest_actions(&plan, &mut manifest);
            manifest_io::save_manifest(&temper_dir, &manifest)?;
        }
    }

    Ok(report)
}

/// Collect fix actions for all .md files in a directory.
fn collect_fixes_for_directory(
    dir: &Path,
    vault_root: &Path,
    plan: &mut FixPlan,
) -> Result<()> {
    let md_files: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
        .collect();

    for file_path in md_files {
        collect_fixes_for_file(&file_path, vault_root, plan)?;
    }

    Ok(())
}

/// Run all fix functions on a single file and add actions to the plan.
fn collect_fixes_for_file(
    file_path: &Path,
    vault_root: &Path,
    plan: &mut FixPlan,
) -> Result<()> {
    let content = fs::read_to_string(file_path)?;
    let Some(fm) = vault::parse_frontmatter(&content) else {
        return Ok(());
    };

    // F1: Legacy field renames
    plan.extend(fix_legacy_fields(file_path, &fm));

    // F2: Infer missing fields
    plan.extend(fix_missing_fields(file_path, &fm, vault_root));

    // F3: Relocation (based on frontmatter vs path)
    plan.extend(fix_relocation(file_path, &fm, vault_root));

    // F4: Filename slugification
    plan.extend(fix_filename(file_path, &fm, vault_root));

    Ok(())
}
```

- [ ] **Step 2: Remove old FixReport struct and fix_directory/fix_file functions**

Delete the old `FixReport` struct (lines 167-172), `fix_directory` (lines 237-249), and `fix_file` (lines 252-332) from `doctor.rs`. They're replaced by the pipeline above.

- [ ] **Step 3: Update `commands/doctor.rs` output formatting**

In `crates/temper-cli/src/commands/doctor.rs`, update `run_fix()` to use `ApplyReport`:

```rust
pub fn run_fix(config: &Config, context: Option<&str>, dry_run: bool) -> Result<()> {
    let report = crate::actions::doctor::fix(config, context, dry_run)?;

    let total = report.fields_renamed + report.fields_set + report.files_renamed
        + report.files_relocated + report.manifest_updated + report.manifest_removed;

    if dry_run {
        output::info(format!(
            "Dry run: would apply {total} fixes ({} field renames, {} fields set, {} file renames, {} relocations, {} manifest updates, {} manifest removals)",
            report.fields_renamed, report.fields_set, report.files_renamed,
            report.files_relocated, report.manifest_updated, report.manifest_removed
        ));
    } else {
        output::success(format!(
            "Fixed: {} field renames, {} fields set, {} file renames, {} relocations",
            report.fields_renamed, report.fields_set, report.files_renamed, report.files_relocated
        ));
        if report.manifest_updated > 0 || report.manifest_removed > 0 {
            output::info(format!(
                "Manifest: {} entries updated, {} stale entries removed",
                report.manifest_updated, report.manifest_removed
            ));
        }
    }

    Ok(())
}
```

- [ ] **Step 4: Verify compilation and existing tests**

```bash
cargo check --workspace --all-features && cargo nextest run --workspace -E 'test(doctor)'
```

Expected: compiles and existing doctor tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/actions/doctor.rs crates/temper-cli/src/commands/doctor.rs
git commit -m "feat: wire fix pipeline into temper doctor fix command

Replace sequential fix_file with composable FixAction pipeline.
New capabilities: field inference, filename slugification, file relocation, manifest reconciliation."
```

---

### Task 8: Fix session and research creation commands

**Files:**
- Modify: `crates/temper-cli/src/commands/session.rs:57` (filename format)
- Modify: `crates/temper-cli/src/commands/session.rs:445` (session_path helper)
- Modify: `crates/temper-cli/src/commands/session.rs:474` (test helper)
- Modify: `crates/temper-cli/src/commands/session.rs:258` (show parser)
- Modify: `crates/temper-cli/src/commands/session.rs:406` (add_session_entry parser)
- Modify: `crates/temper-cli/src/commands/research.rs:23` (filename format)

- [ ] **Step 1: Write tests for new filename format**

Add tests in `crates/temper-cli/tests/doctor_test.rs` (or a new `session_filename_test.rs`):

```rust
#[test]
fn session_filename_uses_hyphen_not_emdash() {
    // After the change, session files should be {date}-{slug}.md
    let slug = temper_cli::vault::slugify("my session title");
    let date = "2026-04-05";
    let filename = format!("{date}-{slug}.md");
    assert_eq!(filename, "2026-04-05-my-session-title.md");
    assert!(!filename.contains('\u{2014}')); // no em-dash
    assert!(!filename.contains(" — "));       // no space-emdash-space
}
```

- [ ] **Step 2: Update session.rs filename format**

In `crates/temper-cli/src/commands/session.rs`:

Line 57 — change:
```rust
let filename = format!("{today} \u{2014} {slug}.md");
```
to:
```rust
let filename = format!("{today}-{slug}.md");
```

Line 445 (`session_path`) — change:
```rust
let filename = format!("{today} \u{2014} {slug}.md");
```
to:
```rust
let filename = format!("{today}-{slug}.md");
```

Line 474 (`write_session` test helper) — change:
```rust
let filename = format!("{date} \u{2014} {slug}.md");
```
to:
```rust
let filename = format!("{date}-{slug}.md");
```

- [ ] **Step 3: Update session.rs parsers to handle both old and new formats**

Line 258 (`show` function) — update the title extraction to handle both formats:
```rust
let title_slug = if let Some(pos) = stem.find(" \u{2014} ") {
    &stem[pos + 3..]
} else if stem.len() > 10 && stem.as_bytes()[10] == b'-' {
    // New format: YYYY-MM-DD-slug
    &stem[11..]
} else {
    stem
};
```

Line 406 (`add_session_entry`) — same pattern:
```rust
let title = if let Some(pos) = stem.find(" \u{2014} ") {
    stem[pos + 3..].to_string()
} else if stem.len() > 10 && stem.as_bytes()[10] == b'-' {
    stem[11..].to_string()
} else {
    stem.to_string()
};
```

- [ ] **Step 4: Update research.rs filename format**

In `crates/temper-cli/src/commands/research.rs`, line 23 — change:
```rust
let filename = format!("{today} \u{2014} {title}.md");
```
to:
```rust
let slug = vault::slugify(title);
let filename = format!("{today}-{slug}.md");
```

- [ ] **Step 5: Verify tests pass**

```bash
cargo nextest run --workspace
```

Expected: all tests pass including session tests that use the new filename format.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/session.rs crates/temper-cli/src/commands/research.rs
git commit -m "fix: session and research commands now produce slugified filenames

Change from '{date} — {title}.md' to '{date}-{slug}.md'.
Parsers handle both old and new formats for backward compatibility."
```

---

### Task 9: Integration test — full doctor fix pipeline

**Files:**
- Create: `crates/temper-cli/tests/doctor_fix_integration_test.rs`

- [ ] **Step 1: Write integration test**

```rust
//! Integration test for the full doctor fix pipeline.

use std::fs;
use tempfile::TempDir;

/// Create a minimal temper config for testing.
fn test_config(vault_root: &std::path::Path) -> temper_cli::config::Config {
    temper_cli::config::Config {
        vault_root: vault_root.to_path_buf(),
        contexts: vec!["temper".to_string()],
        ..Default::default()
    }
}

#[test]
fn doctor_fix_pipeline_end_to_end() {
    let dir = TempDir::new().unwrap();
    let vault = dir.path();

    // Create a task with legacy fields and non-slug filename
    let task_dir = vault.join("temper").join("task");
    fs::create_dir_all(&task_dir).unwrap();
    fs::write(
        task_dir.join("2026-04-05 \u{2014} My Feature!.md"),
        "---\ntype: task\ncontext: temper\ncreated: 2026-04-05\ntitle: My Feature!\n---\nBody\n",
    )
    .unwrap();

    // Create a session with em-dash filename
    let session_dir = vault.join("temper").join("session");
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(
        session_dir.join("2026-04-05 \u{2014} my-session.md"),
        "---\ntype: session\ncontext: temper\ndate: 2026-04-05\ntitle: My Session\n---\nNotes\n",
    )
    .unwrap();

    let config = test_config(vault);

    // Run fix
    let report = temper_cli::actions::doctor::fix(&config, None, false).unwrap();

    // Verify field fixes happened
    assert!(report.fields_renamed > 0);
    assert!(report.fields_set > 0);

    // Verify task file was renamed (date stripped, slugified)
    assert!(!task_dir.join("2026-04-05 \u{2014} My Feature!.md").exists());
    assert!(task_dir.join("my-feature.md").exists());

    // Verify session file was renamed (em-dash → hyphen)
    assert!(!session_dir.join("2026-04-05 \u{2014} my-session.md").exists());
    assert!(session_dir.join("2026-04-05-my-session.md").exists());

    // Verify frontmatter was fixed
    let task_content = fs::read_to_string(task_dir.join("my-feature.md")).unwrap();
    assert!(task_content.contains("temper-type: task"));
    assert!(task_content.contains("temper-context: temper"));
    assert!(task_content.contains("slug: my-feature"));
    assert!(task_content.contains("temper-stage: backlog"));

    // Re-run doctor scan — should have minimal remaining issues
    let scan = temper_cli::actions::doctor::scan(&config, None).unwrap();
    // All auto-fixable issues should be resolved
    assert_eq!(scan.auto_fixable, 0, "auto-fixable issues remain: {:#?}", scan.file_results);
}
```

- [ ] **Step 2: Run integration test**

```bash
cargo nextest run --workspace -E 'test(doctor_fix_pipeline)'
```

Expected: passes.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-cli/tests/doctor_fix_integration_test.rs
git commit -m "test: add end-to-end integration test for doctor fix pipeline"
```

---

### Task 10: Final check — clippy, full test suite, build

**Files:** None (verification only)

- [ ] **Step 1: Run clippy**

```bash
cargo clippy --workspace --all-features -- -D warnings
```

Expected: clean.

- [ ] **Step 2: Run full test suite**

```bash
cargo nextest run --workspace
```

Expected: all tests pass.

- [ ] **Step 3: Run cargo fmt**

```bash
cargo fmt --all
```

- [ ] **Step 4: Run full check suite**

```bash
cargo make check
```

Expected: all checks pass.

- [ ] **Step 5: Final commit if formatting changed**

```bash
git add -A && git commit -m "style: apply cargo fmt"
```

- [ ] **Step 6: Rebuild installed CLI**

```bash
cargo install --path crates/temper-cli
```
