//! Local-mode tests for `temper resource update --body @path`.
//!
//! These exercise the wire-through: that --body actually rewrites the
//! file body in local mode (before this task, the flag was silently
//! ignored).

use tempfile::TempDir;

mod common;

fn test_config(dir: &TempDir) -> temper_cli::config::Config {
    common::init_isolated_auth();
    let state_dir = dir.path().join(".temper");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(state_dir.join("manifest.json"), "{}\n").unwrap();
    std::fs::write(state_dir.join("events.jsonl"), "").unwrap();
    temper_cli::config::Config {
        vault_root: dir.path().to_path_buf(),
        state_dir,
        contexts: vec!["myapp".to_string()],
        subscriptions: Vec::new(),
        skill_output: dir.path().join("temper.md"),
        profile_slug: None,
    }
}

fn write_body_file(dir: &TempDir, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    path
}

fn read_body(file: &std::path::Path) -> String {
    let fm = temper_core::frontmatter::Frontmatter::parse_file(file).unwrap();
    fm.body().to_string()
}

#[test]
fn local_mode_update_rewrites_goal_body_via_body_at_path() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let slug = common::create_goal(&config, "myapp", "Sample goal");
    let goal_file = dir
        .path()
        .join("@me")
        .join("myapp")
        .join("goal")
        .join(format!("{slug}.md"));
    assert!(goal_file.exists(), "goal file should be created");

    let new_body_path = write_body_file(&dir, "new_body.md", "# Rewritten\n\nNew body content.\n");

    let params = temper_cli::commands::resource::UpdateParams {
        slug: &slug,
        doc_type: Some("goal"),
        type_from: None,
        type_to: None,
        context: Some("myapp"),
        context_to: None,
        title: None,
        tags: &[],
        aliases: &[],
        relates_to: &[],
        references: &[],
        depends_on: &[],
        extends: &[],
        preceded_by: &[],
        derived_from: &[],
        stage: None,
        mode: None,
        effort: None,
        goal: None,
        seq: None,
        branch: None,
        pr: None,
        status: None,
        body: Some(format!("@{}", new_body_path.display())),
    };

    temper_cli::commands::resource::update(&config, &params).unwrap();

    let body = read_body(&goal_file);
    assert!(
        body.contains("New body content."),
        "body should be rewritten; got: {body}"
    );
    assert!(
        body.contains("# Rewritten"),
        "body should contain new H1; got: {body}"
    );
}

#[test]
fn local_mode_update_rewrites_task_body_via_body_at_path() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let goal_slug = common::create_goal(&config, "myapp", "Parent goal");
    let task_slug = common::create_task(
        &config,
        "myapp",
        "Sample task",
        Some(&goal_slug),
        None,
        None,
    );
    let task_file = dir
        .path()
        .join("@me")
        .join("myapp")
        .join("task")
        .join(format!("{task_slug}.md"));

    let new_body_path =
        write_body_file(&dir, "task_body.md", "# Task work log\n\nDay 1: started.\n");

    let params = temper_cli::commands::resource::UpdateParams {
        slug: &task_slug,
        doc_type: Some("task"),
        type_from: None,
        type_to: None,
        context: Some("myapp"),
        context_to: None,
        title: None,
        tags: &[],
        aliases: &[],
        relates_to: &[],
        references: &[],
        depends_on: &[],
        extends: &[],
        preceded_by: &[],
        derived_from: &[],
        stage: None,
        mode: None,
        effort: None,
        goal: None,
        seq: None,
        branch: None,
        pr: None,
        status: None,
        body: Some(format!("@{}", new_body_path.display())),
    };

    temper_cli::commands::resource::update(&config, &params).unwrap();

    let body = read_body(&task_file);
    assert!(
        body.contains("Day 1: started."),
        "task body should be rewritten; got: {body}"
    );
}

