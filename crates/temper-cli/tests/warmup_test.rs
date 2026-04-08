use tempfile::TempDir;

fn test_config(dir: &TempDir) -> temper_cli::config::Config {
    let state_dir = dir.path().join(".temper");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(state_dir.join("manifest.json"), "{}\n").unwrap();
    std::fs::write(state_dir.join("events.jsonl"), "").unwrap();
    temper_cli::config::Config {
        vault_root: dir.path().to_path_buf(),
        state_dir,
        contexts: vec!["myapp".to_string()],
        skill_output: dir.path().join("temper.md"),
    }
}

#[test]
fn test_warmup_produces_output() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    temper_cli::commands::task::create(&config, "myapp", "Test", Some(&g_slug), None, None, None)
        .unwrap();

    let result = temper_cli::commands::warmup::run(&config, Some("myapp"), "text");
    assert!(result.is_ok());
}

#[test]
fn test_warmup_shows_in_progress_tasks_with_mode() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Active Work",
        Some(&g_slug),
        Some("build"),
        Some("medium"),
        None,
    )
    .unwrap();
    temper_cli::commands::task::move_task(
        &config,
        &slug,
        Some("in-progress"),
        None,
        None,
        None,
        None,
    )
    .unwrap();

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
