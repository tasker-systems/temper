use tempfile::TempDir;

fn test_config_with_global(dir: &TempDir) -> temper_cli::config::Config {
    let state_dir = dir.path().join(".temper");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(state_dir.join("manifest.json"), "{}\n").unwrap();
    std::fs::write(state_dir.join("events.jsonl"), "").unwrap();

    // skill::generate reads global_config_path(), so we need a real config file
    let config_path = dir.path().join("global-config.toml");
    let vault_path = dir.path().to_string_lossy();
    std::fs::write(
        &config_path,
        format!(
            r#"[vault]
path = "{vault_path}"

[sync.subscriptions]
contexts = ["myapp"]

[skill]
output = "~/.claude/commands/temper.md"
framework = "superpowers"

[auth]
provider = "auth0"

[auth.providers.auth0]
authorize_url = "https://temperkb.us.auth0.com/authorize"
token_url = "https://temperkb.us.auth0.com/oauth/token"
client_id = "test"
audience = "https://temperkb.io/api"
scopes = ["openid"]
"#
        ),
    )
    .unwrap();

    // Point TEMPER_GLOBAL_CONFIG to our test config
    unsafe {
        std::env::set_var("TEMPER_GLOBAL_CONFIG", config_path.to_str().unwrap());
    }

    temper_cli::config::Config {
        vault_root: dir.path().to_path_buf(),
        state_dir,
        contexts: vec!["myapp".to_string()],
        skill_output: dir.path().join("temper.md"),
        skill_framework: "superpowers".to_string(),
    }
}

#[test]
fn test_skill_generate_produces_valid_content() {
    let dir = TempDir::new().unwrap();
    let config = test_config_with_global(&dir);

    let content = temper_cli::commands::skill::generate(&config).unwrap();
    assert!(content.contains("temper"));
    assert!(content.contains("myapp"));
    assert!(content.contains("superpowers"));
    assert!(content.contains("config-hash:"));

    unsafe {
        std::env::remove_var("TEMPER_GLOBAL_CONFIG");
    }
}

#[test]
fn test_skill_install_writes_file() {
    let dir = TempDir::new().unwrap();
    let config = test_config_with_global(&dir);

    let output_path = dir.path().join("skill-output/temper.md");
    temper_cli::commands::skill::install(&config, &output_path).unwrap();
    assert!(output_path.exists());

    unsafe {
        std::env::remove_var("TEMPER_GLOBAL_CONFIG");
    }
}

#[test]
fn test_skill_generate_includes_invocation_section() {
    let dir = TempDir::new().unwrap();
    let config = test_config_with_global(&dir);

    let content = temper_cli::commands::skill::generate(&config).unwrap();
    assert!(content.contains("## Invocation"));
    assert!(content.contains("installed binary"));
    assert!(content.contains("Never use `cargo run`"));

    unsafe {
        std::env::remove_var("TEMPER_GLOBAL_CONFIG");
    }
}

#[test]
fn test_skill_generate_documents_stdin_flag() {
    let dir = TempDir::new().unwrap();
    let config = test_config_with_global(&dir);

    let content = temper_cli::commands::skill::generate(&config).unwrap();
    assert!(content.contains("stdin auto-detected"));

    unsafe {
        std::env::remove_var("TEMPER_GLOBAL_CONFIG");
    }
}

#[test]
fn test_skill_generate_includes_task_start() {
    let dir = TempDir::new().unwrap();
    let config = test_config_with_global(&dir);

    let content = temper_cli::commands::skill::generate(&config).unwrap();
    assert!(content.contains("task start"));
    assert!(content.contains("brainstorming skill"));

    unsafe {
        std::env::remove_var("TEMPER_GLOBAL_CONFIG");
    }
}