#[test]
fn local_mode_update_rewrites_session_body_via_body_at_path() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    temper_cli::commands::session::save(
        &config,
        Some("Working session"),
        Some("myapp"),
        None,
        None,
        None,
        "text",
    )
    .unwrap();

    let session_dir = dir.path().join("@me").join("myapp").join("session");
    let session_files: Vec<std::path::PathBuf> = std::fs::read_dir(&session_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("md"))
        .collect();
    assert_eq!(
        session_files.len(),
        1,
        "expected exactly one .md session file, found: {session_files:?}"
    );
    let session_file = session_files.into_iter().next().unwrap();
    let session_slug = session_file
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .to_string();

    let new_body_path = write_body_file(
        &dir,
        "session_body.md",
        "# Session notes\n\nDecisions: shipped X.\n",
    );

    let params = temper_cli::commands::resource::UpdateParams {
        slug: &session_slug,
        doc_type: Some("session"),
        type_from: None,
        type_to: None,
        context: Some("myapp"),
        context_to: None,
        title: None,
        tags: &[],
        aliases: &[],
        relates_to: &[],
        references: &[],
        depends_on: &[],
        extends: &[],
        preceded_by: &[],
        derived_from: &[],
        stage: None,
        mode: None,
        effort: None,
        goal: None,
        seq: None,
        branch: None,
        pr: None,
        status: None,
        body: Some(format!("@{}", new_body_path.display())),
    };

    temper_cli::commands::resource::update(&config, &params).unwrap();

    let body = read_body(&session_file);
    assert!(
        body.contains("Decisions: shipped X."),
        "session body should be rewritten; got: {body}"
    );
}

#[test]
fn local_mode_update_no_body_flag_preserves_existing_body() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let slug = common::create_goal(&config, "myapp", "Preserved goal");
    let goal_file = dir
        .path()
        .join("@me")
        .join("myapp")
        .join("goal")
        .join(format!("{slug}.md"));
    let original_body = read_body(&goal_file);

    let params = temper_cli::commands::resource::UpdateParams {
        slug: &slug,
        doc_type: Some("goal"),
        type_from: None,
        type_to: None,
        context: Some("myapp"),
        context_to: None,
        title: Some("Renamed goal"),
        tags: &[],
        aliases: &[],
        relates_to: &[],
        references: &[],
        depends_on: &[],
        extends: &[],
        preceded_by: &[],
        derived_from: &[],
        stage: None,
        mode: None,
        effort: None,
        goal: None,
        seq: None,
        branch: None,
        pr: None,
        status: None,
        body: None,
    };

    temper_cli::commands::resource::update(&config, &params).unwrap();

    let body = read_body(&goal_file);
    assert_eq!(
        body, original_body,
        "body must be unchanged when --body is omitted"
    );
}

#[test]
fn local_mode_update_invalid_body_flag_errors_before_mutation() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let slug = common::create_goal(&config, "myapp", "Untouched goal");
    let goal_file = dir
        .path()
        .join("@me")
        .join("myapp")
        .join("goal")
        .join(format!("{slug}.md"));
    let before = std::fs::read_to_string(&goal_file).unwrap();

    let params = temper_cli::commands::resource::UpdateParams {
        slug: &slug,
        doc_type: Some("goal"),
        type_from: None,
        type_to: None,
        context: Some("myapp"),
        context_to: None,
        title: Some("Should not apply"),
        tags: &[],
        aliases: &[],
        relates_to: &[],
        references: &[],
        depends_on: &[],
        extends: &[],
        preceded_by: &[],
        derived_from: &[],
        stage: None,
        mode: None,
        effort: None,
        goal: None,
        seq: None,
        branch: None,
        pr: None,
        status: None,
        body: Some("not-a-valid-flag-value".to_string()),
    };

    let result = temper_cli::commands::resource::update(&config, &params);
    assert!(result.is_err(), "malformed --body must error");

    let after = std::fs::read_to_string(&goal_file).unwrap();
    assert_eq!(
        before, after,
        "goal file must be unchanged when --body errors"
    );
}
