use tempfile::TempDir;

#[test]
fn test_index_empty_vault() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let result = temper_cli::commands::index::run(&config, false, None, None);
    assert!(result.is_ok());

    // State dir should have been created with artifacts
    assert!(dir.path().join(".temper").is_dir());
}
