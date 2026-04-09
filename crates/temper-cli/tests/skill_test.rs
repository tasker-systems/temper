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
        subscriptions: Vec::new(),
        skill_output: dir.path().join("skill-output"),
    }
}

#[test]
fn test_skill_generate_produces_valid_content() {
    let dir = TempDir::new().unwrap();
    let config = test_config_with_global(&dir);

    let content = temper_cli::commands::skill::generate(&config).unwrap();
    // generate() now returns reference.md content (generated from clap)
    assert!(content.contains("temper"));
    assert!(content.contains("# CLI Reference"));
    assert!(content.contains("## Commands"));

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
fn test_skill_generate_includes_reference_sections() {
    let dir = TempDir::new().unwrap();
    let config = test_config_with_global(&dir);

    // generate() now returns reference.md with generated commands and footer
    let content = temper_cli::commands::skill::generate(&config).unwrap();
    assert!(
        content.contains("## Invocation"),
        "should contain invocation section"
    );
    assert!(
        content.contains("## Task Stages"),
        "should contain task stages footer"
    );

    unsafe {
        std::env::remove_var("TEMPER_GLOBAL_CONFIG");
    }
}

#[test]
fn test_skill_generate_includes_command_table() {
    let dir = TempDir::new().unwrap();
    let config = test_config_with_global(&dir);

    // generate() now returns reference.md with the generated command table
    let content = temper_cli::commands::skill::generate(&config).unwrap();
    assert!(content.contains("| Command | Syntax |"));
    assert!(content.contains("| init |"));
    assert!(content.contains("| search |"));

    unsafe {
        std::env::remove_var("TEMPER_GLOBAL_CONFIG");
    }
}

#[test]
fn test_skill_generate_includes_task_commands() {
    let dir = TempDir::new().unwrap();
    let config = test_config_with_global(&dir);

    // generate() now returns reference.md with resource subcommands
    let content = temper_cli::commands::skill::generate(&config).unwrap();
    assert!(content.contains("| resource create |"));
    assert!(content.contains("| resource list |"));
    assert!(content.contains("--mode"));
    assert!(content.contains("--effort"));

    unsafe {
        std::env::remove_var("TEMPER_GLOBAL_CONFIG");
    }
}

#[test]
fn test_skill_generate_includes_skill_only_commands() {
    let dir = TempDir::new().unwrap();
    let config = test_config_with_global(&dir);

    // generate() now returns reference.md which includes skill-only commands in footer
    let content = temper_cli::commands::skill::generate(&config).unwrap();
    assert!(
        content.contains("## Skill-Only Commands"),
        "should contain skill-only commands section"
    );
    assert!(
        content.contains("task start"),
        "should contain task start skill command"
    );
    assert!(
        content.contains("task resume"),
        "should contain task resume skill command"
    );
    assert!(
        content.contains("session start"),
        "should contain session start skill command"
    );

    unsafe {
        std::env::remove_var("TEMPER_GLOBAL_CONFIG");
    }
}
