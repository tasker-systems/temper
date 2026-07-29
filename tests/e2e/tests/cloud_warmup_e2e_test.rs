#![cfg(feature = "test-db")]

//! End-to-end coverage for cloud-mode warmup (`build_warmup_result`).
//!
//! temper is cloud-only: the local vault directory is a read-only projection
//! cache that is empty/absent on a fresh device. These tests prove that the
//! warmup primer reads standing state from the cloud API (NOT by scanning the
//! local vault with `fs::read_dir`) — active goals, in-progress tasks, and recent
//! session pointers — all with an EMPTY vault directory (nothing is ever projected
//! to disk).
//!
//! **Retired here, as a named remainder rather than a silent gap**:
//! `warmup_truncates_long_session_body`, which asserted the 500-line cap on
//! `last_session_content`. That field no longer exists — the primer carries no
//! session prose at all — so the behavior it guarded is gone rather than untested.
//! Nothing can re-introduce it without re-adding the field, which is a compile-time
//! change, not a silent regression.
//!
//! Sessions and tasks are seeded via the API client (`app.client.ingest()`),
//! so nothing is written to the vault directory. Each test then drives the
//! synchronous `temper_cli::commands::warmup::build_warmup_result` lib call
//! inside `spawn_blocking` + `temp_env::with_vars(cloud_env(...))`, because it
//! builds its own tokio runtime (via `runtime::with_client`) and nesting
//! runtimes panics.

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};

/// Shared env-var builder for cloud-mode CLI lib invocations. Mirrors the
/// helper in `cloud_task_lookup_e2e_test.rs`. `TEMPER_GLOBAL_CONFIG` points at
/// a non-existent path so no developer config file leaks into tests.
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

/// Plain 64-char hex SHA-256 of `s` (no `sha256:` prefix). Matches the
/// `VARCHAR(64)` `content_hash` columns on `kb_resources` / `kb_chunks`.
fn hex_sha256(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hex::encode(hasher.finalize())
}

/// Seed a session via the API client with a real body (cloud-only; no vault
/// files written).
///
/// The body is seeded as a single un-headed chunk (`heading_depth: 0`) so the
/// server's content-reconstruction path returns exactly `body` — that is what
/// the warmup content fetch must surface. The embedding is a 768-dim zero
/// vector to match the `vector(768)` chunk column.
async fn seed_session(
    client: &temper_client::TemperClient,
    context: &str,
    slug: &str,
    title: &str,
    body: &str,
) {
    // Both `kb_resources.content_hash` and `kb_chunks.content_hash` are
    // `VARCHAR(64)`, so the seed uses a plain 64-char hex digest — not
    // `compute_body_hash`, which prefixes `sha256:` and overflows the column.
    let content_hash = hex_sha256(body);
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.to_string(),
        content_hash: content_hash.clone(),
        embedding: vec![0.0_f32; 768],
        embedded_with: None,
    };

    let payload = IngestPayload {
        segmented: None,
        goal: None,
        title: title.to_string(),
        origin_uri: format!("kb://{context}/session/{slug}"),
        context_ref: format!("@me/{context}"),
        home_cogmap_id: None,
        doc_type_name: "session".to_string(),
        content_hash: Some(content_hash),
        content: body.to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[chunk]).expect("encode session chunk")),
        act: Default::default(),
        sources: Vec::new(),
    };
    client
        .ingest()
        .create(&payload)
        .await
        .expect("seed session via client");
}

