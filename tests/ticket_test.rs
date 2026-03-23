use tempfile::TempDir;

#[test]
fn test_milestone_create_and_ticket_create() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    assert!(dir.path().join("milestones").join(format!("{ms_slug}.md")).exists());

    let ticket_slug = temper_cli::commands::ticket::create(&config, "myapp", "Build feature", Some(&ms_slug), false).unwrap();
    assert!(dir.path().join("tickets/myapp").join(format!("{ticket_slug}.md")).exists());

    let content = std::fs::read_to_string(dir.path().join("tickets/myapp").join(format!("{ticket_slug}.md"))).unwrap();
    assert!(content.contains("stage: backlog"));
    assert!(content.contains("Build feature"));
}

#[test]
fn test_ticket_move_and_done() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let slug = temper_cli::commands::ticket::create(&config, "myapp", "Test", Some(&ms_slug), false).unwrap();

    temper_cli::commands::ticket::move_ticket(&config, &slug, Some("implement"), None).unwrap();
    temper_cli::commands::ticket::done(&config, &slug, Some("feat/test"), Some("https://github.com/pr/1")).unwrap();

    let content = std::fs::read_to_string(dir.path().join("tickets/myapp").join(format!("{slug}.md"))).unwrap();
    assert!(content.contains("stage: done"));
    assert!(content.contains("feat/test"));
}
