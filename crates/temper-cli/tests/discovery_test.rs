use tempfile::TempDir;

mod common;

fn test_config(dir: &TempDir, contexts: Vec<&str>) -> temper_cli::config::Config {
    common::init_isolated_auth();
    let state_dir = dir.path().join(".temper");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(state_dir.join("manifest.json"), "{}\n").unwrap();
    // Only create events.jsonl if it doesn't exist (some tests append events first)
    let events_path = state_dir.join("events.jsonl");
    if !events_path.exists() {
        std::fs::write(&events_path, "").unwrap();
    }
    temper_cli::config::Config {
        vault_root: dir.path().to_path_buf(),
        state_dir,
        contexts: contexts.into_iter().map(String::from).collect(),
        subscriptions: Vec::new(),
        skill_output: dir.path().join("temper.md"),
        profile_slug: None,
    }
}

#[test]
fn test_append_and_read_event() {
    let dir = TempDir::new().unwrap();
    let state_dir = dir.path().join(".temper");
    std::fs::create_dir_all(&state_dir).unwrap();

    let event = temper_cli::discovery::Event::ResourceCreate {
        ts: "2026-03-23T12:00:00Z".to_string(),
        doc_type: "session".to_string(),
        title: "Test".to_string(),
        path: "sessions/test.md".to_string(),
        context: "myapp".to_string(),
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
    let config = test_config(&dir, vec!["myapp"]);

    let g_slug = common::create_goal(&config, "myapp", "v0.1");
    common::create_task(&config, "myapp", "Test task", Some(&g_slug), None, None);

    let events = temper_cli::commands::events::load_events(&config, None, 20).unwrap();
    assert!(
        events.len() >= 2,
        "should have at least 2 events (goal create + task create)"
    );
}

#[test]
fn test_events_filter_by_context() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir, vec!["myapp", "other"]);

    let g1 = common::create_goal(&config, "myapp", "v0.1");
    let g2 = common::create_goal(&config, "other", "v0.2");
    common::create_task(&config, "myapp", "Task A", Some(&g1), None, None);
    common::create_task(&config, "other", "Task B", Some(&g2), None, None);

    let myapp_events =
        temper_cli::commands::events::load_events(&config, Some("myapp"), 20).unwrap();
    for event in &myapp_events {
        let ctx = temper_cli::commands::events::event_context(event);
        assert_eq!(ctx, "myapp", "filtered events should only be from myapp");
    }
}
