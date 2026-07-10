#![cfg(feature = "test-db")]

//! End-to-end coverage for session→task linking on the live `resource create`
//! path (`temper resource create --type session --task <task-slug>`).
//!
//! The link is asserted by `commands::resource::create` after the session is
//! created: a session→task relationship with `edge_kind = LeadsTo`,
//! `polarity = Forward`, `label = "advances"`, `weight = 1.0` (the session
//! *advances* the task; causal arrow session→task = Forward LeadsTo).
//!
//! Tasks are seeded via the API client (`app.client.ingest()`), so nothing is
//! written to the vault directory. Each test drives the synchronous
//! `temper_cli::commands::resource::create` lib call inside `spawn_blocking` +
//! `temp_env::with_vars(cloud_env(...))`, because the create path (and the
//! `find_task` lookup it calls) build their own tokio runtimes; nesting
//! runtimes would panic.

mod common;

use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::ingest::{pack_chunks, IngestPayload};

/// Shared env-var builder for cloud-mode CLI lib invocations. Mirrors the
/// helper in `cloud_writes_test.rs`.
fn cloud_env<'a>(
    api_url: &'a str,
    token: &'a str,
    global_config: &'a str,
) -> [(&'static str, Option<&'a str>); 4] {
    [
        ("TEMPER_API_URL", Some(api_url)),
        ("TEMPER_TOKEN", Some(token)),
        ("TEMPER_GLOBAL_CONFIG", Some(global_config)),
        ("TEMPER_AUTH_PATH", None),
    ]
}

/// Seed a task via the API client (cloud-only; no vault files written).
async fn seed_task(client: &temper_client::TemperClient, context: &str, slug: &str, title: &str) {
    let payload = IngestPayload {
        segmented: None,
        goal: None,
        title: title.to_string(),
        origin_uri: format!("kb://{context}/task/{slug}"),
        context_ref: format!("@me/{context}"),
        home_cogmap_id: None,
        doc_type_name: "task".to_string(),
        content_hash: None,
        content: String::new(),
        metadata: None,
        managed_meta: Some(serde_json::json!({
            "temper-stage": "backlog",
            "temper-mode": "build",
            "temper-effort": "small",
            "temper-seq": 10,
        })),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };
    client
        .ingest()
        .create(&payload)
        .await
        .expect("seed task via client");
}

/// Resolve a created resource's row by title (verification helper). The slug
/// is §7-dissolved on readback (`row.slug` is always `None` — `temper-slug` is a
/// `KeyFate::Die` key), so this list-and-filter keys on the surviving `title`
/// column instead. Title is the faithful stable handle here: each test creates
/// resources with unique titles, and the production edge readback itself derives
/// `peer_slug` from the peer title (`edge_service`), so a title-keyed lookup
/// mirrors the substrate's own addressing.
async fn resolve_by_title(
    client: &temper_client::TemperClient,
    context: &str,
    doc_type: &str,
    title: &str,
) -> temper_workflow::types::resource::ResourceRow {
    let params = temper_workflow::types::resource::ResourceListParams {
        context_ref: Some(format!("@me/{context}")),
        doc_type_name: Some(doc_type.to_string()),
        ..Default::default()
    };
    let resp = client
        .resources()
        .list(&params)
        .await
        .expect("list for title resolve");
    resp.rows
        .into_iter()
        .find(|r| r.title == title)
        .unwrap_or_else(|| panic!("no {doc_type} with title '{title}' in context '{context}'"))
}

// ---------------------------------------------------------------------------
// Test 1: --task creates exactly one session→task "advances" edge
// ---------------------------------------------------------------------------

