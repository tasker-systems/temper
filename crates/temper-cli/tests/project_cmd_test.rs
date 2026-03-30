use tempfile::TempDir;

#[test]
fn test_context_add_and_remove() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();

    temper_cli::commands::context_cmd::add(dir.path(), "myapp", "/tmp/myapp", Some("org/myapp"))
        .unwrap();

    let content = std::fs::read_to_string(dir.path().join("temper.toml")).unwrap();
    assert!(content.contains("[projects.myapp]"));
    assert!(content.contains("/tmp/myapp"));

    temper_cli::commands::context_cmd::remove(dir.path(), "myapp").unwrap();
    let content = std::fs::read_to_string(dir.path().join("temper.toml")).unwrap();
    assert!(!content.contains("[projects.myapp]"));
}

#[test]
fn test_context_list_empty() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();

    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();
    temper_cli::commands::context_cmd::list(&config).unwrap();
}

#[test]
fn test_context_add_multiple_and_remove_one() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();

    temper_cli::commands::context_cmd::add(dir.path(), "alpha", "/tmp/alpha", Some("org/alpha"))
        .unwrap();
    temper_cli::commands::context_cmd::add(dir.path(), "beta", "/tmp/beta", Some("org/beta"))
        .unwrap();

    let content = std::fs::read_to_string(dir.path().join("temper.toml")).unwrap();
    assert!(content.contains("[projects.alpha]"));
    assert!(content.contains("[projects.beta]"));

    temper_cli::commands::context_cmd::remove(dir.path(), "alpha").unwrap();
    let content = std::fs::read_to_string(dir.path().join("temper.toml")).unwrap();
    assert!(!content.contains("[projects.alpha]"));
    assert!(content.contains("[projects.beta]"));
}
