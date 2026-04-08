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
        subscriptions: Vec::new(),
        skill_output: dir.path().join("temper.md"),
        skill_framework: "superpowers".to_string(),
    }
}

#[test]
fn test_task_create_includes_uuid_id() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "ID Test",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("@me/myapp/task").join(format!("{slug}.md")))
            .unwrap();
    assert!(
        content.contains("id: \"0"),
        "should contain a UUIDv7 id field"
    );
}

#[test]
fn test_goal_create_and_task_create() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    assert!(dir
        .path()
        .join("@me/myapp/goal")
        .join(format!("{g_slug}.md"))
        .exists());

    let task_slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Build feature",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();
    assert!(dir
        .path()
        .join("@me/myapp/task")
        .join(format!("{task_slug}.md"))
        .exists());

    let content = std::fs::read_to_string(
        dir.path()
            .join("@me/myapp/task")
            .join(format!("{task_slug}.md")),
    )
    .unwrap();
    assert!(content.contains("stage: backlog"));
    assert!(content.contains("Build feature"));
}

#[test]
fn test_task_move_to_in_progress() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Test",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();

    temper_cli::commands::task::move_task(
        &config,
        &slug,
        Some("in-progress"),
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("@me/myapp/task").join(format!("{slug}.md")))
            .unwrap();
    assert!(content.contains("stage: in-progress"));
}

#[test]
fn test_task_move_rejects_old_stages() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Test",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();

    let result = temper_cli::commands::task::move_task(
        &config,
        &slug,
        Some("brainstorm"),
        None,
        None,
        None,
        None,
    );
    assert!(result.is_err(), "moving to 'brainstorm' should be rejected");
}

#[test]
fn test_task_move_to_cancelled() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Test",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();

    temper_cli::commands::task::move_task(
        &config,
        &slug,
        Some("cancelled"),
        None,
        None,
        None,
        None,
    )
    .unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("@me/myapp/task").join(format!("{slug}.md")))
            .unwrap();
    assert!(content.contains("stage: cancelled"));
}

#[test]
fn test_task_move_and_done() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Test",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();

    temper_cli::commands::task::move_task(
        &config,
        &slug,
        Some("in-progress"),
        None,
        None,
        None,
        None,
    )
    .unwrap();
    temper_cli::commands::task::done(
        &config,
        &slug,
        Some("feat/test"),
        Some("https://github.com/pr/1"),
        None,
    )
    .unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("@me/myapp/task").join(format!("{slug}.md")))
            .unwrap();
    assert!(content.contains("stage: done"));
    assert!(content.contains("feat/test"));
}

#[test]
fn test_goal_creates_in_context_subdir() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.2", None, "text").unwrap();

    let expected_path = dir
        .path()
        .join("@me/myapp/goal")
        .join(format!("{g_slug}.md"));
    assert!(
        expected_path.exists(),
        "goal should be in context subdir: {}",
        expected_path.display()
    );

    let flat_path = dir.path().join("goal").join(format!("{g_slug}.md"));
    assert!(
        !flat_path.exists(),
        "goal should NOT be at flat path: {}",
        flat_path.display()
    );
}

#[test]
fn test_task_list_json_format() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    temper_cli::commands::task::create(
        &config,
        "myapp",
        "JSON Test",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();

    let result = temper_cli::commands::task::list(&config, Some("myapp"), None, None, "json");
    assert!(result.is_ok());
}

#[test]
fn test_task_create_with_mode_and_effort() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Scoped Task",
        Some(&g_slug),
        Some("build"),
        Some("medium"),
        None,
    )
    .unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("@me/myapp/task").join(format!("{slug}.md")))
            .unwrap();
    assert!(
        content.contains("mode: build"),
        "should contain mode: build"
    );
    assert!(
        content.contains("effort: medium"),
        "should contain effort: medium"
    );
}

#[test]
fn test_task_create_without_mode_effort() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Unscoped Task",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("@me/myapp/task").join(format!("{slug}.md")))
            .unwrap();
    assert!(content.contains("mode: null"), "should contain mode: null");
    assert!(
        content.contains("effort: null"),
        "should contain effort: null"
    );
}

