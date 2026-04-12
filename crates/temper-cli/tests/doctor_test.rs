use std::fs;

use tempfile::TempDir;

fn test_config(dir: &TempDir) -> temper_cli::config::Config {
    let state_dir = dir.path().join(".temper");
    fs::create_dir_all(&state_dir).unwrap();
    fs::write(state_dir.join("manifest.json"), "{}\n").unwrap();
    fs::write(state_dir.join("events.jsonl"), "").unwrap();
    temper_cli::config::Config {
        vault_root: dir.path().to_path_buf(),
        state_dir,
        contexts: vec!["temper".to_string()],
        subscriptions: Vec::new(),
        skill_output: dir.path().join("temper.md"),
    }
}

fn write_vault_file(dir: &TempDir, rel_path: &str, content: &str) {
    let path = dir.path().join(rel_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

// ---------------------------------------------------------------------------
// A valid new-format task (all required base + task fields present)
// ---------------------------------------------------------------------------
const VALID_TASK_FM: &str = r#"---
temper-id: "01900000-0000-7000-8000-000000000001"
temper-type: task
temper-context: temper
temper-created: "2026-01-01T00:00:00Z"
temper-owner: "@me"
title: "Implement feature X"
temper-stage: backlog
slug: implement-feature-x
---

Body text.
"#;

// ---------------------------------------------------------------------------
// A valid new-format goal
// ---------------------------------------------------------------------------
const VALID_GOAL_FM: &str = r#"---
temper-id: "01900000-0000-7000-8000-000000000002"
temper-type: goal
temper-context: temper
temper-created: "2026-01-01T00:00:00Z"
temper-owner: "@me"
title: "Ship v1"
temper-status: active
slug: ship-v1
---

Goal body.
"#;

// ---------------------------------------------------------------------------
// A valid session
// ---------------------------------------------------------------------------
const VALID_SESSION_FM: &str = r#"---
temper-id: "01900000-0000-7000-8000-000000000003"
temper-type: session
temper-context: temper
temper-created: "2026-01-01T00:00:00Z"
temper-owner: "@me"
title: "Session 2026-01-01"
date: "2026-01-01"
---

Session notes.
"#;

// ---------------------------------------------------------------------------
// Old-format task using legacy field names
// ---------------------------------------------------------------------------
const LEGACY_TASK_FM: &str = r#"---
id: "01900000-0000-7000-8000-000000000010"
type: task
context: temper
created: "2026-01-01T00:00:00Z"
title: "Old task"
stage: backlog
slug: old-task
---

Legacy body.
"#;

// ---------------------------------------------------------------------------
// Task with invalid enum value for temper-stage
// ---------------------------------------------------------------------------
const INVALID_STAGE_TASK_FM: &str = r#"---
temper-id: "01900000-0000-7000-8000-000000000020"
temper-type: task
temper-context: temper
temper-created: "2026-01-01T00:00:00Z"
title: "Bad stage task"
temper-stage: active
slug: bad-stage-task
---

Body.
"#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_doctor_valid_task_no_issues() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    write_vault_file(
        &dir,
        "@me/temper/task/implement-feature-x.md",
        VALID_TASK_FM,
    );

    let report = temper_cli::actions::doctor::scan(&config, None).unwrap();

    assert_eq!(report.files_checked, 1, "should scan exactly one file");
    assert_eq!(
        report.total_issues,
        0,
        "valid task should have no issues; got: {:?}",
        report
            .file_results
            .iter()
            .flat_map(|r| &r.issues)
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_doctor_detects_legacy_fields() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    write_vault_file(&dir, "@me/temper/task/old-task.md", LEGACY_TASK_FM);

    let report = temper_cli::actions::doctor::scan(&config, None).unwrap();

    assert_eq!(report.files_checked, 1);
    assert!(
        report.total_issues > 0,
        "legacy fields should produce issues"
    );
    assert!(
        report.auto_fixable > 0,
        "legacy field issues should be auto-fixable"
    );

    // Verify the legacy field messages are present
    let all_messages: Vec<&str> = report
        .file_results
        .iter()
        .flat_map(|r| r.issues.iter().map(|i| i.message.as_str()))
        .collect();
    assert!(
        all_messages.iter().any(|m| m.contains("legacy field")),
        "should mention 'legacy field' in at least one issue message"
    );
}

#[test]
fn test_doctor_detects_invalid_enum() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    write_vault_file(
        &dir,
        "@me/temper/task/bad-stage-task.md",
        INVALID_STAGE_TASK_FM,
    );

    let report = temper_cli::actions::doctor::scan(&config, None).unwrap();

    assert_eq!(report.files_checked, 1);
    assert!(
        report.total_issues > 0,
        "invalid enum value should produce a validation error"
    );
}

