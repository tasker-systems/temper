#![cfg(feature = "test-db")]

//! E2E test: `temper resource show` falls back to the API when the local
//! vault file is missing in local mode but the resource exists upstream.
//!
//! Verifies the Task 7 contract: API-only resource → local-mode show
//! succeeds via API fallback → no local file written. Recovery to disk
//! remains `temper sync run`'s job.

mod common;

use chrono::{Duration, Utc};
use temper_client::auth::{Provider, StoredAuth};

/// Same disk-auth helper used by `publish_tail_test.rs`. Cloud mode reads
/// `TEMPER_TOKEN`; local mode uses the disk store at `TEMPER_AUTH_PATH`.
fn write_auth_json(path: &std::path::Path, jwt: &str) {
    let auth = StoredAuth {
        provider: Provider::Auth0 {
            domain: "test".to_string(),
        },
        access_token: jwt.to_string().into(),
        refresh_token: None,
        expires_at: Utc::now() + Duration::hours(1),
        profile_id: None,
        device_id: Some("e2e-show-fallback-device".to_string()),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create auth dir");
    }
    let bytes = serde_json::to_vec(&auth).expect("serialize StoredAuth");
    std::fs::write(path, bytes).expect("write auth.json");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_show_falls_back_to_api_when_local_missing(pool: sqlx::PgPool) {
    use temper_core::types::ingest::{pack_chunks, IngestPayload};

    let app = common::setup(pool.clone()).await;

    // Auto-provision the user profile + create the "myapp" context.
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("myapp")
        .await
        .expect("create myapp context");

    // Seed: create a task resource API-only — no local file ever written.
    let api_body = "# Server-side body\n\nThis exists upstream only.\n";
    let body_hash = temper_core::hash::compute_body_hash(api_body);
    let payload = IngestPayload {
        title: "Server Only Task".to_string(),
        origin_uri: "kb://myapp/task/server-only-task".to_string(),
        context_name: "myapp".to_string(),
        doc_type_name: "task".to_string(),
        content_hash: Some(body_hash),
        slug: "server-only-task".to_string(),
        content: api_body.to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({
            "temper-title": "Server Only Task",
        })),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };
    app.client
        .ingest()
        .create(&payload)
        .await
        .expect("seed task via /api/ingest");

    // Create the context directory so `resolve_context_with_fallback`
    // accepts "myapp" rather than silently falling back to "default".
    // The task subdirectory is intentionally NOT created — that's how we
    // simulate "context exists locally, but this resource has no file".
    let owner = app.cli_config.owner_for_context("myapp");
    let context_dir = app.vault_dir.path().join(&owner).join("myapp");
    std::fs::create_dir_all(&context_dir).expect("create myapp context dir");

    // Pre-condition: the canonical local path must not exist.
    let me_path = context_dir.join("task").join("server-only-task.md");
    assert!(
        !me_path.exists(),
        "test setup: local file should not exist before show; got: {}",
        me_path.display()
    );

    // Wire local-mode env so the CLI reads TEMPER_API_URL + the disk auth,
    // and load_cloud_config picks up our test API URL.
    let auth_path = app.vault_dir.path().join(".temper/auth.json");
    write_auth_json(&auth_path, &app.token);
    let global_config = app.vault_dir.path().join("no-such-config.toml");

    let api_url = format!("http://{}", app.addr);
    let auth_path_string = auth_path.to_str().unwrap().to_string();
    let global_config_string = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();
    let me_path_for_assert = me_path.clone();

    // Drive local-mode `temper resource show` on a blocking thread. The
    // CLI builds an inner tokio runtime via `runtime::with_client`, so
    // we can't nest. Capture stdout via `gag::BufferRedirect` … but we
    // don't have gag. Instead, use the JSON format and assert via the
    // returned Result + a hook: `show` returns `Result<()>` and prints
    // to stdout. We can't easily intercept stdout from a child thread,
    // so this test focuses on the CONTRACT: show succeeds (Ok) and no
    // file is written. Body-content assertion is covered by the unit
    // test in `commands::resource` and the live API round-trip in
    // `cloud_writes_test::cloud_create_session_round_trip_via_show`.
    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(
            [
                ("TEMPER_API_URL", Some(api_url.as_str())),
                ("TEMPER_AUTH_PATH", Some(auth_path_string.as_str())),
                ("TEMPER_GLOBAL_CONFIG", Some(global_config_string.as_str())),
                ("TEMPER_VAULT_STATE", Some("local")),
                ("TEMPER_TOKEN", None),
            ],
            || {
                temper_cli::commands::resource::show(
                    &cli_config,
                    "task",
                    "server-only-task",
                    Some("myapp"),
                    "text",
                    false, // edges
                )
                .expect("local-mode show should succeed via API fallback");
            },
        );

        // The fallback must not materialize the file locally — recovery
        // to disk is `temper sync run`'s job.
        assert!(
            !me_path_for_assert.exists(),
            "API fallback should not write to vault; got file at {}",
            me_path_for_assert.display()
        );
    })
    .await
    .expect("spawn_blocking joined");
}
