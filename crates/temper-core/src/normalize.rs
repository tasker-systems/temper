//! File normalization primitive — schema validate, apply defaults to YAML
//! frontmatter, rewrite the file if changed, and recompute three hashes.
//!
//! Used by every `temper sync` subcommand and by `temper doctor` to enforce
//! the invariant that on-disk file state matches the normalized form its
//! doc-type schema declares.
//!
//! See [`normalize_file`] for the canonical entry point and
//! [`normalize_file_inspect`] for the read-only dry-run variant. Both share
//! the same validation, default-materialization, and hash-computation logic;
//! only the dry-run variant skips the disk write.

use crate::error::{Result, TemperError};
use crate::hash::{compute_body_hash, compute_frontmatter_hashes_from_yaml};
use crate::schema::{validate_allowing_provisional, ValidationIssue};
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
    let original = std::fs::read_to_string(path)
        .map_err(|e| TemperError::Config(format!("failed to read {}: {e}", path.display())))?;

    let (yaml_text, body) = split_frontmatter_block(&original, path)?;

    let original_value: serde_yaml::Value = serde_yaml::from_str(yaml_text).map_err(|e| {
        TemperError::Config(format!(
            "failed to parse YAML frontmatter in {}: {e}",
            path.display()
        ))
    })?;

    let original_mapping = original_value
        .as_mapping()
        .ok_or_else(|| {
            TemperError::Config(format!(
                "frontmatter in {} is not a YAML mapping",
                path.display()
            ))
        })?
        .clone();

    // Apply defaults to a working copy. Key insertion order is preserved
    // because `serde_yaml::Mapping` is backed by an insertion-ordered
    // `IndexMap`.
    let mut normalized_mapping = original_mapping.clone();
    apply_doc_type_defaults_yaml(doc_type, &mut normalized_mapping);

    // Validate the post-defaults state. Defaults satisfy required-field
    // checks; what remains is genuinely user-attention-required.
    let normalized_value = serde_yaml::Value::Mapping(normalized_mapping.clone());
    let issues = validate_allowing_provisional(doc_type, &normalized_value)?;

    let body_hash = compute_body_hash(body);

    if !issues.is_empty() {
        // Non-conformant: do not rewrite. Hashes describe on-disk reality,
        // so the caller can update its manifest while still blocking sync.
        let original_value_for_hash = serde_yaml::Value::Mapping(original_mapping);
        let (managed_hash, open_hash) =
            compute_frontmatter_hashes_from_yaml(Some(&original_value_for_hash), doc_type);
        return Ok(NormalizeOutcome {
            changed: false,
            body_hash,
            managed_hash,
            open_hash,
            issues,
        });
    }

    // Recompose the file from the normalized mapping. If the result differs
    // from the original on-disk text, that's a "changed" — either defaults
    // were inserted or YAML reserialization shifted bytes.
    let new_content = compose_file(&normalized_value, body)?;
    let changed = new_content != original;

    if changed && write {
        write_atomic(path, &new_content)?;
    }

    let (managed_hash, open_hash) =
        compute_frontmatter_hashes_from_yaml(Some(&normalized_value), doc_type);

    Ok(NormalizeOutcome {
        changed,
        body_hash,
        managed_hash,
        open_hash,
        issues,
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
// Internal: file composition / decomposition
// ---------------------------------------------------------------------------

/// Split a vault file into its YAML frontmatter text and the body that
/// follows. Returns an error if the file does not begin with a `---`
/// frontmatter block.
fn split_frontmatter_block<'a>(content: &'a str, path: &Path) -> Result<(&'a str, &'a str)> {
    // Strip an optional UTF-8 BOM but otherwise require the file to begin
    // with `---` followed by a newline.
    let stripped = content.strip_prefix('\u{feff}').unwrap_or(content);

    let after_open = stripped
        .strip_prefix("---\n")
        .or_else(|| stripped.strip_prefix("---\r\n"))
        .ok_or_else(|| {
            TemperError::Config(format!(
                "missing frontmatter block in {}: file must begin with '---'",
                path.display()
            ))
        })?;

    // Find the closing `---` line.
    let close_idx = find_closing_fence(after_open).ok_or_else(|| {
        TemperError::Config(format!(
            "unterminated frontmatter block in {}: missing closing '---'",
            path.display()
        ))
    })?;

    let yaml_text = &after_open[..close_idx];
    let after_yaml = &after_open[close_idx..];

    // Skip past the closing fence + its trailing newline (or EOF).
    let body = after_yaml
        .strip_prefix("---\n")
        .or_else(|| after_yaml.strip_prefix("---\r\n"))
        .or_else(|| after_yaml.strip_prefix("---"))
        .unwrap_or("");

    Ok((yaml_text, body))
}

/// Locate the byte offset of a `---` line in `after_open` (which begins
/// with the YAML body). Returns the offset of the `-` character.
fn find_closing_fence(after_open: &str) -> Option<usize> {
    let mut search_from = 0;
    while let Some(rel) = after_open[search_from..].find("---") {
        let abs = search_from + rel;
        // Must be at the start of a line (preceded by `\n` or be at the
        // very start, which we've already consumed via strip_prefix).
        let at_line_start = abs == 0 || after_open.as_bytes()[abs - 1] == b'\n';
        // Must be followed by `\n`, `\r\n`, or EOF — to avoid matching
        // `---x` mid-document.
        let after = &after_open[abs + 3..];
        let at_line_end = after.is_empty() || after.starts_with('\n') || after.starts_with("\r\n");
        if at_line_start && at_line_end {
            return Some(abs);
        }
        search_from = abs + 3;
    }
    None
}

/// Recompose a file from a YAML frontmatter value and a body. Format is
/// `---\n<yaml>---\n<body>` — the YAML emitter terminates with `\n` so a
/// closing fence on its own line follows naturally.
fn compose_file(frontmatter: &serde_yaml::Value, body: &str) -> Result<String> {
    let yaml_text = serde_yaml::to_string(frontmatter)
        .map_err(|e| TemperError::Config(format!("failed to serialize frontmatter: {e}")))?;
    // serde_yaml's emitter ends with a single `\n`. Guard against future
    // behavior changes by ensuring exactly one trailing newline before the
    // closing fence.
    let mut yaml_normalized = yaml_text.trim_end_matches('\n').to_string();
    yaml_normalized.push('\n');
    Ok(format!("---\n{yaml_normalized}---\n{body}"))
}

/// Atomically replace `path` with `content` by writing to a sibling temp
/// file and renaming.
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        TemperError::Config(format!("path has no parent directory: {}", path.display()))
    })?;
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| TemperError::Config(format!("invalid file name: {}", path.display())))?;
    let tmp_path = parent.join(format!(".{file_name}.normalize.tmp"));

    std::fs::write(&tmp_path, content)
        .map_err(|e| TemperError::Config(format!("failed to write {}: {e}", tmp_path.display())))?;
    std::fs::rename(&tmp_path, path).map_err(|e| {
        TemperError::Config(format!(
            "failed to rename {} -> {}: {e}",
            tmp_path.display(),
            path.display()
        ))
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
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
title: Test
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
        // Construct content via compose_file so the YAML emitter format
        // matches what normalize would produce.
        let yaml_str = r#"
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-12T00:00:00Z"
title: Test
slug: test
temper-stage: in-progress
"#;
        let value: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap();
        let canonical = compose_file(&value, "body content\n").unwrap();
        let path = write_file(dir.path(), "task.md", &canonical);
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
title: Test
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
title: Test
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
title: Ship
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
        let content = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62887"
temper-type: session
temper-context: temper
temper-created: "2026-04-12T00:00:00Z"
title: Planning
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
title: X
slug: x
---
"#;
        let path = write_file(dir.path(), "task.md", content);
        let _ = normalize_file(&path, "task").expect("normalize ok");

        let on_disk = read_file(&path);
        let pos_title = on_disk.find("title:").expect("title present");
        let pos_slug = on_disk.find("slug:").expect("slug present");
        let pos_stage = on_disk.find("temper-stage:").expect("temper-stage present");
        assert!(
            pos_title < pos_slug,
            "title should appear before slug:\n{on_disk}"
        );
        assert!(
            pos_slug < pos_stage,
            "slug should appear before temper-stage (default appended last):\n{on_disk}"
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
title: Test
slug: test
---
{body}"#
        );
        let path = write_file(dir.path(), "task.md", &content);
        let outcome = normalize_file(&path, "task").expect("normalize ok");
        assert!(outcome.changed, "missing temper-stage triggers rewrite");

        let on_disk = read_file(&path);
        let (_, on_disk_body) = split_frontmatter_block(&on_disk, &path).unwrap();
        assert_eq!(
            on_disk_body, body,
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
title: Test
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

    // 11. normalize_hash_matches_direct_hash_helper
    #[test]
    fn normalize_hash_matches_direct_hash_helper() {
        let dir = tempdir().unwrap();
        let yaml_str = r#"
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed6288b"
temper-type: task
temper-context: temper
temper-created: "2026-04-12T00:00:00Z"
title: Test
slug: test
temper-stage: backlog
"#;
        let value: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap();
        let body = "hello body\n";
        let content = compose_file(&value, body).unwrap();
        let path = write_file(dir.path(), "task.md", &content);

        let outcome = normalize_file(&path, "task").expect("normalize ok");
        assert!(!outcome.changed);

        let direct_body = compute_body_hash(body);
        let (direct_managed, direct_open) =
            compute_frontmatter_hashes_from_yaml(Some(&value), "task");
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
