use tempfile::TempDir;

#[test]
fn test_init_creates_vault_structure() {
    let dir = TempDir::new().unwrap();
    let vault_path = dir.path().join("myvault");

    temper_cli::commands::init::run(&vault_path, true, false, None).unwrap();

    // Cloud-only invariants: vault root + .temper/ state dir exist;
    // no manifest.json or events.jsonl sidecars; no per-context subdirs.
    assert!(vault_path.is_dir(), "vault root should be created");
    assert!(
        vault_path.join(".temper").is_dir(),
        ".temper/ state dir should be created"
    );
    assert!(
        !vault_path.join(".temper/manifest.json").exists(),
        "manifest.json must not be written"
    );
    assert!(
        !vault_path.join(".temper/events.jsonl").exists(),
        "events.jsonl must not be written"
    );
    assert!(
        !vault_path.join("default").exists(),
        "per-context subdir must not be created"
    );
}
