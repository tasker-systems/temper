use std::fs;
use temper_core::types::config::{SkillConfig, TemperConfig};
use tempfile::TempDir;

#[test]
fn test_expand_tilde() {
    let expanded = temper_cli::config::expand_tilde("~/projects/foo");
    assert!(!expanded.to_string_lossy().starts_with("~"));
    assert!(expanded.to_string_lossy().contains("projects/foo"));
}

#[test]
fn test_resolve_vault_from_env() {
    let dir = TempDir::new().unwrap();

    // Create a minimal global config so load_global_config doesn't fail
    let config_path = dir.path().join("config.toml");
    fs::write(
        &config_path,
        "[vault]\npath = \"/tmp/fallback\"\n[sync.subscriptions]\ncontexts = []\n",
    )
    .unwrap();

    // Use TEMPER_VAULT to override vault path
    let vault_dir = dir.path().join("myvault");
    fs::create_dir_all(&vault_dir).unwrap();

    // TEMPER_VAULT takes priority over global config
    unsafe {
        std::env::set_var("TEMPER_VAULT", vault_dir.to_str().unwrap());
        std::env::set_var("TEMPER_GLOBAL_CONFIG", config_path.to_str().unwrap());
    }
    let result = temper_cli::config::resolve_vault(None);
    unsafe {
        std::env::remove_var("TEMPER_VAULT");
        std::env::remove_var("TEMPER_GLOBAL_CONFIG");
    }
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), vault_dir);
}

#[test]
fn test_safe_write_validates_toml() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.toml");
    fs::write(&path, "[vault]\npath = \"sessions\"\n").unwrap();
    let result =
        temper_cli::config::safe_write(&path, |content| content.replace("sessions", "journal"));
    assert!(result.is_ok());
    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("journal"));
}

#[test]
fn test_load_from_uses_explicit_config() {
    let dir = TempDir::new().unwrap();
    let vault_dir = dir.path().join("vault");
    fs::create_dir_all(vault_dir.join(".temper")).unwrap();

    let mut config = TemperConfig::default();
    config.vault.path = vault_dir.to_str().unwrap().to_string();
    config.sync.subscriptions.contexts = vec!["myctx".to_string(), "otherctx".to_string()];
    config.skill = SkillConfig {
        output: vault_dir.join("skills").to_str().unwrap().to_string(),
        framework: "custom-framework".to_string(),
    };

    let result = temper_cli::config::load_from(&config, None);

    assert_eq!(result.vault_root, vault_dir);
    assert_eq!(
        result.contexts,
        vec!["myctx".to_string(), "otherctx".to_string()]
    );
    assert_eq!(result.skill_framework, "custom-framework");
}

#[test]
fn test_load_from_cli_vault_overrides_config() {
    let dir = TempDir::new().unwrap();
    let override_vault = dir.path().join("override-vault");
    fs::create_dir_all(override_vault.join(".temper")).unwrap();

    let config = TemperConfig::default(); // vault.path = "~/vault"

    let result = temper_cli::config::load_from(&config, Some(override_vault.to_str().unwrap()));

    assert_eq!(result.vault_root, override_vault);
    // Ensure it did NOT use the default ~/vault path
    assert_ne!(
        result.vault_root.to_str().unwrap(),
        temper_cli::config::expand_tilde("~/vault")
            .to_str()
            .unwrap()
    );
}

#[test]
fn test_config_doc_type_dir() {
    let config = temper_cli::config::Config {
        vault_root: std::path::PathBuf::from("/tmp/vault"),
        state_dir: std::path::PathBuf::from("/tmp/vault/.temper"),
        contexts: vec!["myapp".to_string()],
        skill_output: std::path::PathBuf::from("/tmp/temper.md"),
        skill_framework: "superpowers".to_string(),
    };
    let dir = config.doc_type_dir("myapp", "task");
    assert_eq!(dir, std::path::PathBuf::from("/tmp/vault/myapp/task"));
}
