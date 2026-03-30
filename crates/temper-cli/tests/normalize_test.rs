use tempfile::TempDir;

#[test]
fn test_normalize_backfills_missing_ids() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug =
        temper_cli::commands::task::create(&config, "myapp", "Test", Some(&g_slug), None, None)
            .unwrap();

    // Strip the id field to simulate a pre-UUIDv7 task
    let path = dir.path().join("tasks/myapp").join(format!("{slug}.md"));
    let content = std::fs::read_to_string(&path).unwrap();
    let stripped = content
        .lines()
        .filter(|l| !l.starts_with("id:"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, format!("{stripped}\n")).unwrap();

    let summary = temper_cli::commands::normalize::run(&config, None, false, false).unwrap();
    assert!(summary.ids_backfilled > 0);

    let updated = std::fs::read_to_string(&path).unwrap();
    assert!(updated.contains("id:"), "task should now have an id field");
}

#[test]
fn test_normalize_migrates_old_stages() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Old Stage",
        Some(&g_slug),
        None,
        None,
    )
    .unwrap();

    let path = dir.path().join("tasks/myapp").join(format!("{slug}.md"));
    let content = std::fs::read_to_string(&path).unwrap();
    let modified = content.replace("stage: backlog", "stage: brainstorm");
    std::fs::write(&path, &modified).unwrap();

    let summary = temper_cli::commands::normalize::run(&config, None, false, false).unwrap();
    assert!(summary.stages_migrated > 0);

    let updated = std::fs::read_to_string(&path).unwrap();
    assert!(updated.contains("stage: in-progress"));
}

#[test]
fn test_normalize_dry_run_makes_no_changes() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug =
        temper_cli::commands::task::create(&config, "myapp", "Dry run", Some(&g_slug), None, None)
            .unwrap();

    let path = dir.path().join("tasks/myapp").join(format!("{slug}.md"));
    let content = std::fs::read_to_string(&path).unwrap();
    let stripped = content
        .lines()
        .filter(|l| !l.starts_with("id:"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, format!("{stripped}\n")).unwrap();
    let before = std::fs::read_to_string(&path).unwrap();

    temper_cli::commands::normalize::run(&config, None, true, false).unwrap();

    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(before, after, "dry-run should not modify files");
}

#[test]
fn test_normalize_moves_misplaced_files() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Misplaced",
        Some(&g_slug),
        None,
        None,
    )
    .unwrap();

    let correct_path = dir.path().join("tasks/myapp").join(format!("{slug}.md"));
    let wrong_dir = dir.path().join("tasks/wrong");
    std::fs::create_dir_all(&wrong_dir).unwrap();
    let wrong_path = wrong_dir.join(format!("{slug}.md"));
    std::fs::rename(&correct_path, &wrong_path).unwrap();

    let summary = temper_cli::commands::normalize::run(&config, None, false, false).unwrap();
    assert!(summary.files_moved > 0);
    assert!(
        correct_path.exists(),
        "file should be moved back to correct context dir"
    );
}

#[test]
fn test_normalize_backfills_missing_effort() {
    let dir = TempDir::new().unwrap();
    temper_cli::commands::init::run(dir.path(), true, false).unwrap();
    let config = temper_cli::config::load(Some(dir.path().to_str().unwrap())).unwrap();

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Legacy Task",
        Some(&g_slug),
        None,
        None,
    )
    .unwrap();

    // Strip the effort field to simulate a pre-effort task
    let path = dir.path().join("tasks/myapp").join(format!("{slug}.md"));
    let content = std::fs::read_to_string(&path).unwrap();
    let stripped = content
        .lines()
        .filter(|l| !l.starts_with("effort:"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, format!("{stripped}\n")).unwrap();

    let summary = temper_cli::commands::normalize::run(&config, None, false, false).unwrap();
    assert!(
        summary.tasks_without_effort > 0,
        "should count tasks without effort"
    );

    let updated = std::fs::read_to_string(&path).unwrap();
    assert!(
        updated.contains("effort: null"),
        "should have backfilled effort: null"
    );
}