#[test]
fn test_task_create_rejects_invalid_mode() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let result = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Bad Mode",
        Some(&g_slug),
        Some("huge"),
        None,
        None,
    );
    assert!(result.is_err(), "invalid mode on create should be rejected");
}

#[test]
fn test_task_move_with_effort() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Effort Move",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();

    temper_cli::commands::task::move_task(&config, &slug, None, None, None, None, Some("large"))
        .unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("@me/myapp/task").join(format!("{slug}.md")))
            .unwrap();
    assert!(
        content.contains("effort: large"),
        "effort should be updated to large"
    );
}

#[test]
fn test_task_move_with_stage_and_mode() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Both Move",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();

    temper_cli::commands::task::move_task(
        &config,
        &slug,
        Some("in-progress"),
        None,
        None,
        Some("build"),
        None,
    )
    .unwrap();

    let content =
        std::fs::read_to_string(dir.path().join("@me/myapp/task").join(format!("{slug}.md")))
            .unwrap();
    assert!(content.contains("stage: in-progress"));
    assert!(content.contains("mode: build"));
}

#[test]
fn test_task_move_rejects_invalid_effort() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Bad Effort",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();

    let result =
        temper_cli::commands::task::move_task(&config, &slug, None, None, None, None, Some("huge"));
    assert!(result.is_err(), "invalid effort should be rejected");
}

#[test]
fn test_task_show_json_includes_mode_effort() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "JSON Mode",
        Some(&g_slug),
        Some("plan"),
        Some("small"),
        None,
    )
    .unwrap();

    let task = temper_cli::commands::task::find_task(&config, &slug, None)
        .unwrap()
        .unwrap();
    assert_eq!(task.mode, Some("plan".to_string()));
    assert_eq!(task.effort, Some("small".to_string()));
}

#[test]
fn test_task_list_filters_by_stage() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();

    // Create two tasks — one stays backlog, one moves to in-progress
    let slug_backlog = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Backlog Task",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();
    let slug_ip = temper_cli::commands::task::create(
        &config,
        "myapp",
        "InProg Task",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();
    temper_cli::commands::task::move_task(
        &config,
        &slug_ip,
        Some("in-progress"),
        None,
        None,
        None,
        None,
    )
    .unwrap();

    // Load all tasks — should have 2
    let all = temper_cli::commands::task::load_tasks(&config, Some("myapp"), None).unwrap();
    assert_eq!(all.len(), 2);

    // list with stage=backlog should succeed (text format, just verify no error)
    let result =
        temper_cli::commands::task::list(&config, Some("myapp"), None, Some("backlog"), "json");
    assert!(result.is_ok());

    // Verify actual filtering by checking load_tasks + manual filter (mirrors list internals)
    let backlog_tasks: Vec<_> = all.iter().filter(|t| t.stage == "backlog").collect();
    assert_eq!(backlog_tasks.len(), 1);
    assert!(backlog_tasks[0].slug.contains(&slug_backlog));

    let ip_tasks: Vec<_> = all.iter().filter(|t| t.stage == "in-progress").collect();
    assert_eq!(ip_tasks.len(), 1);
    assert!(ip_tasks[0].slug.contains(&slug_ip));
}

#[test]
fn test_task_list_rejects_invalid_stage() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    temper_cli::commands::task::create(&config, "myapp", "Task", Some(&g_slug), None, None, None)
        .unwrap();

    let result =
        temper_cli::commands::task::list(&config, Some("myapp"), None, Some("brainstorm"), "text");
    assert!(result.is_err(), "invalid stage should be rejected");
}

#[test]
fn test_find_task_by_seq_number() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug =
        temper_cli::commands::goal::create(&config, "myapp", "v0.1", None, "text").unwrap();
    let slug = temper_cli::commands::task::create(
        &config,
        "myapp",
        "Seq Lookup",
        Some(&g_slug),
        None,
        None,
        None,
    )
    .unwrap();

    // Get the task's seq number
    let task = temper_cli::commands::task::find_task(&config, &slug, None)
        .unwrap()
        .expect("task should exist by slug");
    let seq = task.seq;

    // Look up by seq number string
    let found = temper_cli::commands::task::find_task(&config, &seq.to_string(), None)
        .unwrap()
        .expect("task should be found by seq number");
    assert_eq!(found.slug, slug);
}
