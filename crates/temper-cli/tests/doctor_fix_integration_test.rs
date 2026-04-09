//! Integration test for the full doctor fix pipeline.

use std::fs;

use tempfile::TempDir;

fn test_config(dir: &TempDir) -> temper_cli::config::Config {
    let state_dir = dir.path().join(".temper");
    fs::create_dir_all(&state_dir).unwrap();
    fs::write(state_dir.join("manifest.json"), "{}\n").unwrap();
    fs::write(state_dir.join("events.jsonl"), "").unwrap();
    temper_cli::config::Config {
        vault_root: dir.path().to_path_buf(),
        state_dir,
        contexts: vec!["temper".to_string()],
        subscriptions: Vec::new(),
        skill_output: dir.path().join("temper.md"),
        skill_framework: "superpowers".to_string(),
    }
}

#[test]
fn doctor_fix_pipeline_end_to_end() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);
    let vault = dir.path();

    // Create a task with missing temper-* fields and non-slug filename (em-dash + spaces + punctuation)
    let task_dir = vault.join("@me").join("temper").join("task");
    fs::create_dir_all(&task_dir).unwrap();
    fs::write(
        task_dir.join("2026-04-05 \u{2014} My Feature!.md"),
        "---\ntemper-type: task\ntemper-context: temper\ntitle: My Feature!\n---\nBody\n",
    )
    .unwrap();

    // Create a session with em-dash filename and missing fields
    let session_dir = vault.join("@me").join("temper").join("session");
    fs::create_dir_all(&session_dir).unwrap();
    fs::write(
        session_dir.join("2026-04-05 \u{2014} my-session.md"),
        "---\ntemper-type: session\ntemper-context: temper\ndate: 2026-04-05\ntitle: My Session\n---\nNotes\n",
    )
    .unwrap();

    // Run fix (not dry run)
    let report = temper_cli::actions::doctor::fix(&config, None, false).unwrap();

    // Verify field fixes happened (temper-id, slug, temper-stage, etc. set by inference)
    assert!(report.fields_set > 0, "expected fields set");

    // Verify task file was renamed (slugified, no date prefix for tasks)
    let old_task_path = task_dir.join("2026-04-05 \u{2014} My Feature!.md");
    assert!(
        !old_task_path.exists(),
        "old task file should not exist after fix"
    );
    let new_task_path = task_dir.join("my-feature.md");
    assert!(
        new_task_path.exists(),
        "task should be renamed to my-feature.md"
    );

    // Verify session file was renamed (em-dash → hyphen, date-prefixed slug)
    let old_session_path = session_dir.join("2026-04-05 \u{2014} my-session.md");
    assert!(
        !old_session_path.exists(),
        "old session file should not exist after fix"
    );
    let new_session_path = session_dir.join("2026-04-05-my-session.md");
    assert!(
        new_session_path.exists(),
        "session should be renamed to 2026-04-05-my-session.md"
    );

    // Verify frontmatter was fixed in the task
    let task_content = fs::read_to_string(&new_task_path).unwrap();
    assert!(
        task_content.contains("temper-type: task"),
        "should have temper-type; got:\n{task_content}"
    );
    assert!(
        task_content.contains("temper-context: temper"),
        "should have temper-context; got:\n{task_content}"
    );
    assert!(
        task_content.contains("slug:"),
        "should have slug field; got:\n{task_content}"
    );
    assert!(
        task_content.contains("temper-stage:"),
        "should have temper-stage; got:\n{task_content}"
    );

    // Re-run doctor scan — all auto-fixable issues should be resolved after fix.
    // Task 17 added SetOwnerField so temper-owner is now backfilled on provisional files.
    let scan = temper_cli::actions::doctor::scan(&config, None).unwrap();
    let remaining_fixable: Vec<_> = scan
        .file_results
        .iter()
        .flat_map(|r| r.issues.iter())
        .filter(|i| i.auto_fixable)
        .collect();
    assert_eq!(
        remaining_fixable.len(),
        0,
        "all auto-fixable issues should be resolved after fix; remaining: {:?}",
        remaining_fixable
    );
    // Also verify owner_backfilled was reported in the fix report
    assert!(
        report.owner_backfilled > 0,
        "expected at least one owner backfill"
    );
}
