//! File normalization primitive — schema validate, apply defaults to YAML
//! frontmatter, rewrite the file if changed, and recompute three hashes.
//!
//! Used by every `temper sync` subcommand and by `temper doctor` to enforce
//! the invariant that on-disk file state matches the normalized form its
//! doc-type schema declares.
//!
//! This module is now a thin orchestrator over [`crate::frontmatter::Frontmatter`].
//! All YAML parsing, tier splitting, hashing, canonical serialization, and
//! atomic write logic lives in that module; `normalize_file` adds exactly one
//! responsibility on top: apply doc-type-specific defaults to the parsed
//! value before validating and writing back.

use crate::error::{Result, TemperError};
use crate::frontmatter::{DocType, Frontmatter};
use crate::hash::compute_body_hash;
use crate::schema::ValidationIssue;
use std::path::Path;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of normalizing a single file.
#[derive(Debug, Clone)]
pub struct NormalizeOutcome {
    /// True if the file was rewritten to disk (defaults materialized or
    /// frontmatter reserialized). False if the file was already canonical
    /// or if `issues` contained a non-auto-fixable error (see note below).
    pub changed: bool,

    /// SHA-256 hash of the markdown body (unchanged by normalize, but
    /// returned so callers have a complete triple).
    pub body_hash: String,

    /// Managed-tier hash, computed on the final normalized frontmatter.
    pub managed_hash: String,

    /// Open-tier hash, computed on the final normalized frontmatter.
    pub open_hash: String,

