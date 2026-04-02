use tempfile::TempDir;

#[test]
fn test_check_valid_vault() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();

    let config = temper_cli::config::Config {
        vault_root: dir.path().to_path_buf(),
        state_dir: dir.path().join(".temper"),
        contexts: vec!["default".to_string()],
        skill_output: dir.path().join("temper.md"),
        skill_framework: "superpowers".to_string(),
    };

    let result = temper_cli::commands::check::run(&config, false);
    assert!(result.is_ok());
}
