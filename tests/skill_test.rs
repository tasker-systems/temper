use tempfile::TempDir;

#[test]
fn test_skill_generate_produces_valid_content() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    temper_cli::commands::project::add(dir.path(), "myapp", "/tmp/myapp", Some("org/myapp"))
        .unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let content = temper_cli::commands::skill::generate(&config).unwrap();
    assert!(content.contains("temper"));
    assert!(content.contains("myapp"));
    assert!(content.contains("superpowers"));
    assert!(content.contains("config-hash:"));
}

#[test]
fn test_skill_install_writes_file() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let output_path = dir.path().join("skill-output/temper.md");
    temper_cli::commands::skill::install(&config, &output_path).unwrap();
    assert!(output_path.exists());
}
