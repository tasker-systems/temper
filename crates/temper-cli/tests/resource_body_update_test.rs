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
    }
}

fn write_body_file(dir: &TempDir, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    path
}

fn read_body(file: &std::path::Path) -> String {
    let raw = std::fs::read_to_string(file).unwrap();
    // Strip frontmatter: everything after the second "---\n".
    let after_first = raw.split_once("---\n").map(|(_, r)| r).unwrap_or(&raw);
    let after_second = after_first
        .split_once("---\n")
        .map(|(_, r)| r)
        .unwrap_or(after_first);
    after_second.to_string()
}

#[test]
fn local_mode_update_rewrites_goal_body_via_body_at_path() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let slug =
        temper_cli::commands::goal::create(&config, "myapp", "Sample goal", None, "text").unwrap();
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
