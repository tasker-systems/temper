use tempfile::TempDir;

#[test]
fn test_search_no_index_graceful() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    // Search with no index should not error, just print guidance
    let result = temper_cli::commands::search::run(&config, "test", "text", None, None, 10);
    assert!(result.is_ok());
}

#[test]
fn test_context_no_index_graceful() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    // Context with no index should not error, just print guidance
    let result = temper_cli::commands::context::run(&config, "test-topic", "text", 1, 5);
    assert!(result.is_ok());
}
