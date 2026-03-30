use std::fs;
use tempfile::TempDir;

#[test]
fn test_parse_minimal_config() {
    let dir = TempDir::new().unwrap();
    let toml_content = "[vault]\n";
    fs::write(dir.path().join("temper.toml"), toml_content).unwrap();
    let config =
        temper_cli::config::TemperConfig::from_path(dir.path().join("temper.toml")).unwrap();
    assert_eq!(config.vault.sessions, "sessions");
    assert_eq!(config.vault.tickets, "tickets");
    assert_eq!(config.vault.milestones, "milestones");
    assert_eq!(config.vault.templates, "templates");
    assert_eq!(config.vault.state_dir, ".temper");
}

#[test]
fn test_parse_full_config() {
    let dir = TempDir::new().unwrap();
    let toml_content = r#"
[vault]
sessions = "journal"
state_dir = ".data"

[projects.myapp]
repo = "org/myapp"
path = "/tmp/myapp"

[skill]
framework = "superpowers"
"#;
    fs::write(dir.path().join("temper.toml"), toml_content).unwrap();
    let config =
        temper_cli::config::TemperConfig::from_path(dir.path().join("temper.toml")).unwrap();
    assert_eq!(config.vault.sessions, "journal");
    assert_eq!(config.vault.state_dir, ".data");
    assert!(config.projects.contains_key("myapp"));
}

#[test]
fn test_tilde_expansion() {
    let expanded = temper_cli::config::expand_tilde("~/projects/foo");
    assert!(!expanded.to_string_lossy().starts_with("~"));
    assert!(expanded.to_string_lossy().contains("projects/foo"));
}

#[test]
fn test_resolve_vault_from_env() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("temper.toml"), "[vault]\n").unwrap();
    std::env::set_var("TEMPER_VAULT", dir.path().to_str().unwrap());
    let result = temper_cli::config::resolve_vault(None);
    std::env::remove_var("TEMPER_VAULT");
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), dir.path());
}

#[test]
fn test_safe_write_validates_toml() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("temper.toml");
    fs::write(&path, "[vault]\nsessions = \"sessions\"\n").unwrap();
    let result =
        temper_cli::config::safe_write(&path, |content| content.replace("sessions", "journal"));
    assert!(result.is_ok());
    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("journal"));
}
