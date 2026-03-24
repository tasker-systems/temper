use tempfile::TempDir;

#[test]
fn test_warmup_produces_output() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    temper_cli::commands::project::add(dir.path(), "myapp", "/tmp/myapp", Some("org/myapp"))
        .unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let ms_slug = temper_cli::commands::milestone::create(&config, "myapp", "v0.1", None).unwrap();
    temper_cli::commands::ticket::create(&config, "myapp", "Test", Some(&ms_slug)).unwrap();

    let result = temper_cli::commands::warmup::run(&config, Some("myapp"), "text");
    assert!(result.is_ok());
}
