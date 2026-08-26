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
        idempotency_key: None,
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
        idempotency_key: None,
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

// ---------------------------------------------------------------------------
// Test 6-8: the `pending` block — what is waiting on you
// ---------------------------------------------------------------------------

/// Run `build_warmup_result` against `@me/myapp` through the same env dance every test above uses.
async fn build_for(
    app: &common::E2eTestApp,
    context: &str,
) -> temper_cli::commands::warmup::WarmupResult {
    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();
    let context = context.to_string();

    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::commands::warmup::build_warmup_result(
                &cli_config,
                &context,
                default_limits(),
            )
            .expect("build_warmup_result must succeed in cloud mode")
        })
    })
    .await
    .expect("spawn_blocking joined")
}

/// Seed a pending invitation to `email` from a team `inviter` owns.
///
/// Seeded directly rather than through `POST /teams/{id}/invite` because the thing under test is
/// warmup's READ. What the read actually turns on is `list_for_profile`'s email correlation — the
/// address must be verified and resolve to exactly one profile — and that predicate runs identically
/// whichever way the row arrived.
async fn seed_invitation(pool: &sqlx::PgPool, inviter: uuid::Uuid, email: &str) {
    let team_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('acme-eng', 'Acme Engineering') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("create the inviting team");

    sqlx::query(
        "INSERT INTO kb_team_invitations
             (id, team_id, invited_email, invited_by_profile_id, role, token)
         VALUES (uuid_generate_v7(), $1, $2, $3, 'member', $4)",
    )
    .bind(team_id)
    .bind(email)
    .bind(inviter)
    .bind(format!("tok-{}", uuid::Uuid::now_v7()))
    .execute(pool)
    .await
    .expect("seed the invitation");
}

/// **Before/after on ONE principal**, so the only thing that changes between the two reads is the
/// fact under test. Two principals would also have differed in whatever else distinguishes them.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn warmup_pending_counts_a_waiting_invitation(pool: sqlx::PgPool) {
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

    let before = build_for(&app, "@me/myapp").await;
    assert_eq!(
        before
            .pending
            .expect("the pending block must be readable")
            .invitations,
        0,
        "nothing is waiting yet"
    );

    let inviter = common::provision_and_approve_second(&app).await;
    seed_invitation(&pool, inviter, "e2e@test.example.com").await;

    let after = build_for(&app, "@me/myapp").await;
    assert_eq!(
        after.pending.expect("still readable").invitations,
        1,
        "the invitation surfaces in the primer without anyone knowing to ask for it"
    );
}

/// **`None` is not `Some(0)`.** Before: not an instance admin, so the operator counts are absent —
/// nothing was read. After: an admin who read an empty queue — which is a different fact, and the
/// only one of the two that says anything about the queue.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn warmup_pending_tells_not_an_admin_apart_from_an_empty_queue(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let me = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("myapp", None)
        .await
        .expect("create myapp context");

    let before = build_for(&app, "@me/myapp")
        .await
        .pending
        .expect("readable");
    assert_eq!(
        before.join_requests, None,
        "a non-admin read nothing, and must not be told the queue is empty"
    );
    assert_eq!(before.review_requests, None);

    common::make_system_admin(&pool, me.id).await;

    let after = build_for(&app, "@me/myapp")
        .await
        .pending
        .expect("readable");
    assert_eq!(
        after.join_requests,
        Some(0),
        "an admin reading an empty queue is 0 — the server answered"
    );
    assert_eq!(after.review_requests, Some(0));
}