#[test]
fn test_doctor_valid_session_no_issues() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    write_vault_file(
        &dir,
        "@me/temper/session/session-2026-01-01.md",
        VALID_SESSION_FM,
    );

    let report = temper_cli::actions::doctor::scan(&config, None).unwrap();

    assert_eq!(report.files_checked, 1);
    assert_eq!(
        report.total_issues,
        0,
        "valid session should have no issues; got: {:?}",
        report
            .file_results
            .iter()
            .flat_map(|r| &r.issues)
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_doctor_scans_multiple_doctypes() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    write_vault_file(
        &dir,
        "@me/temper/task/implement-feature-x.md",
        VALID_TASK_FM,
    );
    write_vault_file(&dir, "@me/temper/goal/ship-v1.md", VALID_GOAL_FM);

    let report = temper_cli::actions::doctor::scan(&config, None).unwrap();

    assert_eq!(
        report.files_checked, 2,
        "should scan task and goal files (one each)"
    );
    assert_eq!(
        report.total_issues, 0,
        "both valid files should produce no issues"
    );
}

#[test]
fn test_doctor_context_filter() {
    let dir = TempDir::new().unwrap();
    let mut config = test_config(&dir);
    config.contexts = vec!["temper".to_string(), "other".to_string()];

    write_vault_file(
        &dir,
        "@me/temper/task/implement-feature-x.md",
        VALID_TASK_FM,
    );
    // This file is in 'other' context and should be excluded by the filter
    write_vault_file(
        &dir,
        "@me/other/task/something.md",
        &VALID_TASK_FM.replace("temper-context: temper", "temper-context: other"),
    );

    let report = temper_cli::actions::doctor::scan(&config, Some("temper")).unwrap();

    assert_eq!(
        report.files_checked, 1,
        "context filter should restrict scan to 'temper' context only"
    );
}

#[test]
fn test_doctor_no_frontmatter_reports_issue() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    write_vault_file(
        &dir,
        "@me/temper/task/no-fm.md",
        "Just body text, no frontmatter.\n",
    );

    let report = temper_cli::actions::doctor::scan(&config, None).unwrap();

    assert_eq!(report.files_checked, 1);
    assert!(
        report.total_issues > 0,
        "missing frontmatter should be an issue"
    );
    let has_fm_issue = report
        .file_results
        .iter()
        .flat_map(|r| r.issues.iter())
        .any(|i| i.message.contains("No YAML frontmatter"));
    assert!(has_fm_issue, "should report missing frontmatter");
}

#[test]
fn test_doctor_fix_sets_missing_temper_fields() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    // File with temper-* style but missing some managed fields (temper-id, slug, temper-stage)
    write_vault_file(
        &dir,
        "@me/temper/task/old-task.md",
        "---\ntemper-type: task\ntemper-context: temper\ntemper-stage: backlog\ntemper-created: \"2026-04-03T21:23:32.026022-04:00\"\ntitle: \"Old task\"\nslug: old-task\n---\n\n# Old task\n\nSome content here.\n",
    );

    let result = temper_cli::actions::doctor::fix(&config, None, false).unwrap();
    assert!(
        result.fields_set + result.files_renamed + result.files_relocated >= 1,
        "Should have applied at least one fix"
    );

    let content = fs::read_to_string(dir.path().join("@me/temper/task/old-task.md")).unwrap();
    assert!(content.contains("temper-id:"), "got:\n{content}");
    assert!(content.contains("temper-type:"));
    assert!(content.contains("temper-context:"));
    assert!(content.contains("temper-stage:"));
    assert!(content.contains("temper-created:"));
    assert!(content.contains("Some content here."), "body preserved");
}

