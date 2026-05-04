use serde_yaml::Value;
use temper_core::frontmatter::Frontmatter;
use temper_core::schema::{
    check_legacy_fields, check_unknown_temper_fields, load_schema, validate_frontmatter,
};

fn yaml(s: &str) -> Value {
    serde_yaml::from_str(s).expect("valid YAML")
}

// ---------------------------------------------------------------------------
// load_schema tests
// ---------------------------------------------------------------------------

#[test]
fn test_load_schema_for_each_doctype() {
    for doctype in &["task", "goal", "session", "research", "decision", "concept"] {
        let result = load_schema(doctype);
        assert!(
            result.is_ok(),
            "load_schema({doctype}) failed: {:?}",
            result.err()
        );
    }
}

#[test]
fn test_load_schema_unknown_doctype_fails() {
    let result = load_schema("unknown");
    assert!(result.is_err(), "expected error for unknown doctype");
}

// ---------------------------------------------------------------------------
// validate_frontmatter tests
// ---------------------------------------------------------------------------

fn valid_task_frontmatter() -> &'static str {
    r#"
temper-id: "01930000-0000-7000-8000-000000000001"
temper-type: task
temper-context: my-project
temper-created: "2024-01-01T00:00:00Z"
temper-title: "My Task"
temper-stage: backlog
temper-slug: my-task
"#
}

#[test]
fn test_validate_valid_task_frontmatter() {
    let fm = yaml(valid_task_frontmatter());
    let issues = validate_frontmatter("task", &fm).expect("validate_frontmatter succeeded");
    assert!(
        issues.is_empty(),
        "expected no issues for valid task, got: {:?}",
        issues
    );
}

#[test]
fn test_validate_task_missing_required_field() {
    // Missing temper-stage and slug
    let fm = yaml(
        r#"
temper-id: "01930000-0000-7000-8000-000000000002"
temper-type: task
temper-context: my-project
temper-created: "2024-01-01T00:00:00Z"
temper-title: "My Task"
"#,
    );
    let issues = validate_frontmatter("task", &fm).expect("validate_frontmatter succeeded");
    assert!(
        !issues.is_empty(),
        "expected issues for task missing temper-stage"
    );
    let messages: Vec<_> = issues.iter().map(|i| i.message.as_str()).collect();
    let any_stage = messages
        .iter()
        .any(|m| m.contains("temper-stage") || m.contains("slug") || m.contains("required"));
    assert!(
        any_stage,
        "expected a required-field error, got: {messages:?}"
    );
}

#[test]
fn test_validate_task_invalid_stage_enum() {
    let fm = yaml(
        r#"
temper-id: "01930000-0000-7000-8000-000000000003"
temper-type: task
temper-context: my-project
temper-created: "2024-01-01T00:00:00Z"
temper-title: "My Task"
temper-stage: active
temper-slug: my-task
"#,
    );
    let issues = validate_frontmatter("task", &fm).expect("validate_frontmatter succeeded");
    assert!(
        !issues.is_empty(),
        "expected issues for invalid temper-stage 'active'"
    );
}

#[test]
fn test_validate_valid_session_frontmatter() {
    let fm = yaml(
        r#"
temper-id: "01930000-0000-7000-8000-000000000010"
temper-type: session
temper-context: my-project
temper-created: "2024-01-01T00:00:00Z"
temper-title: "Session 1"
date: "2024-01-01"
"#,
    );
    let issues = validate_frontmatter("session", &fm).expect("validate_frontmatter succeeded");
    assert!(
        issues.is_empty(),
        "expected no issues for valid session, got: {:?}",
        issues
    );
}

#[test]
fn test_validate_additional_properties_preserved() {
    // User-defined fields should not cause errors (additionalProperties: true in schemas)
    let fm = yaml(
        r#"
temper-id: "01930000-0000-7000-8000-000000000020"
temper-type: task
temper-context: my-project
temper-created: "2024-01-01T00:00:00Z"
temper-title: "My Task"
temper-stage: backlog
temper-slug: my-task
my-custom-field: "hello"
another-user-field: 42
"#,
    );
    let issues = validate_frontmatter("task", &fm).expect("validate_frontmatter succeeded");
    assert!(
        issues.is_empty(),
        "user-defined fields should not cause validation errors, got: {:?}",
        issues
    );
}

// ---------------------------------------------------------------------------
// check_legacy_fields tests
// ---------------------------------------------------------------------------

#[test]
fn test_check_legacy_fields_detects_old_names() {
    let fm = yaml(
        r#"
id: "some-old-uuid"
type: task
context: my-project
project: my-project
doc_type: task
temper-title: "My Task"
"#,
    );
    let issues = check_legacy_fields(&fm);
    assert!(!issues.is_empty(), "expected legacy field issues, got none");

    // All legacy field issues should be auto-fixable
    for issue in &issues {
        assert!(
            issue.auto_fixable,
            "legacy field issue should be auto_fixable: {:?}",
            issue
        );
    }

    // Should detect the known legacy fields
    let paths: Vec<_> = issues.iter().map(|i| i.path.as_str()).collect();
    assert!(
        paths.contains(&"id") || paths.contains(&"type") || paths.contains(&"context"),
        "expected 'id', 'type', or 'context' in paths, got: {paths:?}"
    );
}