/// Seed a task; create a session with `--task` pointing at it; assert exactly
/// one session→task edge with `label == "advances"`, `edge_kind == LeadsTo`,
/// `polarity == Forward`, correct source (session) / target (task).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
#[ignore = "deferred: the CLI `--task` link path calls find_task, which requires temper-title in managed_meta; temper-title is a §7-Die key dropped by readback (F1, same gap as cloud_task_lookup). Un-ignore when receive-side identity-key injection lands."]
async fn create_session_with_task_asserts_advances_edge(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("myapp", None)
        .await
        .expect("create myapp context");

    seed_task(&app.client, "myapp", "implement-widget", "Implement Widget").await;

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();

    let title = "Work On Widget";

    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::commands::resource::create(
                &cli_config,
                temper_cli::commands::resource::CreateResourceArgs {
                    open_meta: None,
                    goal: None,
                    doc_type: "session",
                    title,
                    context: Some("@me/myapp"),
                    cogmap: None,
                    mode: None,
                    effort: None,
                    task: Some("implement-widget"),
                    body_flag: None,
                    from: None,
                    format: temper_cli::format::OutputFormat::Json,
                    act: Default::default(),
                    sources: Vec::new(),
                    sources_as_edges: false,
                    no_source: false,
                },
            )
            .expect("cloud create with --task must succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    // Resolve the created session's id and query its edges via the client.
    let session_row = resolve_by_title(&app.client, "myapp", "session", title).await;
    let session_id = *session_row.id.as_uuid();

    let edges = app
        .client
        .resources()
        .edges(session_id)
        .await
        .expect("fetch session edges");

    // Filter to outgoing edges (session is the source).
    let outgoing: Vec<_> = edges.iter().filter(|e| e.direction == "outgoing").collect();
    assert_eq!(
        outgoing.len(),
        1,
        "expected exactly one outgoing session→task edge; got {edges:?}"
    );
    let edge = outgoing[0];
    assert_eq!(edge.label, "advances", "edge label must be 'advances'");
    assert_eq!(
        edge.edge_kind,
        EdgeKind::LeadsTo,
        "edge_kind must be LeadsTo"
    );
    assert_eq!(edge.polarity, Polarity::Forward, "polarity must be Forward");
    assert!(
        (edge.weight - 1.0).abs() < f64::EPSILON,
        "weight must be 1.0; got {}",
        edge.weight
    );
    // The peer (target) of the outgoing edge is the task.
    assert_eq!(
        edge.peer_slug, "implement-widget",
        "edge target must be the task slug"
    );

    // The task is the target: its incoming edge points back at the session.
    let task_row = resolve_by_title(&app.client, "myapp", "task", "Implement Widget").await;
    // The id-based link must target the seeded task's resource id directly.
    assert_eq!(
        uuid::Uuid::from(outgoing[0].peer_resource_id),
        *task_row.id.as_uuid(),
        "outgoing advances edge must target the seeded task's resource id"
    );

    let task_edges = app
        .client
        .resources()
        .edges(*task_row.id.as_uuid())
        .await
        .expect("fetch task edges");
    let incoming: Vec<_> = task_edges
        .iter()
        .filter(|e| e.direction == "incoming")
        .collect();
    assert_eq!(
        incoming.len(),
        1,
        "task must have exactly one incoming session→task edge; got {task_edges:?}"
    );
    assert_eq!(
        uuid::Uuid::from(incoming[0].peer_resource_id),
        session_id,
        "incoming edge source must be the session"
    );
    assert_eq!(incoming[0].label, "advances");
}

// ---------------------------------------------------------------------------
// Test 2: no --task → no relationship edge
// ---------------------------------------------------------------------------

/// Create a session without `--task`; assert it succeeds and produces no edge.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_session_without_task_has_no_edge(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("myapp", None)
        .await
        .expect("create myapp context");

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();

    let title = "Solo Session";

    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::commands::resource::create(
                &cli_config,
                temper_cli::commands::resource::CreateResourceArgs {
                    open_meta: None,
                    goal: None,
                    doc_type: "session",
                    title,
                    context: Some("@me/myapp"),
                    cogmap: None,
                    mode: None,
                    effort: None,
                    task: None,
                    body_flag: None,
                    from: None,
                    format: temper_cli::format::OutputFormat::Json,
                    act: Default::default(),
                    sources: Vec::new(),
                    sources_as_edges: false,
                    no_source: false,
                },
            )
            .expect("cloud create without --task must succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    let session_row = resolve_by_title(&app.client, "myapp", "session", title).await;

    let edges = app
        .client
        .resources()
        .edges(*session_row.id.as_uuid())
        .await
        .expect("fetch session edges");
    assert!(
        edges.is_empty(),
        "session created without --task must have no edges; got {edges:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 3: unknown --task → session still created, no edge, no failure
// ---------------------------------------------------------------------------

/// `--task` with a non-existent slug: the session is still created, no edge is
/// asserted, and `create` returns `Ok(())` (the unknown task is warned + skipped).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_session_with_unknown_task_succeeds_without_edge(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("myapp", None)
        .await
        .expect("create myapp context");

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();

    let title = "Orphan Link Session";

    let result = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::commands::resource::create(
                &cli_config,
                temper_cli::commands::resource::CreateResourceArgs {
                    open_meta: None,
                    goal: None,
                    doc_type: "session",
                    title,
                    context: Some("@me/myapp"),
                    cogmap: None,
                    mode: None,
                    effort: None,
                    task: Some("does-not-exist-anywhere"),
                    body_flag: None,
                    from: None,
                    format: temper_cli::format::OutputFormat::Json,
                    act: Default::default(),
                    sources: Vec::new(),
                    sources_as_edges: false,
                    no_source: false,
                },
            )
        })
    })
    .await
    .expect("spawn_blocking joined");

    assert!(
        result.is_ok(),
        "unknown --task must not fail the create; got {result:?}"
    );

    // The session exists.
    let session_row = resolve_by_title(&app.client, "myapp", "session", title).await;

    // No edge was asserted.
    let edges = app
        .client
        .resources()
        .edges(*session_row.id.as_uuid())
        .await
        .expect("fetch session edges");
    assert!(
        edges.is_empty(),
        "no edge must be asserted for an unknown task; got {edges:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: --task on a non-session doctype → BadRequest error
// ---------------------------------------------------------------------------

/// `create` with `doc_type: "research"` + `task: Some(..)` returns a BadRequest
/// error (the fail-fast guard runs before any create round-trip).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_non_session_with_task_errors(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("myapp", None)
        .await
        .expect("create myapp context");

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();

    let result = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::commands::resource::create(
                &cli_config,
                temper_cli::commands::resource::CreateResourceArgs {
                    open_meta: None,
                    goal: None,
                    doc_type: "research",
                    title: "Research With Task Flag",
                    context: Some("@me/myapp"),
                    cogmap: None,
                    mode: None,
                    effort: None,
                    task: Some("implement-widget"),
                    body_flag: None,
                    from: None,
                    format: temper_cli::format::OutputFormat::Json,
                    act: Default::default(),
                    sources: Vec::new(),
                    sources_as_edges: false,
                    no_source: false,
                },
            )
        })
    })
    .await
    .expect("spawn_blocking joined");

    assert!(
        result.is_err(),
        "--task on a non-session doctype must error; got {result:?}"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("session"),
        "error message should explain --task is session-only; got: {err_msg}"
    );
}