#[test]
fn test_doctor_fix_dry_run_does_not_modify() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    // File with missing temper-id — dry run should count the set action but not apply it
    let original = "---\ntemper-type: task\ntemper-context: temper\ntemper-stage: backlog\ntemper-created: \"2026-04-03T21:23:32.026022-04:00\"\ntitle: \"Old task\"\nslug: old-task\n---\n\n# Old task\n";

    write_vault_file(&dir, "@me/temper/task/old-task.md", original);

    let result = temper_cli::actions::doctor::fix(&config, None, true).unwrap();
    assert!(
        result.fields_set > 0,
        "Dry run should count field sets (temper-id missing)"
    );

    let content = fs::read_to_string(dir.path().join("@me/temper/task/old-task.md")).unwrap();
    assert_eq!(content, original, "Dry run should not modify file");
}

/// Regression for the "doctor errors on provisional files" bug.
///
/// A task with only `temper-provisional-id` (never synced) must scan clean:
/// no schema violations, no "would rewrite" hints, no issues at all. This
/// exercises the normalize pipeline path used by `scan_file` post-consolidation.
#[test]
fn doctor_scan_provisional_task_reports_no_issues() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    write_vault_file(
        &dir,
        "@me/temper/task/provisional-task.md",
        "---\n\
         temper-provisional-id: \"01900000-0000-7000-8000-0000000000aa\"\n\
         temper-type: task\n\
         temper-context: temper\n\
         temper-created: \"2026-01-01T00:00:00Z\"\n\
         temper-owner: \"@me\"\n\
         title: \"Provisional task\"\n\
         temper-stage: backlog\n\
         slug: provisional-task\n\
         ---\n\n\
         Body.\n",
    );

    let report = temper_cli::actions::doctor::scan(&config, None).unwrap();

    assert_eq!(report.files_checked, 1);
    assert_eq!(
        report.total_issues,
        0,
        "provisional task with all required fields should scan clean; got: {:?}",
        report
            .file_results
            .iter()
            .flat_map(|r| &r.issues)
            .collect::<Vec<_>>()
    );
}

/// Regression for default materialization. `doctor fix` must materialize
/// doc-type defaults (here: `temper-stage: backlog` for tasks) via the
/// normalize primitive, not via a bespoke inference rule. The file on disk
/// must contain the default after `fix` runs.
#[test]
fn doctor_fix_materializes_defaults_via_normalize() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    write_vault_file(
        &dir,
        "@me/temper/task/needs-stage.md",
        "---\n\
         temper-id: \"01900000-0000-7000-8000-0000000000bb\"\n\
         temper-type: task\n\
         temper-context: temper\n\
         temper-created: \"2026-01-01T00:00:00Z\"\n\
         temper-owner: \"@me\"\n\
         title: \"Needs stage\"\n\
         slug: needs-stage\n\
         ---\n\n\
         Body.\n",
    );

    let _report = temper_cli::actions::doctor::fix(&config, None, false).unwrap();

    let content = fs::read_to_string(dir.path().join("@me/temper/task/needs-stage.md"))
        .expect("file should still exist");
    assert!(
        content.contains("temper-stage: backlog"),
        "fix should materialize temper-stage: backlog default, got:\n{content}"
    );
}

#[test]
fn test_doctor_fix_backfills_temper_created_from_date() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    write_vault_file(
        &dir,
        "@me/temper/session/my-session.md",
        "---\ntemper-id: \"019d5977-f476-7e41-b4aa-fc4bd2b24426\"\ntemper-type: session\ntemper-context: temper\ntitle: \"My session\"\ndate: \"2026-04-04\"\n---\n\n## Goal\n",
    );

    let result = temper_cli::actions::doctor::fix(&config, None, false).unwrap();
    assert!(
        result.fields_set > 0,
        "Should have set (backfilled) missing fields"
    );

    // The pipeline also renames the file to match the date-prefix convention.
    // Accept either the original name or the renamed file.
    let orig_path = dir.path().join("@me/temper/session/my-session.md");
    let renamed_path = dir
        .path()
        .join("@me/temper/session/2026-04-04-my-session.md");
    let content = if orig_path.exists() {
        fs::read_to_string(&orig_path).unwrap()
    } else {
        fs::read_to_string(&renamed_path).unwrap()
    };
    assert!(content.contains("temper-created:"), "got:\n{content}");
}
