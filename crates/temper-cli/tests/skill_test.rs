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
output = "~/.claude/skills/temper"
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
        skill_output: dir.path().join("skill-output"),
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
    assert!(content.contains("config-hash:"));

    unsafe {
        std::env::remove_var("TEMPER_GLOBAL_CONFIG");
    }
}

#[test]
fn test_skill_install_writes_directory() {
    let dir = TempDir::new().unwrap();
    let config = test_config_with_global(&dir);

    let skill_dir = dir.path().join("skill-output");
    temper_cli::commands::skill::install(&config, &skill_dir).unwrap();

    assert!(skill_dir.join("SKILL.md").exists());
    assert!(skill_dir.join("reference.md").exists());
    assert!(skill_dir.join("subagent-guidance.md").exists());
    assert!(skill_dir.join("session-lifecycle.md").exists());
    assert!(skill_dir.join("workflows/build-small.md").exists());
    assert!(skill_dir.join("workflows/build-medium.md").exists());
    assert!(skill_dir.join("workflows/build-large.md").exists());
    assert!(skill_dir.join("workflows/plan-small.md").exists());
    assert!(skill_dir.join("workflows/plan-medium.md").exists());
    assert!(skill_dir.join("workflows/plan-large.md").exists());
    assert!(skill_dir.join("guidance").is_dir());

    unsafe {
        std::env::remove_var("TEMPER_GLOBAL_CONFIG");
    }
}

#[test]
fn test_skill_generate_includes_vault_and_contexts() {
    let dir = TempDir::new().unwrap();
    let config = test_config_with_global(&dir);

    let content = temper_cli::commands::skill::generate(&config).unwrap();
    let vault_str = dir.path().to_string_lossy();
    assert!(content.contains(&*vault_str), "should contain vault path");
    assert!(content.contains("- `myapp`"), "should contain context list");

    unsafe {
        std::env::remove_var("TEMPER_GLOBAL_CONFIG");
    }
}

#[test]
fn test_skill_generate_includes_modular_structure() {
    let dir = TempDir::new().unwrap();
    let config = test_config_with_global(&dir);

    let content = temper_cli::commands::skill::generate(&config).unwrap();
    assert!(content.contains("## How This Skill Works"));
    assert!(content.contains("reference.md"));
    assert!(content.contains("subagent-guidance.md"));
    assert!(content.contains("session-lifecycle.md"));

    unsafe {
        std::env::remove_var("TEMPER_GLOBAL_CONFIG");
    }
}

#[test]
fn test_skill_generate_includes_task_start() {
    let dir = TempDir::new().unwrap();
    let config = test_config_with_global(&dir);

    let content = temper_cli::commands::skill::generate(&config).unwrap();
    assert!(content.contains("## On Task Start"));
    assert!(content.contains("mode and effort"));

    unsafe {
        std::env::remove_var("TEMPER_GLOBAL_CONFIG");
    }
}

#[test]
fn test_skill_generate_includes_new_routing() {
    let dir = TempDir::new().unwrap();
    let config = test_config_with_global(&dir);

    let content = temper_cli::commands::skill::generate(&config).unwrap();
    assert!(content.contains("## On Task Resume"), "should contain task resume section");
    assert!(content.contains("## On Session Start"), "should contain session start section");
    assert!(content.contains("## On Task Create"), "should contain task create section");
    assert!(content.contains("## Command Routing"), "should contain routing table");

    unsafe {
        std::env::remove_var("TEMPER_GLOBAL_CONFIG");
    }
}
