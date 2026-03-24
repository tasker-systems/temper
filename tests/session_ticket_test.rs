use tempfile::TempDir;

#[test]
fn test_session_save_with_ticket_links_entities() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let ticket_slug = temper_cli::commands::ticket::create(
        &config,
        "myapp",
        "Linked ticket",
        Some(&ms_slug),
        None,
    )
    .unwrap();

    temper_cli::commands::session::save(
        &config,
        Some("Linked Session"),
        Some("myapp"),
        Some("Session body"),
        Some(&ticket_slug),
        None,
        "text",
    )
    .unwrap();

    // Verify ticket was updated with sessions field
    let ticket_content = std::fs::read_to_string(
        dir.path()
            .join("tickets/myapp")
            .join(format!("{ticket_slug}.md")),
    )
    .unwrap();
    assert!(
        ticket_content.contains("sessions:"),
        "ticket should have sessions field"
    );
}

#[test]
fn test_session_save_with_ticket_and_state_moves_ticket() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let ticket_slug =
        temper_cli::commands::ticket::create(&config, "myapp", "Done ticket", Some(&ms_slug), None)
            .unwrap();

    temper_cli::commands::session::save(
        &config,
        Some("Final Session"),
        Some("myapp"),
        Some("Done body"),
        Some(&ticket_slug),
        Some("done"),
        "text",
    )
    .unwrap();

    let ticket_content = std::fs::read_to_string(
        dir.path()
            .join("tickets/myapp")
            .join(format!("{ticket_slug}.md")),
    )
    .unwrap();
    assert!(
        ticket_content.contains("stage: done"),
        "ticket should be marked done"
    );
}
