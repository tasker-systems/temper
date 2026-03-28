use tempfile::TempDir;

#[test]
fn test_actions_load_tickets_returns_correct_results() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None, "text").unwrap();

    // Create two tickets
    temper_cli::actions::ticket::create(&config, "myapp", "First ticket", Some(&ms_slug), None)
        .unwrap();
    temper_cli::actions::ticket::create(&config, "myapp", "Second ticket", Some(&ms_slug), None)
        .unwrap();

    let tickets =
        temper_cli::actions::ticket::load_tickets(&config, Some("myapp"), Some(&ms_slug)).unwrap();
    assert_eq!(tickets.len(), 2, "should load both tickets");
    assert_eq!(tickets[0].title, "First ticket");
    assert_eq!(tickets[1].title, "Second ticket");
    assert_eq!(tickets[0].milestone, ms_slug);
    assert_eq!(tickets[1].milestone, ms_slug);
}

#[test]
fn test_actions_load_tickets_empty_vault() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let tickets = temper_cli::actions::ticket::load_tickets(&config, None, None).unwrap();
    assert!(
        tickets.is_empty(),
        "should return empty vec for fresh vault"
    );
}

#[test]
fn test_actions_find_ticket_by_slug() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug =
        temper_cli::actions::ticket::create(&config, "myapp", "Find Me", Some(&ms_slug), None)
            .unwrap();

    let found = temper_cli::actions::ticket::find_ticket(&config, &slug, None)
        .unwrap()
        .expect("ticket should be found");
    assert_eq!(found.title, "Find Me");
    assert_eq!(found.slug, slug);
}

#[test]
fn test_actions_next_seq_increments() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None, "text").unwrap();

    // First ticket gets seq 10
    temper_cli::actions::ticket::create(&config, "myapp", "T1", Some(&ms_slug), None).unwrap();

    // Next seq should be 20
    let seq = temper_cli::actions::ticket::next_seq(&config, "myapp", &ms_slug).unwrap();
    assert_eq!(seq, 20);
}

#[test]
fn test_actions_reexports_match_commands() {
    // Verify that the re-exports from commands::ticket work identically
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug =
        temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None, "text").unwrap();

    // Create via actions layer
    let slug =
        temper_cli::actions::ticket::create(&config, "myapp", "Via Actions", Some(&ms_slug), None)
            .unwrap();

    // Find via commands layer (re-export)
    let found = temper_cli::commands::ticket::find_ticket(&config, &slug, None)
        .unwrap()
        .expect("re-exported find_ticket should work");
    assert_eq!(found.title, "Via Actions");
}
