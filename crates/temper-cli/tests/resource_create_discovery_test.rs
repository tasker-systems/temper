//! Integration tests asserting that `commands::resource::create` emits
//! `discovery::Event::ResourceCreate` for the four doctypes that had emission
//! pre-B5b (task, goal, session, research) and does NOT emit for concept or
//! decision (which never had emission).

use tempfile::TempDir;

mod common;

fn test_config(dir: &TempDir) -> temper_cli::config::Config {
    common::init_isolated_auth();
    let state_dir = dir.path().join(".temper");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(state_dir.join("manifest.json"), "{}\n").unwrap();
    std::fs::write(state_dir.join("events.jsonl"), "").unwrap();
    // Pre-create the context directory so `resolve_context_with_fallback`
    // does not redirect "myapp" → "default" for tests that skip a prereq step.
    std::fs::create_dir_all(dir.path().join("@me/myapp")).unwrap();
    temper_cli::config::Config {
        vault_root: dir.path().to_path_buf(),
        state_dir,
        contexts: vec!["myapp".to_string()],
        subscriptions: Vec::new(),
        skill_output: dir.path().join("temper.md"),
        profile_slug: None,
    }
}

/// Read all events from events.jsonl and return those with `doc_type` matching
/// the supplied string.
fn events_with_doc_type(
    config: &temper_cli::config::Config,
    doc_type: &str,
) -> Vec<temper_cli::discovery::Event> {
    let path = config.state_dir.join("events.jsonl");
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<temper_cli::discovery::Event>(line).ok())
        .filter(|e| matches!(e, temper_cli::discovery::Event::ResourceCreate { doc_type: dt, .. } if dt == doc_type))
        .collect()
}

/// Create a goal (needed as prerequisite for task creation) and return its slug.
fn create_prereq_goal(config: &temper_cli::config::Config, context: &str) -> String {
    common::create_goal(config, context, "Prereq Goal")
}

// ---------------------------------------------------------------------------
// Positive tests — emission for task, goal, session, research
// ---------------------------------------------------------------------------

#[test]
fn resource_create_task_emits_discovery_event() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let g_slug = create_prereq_goal(&config, "myapp");

    temper_cli::commands::resource::create(
        &config,
        "task",
        "My New Task",
        Some("myapp"),
        Some(&g_slug),
        Some("build"),
        Some("small"),
        None,
        None,
        "text",
    )
    .unwrap();

    let create_events = events_with_doc_type(&config, "task");
    assert_eq!(
        create_events.len(),
        1,
        "expected exactly one ResourceCreate event for doc_type=task, got: {create_events:?}"
    );

    if let temper_cli::discovery::Event::ResourceCreate {
        doc_type,
        title,
        context,
        ..
    } = &create_events[0]
    {
        assert_eq!(doc_type, "task");
        assert_eq!(title, "My New Task");
        assert_eq!(context, "myapp");
    } else {
        panic!("event was not ResourceCreate");
    }
}

#[test]
fn resource_create_goal_emits_discovery_event() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    temper_cli::commands::resource::create(
        &config,
        "goal",
        "Sprint Goal",
        Some("myapp"),
        None,
        None,
        None,
        None,
        None,
        "text",
    )
    .unwrap();

    let create_events = events_with_doc_type(&config, "goal");
    assert_eq!(
        create_events.len(),
        1,
        "expected exactly one ResourceCreate event for doc_type=goal, got: {create_events:?}"
    );

    if let temper_cli::discovery::Event::ResourceCreate {
        doc_type,
        title,
        context,
        ..
    } = &create_events[0]
    {
        assert_eq!(doc_type, "goal");
        assert_eq!(title, "Sprint Goal");
        assert_eq!(context, "myapp");
    } else {
        panic!("event was not ResourceCreate");
    }
}

#[test]
fn resource_create_session_emits_discovery_event() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    temper_cli::commands::resource::create(
        &config,
        "session",
        "Dev Session",
        Some("myapp"),
        None,
        None,
        None,
        None,
        None,
        "text",
    )
    .unwrap();

    let create_events = events_with_doc_type(&config, "session");
    assert_eq!(
        create_events.len(),
        1,
        "expected exactly one ResourceCreate event for doc_type=session, got: {create_events:?}"
    );

    if let temper_cli::discovery::Event::ResourceCreate {
        doc_type,
        title,
        context,
        ..
    } = &create_events[0]
    {
        assert_eq!(doc_type, "session");
        assert_eq!(title, "Dev Session");
        assert_eq!(context, "myapp");
    } else {
        panic!("event was not ResourceCreate");
    }
}

#[test]
fn resource_create_research_emits_discovery_event() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    temper_cli::commands::resource::create(
        &config,
        "research",
        "Auth Flow Study",
        Some("myapp"),
        None,
        None,
        None,
        None,
        None,
        "text",
    )
    .unwrap();

    let create_events = events_with_doc_type(&config, "research");
    assert_eq!(
        create_events.len(),
        1,
        "expected exactly one ResourceCreate event for doc_type=research, got: {create_events:?}"
    );

    if let temper_cli::discovery::Event::ResourceCreate {
        doc_type,
        title,
        context,
        ..
    } = &create_events[0]
    {
        assert_eq!(doc_type, "research");
        assert_eq!(title, "Auth Flow Study");
        assert_eq!(context, "myapp");
    } else {
        panic!("event was not ResourceCreate");
    }
}

// ---------------------------------------------------------------------------
// Negative tests — no emission for concept or decision (never had it pre-B5b)
// ---------------------------------------------------------------------------

/// Helper: read ALL ResourceCreate events from events.jsonl.
fn all_resource_create_events(
    config: &temper_cli::config::Config,
) -> Vec<temper_cli::discovery::Event> {
    let path = config.state_dir.join("events.jsonl");
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<temper_cli::discovery::Event>(line).ok())
        .filter(|e| matches!(e, temper_cli::discovery::Event::ResourceCreate { .. }))
        .collect()
}

#[test]
fn resource_create_concept_does_not_emit_discovery_event() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    temper_cli::commands::resource::create(
        &config,
        "concept",
        "Domain Concept",
        Some("myapp"),
        None,
        None,
        None,
        None,
        None,
        "text",
    )
    .unwrap();

    let create_events = all_resource_create_events(&config);
    assert!(
        create_events.is_empty(),
        "concept create should not emit ResourceCreate event, but found: {create_events:?}"
    );
}

#[test]
fn resource_create_decision_does_not_emit_discovery_event() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    temper_cli::commands::resource::create(
        &config,
        "decision",
        "Use PostgreSQL",
        Some("myapp"),
        None,
        None,
        None,
        None,
        None,
        "text",
    )
    .unwrap();

    let create_events = all_resource_create_events(&config);
    assert!(
        create_events.is_empty(),
        "decision create should not emit ResourceCreate event, but found: {create_events:?}"
    );
}