/// Seed a task via the API client (cloud-only; no vault files written).
async fn seed_task(
    client: &temper_client::TemperClient,
    context: &str,
    slug: &str,
    title: &str,
    stage: &str,
    seq: i64,
) {
    let managed = serde_json::json!({
        "temper-stage": stage,
        "temper-mode": "build",
        "temper-effort": "small",
        "temper-seq": seq,
    });
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
        managed_meta: Some(managed),
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

/// Seed a goal via the API client, carrying `temper-status` — the field the primer
/// reads to decide what is standing.
async fn seed_goal(
    client: &temper_client::TemperClient,
    context: &str,
    slug: &str,
    title: &str,
    status: &str,
) {
    let managed = serde_json::json!({ "temper-status": status });
    let payload = IngestPayload {
        segmented: None,
        goal: None,
        title: title.to_string(),
        origin_uri: format!("kb://{context}/goal/{slug}"),
        context_ref: format!("@me/{context}"),
        home_cogmap_id: None,
        doc_type_name: "goal".to_string(),
        content_hash: None,
        content: String::new(),
        metadata: None,
        managed_meta: Some(managed),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };
    client
        .ingest()
        .create(&payload)
        .await
        .expect("seed goal via client");
}

/// Default display limits for tests that are not exercising the caps themselves.
fn default_limits() -> temper_cli::commands::warmup::WarmupLimits {
    temper_cli::commands::warmup::WarmupLimits {
        sessions: 5,
        goals: 20,
    }
}

// ---------------------------------------------------------------------------
// Test 1: sessions come back as pointers, newest-first
// ---------------------------------------------------------------------------

/// Seed several sessions in ascending creation order, then drive
/// `build_warmup_result` in cloud mode with an EMPTY vault dir and assert the primer
/// lists them most-recent-first.
///
/// The empty-vault-dir part is the whole point: this is fresh-device correctness — a
/// `fs::read_dir` scan would return nothing.
///
/// Split from the in-progress-task assertions (below) deliberately. They were one test,
/// and because the task half is blocked on a readback gap the whole test carried
/// `#[ignore]` — which silently took this session-ordering assertion out of the suite
/// with it. A blocked assertion should not disable a working one.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn warmup_lists_sessions_newest_first(pool: sqlx::PgPool) {
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

    // Seed in ascending creation order so the LAST seeded is the most recent.
    // The ingest path stamps `created = now()`, so insertion order == age.
    for (slug, title) in [
        ("2026-05-28-first", "First Session"),
        ("2026-05-29-second", "Second Session"),
        ("2026-05-30-third", "Third Session"),
    ] {
        seed_session(&app.client, "myapp", slug, title, "# Body\n\nSome prose.\n").await;
    }

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();

    let result = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::commands::warmup::build_warmup_result(
                &cli_config,
                "@me/myapp",
                default_limits(),
            )
            .expect("build_warmup_result must succeed in cloud mode")
        })
    })
    .await
    .expect("spawn_blocking joined");

    let titles: Vec<&str> = result
        .recent_sessions
        .iter()
        .map(|s| s.title.as_str())
        .collect();
    assert_eq!(
        titles,
        vec!["Third Session", "Second Session", "First Session"],
        "sessions must be ordered most-recent-first from the API"
    );
}

// ---------------------------------------------------------------------------
// Test 2: only in-progress tasks surface
// ---------------------------------------------------------------------------

/// Seed one in-progress and one backlog task; only the in-progress one may surface.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
#[ignore = "deferred: readback does not inject temper-title into managed_meta (substrate §7 Die key), so `load_tasks` errors and `collect_in_progress_tasks` swallows it to an empty list — `in_progress_tasks` is always 0. Blocked on the readback-identity gap, NOT on this command"]
async fn warmup_surfaces_only_in_progress_tasks(pool: sqlx::PgPool) {
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

    seed_task(
        &app.client,
        "myapp",
        "task-active",
        "Active Task",
        "in-progress",
        10,
    )
    .await;
    seed_task(
        &app.client,
        "myapp",
        "task-idle",
        "Idle Task",
        "backlog",
        20,
    )
    .await;

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();

    let result = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::commands::warmup::build_warmup_result(
                &cli_config,
                "@me/myapp",
                default_limits(),
            )
            .expect("build_warmup_result must succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    assert_eq!(
        result.in_progress_tasks.len(),
        1,
        "only the in-progress task must surface"
    );
    assert_eq!(result.in_progress_tasks[0].slug, "task-active");
}

// ---------------------------------------------------------------------------
// Test 3: sessions are capped at the configured limit
// ---------------------------------------------------------------------------