#[test]
fn test_check_legacy_fields_clean_doc_has_none() {
    // Inline fixture with canonical (post-temper-prefix) keys. We deliberately
    // do NOT use `valid_task_frontmatter()` here because that fixture still
    // uses bare `title:` and `slug:` (required by the current schemas until
    // the schema-rename tasks land); those bare keys are now in
    // LEGACY_FIELDS and would trigger the scanner.
    let fm = yaml(
        r#"
temper-id: "01930000-0000-7000-8000-000000000001"
temper-type: task
temper-context: my-project
temper-created: "2024-01-01T00:00:00Z"
temper-temper-title: "My Task"
temper-stage: backlog
temper-slug: my-task
"#,
    );
    let issues = check_legacy_fields(&fm);
    assert!(
        issues.is_empty(),
        "expected no legacy issues for clean doc, got: {:?}",
        issues
    );
}

// ---------------------------------------------------------------------------
// check_unknown_temper_fields tests
// ---------------------------------------------------------------------------

#[test]
fn test_check_unknown_temper_fields() {
    let fm = yaml(
        r#"
temper-id: "01930000-0000-7000-8000-000000000030"
temper-type: task
temper-context: my-project
temper-created: "2024-01-01T00:00:00Z"
temper-stge: backlog
temper-title: "My Task"
"#,
    );
    let issues = check_unknown_temper_fields(&fm);
    assert!(
        !issues.is_empty(),
        "expected unknown temper-* field issue for typo 'temper-stge'"
    );
    let paths: Vec<_> = issues.iter().map(|i| i.path.as_str()).collect();
    assert!(
        paths.contains(&"temper-stge"),
        "expected 'temper-stge' in unknown fields, got: {paths:?}"
    );
}

#[test]
fn test_check_unknown_temper_fields_known_fields_ok() {
    let fm = yaml(valid_task_frontmatter());
    let issues = check_unknown_temper_fields(&fm);
    assert!(
        issues.is_empty(),
        "expected no unknown temper-* issues for valid doc, got: {:?}",
        issues
    );
}

// ---------------------------------------------------------------------------
// hash tier tests (using temper_core::hash)
// ---------------------------------------------------------------------------

#[test]
fn test_hash_tiers_separate_managed_and_open() {
    let fm1 = Frontmatter::try_from(
        r#"---
temper-id: "01930000-0000-7000-8000-000000000040"
temper-type: task
temper-context: my-project
temper-created: "2024-01-01T00:00:00Z"
temper-title: "My Task"
open-field: "hello"
---
"#,
    )
    .expect("parse task fixture");
    let fm2 = Frontmatter::try_from(
        r#"---
temper-id: "01930000-0000-7000-8000-000000000041"
temper-type: goal
temper-context: my-project
temper-created: "2024-01-01T00:00:00Z"
temper-title: "My Task"
open-field: "hello"
---
"#,
    )
    .expect("parse goal fixture");

    let (meta1, open1) = fm1.hashes();
    let (meta2, open2) = fm2.hashes();

    // managed hash should differ because doc-type defaults differ
    assert_ne!(
        meta1, meta2,
        "managed_hash should differ when doc_type changes"
    );

    // open hash should be the same (same non-temper fields)
    assert_eq!(
        open1, open2,
        "open_hash should be equal when open fields are the same"
    );

    // hashes should be prefixed with sha256:
    assert!(
        meta1.starts_with("sha256:"),
        "managed_hash should start with 'sha256:'"
    );
    assert!(
        open1.starts_with("sha256:"),
        "open_hash should start with 'sha256:'"
    );
}

#[test]
fn test_hash_tiers_open_hash_changes_with_open_fields() {
    let fm1 = Frontmatter::try_from(
        r#"---
temper-id: "01930000-0000-7000-8000-000000000050"
temper-type: task
temper-context: my-project
temper-created: "2024-01-01T00:00:00Z"
temper-title: "My Task"
custom: "hello"
---
"#,
    )
    .expect("parse fm1");
    let fm2 = Frontmatter::try_from(
        r#"---
temper-id: "01930000-0000-7000-8000-000000000050"
temper-type: task
temper-context: my-project
temper-created: "2024-01-01T00:00:00Z"
temper-title: "My Task"
custom: "world"
---
"#,
    )
    .expect("parse fm2");

    let (_, open1) = fm1.hashes();
    let (_, open2) = fm2.hashes();

    assert_ne!(
        open1, open2,
        "open_hash should change when open fields change"
    );
}
