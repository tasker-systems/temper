use tempfile::TempDir;

#[test]
fn test_session_save_creates_note() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let result = temper_cli::commands::session::save(
        &config, Some("Test Session"), Some("myapp"), None,
    );
    assert!(result.is_ok());

    let session_dir = dir.path().join("sessions/myapp");
    assert!(session_dir.is_dir());
    let entries: Vec<_> = std::fs::read_dir(&session_dir).unwrap().collect();
    assert_eq!(entries.len(), 1);
}

#[test]
fn test_session_save_idempotent_without_stdin() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    temper_cli::commands::session::save(&config, Some("Test"), Some("myapp"), None).unwrap();

    let session_dir = dir.path().join("sessions/myapp");
    let entries: Vec<_> = std::fs::read_dir(&session_dir).unwrap().collect();
    let path = entries[0].as_ref().unwrap().path();
    let before = std::fs::read_to_string(&path).unwrap();

    temper_cli::commands::session::save(&config, Some("Test"), Some("myapp"), None).unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(before, after);
}

#[test]
fn test_session_save_replaces_body_with_stdin() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    temper_cli::commands::session::save(&config, Some("My Session"), Some("proj"), None).unwrap();

    let session_dir = dir.path().join("sessions/proj");
    let entries: Vec<_> = std::fs::read_dir(&session_dir).unwrap().collect();
    let path = entries[0].as_ref().unwrap().path();

    temper_cli::commands::session::save(
        &config,
        Some("My Session"),
        Some("proj"),
        Some("New body content here."),
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
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    temper_cli::commands::session::save(&config, Some("Alpha"), Some("proj"), None).unwrap();
    temper_cli::commands::session::save(&config, Some("Beta"), Some("other"), None).unwrap();

    // list all
    let result = temper_cli::commands::session::list(&config, None);
    assert!(result.is_ok());

    // list filtered
    let result = temper_cli::commands::session::list(&config, Some("proj"));
    assert!(result.is_ok());
}