    /// Schema violations and other validation issues. An empty vector
    /// means the file is conformant. A non-empty vector means the file
    /// either (a) needs user attention before it can be synced, or (b)
    /// contains only auto-fixable issues that normalize has already
    /// fixed (after the fix, this vector should be empty).
    pub issues: Vec<ValidationIssue>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Read a vault file, validate its frontmatter against the doc-type schema,
/// materialize any missing doc-type defaults by rewriting the file, and
/// return the three hashes computed on the final normalized state.
///
/// If the file has non-auto-fixable validation issues (an invalid enum
/// value the user typed manually, a missing required field normalize
/// cannot fill in), the file is NOT rewritten. Hashes are still computed
/// on the as-read state so the caller can update its manifest to reflect
/// on-disk reality, and `issues` is non-empty to signal that sync must be
/// blocked for this file until the user resolves them.
///
/// # Errors
/// Returns [`TemperError::Config`] if the file is missing, has no
/// frontmatter block, or contains invalid YAML.
pub fn normalize_file(path: &Path, doc_type: &str) -> Result<NormalizeOutcome> {
    normalize_impl(path, doc_type, true)
}

/// Dry-run variant of [`normalize_file`]. Reads the file, runs the same
/// validation and default-materialization logic, but never writes to disk.
/// `changed` indicates whether a real [`normalize_file`] call WOULD rewrite
/// the file. Used by `temper doctor` scan to report what would change
/// without changing it.
///
/// # Errors
/// Same conditions as [`normalize_file`].
pub fn normalize_file_inspect(path: &Path, doc_type: &str) -> Result<NormalizeOutcome> {
    normalize_impl(path, doc_type, false)
}

// ---------------------------------------------------------------------------
// Internal: shared implementation
// ---------------------------------------------------------------------------

/// Shared implementation for [`normalize_file`] and [`normalize_file_inspect`].
/// When `write` is `false`, the on-disk file is never modified — the
/// returned `changed` flag indicates only what *would* happen.
fn normalize_impl(path: &Path, doc_type: &str, write: bool) -> Result<NormalizeOutcome> {
    // Single read — reused for both parsing and the change-comparison below.
    let original = std::fs::read_to_string(path)
        .map_err(|e| TemperError::Config(format!("failed to read {}: {e}", path.display())))?;
    let mut fm = Frontmatter::try_from(original.as_str())?;

    // Sanity check: the filesystem-inferred doc_type should agree with
    // what the frontmatter declares. Parse the caller-supplied string into a
    // typed DocType so an unknown caller string surfaces as a clean error
    // rather than a misleading mismatch.
    let expected = DocType::from_str(doc_type).map_err(|e| {
        TemperError::Config(format!(
            "normalize_file called with unknown doctype '{doc_type}' for {}: {e}",
            path.display()
        ))
    })?;
    if fm.doc_type() != expected {
        return Err(TemperError::Config(format!(
            "doc_type mismatch for {}: frontmatter says '{}', caller says '{}'",
            path.display(),
            fm.doc_type().as_str(),
            doc_type
        )));
    }

    let body_hash = compute_body_hash(fm.body());

    // Snapshot hashes BEFORE applying defaults. We need these if validation
    // fails, so the caller's manifest can record on-disk reality.
    let pre_defaults_hashes = fm.hashes();

    // Apply doc-type-specific defaults by mutating the YAML mapping in place.
    // The `Frontmatter` type's alias-normalized + mapping invariant still
    // holds because we only insert canonical keys.
    if let Some(mapping) = fm.value_mut().as_mapping_mut() {
        apply_doc_type_defaults_yaml(doc_type, mapping);
    }

    // Validate the post-defaults state.
    let issues = fm.validate()?;

    if !issues.is_empty() {
        // Non-conformant: don't rewrite. Hashes describe pre-defaults state.
        let (managed_hash, open_hash) = pre_defaults_hashes;
        return Ok(NormalizeOutcome {
            changed: false,
            body_hash,
            managed_hash,
            open_hash,
            issues,
        });
    }

    // Compare canonical-serialized output against the on-disk text.
    // If they differ, write atomically via `Frontmatter::write_to`.
    let new_content = fm.serialize()?;
    let changed = new_content != original;

    if changed && write {
        fm.write_to(path)?;
    }

    let (managed_hash, open_hash) = fm.hashes();

    Ok(NormalizeOutcome {
        changed,
        body_hash,
        managed_hash,
        open_hash,
        issues: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// Doc-type default materialization
// ---------------------------------------------------------------------------

/// Returns the default keys `apply_doc_type_defaults_yaml` may insert for
/// a given doc type. Single source of truth for both materialization (this
/// module) and drift-free detection (doctor scan, external consumers).
///
/// Values are not returned because some defaults are dynamic (e.g. today's
/// date for sessions). Callers that need to know "is any default missing"
/// should use [`is_missing_default`]; callers that need to materialize use
/// `apply_doc_type_defaults_yaml`.
pub fn default_keys_for(doc_type: &str) -> &'static [&'static str] {
    match doc_type {
        "task" => &["temper-stage"],
        "goal" => &["temper-status"],
        "session" | "research" => &["date"],
        _ => &[],
    }
}

/// Returns true if the frontmatter is missing at least one of the default
/// keys `apply_doc_type_defaults_yaml` would materialize for its doc type.
/// Used by doctor scan to distinguish material default-materialization from
/// cosmetic YAML re-serialization.
pub fn is_missing_default(fm: &serde_yaml::Value, doc_type: &str) -> bool {
    let Some(mapping) = fm.as_mapping() else {
        return false;
    };
    default_keys_for(doc_type)
        .iter()
        .any(|k| !mapping.contains_key(serde_yaml::Value::String((*k).to_string())))
}

/// Apply doc-type-specific defaults to a YAML frontmatter mapping in place.
/// Only adds keys that are absent. Mirrors `defaults::apply_doc_type_defaults`
/// but operates on `serde_yaml::Mapping` to preserve key ordering as typed
/// by the user.
///
/// The set of keys touched here must match [`default_keys_for`] — that
/// function is the shared source of truth consumers outside this module
/// reference for drift detection.
fn apply_doc_type_defaults_yaml(doc_type: &str, mapping: &mut serde_yaml::Mapping) {
    use serde_yaml::Value;

    fn ensure(mapping: &mut serde_yaml::Mapping, key: &str, value: Value) {
        let key_v = Value::String(key.to_string());
        if !mapping.contains_key(&key_v) {
            mapping.insert(key_v, value);
        }
    }

    match doc_type {
        "task" => {
            ensure(
                mapping,
                "temper-stage",
                Value::String("backlog".to_string()),
            );
        }
        "goal" => {
            ensure(
                mapping,
                "temper-status",
                Value::String("active".to_string()),
            );
        }
        "session" | "research" => {
            let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
            ensure(mapping, "date", Value::String(today));
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::Frontmatter;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn write_file(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, content).expect("write fixture");
        path
    }

    fn read_file(path: &Path) -> String {
        fs::read_to_string(path).expect("read file")
    }

    // 1. normalize_task_missing_stage_rewrites_with_default
    #[test]
    fn normalize_task_missing_stage_rewrites_with_default() {
        let dir = tempdir().unwrap();
        let content = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-12T00:00:00Z"
temper-title: Test
slug: test
---
body
"#;
        let path = write_file(dir.path(), "task.md", content);
        let outcome = normalize_file(&path, "task").expect("normalize ok");
        assert!(
            outcome.changed,
            "should rewrite to add default temper-stage"
        );
        assert!(
            outcome.issues.is_empty(),
            "no issues expected, got: {:?}",
            outcome.issues
        );
        let on_disk = read_file(&path);
        assert!(
            on_disk.contains("temper-stage: backlog"),
            "file should contain temper-stage: backlog, got:\n{on_disk}"
        );

        // Second normalize: now canonical, no further changes, hashes match.
        let second = normalize_file(&path, "task").expect("normalize ok");
        assert!(!second.changed, "second normalize should be a no-op");
        assert_eq!(outcome.managed_hash, second.managed_hash);
        assert_eq!(outcome.open_hash, second.open_hash);
        assert_eq!(outcome.body_hash, second.body_hash);
    }

    // 2. normalize_task_already_canonical_is_noop
    #[test]
    fn normalize_task_already_canonical_is_noop() {
        let dir = tempdir().unwrap();
        // Inline canonical form matches the order emitted by
        // Frontmatter::serialize — identity → tier1 (temper-context before
        // temper-type per TIER1_SYSTEM_FIELDS order) → managed in schema order.
        // serde_yaml serializes UUID and datetime strings without quotes.
        // title is renamed to temper-title per temper-prefix contract.
        let canonical = r#"---
temper-id: 019d8110-8ff3-70c2-85ae-57e04ed62885
temper-context: temper
temper-type: task
temper-created: 2026-04-12T00:00:00Z
temper-title: Test
slug: test
temper-stage: in-progress
---
body content
"#;
        let path = write_file(dir.path(), "task.md", canonical);
        let before = read_file(&path);

        let outcome = normalize_file(&path, "task").expect("normalize ok");
        assert!(!outcome.changed, "canonical file should not be rewritten");
        assert!(outcome.issues.is_empty());
        let after = read_file(&path);
        assert_eq!(before, after, "file content should be byte-identical");
        assert!(outcome.managed_hash.starts_with("sha256:"));
        assert!(outcome.open_hash.starts_with("sha256:"));
        assert!(outcome.body_hash.starts_with("sha256:"));
    }

    // 3. normalize_task_invalid_enum_does_not_rewrite
    #[test]
    fn normalize_task_invalid_enum_does_not_rewrite() {
        let dir = tempdir().unwrap();
        let content = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-12T00:00:00Z"
temper-title: Test
slug: test
temper-stage: frobnicate
---
body
"#;
        let path = write_file(dir.path(), "task.md", content);
        let before = read_file(&path);

        let outcome = normalize_file(&path, "task").expect("normalize ok");
        assert!(
            !outcome.changed,
            "invalid enum file should not be rewritten"
        );
        let after = read_file(&path);
        assert_eq!(before, after, "file content must be unchanged");

        assert!(!outcome.issues.is_empty(), "expected validation issues");
        let any_relevant = outcome.issues.iter().any(|i| {
            i.message.contains("frobnicate")
                || i.message.contains("temper-stage")
                || i.path.contains("temper-stage")
        });
        assert!(
            any_relevant,
            "expected an issue mentioning temper-stage or frobnicate, got: {:?}",
            outcome.issues
        );

        // Hashes still populated.
        assert!(outcome.managed_hash.starts_with("sha256:"));
        assert!(outcome.open_hash.starts_with("sha256:"));
        assert!(outcome.body_hash.starts_with("sha256:"));
    }

    // 4. normalize_provisional_task_validates_clean
    #[test]
    fn normalize_provisional_task_validates_clean() {
        let dir = tempdir().unwrap();
        let content = r#"---
temper-provisional-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-12T00:00:00Z"
temper-title: Test
slug: test
---
body
"#;
        let path = write_file(dir.path(), "task.md", content);
        let outcome = normalize_file(&path, "task").expect("normalize ok");
        assert!(
            outcome.issues.is_empty(),
            "provisional file should validate clean, got: {:?}",
            outcome.issues
        );
        let on_disk = read_file(&path);
        assert!(on_disk.contains("temper-provisional-id"));
        assert!(
            !on_disk.contains("temper-id:"),
            "normalize must not introduce a synthetic temper-id"
        );
    }

    // 5. normalize_goal_missing_status_rewrites_with_default
    #[test]
    fn normalize_goal_missing_status_rewrites_with_default() {
        let dir = tempdir().unwrap();
        let content = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62886"
temper-type: goal
temper-context: temper
temper-created: "2026-04-12T00:00:00Z"
temper-title: Ship
slug: ship
---
"#;
        let path = write_file(dir.path(), "goal.md", content);
        let outcome = normalize_file(&path, "goal").expect("normalize ok");
        assert!(outcome.changed);
        assert!(outcome.issues.is_empty(), "got: {:?}", outcome.issues);
        let on_disk = read_file(&path);
        assert!(
            on_disk.contains("temper-status: active"),
            "should contain temper-status: active, got:\n{on_disk}"
        );
    }

    // 6. normalize_session_missing_date_rewrites_with_default
    #[test]
    fn normalize_session_missing_date_rewrites_with_default() {
        let dir = tempdir().unwrap();
        // Session uses canonical form with temper-title
        let content = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62887"
temper-type: session
temper-context: temper
temper-created: "2026-04-12T00:00:00Z"
temper-title: Planning
slug: planning
---
"#;
        let path = write_file(dir.path(), "session.md", content);
        let outcome = normalize_file(&path, "session").expect("normalize ok");
        assert!(outcome.changed);
        assert!(outcome.issues.is_empty(), "got: {:?}", outcome.issues);
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let on_disk = read_file(&path);
        assert!(
            on_disk.contains(&format!("date: {today}"))
                || on_disk.contains(&format!("date: '{today}'")),
            "should contain today's date, got:\n{on_disk}"
        );
    }

    // 7. normalize_preserves_key_order
    #[test]
    fn normalize_preserves_key_order() {
        let dir = tempdir().unwrap();
        let content = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62888"
temper-type: task
temper-context: temper
temper-created: "2026-04-12T00:00:00Z"
temper-title: X
slug: x
---
"#;
        let path = write_file(dir.path(), "task.md", content);
        let _ = normalize_file(&path, "task").expect("normalize ok");

        let on_disk = read_file(&path);
        let pos_title = on_disk.find("temper-title:").expect("temper-title present");
        let pos_slug = on_disk.find("slug:").expect("slug present");
        let pos_stage = on_disk.find("temper-stage:").expect("temper-stage present");
        assert!(
            pos_title < pos_slug,
            "temper-title should appear before slug:\n{on_disk}"
        );
        assert!(
            pos_slug < pos_stage,
            "slug should appear before temper-stage (canonical display ordering places managed fields after open fields):\n{on_disk}"
        );
    }

    // 8. normalize_preserves_body_content_exactly
    #[test]
    fn normalize_preserves_body_content_exactly() {
        let dir = tempdir().unwrap();
        let body =
            "# Heading\n\n```rust\nfn main() {}\n```\n\nLine with trailing space   \n\nLast line\n";
        let content = format!(
            r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62889"
temper-type: task
temper-context: temper
temper-created: "2026-04-12T00:00:00Z"
temper-title: Test
slug: test
---
{body}"#
        );
        let path = write_file(dir.path(), "task.md", &content);
        let outcome = normalize_file(&path, "task").expect("normalize ok");
        assert!(outcome.changed, "missing temper-stage triggers rewrite");

        let on_disk = read_file(&path);
        let fm_on_disk = Frontmatter::try_from(on_disk.as_str()).expect("parse normalized file");
        assert_eq!(
            fm_on_disk.body(),
            body,
            "body should be byte-identical after normalize"
        );
    }

    // 9. normalize_file_inspect_does_not_write
    #[test]
    fn normalize_file_inspect_does_not_write() {
        let dir = tempdir().unwrap();
        let content = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed6288a"
temper-type: task
temper-context: temper
temper-created: "2026-04-12T00:00:00Z"
temper-title: Test
slug: test
---
body
"#;
        let path = write_file(dir.path(), "task.md", content);
        let before = read_file(&path);

        let outcome = normalize_file_inspect(&path, "task").expect("inspect ok");
        assert!(outcome.changed, "would rewrite (missing temper-stage)");
        let after = read_file(&path);
        assert_eq!(before, after, "inspect must not write to disk");
    }

    // 10. normalize_file_missing_frontmatter_errors
    #[test]
    fn normalize_file_missing_frontmatter_errors() {
        let dir = tempdir().unwrap();
        let path = write_file(dir.path(), "task.md", "no frontmatter here\n");
        let result = normalize_file(&path, "task");
        assert!(result.is_err(), "missing frontmatter should error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("frontmatter") || err.contains("task.md"),
            "error should mention frontmatter or path, got: {err}"
        );
    }

    // 11. normalize_hash_matches_frontmatter_hashes_helper
    #[test]
    fn normalize_hash_matches_frontmatter_hashes_helper() {
        let dir = tempdir().unwrap();
        // Canonical form: serde_yaml serializes UUIDs and datetimes unquoted;
        // tier1 order: temper-context before temper-type per TIER1_SYSTEM_FIELDS.
        // title is renamed to temper-title per temper-prefix contract.
        let canonical_text = r#"---
temper-id: 019d8110-8ff3-70c2-85ae-57e04ed6288b
temper-context: temper
temper-type: task
temper-created: 2026-04-12T00:00:00Z
temper-title: Test
slug: test
temper-stage: backlog
---
hello body
"#;
        let path = write_file(dir.path(), "task.md", canonical_text);

        let outcome = normalize_file(&path, "task").expect("normalize ok");
        assert!(!outcome.changed);

        // Compare against a fresh Frontmatter parse of the same canonical text.
        let fm = Frontmatter::try_from(canonical_text).expect("parse ok");
        let (direct_managed, direct_open) = fm.hashes();
        let direct_body = compute_body_hash(fm.body());

        assert_eq!(outcome.body_hash, direct_body);
        assert_eq!(outcome.managed_hash, direct_managed);
        assert_eq!(outcome.open_hash, direct_open);
    }

    // 12. apply_doc_type_defaults_yaml_no_overwrite
    #[test]
    fn apply_doc_type_defaults_yaml_no_overwrite() {
        let mut mapping = serde_yaml::Mapping::new();
        mapping.insert(
            serde_yaml::Value::String("temper-stage".to_string()),
            serde_yaml::Value::String("in-progress".to_string()),
        );
        apply_doc_type_defaults_yaml("task", &mut mapping);
        let value = mapping
            .get(serde_yaml::Value::String("temper-stage".to_string()))
            .and_then(|v| v.as_str())
            .unwrap();
        assert_eq!(value, "in-progress", "must not overwrite explicit value");
    }
}
