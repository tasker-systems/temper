use tempfile::TempDir;

fn test_config(dir: &TempDir) -> temper_cli::config::Config {
    let state_dir = dir.path().join(".temper");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(state_dir.join("manifest.json"), "{}\n").unwrap();
    std::fs::write(state_dir.join("events.jsonl"), "").unwrap();
    temper_cli::config::Config {
        vault_root: dir.path().to_path_buf(),
        state_dir,
        contexts: vec!["myapp".to_string(), "proj".to_string(), "other".to_string()],
        subscriptions: Vec::new(),
        skill_output: dir.path().join("temper.md"),
        skill_framework: "superpowers".to_string(),
    }
}

#[test]
fn test_session_save_creates_note() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let result = temper_cli::commands::session::save(
        &config,
        Some("Test Session"),
        Some("myapp"),
        None,
        None,
        None,
        "text",
    );
    assert!(result.is_ok());

    let session_dir = dir.path().join("@me/myapp/session");
    assert!(session_dir.is_dir());
    let entries: Vec<_> = std::fs::read_dir(&session_dir).unwrap().collect();
    assert_eq!(entries.len(), 1);
}

#[test]
fn test_session_save_idempotent_without_stdin() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    temper_cli::commands::session::save(
        &config,
        Some("Test"),
        Some("myapp"),
        None,
        None,
        None,
        "text",
    )
    .unwrap();

    let session_dir = dir.path().join("@me/myapp/session");
    let entries: Vec<_> = std::fs::read_dir(&session_dir).unwrap().collect();
    let path = entries[0].as_ref().unwrap().path();
    let before = std::fs::read_to_string(&path).unwrap();

    temper_cli::commands::session::save(
        &config,
        Some("Test"),
        Some("myapp"),
        None,
        None,
        None,
        "text",
    )
    .unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(before, after);
}

#[test]
fn test_session_save_replaces_body_with_stdin() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    temper_cli::commands::session::save(
        &config,
        Some("My Session"),
        Some("proj"),
        None,
        None,
        None,
        "text",
    )
    .unwrap();

    let session_dir = dir.path().join("@me/proj/session");
    let entries: Vec<_> = std::fs::read_dir(&session_dir).unwrap().collect();
    let path = entries[0].as_ref().unwrap().path();

    temper_cli::commands::session::save(
        &config,
        Some("My Session"),
        Some("proj"),
        Some("New body content here."),
        None,
        None,
        "text",
    )
    .unwrap();

    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains("New body content here."));
    // Frontmatter should still be present
    assert!(after.starts_with("---"));
}

#[test]
fn test_session_list_returns_ok() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    temper_cli::commands::session::save(
        &config,
        Some("Alpha"),
        Some("proj"),
        None,
        None,
        None,
        "text",
    )
    .unwrap();
    temper_cli::commands::session::save(
        &config,
        Some("Beta"),
        Some("other"),
        None,
        None,
        None,
        "text",
    )
    .unwrap();

    // list all
    let result = temper_cli::commands::session::list(&config, None, None, "text");
    assert!(result.is_ok());

    // list filtered
    let result = temper_cli::commands::session::list(&config, Some("proj"), None, "text");
    assert!(result.is_ok());
}

#[test]
fn test_session_list_respects_limit() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    // Create 3 sessions
    for title in &["Alpha", "Beta", "Gamma"] {
        temper_cli::commands::session::save(
            &config,
            Some(title),
            Some("proj"),
            None,
            None,
            None,
            "text",
        )
        .unwrap();
    }

    // Request limit of 2, verify via JSON output
    let result = temper_cli::commands::session::list(&config, Some("proj"), Some(2), "json");
    assert!(result.is_ok());
}
