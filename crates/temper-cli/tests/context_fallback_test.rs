use std::path::PathBuf;
use tempfile::TempDir;

fn test_config(vault_path: PathBuf) -> temper_cli::config::Config {
    temper_cli::config::Config {
        vault_root: vault_path.clone(),
        state_dir: vault_path.join(".temper"),
        contexts: vec![],
        subscriptions: Vec::new(),
        skill_output: PathBuf::from("/tmp/skill"),
        skill_framework: "superpowers".to_string(),
    }
}

#[test]
fn test_resolve_context_with_fallback_uses_default_for_missing() {
    let dir = TempDir::new().unwrap();
    let vault_path = dir.path().join("vault");
    std::fs::create_dir_all(vault_path.join("default")).unwrap();

    let config = test_config(vault_path);

    let resolved = temper_cli::commands::resolve_context_with_fallback(&config, "nonexistent");
    assert_eq!(&*resolved, "default");
}

#[test]
fn test_resolve_context_with_fallback_keeps_existing() {
    let dir = TempDir::new().unwrap();
    let vault_path = dir.path().join("vault");
    std::fs::create_dir_all(vault_path.join("myctx")).unwrap();

    let config = test_config(vault_path);

    let resolved = temper_cli::commands::resolve_context_with_fallback(&config, "myctx");
    assert_eq!(&*resolved, "myctx");
}
