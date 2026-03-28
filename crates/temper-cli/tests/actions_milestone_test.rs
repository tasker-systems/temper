use tempfile::TempDir;

#[test]
fn test_actions_load_milestones_returns_correct_results() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    // Create two milestones
    temper_cli::actions::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    temper_cli::actions::milestone::create(&config, "myapp", "v0.2", None).unwrap();

    let milestones =
        temper_cli::actions::milestone::load_milestones(&config, Some("myapp")).unwrap();
    assert_eq!(milestones.len(), 2, "should load both milestones");
    assert_eq!(milestones[0].title, "v0.1");
    assert_eq!(milestones[1].title, "v0.2");
    assert_eq!(milestones[0].project, "myapp");
}

#[test]
fn test_actions_load_milestones_empty_vault() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let milestones = temper_cli::actions::milestone::load_milestones(&config, None).unwrap();
    assert!(
        milestones.is_empty(),
        "should return empty vec for fresh vault"
    );
}

#[test]
fn test_actions_find_milestone_by_slug() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let slug = temper_cli::actions::milestone::create(&config, "myapp", "Find Me", None).unwrap();

    let found = temper_cli::actions::milestone::find_milestone(&config, &slug, None)
        .unwrap()
        .expect("milestone should be found");
    assert_eq!(found.title, "Find Me");
    assert_eq!(found.slug, slug);
}

#[test]
fn test_actions_next_seq_increments() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    // First milestone gets seq 10
    temper_cli::actions::milestone::create(&config, "myapp", "v0.1", None).unwrap();

    // Next seq should be 20
    let seq = temper_cli::actions::milestone::next_seq(&config, "myapp").unwrap();
    assert_eq!(seq, 20);
}

#[test]
fn test_actions_ensure_maintenance_creates_milestone() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let slug = temper_cli::actions::milestone::ensure_maintenance(&config, "myapp").unwrap();
    assert_eq!(slug, "myapp-maintenance");

    let path = dir.path().join("milestones/myapp/myapp-maintenance.md");
    assert!(path.exists(), "maintenance milestone file should exist");
}

#[test]
fn test_actions_ensure_maintenance_idempotent() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    // Call twice — should not error or create duplicate
    temper_cli::actions::milestone::ensure_maintenance(&config, "myapp").unwrap();
    let slug = temper_cli::actions::milestone::ensure_maintenance(&config, "myapp").unwrap();
    assert_eq!(slug, "myapp-maintenance");

    let milestones =
        temper_cli::actions::milestone::load_milestones(&config, Some("myapp")).unwrap();
    assert_eq!(
        milestones.len(),
        1,
        "should only have one maintenance milestone"
    );
}

#[test]
fn test_actions_update_milestone_status() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let slug = temper_cli::actions::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    temper_cli::actions::milestone::update(&config, &slug, "completed", None).unwrap();

    let found = temper_cli::actions::milestone::find_milestone(&config, &slug, None)
        .unwrap()
        .expect("milestone should still exist");
    assert_eq!(found.status, "completed");
}

#[test]
fn test_actions_update_milestone_rejects_invalid_status() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let slug = temper_cli::actions::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    let result = temper_cli::actions::milestone::update(&config, &slug, "invalid", None);
    assert!(result.is_err(), "invalid status should be rejected");
}

#[test]
fn test_actions_reexports_match_commands() {
    // Verify that the re-exports from commands::milestone work identically
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    // Create via actions layer
    let slug =
        temper_cli::actions::milestone::create(&config, "myapp", "Via Actions", None).unwrap();

    // Find via commands layer (re-export)
    let found = temper_cli::commands::milestone::find_milestone(&config, &slug, None)
        .unwrap()
        .expect("re-exported find_milestone should work");
    assert_eq!(found.title, "Via Actions");
}
