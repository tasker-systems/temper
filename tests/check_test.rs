use tempfile::TempDir;

#[test]
fn test_check_valid_vault() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true).unwrap();

    let vault_str = dir.path().to_str().unwrap();
    let config = temper_cli::config::load(Some(vault_str)).unwrap();

    let result = temper_cli::commands::check::run(&config, false);
    assert!(result.is_ok());
}
