//! FixAction enum and FixPlan collector for the doctor fix pipeline.
//!
//! This module defines the data model used by `temper doctor fix`. Actions are
//! collected into a `FixPlan`, sorted by phase, and then applied by the
//! applicator (added in a later task).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_yaml::Value;
use uuid::Uuid;

/// A single corrective action to apply to the vault.
#[derive(Debug, Clone, PartialEq)]
pub enum FixAction {
    /// Rename a YAML frontmatter key in a file (phase 0).
    RenameField {
        path: PathBuf,
        old_key: String,
        new_key: String,
    },
    /// Set (insert or overwrite) a YAML frontmatter key in a file (phase 0).
    SetField {
        path: PathBuf,
        key: String,
        value: String,
        reason: String,
    },
    /// Rename a file on disk (phase 1).
    RenameFile {
        old_path: PathBuf,
        new_path: PathBuf,
        reason: String,
    },
    /// Move a file to a different directory (phase 1).
    RelocateFile {
        old_path: PathBuf,
        new_path: PathBuf,
        reason: String,
    },
    /// Update the manifest record for a file that has moved (phase 2).
    UpdateManifest {
        temper_id: Uuid,
        old_path: String,
        new_path: String,
    },
    /// Remove a manifest record whose file no longer exists (phase 2).
    RemoveManifest { temper_id: Uuid, reason: String },
}

/// Sentinel path used as a stand-in for manifest actions in `target_path()`.
static MANIFEST_PATH: OnceLock<PathBuf> = OnceLock::new();

fn manifest_path() -> &'static PathBuf {
    MANIFEST_PATH.get_or_init(|| PathBuf::from(".temper/manifest.json"))
}

impl FixAction {
    /// Execution phase for ordering purposes.
    ///
    /// * `0` — field-level fixes (must happen before files move)
    /// * `1` — file renames / relocations
    /// * `2` — manifest record updates
    pub fn phase(&self) -> u8 {
        match self {
            Self::RenameField { .. } | Self::SetField { .. } => 0,
            Self::RenameFile { .. } | Self::RelocateFile { .. } => 1,
            Self::UpdateManifest { .. } | Self::RemoveManifest { .. } => 2,
        }
    }

    /// Primary path for display grouping.
    ///
    /// Returns the file being modified for field and file actions. For manifest
    /// actions, returns a static sentinel path (`".temper/manifest.json"`).
    pub fn target_path(&self) -> &PathBuf {
        match self {
            Self::RenameField { path, .. } | Self::SetField { path, .. } => path,
            Self::RenameFile { old_path, .. } | Self::RelocateFile { old_path, .. } => old_path,
            Self::UpdateManifest { .. } | Self::RemoveManifest { .. } => manifest_path(),
        }
    }
}

/// A collected set of fix actions to apply to the vault.
#[derive(Debug, Default)]
pub struct FixPlan {
    pub actions: Vec<FixAction>,
}

impl FixPlan {
    /// Create an empty plan.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a single action.
    pub fn add(&mut self, action: FixAction) {
        self.actions.push(action);
    }

    /// Extend the plan with an iterator of actions.
    pub fn extend(&mut self, iter: impl IntoIterator<Item = FixAction>) {
        self.actions.extend(iter);
    }

    /// Sort actions by phase so they execute in the correct order.
    ///
    /// Within a phase the original insertion order is preserved (stable sort).
    pub fn sort(&mut self) {
        self.actions.sort_by_key(|a| a.phase());
    }

    /// Returns `true` if there are no actions in the plan.
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Count actions broken down by phase.
    ///
    /// Returns `(phase0_count, phase1_count, phase2_count)`.
    pub fn count_by_phase(&self) -> (usize, usize, usize) {
        let mut p0 = 0usize;
        let mut p1 = 0usize;
        let mut p2 = 0usize;
        for action in &self.actions {
            match action.phase() {
                0 => p0 += 1,
                1 => p1 += 1,
                _ => p2 += 1,
            }
        }
        (p0, p1, p2)
    }
}

// ---------------------------------------------------------------------------
// Legacy field rename map (F1)
// ---------------------------------------------------------------------------

