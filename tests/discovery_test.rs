use tempfile::TempDir;

#[test]
fn test_append_and_read_event() {
    let dir = TempDir::new().unwrap();
    let state_dir = dir.path().join(".temper");
    std::fs::create_dir_all(&state_dir).unwrap();

    let event = temper_cli::discovery::Event::NoteCreate {
        ts: "2026-03-23T12:00:00Z".to_string(),
        note_type: "session".to_string(),
        title: "Test".to_string(),
        path: "sessions/test.md".to_string(),
        project: "myapp".to_string(),
    };
    temper_cli::discovery::append_event(&state_dir, &event).unwrap();

    let log_path = state_dir.join("events.jsonl");
    let content = std::fs::read_to_string(&log_path).unwrap();
    assert!(content.contains("session"));
    assert!(content.contains("myapp"));
}

#[test]
fn test_events_list_returns_recent_events() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    temper_cli::commands::project::add(dir.path(), "myapp", "/tmp/myapp", Some("org/myapp"))
        .unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    temper_cli::commands::ticket::create(&config, "myapp", "Test ticket", Some(&ms_slug)).unwrap();

    let events = temper_cli::commands::events::load_events(&config, None, 20).unwrap();
    assert!(
        events.len() >= 2,
        "should have at least 2 events (milestone create + ticket create)"
    );
}

#[test]
fn test_events_filter_by_project() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    temper_cli::commands::project::add(dir.path(), "myapp", "/tmp/myapp", Some("org/myapp"))
        .unwrap();
    temper_cli::commands::project::add(dir.path(), "other", "/tmp/other", Some("org/other"))
        .unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms1 = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let ms2 = temper_cli::commands::milestone::create(&config, "other", "v0.2", None).unwrap();
    temper_cli::commands::ticket::create(&config, "myapp", "Ticket A", Some(&ms1)).unwrap();
    temper_cli::commands::ticket::create(&config, "other", "Ticket B", Some(&ms2)).unwrap();

    let myapp_events =
        temper_cli::commands::events::load_events(&config, Some("myapp"), 20).unwrap();
    for event in &myapp_events {
        let project = temper_cli::commands::events::event_project(event);
        assert_eq!(
            project, "myapp",
            "filtered events should only be from myapp"
        );
    }
}
