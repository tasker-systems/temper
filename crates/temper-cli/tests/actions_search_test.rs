use tempfile::TempDir;

#[test]
fn test_actions_search_empty_vault_returns_empty_results() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let results = temper_cli::actions::search::run(&config, "anything", None, None, 10).unwrap();
    assert!(
        results.hits.is_empty(),
        "should return empty hits for vault with no index"
    );
    assert_eq!(results.query, "anything");
}

#[test]
fn test_actions_search_empty_vault_with_filters_returns_empty_results() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let results =
        temper_cli::actions::search::run(&config, "test query", Some("concept"), Some("myapp"), 5)
            .unwrap();
    assert!(
        results.hits.is_empty(),
        "should return empty hits when no index exists"
    );
    assert_eq!(results.query, "test query");
}
