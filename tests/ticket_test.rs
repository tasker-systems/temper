use tempfile::TempDir;

#[test]
fn test_ticket_create_includes_uuid_id() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let slug =
        temper_cli::commands::ticket::create(&config, "myapp", "ID Test", Some(&ms_slug)).unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("tickets/myapp").join(format!("{slug}.md")))
            .unwrap();
    assert!(
        content.contains("id: \"0"),
        "should contain a UUIDv7 id field"
    );
}

#[test]
fn test_milestone_create_and_ticket_create() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    assert!(dir
        .path()
        .join("milestones/myapp")
        .join(format!("{ms_slug}.md"))
        .exists());

    let ticket_slug =
        temper_cli::commands::ticket::create(&config, "myapp", "Build feature", Some(&ms_slug))
            .unwrap();
    assert!(dir
        .path()
        .join("tickets/myapp")
        .join(format!("{ticket_slug}.md"))
        .exists());

    let content = std::fs::read_to_string(
        dir.path()
            .join("tickets/myapp")
            .join(format!("{ticket_slug}.md")),
    )
    .unwrap();
    assert!(content.contains("stage: backlog"));
    assert!(content.contains("Build feature"));
}

#[test]
fn test_ticket_move_to_in_progress() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let slug =
        temper_cli::commands::ticket::create(&config, "myapp", "Test", Some(&ms_slug)).unwrap();

    temper_cli::commands::ticket::move_ticket(&config, &slug, Some("in-progress"), None, None)
        .unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("tickets/myapp").join(format!("{slug}.md")))
            .unwrap();
    assert!(content.contains("stage: in-progress"));
}

#[test]
fn test_ticket_move_rejects_old_stages() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let slug =
        temper_cli::commands::ticket::create(&config, "myapp", "Test", Some(&ms_slug)).unwrap();

    let result =
        temper_cli::commands::ticket::move_ticket(&config, &slug, Some("brainstorm"), None, None);
    assert!(result.is_err(), "moving to 'brainstorm' should be rejected");
}

#[test]
fn test_ticket_move_to_cancelled() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let slug =
        temper_cli::commands::ticket::create(&config, "myapp", "Test", Some(&ms_slug)).unwrap();

    temper_cli::commands::ticket::move_ticket(&config, &slug, Some("cancelled"), None, None)
        .unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("tickets/myapp").join(format!("{slug}.md")))
            .unwrap();
    assert!(content.contains("stage: cancelled"));
}

#[test]
fn test_ticket_move_and_done() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let slug =
        temper_cli::commands::ticket::create(&config, "myapp", "Test", Some(&ms_slug)).unwrap();

    temper_cli::commands::ticket::move_ticket(&config, &slug, Some("in-progress"), None, None)
        .unwrap();
    temper_cli::commands::ticket::done(
        &config,
        &slug,
        Some("feat/test"),
        Some("https://github.com/pr/1"),
        None,
    )
    .unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("tickets/myapp").join(format!("{slug}.md")))
            .unwrap();
    assert!(content.contains("stage: done"));
    assert!(content.contains("feat/test"));
}

#[test]
fn test_milestone_creates_in_project_subdir() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.2", None).unwrap();

    let expected_path = dir
        .path()
        .join("milestones/myapp")
        .join(format!("{ms_slug}.md"));
    assert!(
        expected_path.exists(),
        "milestone should be in project subdir: {}",
        expected_path.display()
    );

    let flat_path = dir.path().join("milestones").join(format!("{ms_slug}.md"));
    assert!(
        !flat_path.exists(),
        "milestone should NOT be at flat path: {}",
        flat_path.display()
    );
}
