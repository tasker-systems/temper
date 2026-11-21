#![cfg(feature = "test-db")]

//! End-to-end coverage for session→task linking on the live `resource create`
//! path (`temper resource create --type session --task <task>`), where `<task>` is the task's
//! ref, bare UUID or title slug, and the create's JSON reports the outcome as `task_link`.
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
        idempotency_key: None,
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
) -> temper_core::types::resource_view::ResourceView {
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
                    preserve_source: false,
                    idempotency_key: None,
                    idempotency_key_given: false,
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
    let outgoing: Vec<_> = edges
        .iter()
        .filter(|e| e.direction == temper_core::types::blob::BlobRelationEdgeDirection::Outgoing)
        .collect();
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
        edge.peer_slug.as_deref(),
        Some("implement-widget"),
        "edge target must be the task slug"
    );

    // The task is the target: its incoming edge points back at the session.
    let task_row = resolve_by_title(&app.client, "myapp", "task", "Implement Widget").await;
    // The id-based link must target the seeded task's resource id directly.
    assert_eq!(
        outgoing[0].peer_id,
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
        .filter(|e| e.direction == temper_core::types::blob::BlobRelationEdgeDirection::Incoming)
        .collect();
    assert_eq!(
        incoming.len(),
        1,
        "task must have exactly one incoming session→task edge; got {task_edges:?}"
    );
    assert_eq!(
        incoming[0].peer_id, session_id,
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
                    preserve_source: false,
                    idempotency_key: None,
                    idempotency_key_given: false,
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
                    preserve_source: false,
                    idempotency_key: None,
                    idempotency_key_given: false,
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
                    preserve_source: false,
                    idempotency_key: None,
                    idempotency_key_given: false,
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
                    preserve_source: false,
                    idempotency_key: None,
                    idempotency_key_given: false,
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

// ---------------------------------------------------------------------------
// Tests 6–9: the real binary — `--task` takes a ref, a UUID or a title slug,
// and the create's JSON says whether the link landed (`task_link`).
// ---------------------------------------------------------------------------

/// Run `temper resource create --type session … --task <needle>` through the real binary and
/// parse its stdout as exactly one JSON document. Stdout only: the tracing warn rides stderr.
async fn create_session_via_cli(
    app: &common::E2eTestApp,
    title: &str,
    task_needle: &str,
) -> serde_json::Value {
    let args = [
        "resource",
        "create",
        "--type",
        "session",
        "--title",
        title,
        "--context",
        "@me/myapp",
        "--task",
        task_needle,
        "--format",
        "json",
    ];
    let output = common::run_temper_cli_with_stdin(app, "Session body.\n", &args)
        .await
        .expect("cli run");
    assert!(
        output.status.success(),
        "cli {args:?} failed: stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "cli {args:?} did not emit exactly one JSON document ({e}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

/// Seed a context and one task; return the app and the task's id.
async fn app_with_one_task(pool: sqlx::PgPool) -> (common::E2eTestApp, uuid::Uuid) {
    let app = common::setup(pool).await;
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
    let task_id = *resolve_by_title(&app.client, "myapp", "task", "Implement Widget")
        .await
        .id
        .as_uuid();
    (app, task_id)
}

/// The session's outgoing `advances` edge targets exactly `task_id`.
async fn assert_one_advances_edge_to(
    app: &common::E2eTestApp,
    created: &serde_json::Value,
    task_id: uuid::Uuid,
) {
    let session_id: uuid::Uuid = created["id"]
        .as_str()
        .expect("created id")
        .parse()
        .expect("created id is a uuid");
    let edges = app
        .client
        .resources()
        .edges(session_id)
        .await
        .expect("fetch session edges");
    let outgoing: Vec<_> = edges
        .iter()
        .filter(|e| e.direction == temper_core::types::blob::BlobRelationEdgeDirection::Outgoing)
        .collect();
    assert_eq!(outgoing.len(), 1, "one outgoing edge; got {edges:?}");
    assert_eq!(outgoing[0].label, "advances");
    assert_eq!(outgoing[0].peer_id, task_id);
}

/// The JSON reports a landed link: status, the task's id and ref, and the edge handle.
fn assert_linked(created: &serde_json::Value, needle: &str, task_id: uuid::Uuid) {
    let link = &created["task_link"];
    assert_eq!(link["status"], "linked", "task_link: {created}");
    assert_eq!(link["requested"], needle);
    assert_eq!(link["task_id"], task_id.to_string());
    assert_eq!(
        link["task_ref"],
        format!("implement-widget-{task_id}"),
        "task_ref is the decorated ref"
    );
    assert!(
        link["edge_handle"]
            .as_str()
            .is_some_and(|h| h.parse::<uuid::Uuid>().is_ok()),
        "edge_handle is a uuid: {link}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cli_task_flag_accepts_a_decorated_ref(pool: sqlx::PgPool) {
    let (app, task_id) = app_with_one_task(pool).await;
    let needle = format!("implement-widget-{task_id}");
    let created = create_session_via_cli(&app, "Ref Linked Session", &needle).await;
    assert_linked(&created, &needle, task_id);
    assert_one_advances_edge_to(&app, &created, task_id).await;
}

/// A ref resolves on its trailing UUID: a stale slug half still links the task it names.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cli_task_flag_accepts_a_bare_uuid_and_a_stale_slug_ref(pool: sqlx::PgPool) {
    let (app, task_id) = app_with_one_task(pool).await;

    let bare = task_id.to_string();
    let created = create_session_via_cli(&app, "Uuid Linked Session", &bare).await;
    assert_linked(&created, &bare, task_id);
    assert_one_advances_edge_to(&app, &created, task_id).await;

    let stale = format!("a-renamed-title-{task_id}");
    let created = create_session_via_cli(&app, "Stale Ref Linked Session", &stale).await;
    assert_linked(&created, &stale, task_id);
    assert_one_advances_edge_to(&app, &created, task_id).await;
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cli_task_flag_accepts_a_title_slug(pool: sqlx::PgPool) {
    let (app, task_id) = app_with_one_task(pool).await;
    let created = create_session_via_cli(&app, "Slug Linked Session", "implement-widget").await;
    assert_linked(&created, "implement-widget", task_id);
    assert_one_advances_edge_to(&app, &created, task_id).await;
}

/// An unmatched `--task` still creates the session, exits 0, and says so on stdout: the JSON's
/// `task_link.status` is `not_found` for an unknown slug and for a ref that names nothing, and
/// `lookup_failed` for an ambiguous slug suffix and for a ref that names a resource that is not a
/// task. No edge in any case.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cli_task_flag_unmatched_is_visible_in_the_json(pool: sqlx::PgPool) {
    let (app, _task_id) = app_with_one_task(pool).await;
    // A second `-widget` task makes the `widget` suffix ambiguous.
    seed_task(&app.client, "myapp", "other-widget", "Other Widget").await;

    let cases = [
        ("does-not-exist-anywhere", "not_found"),
        (
            "implement-widget-01a1f8f5-4b02-72e2-9ae8-000000000000",
            "not_found",
        ),
        ("widget", "lookup_failed"),
    ];
    for (i, (needle, status)) in cases.iter().enumerate() {
        let created = create_session_via_cli(&app, &format!("Unlinked Session {i}"), needle).await;
        let link = &created["task_link"];
        assert_eq!(link["status"], *status, "needle {needle}: {created}");
        assert_eq!(link["requested"], *needle);
        assert!(
            link["detail"].as_str().is_some_and(|d| d.contains(needle)),
            "detail names the needle: {link}"
        );
        assert!(
            link.get("task_id").is_none(),
            "no task_id on a miss: {link}"
        );
        let session_id: uuid::Uuid = created["id"].as_str().unwrap().parse().unwrap();
        let edges = app.client.resources().edges(session_id).await.unwrap();
        assert!(edges.is_empty(), "no edge on a miss; got {edges:?}");
    }

    // A ref to a session — not a task — is refused by the lookup, never linked.
    let first = create_session_via_cli(&app, "Session As Target", "does-not-exist-anywhere").await;
    let session_ref = first["ref"].as_str().expect("ref").to_string();
    let created = create_session_via_cli(&app, "Session To Session", &session_ref).await;
    let link = &created["task_link"];
    assert_eq!(link["status"], "lookup_failed", "{created}");
    assert!(
        link["detail"]
            .as_str()
            .is_some_and(|d| d.contains("not a task")),
        "detail says it is not a task: {link}"
    );
    let session_id: uuid::Uuid = created["id"].as_str().unwrap().parse().unwrap();
    assert!(app
        .client
        .resources()
        .edges(session_id)
        .await
        .unwrap()
        .is_empty());
}
