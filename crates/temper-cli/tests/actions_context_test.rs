use tempfile::TempDir;

#[test]
fn test_actions_context_empty_vault_returns_empty_results() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let results = temper_cli::actions::context::run(&config, "anything", 1, 10).unwrap();
    assert!(
        results.hops.is_empty(),
        "should return empty hops for vault with no index"
    );
}

#[test]
fn test_actions_context_empty_vault_with_depth_returns_empty_results() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let results = temper_cli::actions::context::run(&config, "some topic", 2, 5).unwrap();
    assert!(
        results.hops.is_empty(),
        "should return empty hops when no index exists even with depth > 1"
    );
}