// ---------------------------------------------------------------------------
// Test 5: ambiguous --task suffix → session still created, no edge, no failure
// ---------------------------------------------------------------------------

/// An ambiguous `--task` suffix makes `find_task` return `Err` (not `None`).
/// The link is a best-effort tail on an already-committed session, so the
/// lookup error is warned + skipped: the session is still created, no edge is
/// asserted, and `create` returns `Ok(())`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
#[ignore = "deferred: the CLI `--task` link path calls find_task, which requires temper-title in managed_meta; temper-title is a §7-Die key dropped by readback (F1, same gap as cloud_task_lookup). Un-ignore when receive-side identity-key injection lands."]
async fn create_session_with_ambiguous_task_succeeds_without_edge(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("myapp", None)
        .await
        .expect("create myapp context");

    // Two tasks sharing the "-widget" suffix → `--task widget` is ambiguous.
    seed_task(&app.client, "myapp", "alpha-widget", "Alpha Widget").await;
    seed_task(&app.client, "myapp", "beta-widget", "Beta Widget").await;

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();

    let title = "Ambiguous Link Session";

    let result = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::commands::resource::create(
                &cli_config,
                temper_cli::commands::resource::CreateResourceArgs {
                    open_meta: None,
                    goal: None,
                    doc_type: "session",
                    title,
                    context: Some("@me/myapp"),
                    cogmap: None,
                    mode: None,
                    effort: None,
                    task: Some("widget"),
                    body_flag: None,
                    from: None,
                    format: temper_cli::format::OutputFormat::Json,
                    act: Default::default(),
                    sources: Vec::new(),
                    sources_as_edges: false,
                    no_source: false,
                },
            )
        })
    })
    .await
    .expect("spawn_blocking joined");

    assert!(
        result.is_ok(),
        "an ambiguous --task lookup must not fail the create; got {result:?}"
    );

    let session_row = resolve_by_title(&app.client, "myapp", "session", title).await;

    let edges = app
        .client
        .resources()
        .edges(*session_row.id.as_uuid())
        .await
        .expect("fetch session edges");
    assert!(
        edges.is_empty(),
        "no edge must be asserted for an ambiguous task suffix; got {edges:?}"
    );
}