/// Map of (old_key, new_key) pairs for legacy frontmatter field renames.
static LEGACY_FIELD_MAP: &[(&str, &str)] = &[
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

/// Produce `RenameField` actions for any legacy frontmatter keys found in `fm`.
///
/// If the old key exists but the new key is already present, the rename is
/// skipped (the caller should handle the conflict separately).
pub fn fix_legacy_fields(path: &Path, fm: &Value) -> Vec<FixAction> {
    let mut actions = Vec::new();
    let map = match fm.as_mapping() {
        Some(m) => m,
        None => return actions,
    };

    for (old_key, new_key) in LEGACY_FIELD_MAP {
        let old_exists = map.contains_key(Value::String(old_key.to_string()));
        let new_exists = map.contains_key(Value::String(new_key.to_string()));
        if old_exists && !new_exists {
            actions.push(FixAction::RenameField {
                path: path.to_path_buf(),
                old_key: old_key.to_string(),
                new_key: new_key.to_string(),
            });
        }
    }

    actions
}

// ---------------------------------------------------------------------------
// Field inference helpers (F2)
// ---------------------------------------------------------------------------

/// Extract a helper string value from a `serde_yaml::Value` mapping.
pub fn fm_str(fm: &Value, key: &str) -> Option<String> {
    fm.get(key)?.as_str().map(|s| s.to_string())
}

/// Extract a `YYYY-MM-DD` date from the start of a filename (without extension).
///
/// Accepts filenames starting with `YYYY-MM-DD` optionally followed by `—`
/// (em-dash U+2014), `--`, or `-`.
pub fn extract_date_from_filename(filename: &str) -> Option<String> {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);

    // Must start with YYYY-MM-DD pattern
    if stem.len() < 10 {
        return None;
    }
    let candidate = &stem[..10];
    // Validate format: digits-digits-digits
    let parts: Vec<&str> = candidate.splitn(3, '-').collect();
    if parts.len() == 3
        && parts[0].len() == 4
        && parts[0].chars().all(|c| c.is_ascii_digit())
        && parts[1].len() == 2
        && parts[1].chars().all(|c| c.is_ascii_digit())
        && parts[2].len() == 2
        && parts[2].chars().all(|c| c.is_ascii_digit())
    {
        Some(candidate.to_string())
    } else {
        None
    }
}

/// Derive a slug from a filename, stripping a leading date prefix and separators.
///
/// Strips `YYYY-MM-DD` prefix, then any leading `—` (em-dash), `--`, or `-`,
/// then the file extension, and finally slugifies the remainder.
pub fn slug_from_filename(filename: &str) -> String {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename)
        .to_string();

    let rest = if extract_date_from_filename(filename).is_some() {
        let after_date = &stem[10..]; // skip "YYYY-MM-DD"
                                      // Strip leading em-dash (U+2014), "--", or "-"
        if let Some(s) = after_date.strip_prefix('\u{2014}') {
            s
        } else if let Some(s) = after_date.strip_prefix("--") {
            s
        } else if let Some(s) = after_date.strip_prefix('-') {
            s
        } else {
            after_date
        }
    } else {
        &stem
    };

    crate::vault::slugify(rest)
}

