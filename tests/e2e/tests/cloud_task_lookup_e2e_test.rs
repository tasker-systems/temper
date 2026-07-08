#![cfg(feature = "test-db")]

//! End-to-end coverage for cloud-mode task lookup (`load_tasks` / `find_task`).
//!
//! temper is cloud-only: the local vault directory is a read-only projection
//! cache that is empty/absent on a fresh device. These tests prove that
//! `load_tasks` lists tasks from the cloud API (NOT by scanning the local
//! vault with `fs::read_dir`) and that `find_task` resolves slugs through
//! that same cloud-backed path — even when no projection files exist on disk.
//!
//! Post-Phase-2, `managed_meta` is Property-only: identity (`title`) is a
//! first-class row column, and `load_tasks` sources it from the full list row
//! (not `managed_meta`). Slugs are title-derived (`sluggify`) for `find_task`
//! matching; restoring stored-slug / `branch`/`pr` / goal-edge fidelity is task
//! 019f3d55.
//!
//! Tasks are seeded via the API client (`app.client.ingest()`), so nothing is
//! ever written to the vault directory. Each test then drives the synchronous
//! `temper_cli::actions::task::{load_tasks, find_task}` lib calls inside
//! `spawn_blocking` + `temp_env::with_vars(cloud_env(...))`, because those
//! functions build their own tokio runtime (via `runtime::with_client`) and
//! nesting runtimes panics.

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload};

/// Shared env-var builder for cloud-mode CLI lib invocations. Mirrors the
/// helper in `cloud_writes_test.rs`. `TEMPER_GLOBAL_CONFIG` points at a
/// non-existent path so no developer config file leaks into tests.
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
///
/// Identity (`title`/`slug`) travels as first-class top-level `IngestPayload`
/// fields; `managed_meta` carries only the Property vocabulary (stage/mode/
/// effort/seq) — an identity key here would be rejected by the closed vocabulary.
async fn seed_task(
    client: &temper_client::TemperClient,
    context: &str,
    slug: &str,
    title: &str,
    stage: &str,
    seq: Option<i64>,
) {
    let mut managed = serde_json::Map::new();
    managed.insert("temper-stage".to_string(), serde_json::json!(stage));
    managed.insert("temper-mode".to_string(), serde_json::json!("build"));
    managed.insert("temper-effort".to_string(), serde_json::json!("small"));
    if let Some(s) = seq {
        managed.insert("temper-seq".to_string(), serde_json::json!(s));
    }

    let payload = IngestPayload {
        title: title.to_string(),
        origin_uri: format!("kb://{context}/task/{slug}"),
        context_ref: format!("@me/{context}"),
        home_cogmap_id: None,
        doc_type_name: "task".to_string(),
        content_hash: None,
        slug: slug.to_string(),
        content: String::new(),
        metadata: None,
        managed_meta: Some(serde_json::Value::Object(managed)),
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

// ---------------------------------------------------------------------------
// Test 1: load_tasks returns API tasks sorted by seq with correct mapping
// ---------------------------------------------------------------------------

/// Seed three tasks in a context via the API (varied stage/seq), then drive
/// `load_tasks` in cloud mode and assert it returns the server's tasks — sorted
/// by seq ascending — with correct title/slug/stage/mode/effort/seq/context
/// mapping. The local vault dir is empty (nothing is ever projected), proving
/// the result comes from the API, not a disk scan.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn load_tasks_returns_api_tasks_sorted_by_seq(pool: sqlx::PgPool) {
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

    // Seed out of seq order to prove sorting happens. Slug == sluggify(title).
    seed_task(&app.client, "myapp", "task-c", "Task C", "done", Some(30)).await;
    seed_task(
        &app.client,
        "myapp",
        "task-a",
        "Task A",
        "backlog",
        Some(10),
    )
    .await;
    seed_task(
        &app.client,
        "myapp",
        "task-b",
        "Task B",
        "in-progress",
        Some(20),
    )
    .await;

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();

    let tasks = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::actions::task::load_tasks(&cli_config, Some("myapp"))
                .expect("load_tasks must succeed in cloud mode")
        })
    })
    .await
    .expect("spawn_blocking joined");

    assert_eq!(tasks.len(), 3, "expected all three seeded tasks");

    // Sorted by seq ascending: a(10), b(20), c(30). Slug is title-derived.
    let slugs: Vec<&str> = tasks.iter().map(|t| t.slug.as_str()).collect();
    assert_eq!(
        slugs,
        vec!["task-a", "task-b", "task-c"],
        "tasks must be sorted by seq ascending"
    );

    // Field mapping on the first task — identity + workflow projections come from
    // the row's top-level columns.
    let a = &tasks[0];
    assert_eq!(a.slug, "task-a");
    assert_eq!(a.title, "Task A");
    assert_eq!(a.stage, "backlog");
    assert_eq!(a.mode.as_deref(), Some("build"));
    assert_eq!(a.effort.as_deref(), Some("small"));
    assert_eq!(a.seq, Some(10));
    assert_eq!(
        a.context, "myapp",
        "context must come from the resource's context, not managed_meta"
    );
}

// ---------------------------------------------------------------------------
// Test 2: find_task resolves by exact slug, by unique suffix, and returns None
// ---------------------------------------------------------------------------

/// `find_task` resolves a task by exact (title-derived) slug and by an
/// unambiguous suffix, and returns `None` for an unknown identifier — all
/// through the cloud path with an empty local vault.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn find_task_resolves_by_slug_and_suffix(pool: sqlx::PgPool) {
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

    // Slug is title-derived: "Implement Widget" -> "implement-widget".
    seed_task(
        &app.client,
        "myapp",
        "implement-widget",
        "Implement Widget",
        "backlog",
        Some(10),
    )
    .await;
    seed_task(
        &app.client,
        "myapp",
        "refactor-gadget",
        "Refactor Gadget",
        "backlog",
        Some(20),
    )
    .await;

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();

    let (exact, suffix, missing) = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            let exact = temper_cli::actions::task::find_task(
                &cli_config,
                "implement-widget",
                Some("myapp"),
            )
            .expect("find_task exact must succeed");
            // Unique suffix: only the widget task ends with "widget".
            let suffix = temper_cli::actions::task::find_task(&cli_config, "widget", Some("myapp"))
                .expect("find_task suffix must succeed");
            let missing = temper_cli::actions::task::find_task(
                &cli_config,
                "does-not-exist-anywhere",
                Some("myapp"),
            )
            .expect("find_task missing must succeed (Ok(None))");
            (exact, suffix, missing)
        })
    })
    .await
    .expect("spawn_blocking joined");

    let exact = exact.expect("exact slug must resolve");
    assert_eq!(exact.slug, "implement-widget");
    assert_eq!(exact.title, "Implement Widget");

    let suffix = suffix.expect("unique suffix must resolve");
    assert_eq!(
        suffix.slug, "implement-widget",
        "suffix 'widget' must resolve to the implement-widget task"
    );

    assert!(
        missing.is_none(),
        "unknown identifier must resolve to None; got {missing:?}"
    );
}