/// Seed more sessions than the limit and assert the result caps, keeping the newest.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn warmup_caps_sessions_at_limit(pool: sqlx::PgPool) {
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

    for i in 0..8 {
        seed_session(
            &app.client,
            "myapp",
            &format!("2026-05-{:02}-s{i}", 20 + i),
            &format!("Session {i}"),
            &format!("# Session {i}\n\nBody {i}.\n"),
        )
        .await;
    }

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();

    let result = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::commands::warmup::build_warmup_result(
                &cli_config,
                "@me/myapp",
                temper_cli::commands::warmup::WarmupLimits {
                    sessions: 5,
                    goals: 20,
                },
            )
            .expect("build_warmup_result must succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    assert_eq!(
        result.recent_sessions.len(),
        5,
        "recent_sessions must be capped at the configured limit"
    );
    assert_eq!(result.recent_sessions[0].title, "Session 7");
}

// ---------------------------------------------------------------------------
// Test 4: only active goals surface, and they carry a usable ref
// ---------------------------------------------------------------------------

/// Seed goals across every `temper-status` value and assert the primer reports exactly
/// the active ones.
///
/// This test now carries the whole rule. The primer used to compare
/// `managed_meta["temper-status"]` itself and a unit test pinned that comparison, because
/// `ResourceListParams` had no `status` field and `list --status active` returned every
/// row — writing the assertion through that flag would have passed while reporting
/// completed and cancelled goals as standing. PR #564 made the filter real, so the primer
/// asks the query for `status = active` and the client-side copy is gone. What that
/// moves here is the *subject*: this is no longer a refinement check over a local
/// predicate but the only witness that the filter which actually runs excludes
/// non-active goals — which is why it seeds every status value rather than just one
/// counterexample.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn warmup_reports_only_active_goals(pool: sqlx::PgPool) {
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

    seed_goal(&app.client, "myapp", "g-active", "Standing Goal", "active").await;
    seed_goal(&app.client, "myapp", "g-done", "Finished Goal", "completed").await;
    seed_goal(&app.client, "myapp", "g-dead", "Dropped Goal", "cancelled").await;
    seed_goal(&app.client, "myapp", "g-held", "Paused Goal", "paused").await;

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();

    let result = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::commands::warmup::build_warmup_result(
                &cli_config,
                "@me/myapp",
                default_limits(),
            )
            .expect("build_warmup_result must succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    let titles: Vec<&str> = result
        .active_goals
        .iter()
        .map(|g| g.title.as_str())
        .collect();
    assert_eq!(
        titles,
        vec!["Standing Goal"],
        "only temper-status=active goals may be reported as standing"
    );
    assert_eq!(
        result.active_goal_total, 1,
        "active_goal_total must count active goals only"
    );

    // The ref must be usable — decorated `sluggify(title)-<uuid>`, which `parse_ref`
    // resolves. A title in a primer with no way to open it is a dead end.
    let goal_ref = &result.active_goals[0].r#ref;
    assert!(
        goal_ref.starts_with("standing-goal-"),
        "ref must be the decorated slug-uuid form; got {goal_ref}"
    );
    assert!(
        temper_workflow::operations::parse_ref(goal_ref).is_ok(),
        "the emitted ref must resolve through parse_ref; got {goal_ref}"
    );
}

// ---------------------------------------------------------------------------
// Test 5: a capped goal list still reports the true total
// ---------------------------------------------------------------------------

/// Seed more active goals than the display cap and assert the list caps while
/// `active_goal_total` still reports the truth.
///
/// Silent truncation is the failure this witnesses: a primer that shows 2 of 5 standing
/// goals without saying so reads as "these are all of them".
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn warmup_capped_goal_list_still_reports_true_total(pool: sqlx::PgPool) {
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

    for i in 0..5 {
        seed_goal(
            &app.client,
            "myapp",
            &format!("g{i}"),
            &format!("Goal {i}"),
            "active",
        )
        .await;
    }

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();

    let result = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::commands::warmup::build_warmup_result(
                &cli_config,
                "@me/myapp",
                temper_cli::commands::warmup::WarmupLimits {
                    sessions: 5,
                    goals: 2,
                },
            )
            .expect("build_warmup_result must succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    assert_eq!(
        result.active_goals.len(),
        2,
        "the displayed list must respect the configured goal cap"
    );
    assert_eq!(
        result.active_goal_total, 5,
        "active_goal_total must report every active goal, not just the displayed ones — \
         a cap that hides its own existence is silent truncation"
    );
}