/// **The trap, through the real binary.** Someone invited to a team so they can work in that team's
/// context asks for that context and is refused — because the invitation they have not accepted is
/// what would grant it.
///
/// This drives `target/debug/temper` rather than calling in-process, because the thing under test IS
/// the stderr of a failed process: exit status and stream are the assertion. The non-zero exit is
/// asserted deliberately, so that a later change cannot quietly soften warmup's context contract and
/// still pass.
///
/// NOTE: `cargo nextest run -p temper-e2e` builds this crate and temper-cli's *lib*, not the
/// *binary* this execs. Run `cargo build -p temper-cli` first or you will test a stale binary.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn warmup_says_what_is_waiting_without_blaming_it_for_the_context(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    // Silence first: the same failing command, with nothing waiting, must say nothing extra.
    let quiet = common::run_temper_cli(&app, &["warmup", "--context", "@me/nope"])
        .await
        .expect("cli ran");
    assert!(!quiet.status.success(), "an unreadable context still fails");
    let quiet_err = String::from_utf8_lossy(&quiet.stderr);
    assert!(
        !quiet_err.contains("temper invitations"),
        "a hint on every failed warmup is noise: {quiet_err}"
    );

    let inviter = common::provision_and_approve_second(&app).await;
    seed_invitation(&pool, inviter, "e2e@test.example.com").await;

    let out = common::run_temper_cli(&app, &["warmup", "--context", "@me/nope"])
        .await
        .expect("cli ran");

    assert!(
        !out.status.success(),
        "the command must STILL fail — the hint does not soften the contract"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("1 team invitation waiting on you"),
        "the hint names what is waiting: {stderr}"
    );
    assert!(
        stderr.contains("temper invitations"),
        "and points at the one command that needs no context: {stderr}"
    );

    // --- and NOT a cause, because none was established ---
    //
    // `@me/nope` is a personal context. The invitation seeded above is to `+acme-eng`, and no team
    // invitation can grant an `@me/...` context — there is no arrangement of accepting it that
    // makes this command succeed. The shipped hint said "Accepting one may be what grants you the
    // context this command could not read" here anyway, because it fired on *any* context failure
    // whenever *any* invitation existed. The hedge was carrying cases where the answer is flatly
    // no, at the moment the reader has least context to doubt it.
    assert!(
        !stderr.contains("grants you the context"),
        "no invitation was shown to grant this context, so none may be blamed for it: {stderr}"
    );
    assert!(
        !stderr.contains("may be"),
        "and a hedge is not a substitute for checking: {stderr}"
    );
}

/// **The newcomer — the population this whole feature exists for — driven end to end.**
///
/// `handlers::invitations::list_mine` is mounted in `auth_only_routes()`; the two operator queues
/// are in `gated_routes()` behind `require_system_access`. A principal who has signed in but holds
/// no approved standing therefore reads their own invitations fine and gets
/// `403 SYSTEM_ACCESS_REQUIRED` from both queues — a THIRD `403` arm, distinct from `Forbidden` and
/// `ForbiddenDetail` and checked before either.
///
/// Propagating that arm collapsed `pending` to `null`, which silenced the hint — so the invited
/// newcomer who cannot yet read the team's context got no count and no way out. The trap the hint
/// was written to close, closed against precisely the person it was for.
///
/// Every other test in this file runs as `common::setup`'s already-approved principal, which is why
/// none of them could see this. Standing is stripped here deliberately, and the assertion is the
/// stderr of a real failed process.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn warmup_hints_the_newcomer_who_has_no_system_access_yet(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let me = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let inviter = common::provision_and_approve_second(&app).await;
    seed_invitation(&pool, inviter, "e2e@test.example.com").await;

    // Strip system access: signed in and provisioned, but not yet admitted — the state an invitee
    // is in before anything has been accepted on their behalf.
    sqlx::query("DELETE FROM kb_principal_standing WHERE profile_id = $1")
        .bind(me.id)
        .execute(&pool)
        .await
        .expect("strip standing");

    let out = common::run_temper_cli(&app, &["warmup", "--context", "+acme-eng/work"])
        .await
        .expect("cli ran");

    assert!(
        !out.status.success(),
        "the context is still unreadable, so the command still fails"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("1 team invitation waiting on you"),
        "the newcomer must be told what is waiting: {stderr}"
    );
    assert!(
        stderr.contains("temper invitations"),
        "and pointed at the command that needs no context: {stderr}"
    );

    // **Here the cause IS established, so it is stated flatly.** The seeded invitation is to
    // `acme-eng` and the requested context is `+acme-eng/work` — the very team that owns it. The
    // server was asked that question by slug and answered yes, so the hint names the invitation
    // instead of gesturing at "one" of an unnamed set, and drops the hedge it never earned.
    assert!(
        stderr.contains("Accepting your invitation to +acme-eng grants you the context"),
        "an established cause is named definitely: {stderr}"
    );
    assert!(
        !stderr.contains("may be"),
        "a checked relation needs no hedge: {stderr}"
    );
    // And SILENTLY, which is the separate half. A 403 on an operator queue is a refusal — "not
    // yours to see" — not a failure to read, so it must not surface as a warning. Without the
    // `SystemAccessRequired` arm in `admin_count` this still yields the hint (the per-field
    // degradation rescues it), but every newcomer's every session start also carries
    // "could not read the join request queue: system access required". That is the noise the
    // arm exists to prevent, and this assertion is what witnesses it.
    assert!(
        !stderr.contains("could not read the"),
        "a refusal must not be reported as a failed read: {stderr}"
    );
}

