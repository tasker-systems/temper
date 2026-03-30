use tempfile::TempDir;

#[test]
fn test_init_creates_vault_structure() {
    let dir = TempDir::new().unwrap();
    let vault_path = dir.path().join("myvault");

    temper_cli::commands::init::run(&vault_path, true, false).unwrap();

    assert!(vault_path.join("temper.toml").exists());
    assert!(vault_path.join("sessions").is_dir());
    assert!(vault_path.join("tasks").is_dir());
    assert!(vault_path.join("goals").is_dir());
    assert!(vault_path.join("templates").is_dir());
    assert!(vault_path.join("templates/session.md").exists());
    assert!(vault_path.join("templates/task.md").exists());
    assert!(vault_path.join("templates/goal.md").exists());

    // Verify temper.toml is valid
    let content = std::fs::read_to_string(vault_path.join("temper.toml")).unwrap();
    let _config: temper_cli::config::TemperConfig = toml::from_str(&content).unwrap();
}
