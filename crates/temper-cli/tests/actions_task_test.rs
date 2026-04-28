use tempfile::TempDir;

mod common;

fn test_config(dir: &TempDir) -> temper_cli::config::Config {
    common::init_isolated_auth();
    let state_dir = dir.path().join(".temper");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(state_dir.join("manifest.json"), "{}\n").unwrap();
    std::fs::write(state_dir.join("events.jsonl"), "").unwrap();
    temper_cli::config::Config {
        vault_root: dir.path().to_path_buf(),
        state_dir,
        contexts: vec!["myapp".to_string()],
        subscriptions: Vec::new(),
        skill_output: dir.path().join("temper.md"),
    }
}

#[test]
fn test_actions_load_tasks_returns_correct_results() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();

    // Create two tasks
    temper_cli::actions::task::create(
        &config,
        "myapp",
        "First task",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();
    temper_cli::actions::task::create(
        &config,
        "myapp",
        "Second task",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();

    let tasks =
        temper_cli::actions::task::load_tasks(&config, Some("myapp"), Some(&g_slug)).unwrap();
    assert_eq!(tasks.len(), 2, "should load both tasks");
    assert_eq!(tasks[0].title, "First task");
    assert_eq!(tasks[1].title, "Second task");
    assert_eq!(tasks[0].goal.as_deref(), Some(g_slug.as_str()));
    assert_eq!(tasks[1].goal.as_deref(), Some(g_slug.as_str()));
}

#[test]
fn test_actions_load_tasks_empty_vault() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let tasks = temper_cli::actions::task::load_tasks(&config, None, None).unwrap();
    assert!(tasks.is_empty(), "should return empty vec for fresh vault");
}

#[test]
fn test_actions_find_task_by_slug() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::actions::task::create(
        &config,
        "myapp",
        "Find Me",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();

    let found = temper_cli::actions::task::find_task(&config, &slug, None)
        .unwrap()
        .expect("task should be found");
    assert_eq!(found.title, "Find Me");
    assert_eq!(found.slug, slug);
}

#[test]
fn test_actions_next_seq_increments() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();

    // First task gets seq 10
    temper_cli::actions::task::create(&config, "myapp", "T1", Some(&g_slug), None, None, None)
        .unwrap();

    // Next seq should be 20
    let seq = temper_cli::actions::task::next_seq(&config, "myapp", &g_slug).unwrap();
    assert_eq!(seq, 20);
}

#[test]
fn test_actions_reexports_match_commands() {
    // Verify that the re-exports from commands::task work identically
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();

    // Create via actions layer
    let slug = temper_cli::actions::task::create(
        &config,
        "myapp",
        "Via Actions",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();

    // Find via commands layer (re-export)
    let found = temper_cli::commands::task::find_task(&config, &slug, None)
        .unwrap()
        .expect("re-exported find_task should work");
    assert_eq!(found.title, "Via Actions");
}