/// **What crosses the wire, asserted on the wire.**
///
/// The primer used to obtain `"invitations": n` by calling `GET /api/invitations/mine`, receiving
/// every row — team, role, expiry, and each invitation's redemption `token` — and keeping only
/// `.len()`. The token is a bearer capability. It is legitimately the caller's to hold, which is
/// exactly why the leak is easy to wave through: nothing is disclosed to a stranger. What is wrong
/// is that credential material moved at all, on a command the `SessionStart` hook runs at the
/// start of every session on every machine, to produce an integer.
///
/// So the claim to witness is not "the CLI does not print the token" and not "the CLI does not
/// store it" — the CLI never did either. It is that **the token was never transferred**, and the
/// only honest way to see that is to watch the socket. `setup_recording` records every path the
/// server is asked for, and the assertion that matters is a NEGATIVE one: the token-bearing route
/// is not among them. An absent request leaves no trace in a response, a database, or a process's
/// output, so no other observable could carry this.
///
/// The same holds for the two operator queues, whose rows carry other principals' handles,
/// display names, emails and messages.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn warmup_counts_without_fetching_what_it_counts(pool: sqlx::PgPool) {
    let (app, requests) = common::setup_recording(pool.clone()).await;
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

    let inviter = common::provision_and_approve_second(&app).await;
    seed_invitation(&pool, inviter, "e2e@test.example.com").await;

    let out = common::run_temper_cli(&app, &["warmup", "--context", "@me/myapp"])
        .await
        .expect("cli ran");
    assert!(
        out.status.success(),
        "the primer must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let paths = requests.lock().expect("request log").clone();
    let asked = |p: &str| paths.iter().any(|seen| seen == p);

    // It counted.
    assert!(
        asked("/api/invitations/mine/count"),
        "the primer must read the count route: {paths:?}"
    );
    assert!(
        asked("/api/access/admin/requests/count"),
        "and the operator queues by count too: {paths:?}"
    );
    assert!(asked("/api/access/admin/reviews/count"), "{paths:?}");

    // And it did not fetch what it counted. These three are the whole point.
    assert!(
        !asked("/api/invitations/mine"),
        "no invitation token may cross the wire to produce a count: {paths:?}"
    );
    assert!(
        !asked("/api/access/admin/requests"),
        "nor another principal's identity: {paths:?}"
    );
    assert!(!asked("/api/access/admin/reviews"), "{paths:?}");

    // Belt and braces, one level lower: the count response body itself carries no token. The
    // fixture's tokens are `tok-<uuid>`, so a leak would be visible as plain text.
    let body = app
        .reqwest_client
        .get(app.url("/api/invitations/mine/count"))
        .bearer_auth(&app.token)
        .send()
        .await
        .expect("count request")
        .text()
        .await
        .expect("count body");
    assert!(
        !body.contains("tok-") && !body.contains("token"),
        "the count answers with integers, not with capabilities: {body}"
    );
}
