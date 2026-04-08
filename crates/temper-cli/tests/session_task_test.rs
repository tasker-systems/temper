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
        subscriptions: Vec::new(),
        skill_output: dir.path().join("temper.md"),
        skill_framework: "superpowers".to_string(),
    }
}

#[test]
fn test_session_save_with_task_links_entities() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let task_slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Linked task",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();

    temper_cli::commands::session::save(
        &config,
        Some("Linked Session"),
        Some("myapp"),
        Some("Session body"),
        Some(&task_slug),
        None,
        "text",
    )
    .unwrap();

    // Verify task was updated with sessions field
    let task_content = std::fs::read_to_string(
        dir.path()
            .join("myapp/task")
            .join(format!("{task_slug}.md")),
    )
    .unwrap();
    assert!(
        task_content.contains("sessions:"),
        "task should have sessions field"
    );
}

#[test]
fn test_session_save_with_task_and_state_moves_task() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let task_slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Done task",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();

    temper_cli::commands::session::save(
        &config,
        Some("Final Session"),
        Some("myapp"),
        Some("Done body"),
        Some(&task_slug),
        Some("done"),
        "text",
    )
    .unwrap();

    let task_content = std::fs::read_to_string(
        dir.path()
            .join("myapp/task")
            .join(format!("{task_slug}.md")),
    )
    .unwrap();
    assert!(
        task_content.contains("temper-stage: done"),
        "task should be marked done"
    );
}
