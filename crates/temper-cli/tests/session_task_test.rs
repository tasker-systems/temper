use tempfile::TempDir;

#[test]
fn test_session_save_with_task_links_entities() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let task_slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Linked task",
        Some(&g_slug),
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
            .join("tasks/myapp")
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
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let task_slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Done task",
        Some(&g_slug),
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
            .join("tasks/myapp")
            .join(format!("{task_slug}.md")),
    )
    .unwrap();
    assert!(
        task_content.contains("stage: done"),
        "task should be marked done"
    );
}
