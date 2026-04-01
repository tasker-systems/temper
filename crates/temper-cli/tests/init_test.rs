use tempfile::TempDir;

#[test]
fn test_init_creates_vault_structure() {
    let dir = TempDir::new().unwrap();
    let vault_path = dir.path().join("myvault");

    temper_cli::commands::init::run(&vault_path, true, false).unwrap();

    // New structure: .temper/manifest.json, .temper/events.jsonl, default/
    assert!(vault_path.join(".temper/manifest.json").exists());
    assert!(vault_path.join(".temper/events.jsonl").exists());
    assert!(vault_path.join("default").is_dir());
}
