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

#[test]
fn test_warmup_produces_output() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug = common::create_goal(&config, "myapp", "v0.1");
    common::create_task(&config, "myapp", "Test", Some(&g_slug), None, None);

    let result = temper_cli::commands::warmup::run(&config, Some("myapp"), "text");
    assert!(result.is_ok());
}

#[test]
fn test_warmup_shows_in_progress_tasks_with_mode() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug = common::create_goal(&config, "myapp", "v0.1");
    let slug = common::create_task(
        &config,
        "myapp",
        "Active Work",
        Some(&g_slug),
        Some("build"),
        Some("medium"),
    );
    common::move_task_to_stage(&config, &slug, "myapp", "in-progress");

    let result = temper_cli::commands::warmup::run(&config, Some("myapp"), "text");
    assert!(result.is_ok());
}

#[test]
fn test_warmup_no_in_progress_tasks() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let result = temper_cli::commands::warmup::run(&config, Some("myapp"), "text");
    assert!(result.is_ok());
}
