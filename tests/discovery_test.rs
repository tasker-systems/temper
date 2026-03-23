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
