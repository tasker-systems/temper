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
fn test_normalize_backfills_missing_ids() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug =
        temper_cli::commands::task::create(&config, "myapp", "Test", Some(&g_slug), None, None)
            .unwrap();

    // Strip the id field to simulate a pre-UUIDv7 task
    let path = dir.path().join("myapp/task").join(format!("{slug}.md"));
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
    let config = test_config(&dir);

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

    let path = dir.path().join("myapp/task").join(format!("{slug}.md"));
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
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug =
        temper_cli::commands::task::create(&config, "myapp", "Dry run", Some(&g_slug), None, None)
            .unwrap();

    let path = dir.path().join("myapp/task").join(format!("{slug}.md"));
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
fn test_normalize_detects_misplaced_files() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

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

    // Edit the frontmatter context to differ from the directory context
    let path = dir.path().join("myapp/task").join(format!("{slug}.md"));
    let content = std::fs::read_to_string(&path).unwrap();
    let modified = content.replace("context: \"myapp\"", "context: \"other\"");
    std::fs::write(&path, &modified).unwrap();

    let summary = temper_cli::commands::normalize::run(&config, None, false, false).unwrap();
    assert!(
        summary.files_moved > 0,
        "should detect context mismatch between frontmatter and directory"
    );
}

#[test]
fn test_normalize_backfills_missing_effort() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

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
    let path = dir.path().join("myapp/task").join(format!("{slug}.md"));
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
