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
        profile_slug: None,
    }
}

#[test]
fn test_actions_load_goals_returns_correct_results() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    // Create two goals via the canonical resource API
    common::create_goal(&config, "myapp", "v0.1");
    common::create_goal(&config, "myapp", "v0.2");

    let goals = temper_cli::actions::goal::load_goals(&config, Some("myapp")).unwrap();
    assert_eq!(goals.len(), 2, "should load both goals");
    assert_eq!(goals[0].title, "v0.1");
    assert_eq!(goals[1].title, "v0.2");
    assert_eq!(goals[0].context, "myapp");
}

#[test]
fn test_actions_load_goals_empty_vault() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let goals = temper_cli::actions::goal::load_goals(&config, None).unwrap();
    assert!(goals.is_empty(), "should return empty vec for fresh vault");
}

#[test]
fn test_actions_find_goal_by_slug() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let slug = common::create_goal(&config, "myapp", "Find Me");

    let found = temper_cli::actions::goal::find_goal(&config, &slug, None)
        .unwrap()
        .expect("goal should be found");
    assert_eq!(found.title, "Find Me");
    assert_eq!(found.slug, slug);
}

#[test]
fn test_actions_next_seq_increments() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    // First goal gets seq 10
    common::create_goal(&config, "myapp", "v0.1");

    // Next seq should be 20
    let seq = temper_cli::actions::goal::next_seq(&config, "myapp").unwrap();
    assert_eq!(seq, 20);
}

#[test]
fn test_actions_ensure_maintenance_creates_goal() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let slug = temper_cli::actions::goal::ensure_maintenance(&config, "myapp").unwrap();
    assert_eq!(slug, "myapp-maintenance");

    let path = dir.path().join("@me/myapp/goal/myapp-maintenance.md");
    assert!(path.exists(), "maintenance goal file should exist");
}

#[test]
fn test_actions_ensure_maintenance_idempotent() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    // Call twice — should not error or create duplicate
    temper_cli::actions::goal::ensure_maintenance(&config, "myapp").unwrap();
    let slug = temper_cli::actions::goal::ensure_maintenance(&config, "myapp").unwrap();
    assert_eq!(slug, "myapp-maintenance");

    let goals = temper_cli::actions::goal::load_goals(&config, Some("myapp")).unwrap();
    assert_eq!(goals.len(), 1, "should only have one maintenance goal");
}

#[test]
fn test_actions_reexports_match_commands() {
    // Verify that the re-exports from commands::goal work identically
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    // Create via resource API
    let slug = common::create_goal(&config, "myapp", "Via Actions");

    // Find via commands layer (re-export)
    let found = temper_cli::commands::goal::find_goal(&config, &slug, None)
        .unwrap()
        .expect("re-exported find_goal should work");
    assert_eq!(found.title, "Via Actions");
}
