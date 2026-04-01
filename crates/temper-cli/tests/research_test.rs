use tempfile::TempDir;

fn test_config(dir: &TempDir) -> temper_cli::config::Config {
    let state_dir = dir.path().join(".temper");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(state_dir.join("manifest.json"), "{}\n").unwrap();
    std::fs::write(state_dir.join("events.jsonl"), "").unwrap();
    temper_cli::config::Config {
        vault_root: dir.path().to_path_buf(),
        state_dir,
        contexts: vec!["myapp".to_string()],
        skill_output: dir.path().join("temper.md"),
        skill_framework: "superpowers".to_string(),
    }
}

#[test]
fn test_research_save_creates_note() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let result = temper_cli::commands::research::save(
        &config,
        "LLM Context Windows",
        Some("myapp"),
        None,
        "text",
    );
    assert!(result.is_ok());

    let research_dir = dir.path().join("research/myapp");
    assert!(research_dir.is_dir());
    let entries: Vec<_> = std::fs::read_dir(&research_dir).unwrap().collect();
    assert_eq!(entries.len(), 1);

    let path = entries[0].as_ref().unwrap().path();
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("type: research"));
    assert!(content.contains("LLM Context Windows"));
    assert!(content.contains("id:"));
}

#[test]
fn test_research_save_idempotent_without_stdin() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    temper_cli::commands::research::save(&config, "Topic", Some("myapp"), None, "text").unwrap();

    let research_dir = dir.path().join("research/myapp");
    let entries: Vec<_> = std::fs::read_dir(&research_dir).unwrap().collect();
    let path = entries[0].as_ref().unwrap().path();
    let before = std::fs::read_to_string(&path).unwrap();

    temper_cli::commands::research::save(&config, "Topic", Some("myapp"), None, "text").unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(before, after);
}

#[test]
fn test_research_save_with_stdin_replaces_body() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    temper_cli::commands::research::save(&config, "Topic", Some("myapp"), None, "text").unwrap();
    temper_cli::commands::research::save(
        &config,
        "Topic",
        Some("myapp"),
        Some("Updated findings"),
        "text",
    )
    .unwrap();

    let research_dir = dir.path().join("research/myapp");
    let entries: Vec<_> = std::fs::read_dir(&research_dir).unwrap().collect();
    let path = entries[0].as_ref().unwrap().path();
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("Updated findings"));
    assert!(content.starts_with("---"));
}