/// Humanize a slug into title-case words.
///
/// `"my-feature-x"` → `"My Feature X"`
pub fn humanize_slug(slug: &str) -> String {
    slug.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    let upper: String = first.to_uppercase().collect();
                    upper + chars.as_str()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Infer `(context, doc_type)` from the file's path relative to `vault_root`.
///
/// Handles two layouts:
/// - `research/{context}/{file}.md` → `(context, "research")`
/// - `{context}/{doc_type}/{file}.md` → `(context, doc_type)`
pub fn infer_from_path(path: &Path, vault_root: &Path) -> Option<(String, String)> {
    let rel = path.strip_prefix(vault_root).ok()?;
    let parts: Vec<&str> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();

    // Need at least context/file.md (2 parts) or context/doc_type/file.md (3 parts)
    match parts.len() {
        2 => {
            // {context}/{file}.md — doc_type unknown
            None
        }
        3 => {
            let first = parts[0];
            let second = parts[1];
            // research/{context}/{file}.md → (context, "research")
            if first == "research" {
                Some((second.to_string(), "research".to_string()))
            } else {
                // {context}/{doc_type}/{file}.md
                Some((first.to_string(), second.to_string()))
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// InferContext and rule types (F2)
// ---------------------------------------------------------------------------

/// Context gathered once per file for all inference rules.
pub struct InferContext<'a> {
    pub path: &'a Path,
    pub filename: String,
    pub fm: &'a Value,
    /// If the path is deep enough, the directory-level context.
    pub path_context: Option<String>,
    /// If the path is deep enough, the directory-level doc_type.
    pub path_doc_type: Option<String>,
    /// `temper-type` from frontmatter, or path_doc_type as fallback.
    pub effective_doc_type: Option<String>,
}

impl<'a> InferContext<'a> {
    pub fn new(path: &'a Path, fm: &'a Value, vault_root: &'a Path) -> Self {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let (path_context, path_doc_type) = match infer_from_path(path, vault_root) {
            Some((ctx, dt)) => (Some(ctx), Some(dt)),
            None => (None, None),
        };

        let effective_doc_type = fm_str(fm, "temper-type").or_else(|| path_doc_type.clone());

        Self {
            path,
            filename,
            fm,
            path_context,
            path_doc_type,
            effective_doc_type,
        }
    }
}

/// Signature for a single inference rule function.
///
/// Returns `Some((key, value, reason))` when the rule fires, `None` otherwise.
type InferFn = fn(&InferContext<'_>) -> Option<(String, String, String)>;

// ---------------------------------------------------------------------------
// Individual inference rules
// ---------------------------------------------------------------------------

fn infer_temper_id(ctx: &InferContext<'_>) -> Option<(String, String, String)> {
    if fm_str(ctx.fm, "temper-id").is_some() {
        return None;
    }
    let id = crate::ids::generate_id();
    Some(("temper-id".to_string(), id, "generated UUIDv7".to_string()))
}

fn infer_temper_type(ctx: &InferContext<'_>) -> Option<(String, String, String)> {
    if fm_str(ctx.fm, "temper-type").is_some() {
        return None;
    }
    let dt = ctx.path_doc_type.as_ref()?;
    Some((
        "temper-type".to_string(),
        dt.clone(),
        "inferred from directory path".to_string(),
    ))
}

fn infer_temper_context(ctx: &InferContext<'_>) -> Option<(String, String, String)> {
    if fm_str(ctx.fm, "temper-context").is_some() {
        return None;
    }
    let c = ctx.path_context.as_ref()?;
    Some((
        "temper-context".to_string(),
        c.clone(),
        "inferred from directory path".to_string(),
    ))
}

fn infer_title(ctx: &InferContext<'_>) -> Option<(String, String, String)> {
    if fm_str(ctx.fm, "title").is_some() {
        return None;
    }
    let slug = slug_from_filename(&ctx.filename);
    if slug.is_empty() {
        return None;
    }
    let title = humanize_slug(&slug);
    Some((
        "title".to_string(),
        title,
        "humanized from filename".to_string(),
    ))
}

fn infer_slug(ctx: &InferContext<'_>) -> Option<(String, String, String)> {
    if fm_str(ctx.fm, "slug").is_some() {
        return None;
    }
    // Prefer title from frontmatter, fall back to filename
    let slug = if let Some(title) = fm_str(ctx.fm, "title") {
        crate::vault::slugify(&title)
    } else {
        slug_from_filename(&ctx.filename)
    };
    if slug.is_empty() {
        return None;
    }
    Some((
        "slug".to_string(),
        slug,
        "slugified from title or filename".to_string(),
    ))
}

fn infer_date(ctx: &InferContext<'_>) -> Option<(String, String, String)> {
    if fm_str(ctx.fm, "date").is_some() {
        return None;
    }
    // Only apply to session/research doc types
    let dt = ctx.effective_doc_type.as_deref()?;
    if dt != "session" && dt != "research" {
        return None;
    }
    let date = extract_date_from_filename(&ctx.filename)?;
    Some((
        "date".to_string(),
        date,
        "extracted from filename date prefix".to_string(),
    ))
}

fn infer_temper_created(ctx: &InferContext<'_>) -> Option<(String, String, String)> {
    if fm_str(ctx.fm, "temper-created").is_some() {
        return None;
    }
    // Use date field if already present, else extract from filename
    let date = fm_str(ctx.fm, "date").or_else(|| extract_date_from_filename(&ctx.filename))?;
    Some((
        "temper-created".to_string(),
        date,
        "derived from date field or filename".to_string(),
    ))
}

fn infer_temper_stage(ctx: &InferContext<'_>) -> Option<(String, String, String)> {
    if fm_str(ctx.fm, "temper-stage").is_some() {
        return None;
    }
    let dt = ctx.effective_doc_type.as_deref()?;
    if dt != "task" {
        return None;
    }
    Some((
        "temper-stage".to_string(),
        "backlog".to_string(),
        "default stage for tasks".to_string(),
    ))
}

/// All registered inference rules, applied in order.
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

// ---------------------------------------------------------------------------
// F2: fix_missing_fields
// ---------------------------------------------------------------------------

/// Produce `SetField` actions for any frontmatter fields that can be inferred.
pub fn fix_missing_fields(path: &Path, fm: &Value, vault_root: &Path) -> Vec<FixAction> {
    let ctx = InferContext::new(path, fm, vault_root);
    let mut actions = Vec::new();

    for rule in INFER_RULES {
        if let Some((key, value, reason)) = rule(&ctx) {
            actions.push(FixAction::SetField {
                path: path.to_path_buf(),
                key,
                value,
                reason,
            });
        }
    }

    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_path(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn sample_rename_field() -> FixAction {
        FixAction::RenameField {
            path: make_path("vault/task/foo.md"),
            old_key: "type".to_string(),
            new_key: "temper-type".to_string(),
        }
    }

    fn sample_set_field() -> FixAction {
        FixAction::SetField {
            path: make_path("vault/task/foo.md"),
            key: "temper-id".to_string(),
            value: Uuid::now_v7().to_string(),
            reason: "missing temper-id".to_string(),
        }
    }

    fn sample_rename_file() -> FixAction {
        FixAction::RenameFile {
            old_path: make_path("vault/task/old.md"),
            new_path: make_path("vault/task/new.md"),
            reason: "slug normalisation".to_string(),
        }
    }

    fn sample_relocate_file() -> FixAction {
        FixAction::RelocateFile {
            old_path: make_path("vault/goal/foo.md"),
            new_path: make_path("vault/project/goal/foo.md"),
            reason: "wrong directory".to_string(),
        }
    }

    fn sample_update_manifest() -> FixAction {
        FixAction::UpdateManifest {
            temper_id: Uuid::now_v7(),
            old_path: "task/old.md".to_string(),
            new_path: "task/new.md".to_string(),
        }
    }

    fn sample_remove_manifest() -> FixAction {
        FixAction::RemoveManifest {
            temper_id: Uuid::now_v7(),
            reason: "file deleted".to_string(),
        }
    }

    #[test]
    fn fix_action_phase_ordering() {
        assert_eq!(sample_rename_field().phase(), 0);
        assert_eq!(sample_set_field().phase(), 0);
        assert_eq!(sample_rename_file().phase(), 1);
        assert_eq!(sample_relocate_file().phase(), 1);
        assert_eq!(sample_update_manifest().phase(), 2);
        assert_eq!(sample_remove_manifest().phase(), 2);
    }

    #[test]
    fn fix_plan_sorts_by_phase() {
        let mut plan = FixPlan::new();
        // Add out of order: manifest first, then field, then file.
        plan.add(sample_update_manifest());
        plan.add(sample_rename_field());
        plan.add(sample_rename_file());
        plan.sort();

        let phases: Vec<u8> = plan.actions.iter().map(|a| a.phase()).collect();
        assert_eq!(phases, vec![0, 1, 2]);
    }

    #[test]
    fn fix_plan_count_by_phase() {
        let mut plan = FixPlan::new();
        plan.add(sample_rename_field());
        plan.add(sample_set_field());
        plan.add(sample_rename_file());
        plan.add(sample_relocate_file());
        plan.add(sample_update_manifest());
        plan.add(sample_remove_manifest());

        let (p0, p1, p2) = plan.count_by_phase();
        assert_eq!(p0, 2);
        assert_eq!(p1, 2);
        assert_eq!(p2, 2);
    }

    // -----------------------------------------------------------------------
    // Helper for building YAML frontmatter in tests
    // -----------------------------------------------------------------------

    fn yaml_fm(pairs: &[(&str, &str)]) -> Value {
        let mut map = serde_yaml::Mapping::new();
        for (k, v) in pairs {
            map.insert(Value::String(k.to_string()), Value::String(v.to_string()));
        }
        Value::Mapping(map)
    }

    fn empty_fm() -> Value {
        Value::Mapping(serde_yaml::Mapping::new())
    }

    // -----------------------------------------------------------------------
    // F1 tests
    // -----------------------------------------------------------------------

    #[test]
    fn f1_renames_legacy_type_field() {
        let path = PathBuf::from("/vault/project/task/foo.md");
        let fm = yaml_fm(&[("type", "task")]);
        let actions = fix_legacy_fields(&path, &fm);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            FixAction::RenameField {
                old_key, new_key, ..
            } => {
                assert_eq!(old_key, "type");
                assert_eq!(new_key, "temper-type");
            }
            other => panic!("expected RenameField, got {other:?}"),
        }
    }

    #[test]
    fn f1_skips_when_new_field_exists() {
        let path = PathBuf::from("/vault/project/task/foo.md");
        // Both old and new key present — skip rename to avoid clobbering
        let fm = yaml_fm(&[("type", "task"), ("temper-type", "task")]);
        let actions = fix_legacy_fields(&path, &fm);
        assert!(
            actions.is_empty(),
            "should skip when new key already exists"
        );
    }

    #[test]
    fn f1_no_actions_for_clean_frontmatter() {
        let path = PathBuf::from("/vault/project/task/foo.md");
        let fm = yaml_fm(&[("temper-type", "task"), ("temper-id", "some-uuid")]);
        let actions = fix_legacy_fields(&path, &fm);
        assert!(actions.is_empty());
    }

    // -----------------------------------------------------------------------
    // Helper function tests
    // -----------------------------------------------------------------------

    #[test]
    fn extract_date_from_filename_valid() {
        assert_eq!(
            extract_date_from_filename("2026-01-15-my-session.md"),
            Some("2026-01-15".to_string())
        );
    }

    #[test]
    fn extract_date_from_filename_no_date() {
        assert_eq!(extract_date_from_filename("my-session.md"), None);
    }

    #[test]
    fn slug_from_filename_strips_date_and_emdash() {
        // em-dash separator: 2026-01-15—my-session.md
        let filename = "2026-01-15\u{2014}my-session.md";
        assert_eq!(slug_from_filename(filename), "my-session");
    }

    #[test]
    fn slug_from_filename_strips_date_hyphen() {
        assert_eq!(slug_from_filename("2026-01-15-my-session.md"), "my-session");
    }

    #[test]
    fn slug_from_filename_no_date() {
        assert_eq!(slug_from_filename("my-feature.md"), "my-feature");
    }

    #[test]
    fn humanize_slug_capitalizes_words() {
        assert_eq!(humanize_slug("my-feature-x"), "My Feature X");
    }

    #[test]
    fn humanize_slug_single_word() {
        assert_eq!(humanize_slug("feature"), "Feature");
    }

    // -----------------------------------------------------------------------
    // Individual rule tests
    // -----------------------------------------------------------------------

    fn vault_root() -> PathBuf {
        PathBuf::from("/vault")
    }

    #[test]
    fn rule_infer_temper_id_generates_when_missing() {
        let path = PathBuf::from("/vault/project/task/foo.md");
        let fm = empty_fm();
        let root = vault_root();
        let ctx = InferContext::new(&path, &fm, &root);
        let result = infer_temper_id(&ctx);
        assert!(result.is_some());
        let (key, value, _) = result.unwrap();
        assert_eq!(key, "temper-id");
        // Must be a valid UUID
        assert!(Uuid::parse_str(&value).is_ok(), "value should be a UUID");
    }

    #[test]
    fn rule_infer_temper_id_skips_when_present() {
        let path = PathBuf::from("/vault/project/task/foo.md");
        let fm = yaml_fm(&[("temper-id", "01234567-0000-7000-8000-000000000000")]);
        let root = vault_root();
        let ctx = InferContext::new(&path, &fm, &root);
        assert!(infer_temper_id(&ctx).is_none());
    }

    #[test]
    fn rule_infer_temper_type_from_directory() {
        let path = PathBuf::from("/vault/project/task/foo.md");
        let fm = empty_fm();
        let root = vault_root();
        let ctx = InferContext::new(&path, &fm, &root);
        let result = infer_temper_type(&ctx);
        assert!(result.is_some());
        let (key, value, _) = result.unwrap();
        assert_eq!(key, "temper-type");
        assert_eq!(value, "task");
    }

    #[test]
    fn rule_infer_temper_context_from_directory() {
        let path = PathBuf::from("/vault/project/task/foo.md");
        let fm = empty_fm();
        let root = vault_root();
        let ctx = InferContext::new(&path, &fm, &root);
        let result = infer_temper_context(&ctx);
        assert!(result.is_some());
        let (key, value, _) = result.unwrap();
        assert_eq!(key, "temper-context");
        assert_eq!(value, "project");
    }

    #[test]
    fn rule_infer_title_from_filename() {
        let path = PathBuf::from("/vault/project/task/2026-01-15-my-task.md");
        let fm = empty_fm();
        let root = vault_root();
        let ctx = InferContext::new(&path, &fm, &root);
        let result = infer_title(&ctx);
        assert!(result.is_some());
        let (key, value, _) = result.unwrap();
        assert_eq!(key, "title");
        assert_eq!(value, "My Task");
    }

    #[test]
    fn rule_infer_slug_from_title() {
        let path = PathBuf::from("/vault/project/task/foo.md");
        let fm = yaml_fm(&[("title", "My Great Task")]);
        let root = vault_root();
        let ctx = InferContext::new(&path, &fm, &root);
        let result = infer_slug(&ctx);
        assert!(result.is_some());
        let (key, value, _) = result.unwrap();
        assert_eq!(key, "slug");
        assert_eq!(value, "my-great-task");
    }

    #[test]
    fn rule_infer_date_from_filename_for_session() {
        let path = PathBuf::from("/vault/project/session/2026-01-15-standup.md");
        let fm = yaml_fm(&[("temper-type", "session")]);
        let root = vault_root();
        let ctx = InferContext::new(&path, &fm, &root);
        let result = infer_date(&ctx);
        assert!(result.is_some());
        let (key, value, _) = result.unwrap();
        assert_eq!(key, "date");
        assert_eq!(value, "2026-01-15");
    }

    #[test]
    fn rule_infer_date_skips_for_tasks() {
        let path = PathBuf::from("/vault/project/task/2026-01-15-my-task.md");
        let fm = yaml_fm(&[("temper-type", "task")]);
        let root = vault_root();
        let ctx = InferContext::new(&path, &fm, &root);
        // date rule only fires for session/research
        assert!(infer_date(&ctx).is_none());
    }

    #[test]
    fn rule_infer_temper_created_from_date() {
        let path = PathBuf::from("/vault/project/task/2026-03-05-plan.md");
        let fm = yaml_fm(&[("date", "2026-03-05")]);
        let root = vault_root();
        let ctx = InferContext::new(&path, &fm, &root);
        let result = infer_temper_created(&ctx);
        assert!(result.is_some());
        let (key, value, _) = result.unwrap();
        assert_eq!(key, "temper-created");
        assert_eq!(value, "2026-03-05");
    }

    #[test]
    fn rule_infer_stage_backlog_for_tasks() {
        let path = PathBuf::from("/vault/project/task/foo.md");
        let fm = yaml_fm(&[("temper-type", "task")]);
        let root = vault_root();
        let ctx = InferContext::new(&path, &fm, &root);
        let result = infer_temper_stage(&ctx);
        assert!(result.is_some());
        let (key, value, _) = result.unwrap();
        assert_eq!(key, "temper-stage");
        assert_eq!(value, "backlog");
    }

    #[test]
    fn rule_infer_stage_skips_non_tasks() {
        let path = PathBuf::from("/vault/project/session/foo.md");
        let fm = yaml_fm(&[("temper-type", "session")]);
        let root = vault_root();
        let ctx = InferContext::new(&path, &fm, &root);
        assert!(infer_temper_stage(&ctx).is_none());
    }

    // -----------------------------------------------------------------------
    // Aggregate F2 test
    // -----------------------------------------------------------------------

    #[test]
    fn f2_skips_fields_already_present() {
        // A file with all fields already set — fix_missing_fields should emit nothing.
        let path = PathBuf::from("/vault/project/task/my-task.md");
        let root = vault_root();
        let fm = yaml_fm(&[
            ("temper-id", "01234567-0000-7000-8000-000000000000"),
            ("temper-type", "task"),
            ("temper-context", "project"),
            ("title", "My Task"),
            ("slug", "my-task"),
            ("date", "2026-01-01"),
            ("temper-created", "2026-01-01"),
            ("temper-stage", "backlog"),
        ]);
        let actions = fix_missing_fields(&path, &fm, &root);
        assert!(
            actions.is_empty(),
            "expected no actions for fully-populated frontmatter, got: {actions:?}"
        );
    }
}
