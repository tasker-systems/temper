use tempfile::TempDir;

#[test]
fn test_check_valid_vault() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(
        dir.path(),
        true,
        false,
        temper_cli::format::OutputFormat::Json,
        None,
    )
    .unwrap();

    let config = temper_cli::config::Config {
        vault_root: dir.path().to_path_buf(),
        state_dir: dir.path().join(".temper"),
        contexts: vec!["default".to_string()],
        subscriptions: Vec::new(),
        skill_output: dir.path().join("temper.md"),
        profile_slug: None,
    };

    let result =
        temper_cli::commands::check::run(&config, false, temper_cli::format::OutputFormat::Json);
    assert!(result.is_ok());
}
