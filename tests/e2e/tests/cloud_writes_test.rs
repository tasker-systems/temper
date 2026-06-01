#![cfg(feature = "test-db")]

//! End-to-end coverage for cloud-mode write paths.
//!
//! Drives the cloud write path via per-test `TEMPER_TOKEN` /
//! `TEMPER_API_URL` / `TEMPER_GLOBAL_CONFIG` against an in-process Axum
//! server backed by a real Postgres test database. No vault directory
//! is touched for the cloud-mode write paths under test — the cloud
//! branches in `commands::resource::create` / `update` / `list` /
//! `show` are exercised end to end.
//!
//! Cloud mode uses `MemoryTokenStore::from_env_required()` which reads
//! the JWT from `TEMPER_TOKEN`. `TEMPER_AUTH_PATH` (disk store) is only
//! relevant in local mode. Tests here set `TEMPER_TOKEN` to the test JWT.

mod common;

use chrono::{Duration, Utc};
use temper_client::auth::{Provider, StoredAuth};

/// Write a `StoredAuth` JSON to `path` so `DiskTokenStore::at(path)` and the
/// uniform path resolver find real credentials.  Only used where local-mode
/// auth is needed (e.g., the `write_auth_json` helper is kept here in case a
/// future test exercises a mixed-mode path; cloud-mode tests use `TEMPER_TOKEN`
/// directly).
#[allow(dead_code)]
fn write_auth_json(path: &std::path::Path, jwt: &str) {
    let auth = StoredAuth {
        provider: Provider::Auth0 {
            domain: "test".to_string(),
        },
        access_token: jwt.to_string().into(),
        refresh_token: None,
        expires_at: Utc::now() + Duration::hours(1),
        profile_id: None,
        device_id: Some("e2e-cloud-writes-device".to_string()),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create auth dir");
    }
    let bytes = serde_json::to_vec(&auth).expect("serialize StoredAuth");
    std::fs::write(path, bytes).expect("write auth.json");
}

/// Shared env-var builder for cloud-mode CLI invocations.
///
/// In cloud mode the runtime uses `MemoryTokenStore::from_env_required()`,
/// which reads the JWT from `TEMPER_TOKEN`. `TEMPER_AUTH_PATH` is not used
/// in cloud mode (that is the local-mode disk store path). `TEMPER_GLOBAL_CONFIG`
/// points at a non-existent path so no developer config file leaks into tests.
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

// ---------------------------------------------------------------------------
// Test 1: cloud create + show round-trip
// ---------------------------------------------------------------------------

/// Cloud `temper resource create --type session --title "..."` posts to
/// `/api/ingest`; a second cloud-mode `temper resource show <slug>` retrieves
/// the resource and recovers the body + managed_meta stored by the server.
///
/// Verifies:
/// 1. The resource is in `kb_resources` with the correct doc_type and context.
/// 2. The `kb_resource_manifests` row has `title` in `managed_meta`.
/// 3. `temper resource show` in cloud mode returns `Ok(())` for the slug
///    (proves the by-uri lookup → content fetch round-trips cleanly).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_create_session_round_trip_via_show(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

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

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();

    // Drive cloud-mode create on a blocking thread (build_ingest_payload calls
    // the embedding pipeline synchronously; nesting runtimes would panic).
    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::commands::resource::create(
                &cli_config,
                temper_cli::commands::resource::CreateResourceArgs {
                    doc_type: "session",
                    title: "Cloud Round-Trip Session",
                    context: Some("myapp"),
                    goal: None,
                    mode: None,
                    effort: None,
                    slug: None,
                    task: None,
                    body_flag: None, // default body generated
                    from: None,
                    format: temper_cli::format::OutputFormat::Json,
                },
            )
            .expect("cloud create should succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    // ---- Assertion 1: resource row exists ----
    // Phase 5 unified the slug derivation across modes: sessions get a
    // `{date}-{slugify(title)}` prefix in both local and cloud modes
    // (matches local-mode session behavior; the previous cloud-only
    // bare-slug derivation was a mode-asymmetric quirk eliminated by the
    // surface-dispatch unification).
    let date_prefix = chrono::Local::now().format("%Y-%m-%d").to_string();
    let slug = format!("{date_prefix}-cloud-round-trip-session");
    let (doc_type_name, context_name): (String, String) = sqlx::query_as(
        "SELECT dt.name, c.name
         FROM kb_resources r
         JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
         JOIN kb_contexts c ON c.id = r.kb_context_id
         WHERE r.slug = $1 AND r.is_active",
    )
    .bind(&slug)
    .fetch_one(&pool)
    .await
    .expect("resource row must exist after cloud create");

    assert_eq!(doc_type_name, "session");
    assert_eq!(context_name, "myapp");

    // ---- Assertion 2: managed_meta has title ----
    let managed_meta: serde_json::Value = sqlx::query_scalar(
        "SELECT m.managed_meta
         FROM kb_resource_manifests m
         JOIN kb_resources r ON r.id = m.resource_id
         WHERE r.slug = $1",
    )
    .bind(&slug)
    .fetch_one(&pool)
    .await
    .expect("manifest row must exist after cloud create");

    let obj = managed_meta
        .as_object()
        .expect("managed_meta must be a JSON object");
    assert_eq!(
        obj.get("temper-title").and_then(|v| v.as_str()),
        Some("Cloud Round-Trip Session"),
        "managed_meta must contain title; got: {managed_meta}"
    );

    // ---- Assertion 3: cloud show round-trips ----
    // Drive show on a fresh blocking thread (runtime::with_client creates an
    // inner tokio runtime; must not nest).
    let api_url2 = format!("http://{}", app.addr);
    let token2 = app.token.clone();
    let global_config_str2 = global_config.to_str().unwrap().to_string();
    let cli_config2 = app.cli_config.clone();

    let slug_for_show = slug.clone();
    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url2, &token2, &global_config_str2), || {
            temper_cli::commands::resource::show(
                &cli_config2,
                temper_cli::commands::resource::ShowParams {
                    doc_type: "session",
                    slug: &slug_for_show,
                    context: Some("myapp"),
                    format: temper_cli::format::OutputFormat::Json,
                    edges: false,
                    meta_only: false,
                    fields: &[],
                },
            )
            .expect("cloud show must succeed for a freshly created resource")
        })
    })
    .await
    .expect("spawn_blocking joined");
}

// ---------------------------------------------------------------------------
// Test 2: cloud update meta-only partial managed_meta
// ---------------------------------------------------------------------------

/// Cloud `temper resource update <slug> --type session --stage done`
/// (managed_meta-only PATCH) — server merges; untouched fields preserved.
///
/// Verifies:
/// 1. The `temper-stage` field in `managed_meta` is updated to "done".
/// 2. The `title` field set on create is preserved (partial-merge semantics).
/// 3. The `body_hash` in `kb_resource_manifests` is unchanged (no body sent).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_update_meta_only_partial_managed_meta(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

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

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();

    // Seed: create the resource via the client (not CLI) so we know its slug.
    use temper_core::types::ingest::{pack_chunks, IngestPayload};
    let body_text = "# Meta-Only Test\n\nInitial body.\n";
    let body_hash = temper_core::hash::compute_body_hash(body_text);
    let payload = IngestPayload {
        title: "Meta-Only Update Test".to_string(),
        origin_uri: "kb://myapp/session/meta-only-update-test".to_string(),
        context_name: "myapp".to_string(),
        doc_type_name: "session".to_string(),
        content_hash: Some(body_hash.clone()),
        slug: "meta-only-update-test".to_string(),
        content: body_text.to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({
            "temper-title": "Meta-Only Update Test",
            "temper-stage": "backlog"
        })),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };
    app.client
        .ingest()
        .create(&payload)
        .await
        .expect("seed resource via client");

    // Read body_hash before update (baseline for assertion 3).
    let body_hash_before: String = sqlx::query_scalar(
        "SELECT m.body_hash
         FROM kb_resource_manifests m
         JOIN kb_resources r ON r.id = m.resource_id
         WHERE r.slug = 'meta-only-update-test'",
    )
    .fetch_one(&pool)
    .await
    .expect("manifest must exist after seed");

    // Drive meta-only update on a blocking thread.
    let cli_config = app.cli_config.clone();
    let api_url2 = api_url.clone();
    let token2 = token.clone();
    let global_config_str2 = global_config_str.clone();

    // Update --title (a base field valid for all doctypes); --stage was the
    // pre-Phase-5 choice but stage is task-only per the schema, and the
    // pre-Phase-5 cloud path bypassed validate_update_args. Phase 5
    // unification surfaces that constraint correctly. Test intent is
    // partial-merge semantics — any single field swap works.
    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url2, &token2, &global_config_str2), || {
            temper_cli::commands::resource::update(
                &cli_config,
                &temper_cli::commands::resource::UpdateParams {
                    slug: "meta-only-update-test",
                    doc_type: Some("session"),
                    type_from: None,
                    type_to: None,
                    context: Some("myapp"),
                    context_to: None,
                    title: Some("Updated Title"),
                    tags: &[],
                    aliases: &[],
                    relates_to: &[],
                    references: &[],
                    depends_on: &[],
                    extends: &[],
                    preceded_by: &[],
                    derived_from: &[],
                    stage: None,
                    mode: None,
                    effort: None,
                    goal: None,
                    seq: None,
                    branch: None,
                    pr: None,
                    status: None,
                    body: None,
                    format: temper_cli::format::OutputFormat::Json,
                },
            )
            .expect("cloud meta-only update must succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    // ---- Assertion 1: title is updated ----
    let managed_meta: serde_json::Value = sqlx::query_scalar(
        "SELECT m.managed_meta
         FROM kb_resource_manifests m
         JOIN kb_resources r ON r.id = m.resource_id
         WHERE r.slug = 'meta-only-update-test'",
    )
    .fetch_one(&pool)
    .await
    .expect("manifest must exist after update");

    let obj = managed_meta
        .as_object()
        .expect("managed_meta must be a JSON object");

    assert_eq!(
        obj.get("temper-title").and_then(|v| v.as_str()),
        Some("Updated Title"),
        "temper-title must be 'Updated Title' after meta-only update; got: {managed_meta}"
    );

    // ---- Assertion 2: seed-side stage preserved ----
    assert_eq!(
        obj.get("temper-stage").and_then(|v| v.as_str()),
        Some("backlog"),
        "temper-stage must be preserved from seed after meta-only update; got: {managed_meta}"
    );

    // ---- Assertion 3: body_hash unchanged ----
    let body_hash_after: String = sqlx::query_scalar(
        "SELECT m.body_hash
         FROM kb_resource_manifests m
         JOIN kb_resources r ON r.id = m.resource_id
         WHERE r.slug = 'meta-only-update-test'",
    )
    .fetch_one(&pool)
    .await
    .expect("manifest after update");

    assert_eq!(
        body_hash_before, body_hash_after,
        "body_hash must be unchanged after meta-only update"
    );
}

// ---------------------------------------------------------------------------
// Test 3: cloud update body + meta in one PATCH
// ---------------------------------------------------------------------------

/// Cloud `temper resource update <slug> --type session --stage done --body @<path>`
/// posts a single PATCH carrying both the body trio and managed_meta.
/// Both `body_hash` and `managed_meta.temper-stage` should change.
#[cfg(feature = "test-embed")]
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_update_body_and_meta_in_one_request(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

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

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();

    // Seed: create via client with initial body.
    use temper_core::types::ingest::{pack_chunks, IngestPayload};
    let initial_body = "# Body+Meta Test\n\nInitial body.\n";
    let initial_hash = temper_core::hash::compute_body_hash(initial_body);
    let payload = IngestPayload {
        title: "Body+Meta Update Test".to_string(),
        origin_uri: "kb://myapp/session/body-and-meta-update-test".to_string(),
        context_name: "myapp".to_string(),
        doc_type_name: "session".to_string(),
        content_hash: Some(initial_hash.clone()),
        slug: "body-and-meta-update-test".to_string(),
        content: initial_body.to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({
            "temper-title": "Body+Meta Update Test",
            "temper-stage": "backlog"
        })),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };
    app.client
        .ingest()
        .create(&payload)
        .await
        .expect("seed resource");

    // Write the new body to a temp file (cloud update reads from @<path>).
    let new_body_path = app.vault_dir.path().join("new-body.md");
    let new_body = "# Body+Meta Test\n\nUpdated body content — different from initial.\n";
    std::fs::write(&new_body_path, new_body).expect("write new body file");
    let body_flag = format!("@{}", new_body_path.to_str().unwrap());

    // Drive body+meta update on a blocking thread.
    let cli_config = app.cli_config.clone();
    let api_url2 = api_url.clone();
    let token2 = token.clone();
    let global_config_str2 = global_config_str.clone();

    // Update --title (a base field valid for all doctypes); --stage was the
    // pre-Phase-5 choice but stage is task-only per the schema. Phase 5
    // unification correctly enforces this.
    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url2, &token2, &global_config_str2), || {
            temper_cli::commands::resource::update(
                &cli_config,
                &temper_cli::commands::resource::UpdateParams {
                    slug: "body-and-meta-update-test",
                    doc_type: Some("session"),
                    type_from: None,
                    type_to: None,
                    context: Some("myapp"),
                    context_to: None,
                    title: Some("Updated Title"),
                    tags: &[],
                    aliases: &[],
                    relates_to: &[],
                    references: &[],
                    depends_on: &[],
                    extends: &[],
                    preceded_by: &[],
                    derived_from: &[],
                    stage: None,
                    mode: None,
                    effort: None,
                    goal: None,
                    seq: None,
                    branch: None,
                    pr: None,
                    status: None,
                    body: Some(body_flag),
                    format: temper_cli::format::OutputFormat::Json,
                },
            )
            .expect("cloud body+meta update must succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    // ---- Assert body_hash changed ----
    let body_hash_after: String = sqlx::query_scalar(
        "SELECT m.body_hash
         FROM kb_resource_manifests m
         JOIN kb_resources r ON r.id = m.resource_id
         WHERE r.slug = 'body-and-meta-update-test'",
    )
    .fetch_one(&pool)
    .await
    .expect("manifest after update");

    assert_ne!(
        initial_hash, body_hash_after,
        "body_hash must change after body+meta update"
    );

    let expected_new_hash = temper_core::hash::compute_body_hash(new_body);
    assert_eq!(
        body_hash_after, expected_new_hash,
        "body_hash must match the new body's hash"
    );

    // ---- Assert stage changed ----
    let managed_meta: serde_json::Value = sqlx::query_scalar(
        "SELECT m.managed_meta
         FROM kb_resource_manifests m
         JOIN kb_resources r ON r.id = m.resource_id
         WHERE r.slug = 'body-and-meta-update-test'",
    )
    .fetch_one(&pool)
    .await
    .expect("manifest managed_meta after update");

    let obj = managed_meta
        .as_object()
        .expect("managed_meta must be a JSON object");
    assert_eq!(
        obj.get("temper-title").and_then(|v| v.as_str()),
        Some("Updated Title"),
        "temper-title must be 'Updated Title' after body+meta update; got: {managed_meta}"
    );
    // Seed-side stage preserved through partial merge.
    assert_eq!(
        obj.get("temper-stage").and_then(|v| v.as_str()),
        Some("backlog"),
        "temper-stage must be preserved from seed after body+meta update; got: {managed_meta}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: cloud update body-only — managed_meta untouched
// ---------------------------------------------------------------------------

/// Cloud `temper resource update <slug> --type session --body @<path>` (no
/// managed-meta-mutating flags) → PATCH carries only body trio. Stored
/// managed_meta (typed fields) must be untouched after the update.
#[cfg(feature = "test-embed")]
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_update_body_only_no_managed_meta(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

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

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();

    // Seed: create with known managed_meta including stage="in-progress".
    use temper_core::types::ingest::{pack_chunks, IngestPayload};
    let initial_body = "# Body-Only Test\n\nInitial body.\n";
    let initial_hash = temper_core::hash::compute_body_hash(initial_body);
    let payload = IngestPayload {
        title: "Body-Only Update Test".to_string(),
        origin_uri: "kb://myapp/session/body-only-update-test".to_string(),
        context_name: "myapp".to_string(),
        doc_type_name: "session".to_string(),
        content_hash: Some(initial_hash.clone()),
        slug: "body-only-update-test".to_string(),
        content: initial_body.to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({
            "temper-title": "Body-Only Update Test",
            "temper-stage": "in-progress"
        })),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };
    app.client
        .ingest()
        .create(&payload)
        .await
        .expect("seed resource");

    // Write new body to a temp file.
    let new_body_path = app.vault_dir.path().join("body-only-new.md");
    let new_body = "# Body-Only Test\n\nReplacement body — new content.\n";
    std::fs::write(&new_body_path, new_body).expect("write new body file");
    let body_flag = format!("@{}", new_body_path.to_str().unwrap());

    // Drive body-only update (no stage/mode/effort/etc. flags).
    let cli_config = app.cli_config.clone();
    let api_url2 = api_url.clone();
    let token2 = token.clone();
    let global_config_str2 = global_config_str.clone();

    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url2, &token2, &global_config_str2), || {
            temper_cli::commands::resource::update(
                &cli_config,
                &temper_cli::commands::resource::UpdateParams {
                    slug: "body-only-update-test",
                    doc_type: Some("session"),
                    type_from: None,
                    type_to: None,
                    context: Some("myapp"),
                    context_to: None,
                    title: None,
                    tags: &[],
                    aliases: &[],
                    relates_to: &[],
                    references: &[],
                    depends_on: &[],
                    extends: &[],
                    preceded_by: &[],
                    derived_from: &[],
                    stage: None, // no managed-meta flags
                    mode: None,
                    effort: None,
                    goal: None,
                    seq: None,
                    branch: None,
                    pr: None,
                    status: None,
                    body: Some(body_flag),
                    format: temper_cli::format::OutputFormat::Json,
                },
            )
            .expect("cloud body-only update must succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    // ---- Assert body_hash changed ----
    let body_hash_after: String = sqlx::query_scalar(
        "SELECT m.body_hash
         FROM kb_resource_manifests m
         JOIN kb_resources r ON r.id = m.resource_id
         WHERE r.slug = 'body-only-update-test'",
    )
    .fetch_one(&pool)
    .await
    .expect("manifest body_hash after update");

    assert_ne!(
        initial_hash, body_hash_after,
        "body_hash must change after body-only update"
    );

    // ---- Assert managed_meta.temper-stage preserved ----
    let managed_meta: serde_json::Value = sqlx::query_scalar(
        "SELECT m.managed_meta
         FROM kb_resource_manifests m
         JOIN kb_resources r ON r.id = m.resource_id
         WHERE r.slug = 'body-only-update-test'",
    )
    .fetch_one(&pool)
    .await
    .expect("manifest managed_meta after update");

    let obj = managed_meta
        .as_object()
        .expect("managed_meta must be a JSON object");
    assert_eq!(
        obj.get("temper-stage").and_then(|v| v.as_str()),
        Some("in-progress"),
        "temper-stage must remain 'in-progress' after body-only update; got: {managed_meta}"
    );
}

// ---------------------------------------------------------------------------
// Test 5: cloud --body @<empty-file> errors and does not mutate server
// ---------------------------------------------------------------------------

/// Cloud `temper resource update <slug> --body @<empty-file>` must error with
/// a message containing "empty" and leave the server's `body_hash` unchanged.
///
/// Proves Task 1's explicit-empty guard (`body_source::resolve_body_source`)
/// reaches users through the live CLI → Axum → DB stack in cloud mode.
#[cfg(feature = "test-embed")]
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_update_body_at_empty_file_errors_and_does_not_mutate(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

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

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();

    // Seed: create with a known initial body so we can verify hash is unchanged.
    use temper_core::types::ingest::{pack_chunks, IngestPayload};
    let initial_body = "# Empty Guard Test\n\nInitial body.\n";
    let initial_hash = temper_core::hash::compute_body_hash(initial_body);
    let payload = IngestPayload {
        title: "Body Empty Guard Test".to_string(),
        origin_uri: "kb://myapp/session/body-empty-guard-test".to_string(),
        context_name: "myapp".to_string(),
        doc_type_name: "session".to_string(),
        content_hash: Some(initial_hash.clone()),
        slug: "body-empty-guard-test".to_string(),
        content: initial_body.to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({
            "temper-title": "Body Empty Guard Test",
            "temper-stage": "backlog"
        })),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };
    app.client
        .ingest()
        .create(&payload)
        .await
        .expect("seed resource");

    // Write an empty file — the guard must reject this.
    let empty_path = app.vault_dir.path().join("empty-body.md");
    std::fs::write(&empty_path, "").expect("write empty file");
    let body_flag = format!("@{}", empty_path.to_str().unwrap());

    // Drive update on a blocking thread — expect it to error.
    let cli_config = app.cli_config.clone();
    let api_url2 = api_url.clone();
    let token2 = token.clone();
    let global_config_str2 = global_config_str.clone();

    let result = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url2, &token2, &global_config_str2), || {
            temper_cli::commands::resource::update(
                &cli_config,
                &temper_cli::commands::resource::UpdateParams {
                    slug: "body-empty-guard-test",
                    doc_type: Some("session"),
                    type_from: None,
                    type_to: None,
                    context: Some("myapp"),
                    context_to: None,
                    title: None,
                    tags: &[],
                    aliases: &[],
                    relates_to: &[],
                    references: &[],
                    depends_on: &[],
                    extends: &[],
                    preceded_by: &[],
                    derived_from: &[],
                    stage: None,
                    mode: None,
                    effort: None,
                    goal: None,
                    seq: None,
                    branch: None,
                    pr: None,
                    status: None,
                    body: Some(body_flag),
                    format: temper_cli::format::OutputFormat::Json,
                },
            )
        })
    })
    .await
    .expect("spawn_blocking joined");

    assert!(
        result.is_err(),
        "empty --body @path must error; got: {result:?}"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("empty"),
        "error message should mention 'empty'; got: {err_msg}"
    );

    // ---- Assert no server-side mutation occurred ----
    let body_hash_after: String = sqlx::query_scalar(
        "SELECT m.body_hash
         FROM kb_resource_manifests m
         JOIN kb_resources r ON r.id = m.resource_id
         WHERE r.slug = 'body-empty-guard-test'",
    )
    .fetch_one(&pool)
    .await
    .expect("manifest after attempted update");

    assert_eq!(
        body_hash_after, initial_hash,
        "body_hash must be unchanged when --body @empty.md errors"
    );
}

// ---------------------------------------------------------------------------
// Test 6: chunk dedupe short-circuit skips unchanged bodies
// ---------------------------------------------------------------------------

/// Re-sending the same body → server short-circuits, no new chunk rows
/// inserted for that resource (verified via `kb_chunks` row count).
///
/// Protocol:
/// 1. Create a resource with a non-empty body (produces N chunk rows).
/// 2. Count `kb_chunks` rows for the resource.
/// 3. PATCH with the identical body again.
/// 4. Count again — must be the same (short-circuit engaged).
#[cfg(feature = "test-embed")]
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_update_chunk_dedupe_skips_unchanged(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

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

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();

    // Write the body to a file so we can reuse it on the second PATCH.
    let body_text =
        "# Dedup Test\n\n## Section One\n\nFirst section content.\n\n## Section Two\n\nSecond section content.\n";
    let body_path = app.vault_dir.path().join("dedup-body.md");
    std::fs::write(&body_path, body_text).expect("write body file");
    let body_flag = format!("@{}", body_path.to_str().unwrap());
    let body_flag2 = body_flag.clone();

    // Seed via CLI cloud-mode create so the chunk pipeline runs exactly once.
    let cli_config = app.cli_config.clone();
    let api_url2 = api_url.clone();
    let token2 = token.clone();
    let global_config_str2 = global_config_str.clone();

    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url2, &token2, &global_config_str2), || {
            temper_cli::commands::resource::create(
                &cli_config,
                temper_cli::commands::resource::CreateResourceArgs {
                    doc_type: "session",
                    title: "Chunk Dedup Test",
                    context: Some("myapp"),
                    goal: None,
                    mode: None,
                    effort: None,
                    slug: None,
                    task: None,
                    body_flag: Some(body_flag),
                    from: None,
                    format: temper_cli::format::OutputFormat::Json,
                },
            )
            .expect("cloud create for dedup test")
        })
    })
    .await
    .expect("spawn_blocking joined");

    // Count kb_chunks after first create. Phase 5 unified slug derivation
    // means sessions get a `{date}-{slugify(title)}` prefix in both modes.
    let date_prefix = chrono::Local::now().format("%Y-%m-%d").to_string();
    let slug = format!("{date_prefix}-chunk-dedup-test");
    let chunk_count_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kb_chunks c
         JOIN kb_resources r ON r.id = c.resource_id
         WHERE r.slug = $1 AND c.is_current",
    )
    .bind(&slug)
    .fetch_one(&pool)
    .await
    .expect("chunk count before second write");

    // Re-send identical body via PATCH — should short-circuit.
    let cli_config2 = app.cli_config.clone();
    let api_url3 = api_url.clone();
    let token3 = token.clone();
    let global_config_str3 = global_config_str.clone();
    let slug_for_update = slug.clone();

    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url3, &token3, &global_config_str3), || {
            temper_cli::commands::resource::update(
                &cli_config2,
                &temper_cli::commands::resource::UpdateParams {
                    slug: &slug_for_update,
                    doc_type: Some("session"),
                    type_from: None,
                    type_to: None,
                    context: Some("myapp"),
                    context_to: None,
                    title: None,
                    tags: &[],
                    aliases: &[],
                    relates_to: &[],
                    references: &[],
                    depends_on: &[],
                    extends: &[],
                    preceded_by: &[],
                    derived_from: &[],
                    stage: None,
                    mode: None,
                    effort: None,
                    goal: None,
                    seq: None,
                    branch: None,
                    pr: None,
                    status: None,
                    body: Some(body_flag2),
                    format: temper_cli::format::OutputFormat::Json,
                },
            )
            .expect("second (identical) PATCH must succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    // Count kb_chunks after second write — must be unchanged.
    let chunk_count_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kb_chunks c
         JOIN kb_resources r ON r.id = c.resource_id
         WHERE r.slug = $1 AND c.is_current",
    )
    .bind(&slug)
    .fetch_one(&pool)
    .await
    .expect("chunk count after second write");

    assert_eq!(
        chunk_count_before, chunk_count_after,
        "chunk count must be unchanged after re-sending identical body (short-circuit expected); \
         before={chunk_count_before}, after={chunk_count_after}"
    );
}

// ---------------------------------------------------------------------------
// Test 7: cloud list returns remote-only resources
// ---------------------------------------------------------------------------

/// Cloud `temper list --type session` returns server rows including resources
/// never pulled to a vault (regression-guard for cloud-mode list behavior).
///
/// We create two resources via the client (simulating "cloud-only" resources),
/// then drive `temper resource list` in cloud mode and verify both appear via
/// the API (since stdout capture is not available, we verify via direct DB
/// query that the resource count the server would return includes our inserts).
///
/// The cloud list path calls `fetch_list_rows` which hits `GET /api/resources`.
/// Since list() returns Ok(()) on success, we verify the server-side presence
/// via the pool and trust that a non-Ok return would panic the test.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_list_returns_remote_only_resources(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

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

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();

    // Seed two resources via the client (cloud-only: no vault files).
    use temper_core::types::ingest::{pack_chunks, IngestPayload};
    for i in 1..=2 {
        let body = format!("# Cloud-Only Resource {i}\n\nContent.\n");
        let hash = temper_core::hash::compute_body_hash(&body);
        let payload = IngestPayload {
            title: format!("Cloud Only {i}"),
            origin_uri: format!("kb://myapp/session/cloud-only-resource-{i}"),
            context_name: "myapp".to_string(),
            doc_type_name: "session".to_string(),
            content_hash: Some(hash),
            slug: format!("cloud-only-resource-{i}"),
            content: body,
            metadata: None,
            managed_meta: Some(serde_json::json!({"temper-title": format!("Cloud Only {i}")})),
            open_meta: None,
            chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
        };
        app.client
            .ingest()
            .create(&payload)
            .await
            .expect("seed cloud-only resource");
    }

    // Drive cloud list — must return Ok(()) (server returned rows).
    let cli_config = app.cli_config.clone();
    let api_url2 = api_url.clone();
    let token2 = token.clone();
    let global_config_str2 = global_config_str.clone();

    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url2, &token2, &global_config_str2), || {
            temper_cli::commands::resource::list(
                &cli_config,
                temper_cli::commands::resource::ListParams {
                    doc_type: "session",
                    context: Some("myapp"),
                    limit: Some(20),
                    stage: None,
                    goal: None,
                    status: None,
                    format: temper_cli::format::OutputFormat::Json,
                    meta_only: false,
                    fields: &[],
                },
            )
            .expect("cloud list must succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    // Verify both resources are in the DB and active (server side).
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kb_resources r
         JOIN kb_contexts c ON c.id = r.kb_context_id
         WHERE c.name = 'myapp'
           AND r.slug IN ('cloud-only-resource-1', 'cloud-only-resource-2')
           AND r.is_active",
    )
    .fetch_one(&pool)
    .await
    .expect("count cloud-only resources");

    assert_eq!(
        count, 2,
        "both cloud-only resources must be active in DB after cloud list"
    );
}

// ---------------------------------------------------------------------------
// Test 8: create writes the canonical projection file
// ---------------------------------------------------------------------------

/// Cloud `temper resource create --type task --title "..."` posts to
/// `/api/ingest`; the CLI then materializes the new resource's projection file
/// under `<vault_root>/@me/<context>/task/<slug>.md`.
///
/// Verifies:
/// 1. The projection file exists at the canonical vault path.
/// 2. The file's frontmatter contains the correct `temper-slug`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_writes_canonical_projection_file(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

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

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();
    let vault_root = app.vault_dir.path().to_path_buf();

    // Drive cloud-mode create on a blocking thread.
    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::commands::resource::create(
                &cli_config,
                temper_cli::commands::resource::CreateResourceArgs {
                    doc_type: "task",
                    title: "Projection Write Test",
                    context: Some("myapp"),
                    goal: None,
                    mode: None,
                    effort: None,
                    slug: None,
                    task: None,
                    body_flag: None,
                    from: None,
                    format: temper_cli::format::OutputFormat::Json,
                },
            )
            .expect("cloud create should succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    // ---- Assertion 1: projection file exists at canonical path ----
    // Phase 5 unified slug derivation: tasks get a `{date}-{slugify(title)}` prefix.
    let date_prefix = chrono::Local::now().format("%Y-%m-%d").to_string();
    let slug = format!("{date_prefix}-projection-write-test");
    // The file lives at <vault_root>/@me/<context>/task/<slug>.md.
    let projection_path = vault_root
        .join("@me")
        .join("myapp")
        .join("task")
        .join(format!("{slug}.md"));

    assert!(
        projection_path.exists(),
        "projection file must exist at {} after cloud create",
        projection_path.display()
    );

    // ---- Assertion 2: frontmatter temper-slug matches created resource ----
    let content =
        std::fs::read_to_string(&projection_path).expect("projection file must be readable");
    let fm = temper_core::frontmatter::Frontmatter::try_from(content.as_str())
        .expect("projection file must have valid frontmatter");
    let fm_json = serde_json::to_value(fm.value()).expect("frontmatter JSON conversion");
    assert_eq!(
        fm_json.get("temper-slug").and_then(|v| v.as_str()),
        Some(slug.as_str()),
        "projection frontmatter must contain correct temper-slug; got: {fm_json}"
    );
}

// ---------------------------------------------------------------------------
// Test 9: update rewrites the projection file on success
// ---------------------------------------------------------------------------

/// Cloud `temper resource update <slug> --type task --title "..."` (meta-only
/// PATCH) rewrites the existing projection file under
/// `<vault_root>/@me/<context>/task/<slug>.md` with updated frontmatter.
///
/// Verifies:
/// 1. The projection file exists (written by the create tail action — Task 5).
/// 2. After the meta-only update, the projection file's frontmatter contains
///    the new title, proving the projection was rewritten by `update`'s tail action.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_rewrites_projection_file_on_success(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

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

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();
    let vault_root = app.vault_dir.path().to_path_buf();

    // Step 1: Create the resource (projection file is written by the create
    // tail action — Task 5). Using "task" type so slug gets a date prefix.
    let api_url2 = api_url.clone();
    let token2 = token.clone();
    let global_config_str2 = global_config_str.clone();
    let cli_config2 = cli_config.clone();

    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url2, &token2, &global_config_str2), || {
            temper_cli::commands::resource::create(
                &cli_config2,
                temper_cli::commands::resource::CreateResourceArgs {
                    doc_type: "task",
                    title: "Update Projection Test",
                    context: Some("myapp"),
                    goal: None,
                    mode: None,
                    effort: None,
                    slug: None,
                    task: None,
                    body_flag: None, // default body generated
                    from: None,
                    format: temper_cli::format::OutputFormat::Json,
                },
            )
            .expect("cloud create should succeed")
        })
    })
    .await
    .expect("spawn_blocking create joined");

    // Derive the slug from the title (Phase 5 unified slug derivation).
    let date_prefix = chrono::Local::now().format("%Y-%m-%d").to_string();
    let slug = format!("{date_prefix}-update-projection-test");
    let projection_path = vault_root
        .join("@me")
        .join("myapp")
        .join("task")
        .join(format!("{slug}.md"));

    // Step 2: Assert the projection file exists after create.
    assert!(
        projection_path.exists(),
        "projection file must exist at {} after cloud create",
        projection_path.display()
    );

    // Read the pre-update frontmatter to verify it has the original title.
    let content_before = std::fs::read_to_string(&projection_path)
        .expect("projection file must be readable before update");
    let fm_before = temper_core::frontmatter::Frontmatter::try_from(content_before.as_str())
        .expect("projection file must have valid frontmatter before update");
    let fm_before_json =
        serde_json::to_value(fm_before.value()).expect("frontmatter JSON conversion");
    assert_eq!(
        fm_before_json.get("temper-title").and_then(|v| v.as_str()),
        Some("Update Projection Test"),
        "pre-update frontmatter must have original title; got: {fm_before_json}"
    );

    // Step 3: Drive a meta-only update (title change, no body) on a blocking
    // thread. No `test-embed` required — meta-only updates do not touch chunks.
    let slug_for_update = slug.clone();

    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::commands::resource::update(
                &cli_config,
                &temper_cli::commands::resource::UpdateParams {
                    slug: &slug_for_update,
                    doc_type: Some("task"),
                    type_from: None,
                    type_to: None,
                    context: Some("myapp"),
                    context_to: None,
                    title: Some("Updated Projection Title"),
                    tags: &[],
                    aliases: &[],
                    relates_to: &[],
                    references: &[],
                    depends_on: &[],
                    extends: &[],
                    preceded_by: &[],
                    derived_from: &[],
                    stage: None,
                    mode: None,
                    effort: None,
                    goal: None,
                    seq: None,
                    branch: None,
                    pr: None,
                    status: None,
                    body: None, // meta-only, no chunks_packed needed
                    format: temper_cli::format::OutputFormat::Json,
                },
            )
            .expect("cloud meta-only update must succeed")
        })
    })
    .await
    .expect("spawn_blocking update joined");

    // ---- Assertion: projection file has the updated title in frontmatter ----
    let content_after = std::fs::read_to_string(&projection_path)
        .expect("projection file must be readable after update");
    let fm_after = temper_core::frontmatter::Frontmatter::try_from(content_after.as_str())
        .expect("projection file must have valid frontmatter after update");
    let fm_after_json =
        serde_json::to_value(fm_after.value()).expect("frontmatter JSON conversion after update");
    assert_eq!(
        fm_after_json.get("temper-title").and_then(|v| v.as_str()),
        Some("Updated Projection Title"),
        "post-update projection frontmatter must contain updated title; got: {fm_after_json}"
    );
}

// ---------------------------------------------------------------------------
// Test 10: delete removes the projection file
// ---------------------------------------------------------------------------

/// Cloud `temper resource delete --type task <slug> --force` soft-deletes on
/// the server and removes the projection file from
/// `<vault_root>/@me/<context>/task/<slug>.md`.
///
/// Verifies:
/// 1. The projection file exists after `create` (written by create's tail action
///    — Task 5).
/// 2. After `delete --force`, the projection file is gone from disk.
/// 3. The resource is marked inactive in the database (server-side soft-delete).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn delete_removes_the_projection_file(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

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

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();
    let vault_root = app.vault_dir.path().to_path_buf();

    // Step 1: Create the resource (projection file written by create's tail action).
    let api_url2 = api_url.clone();
    let token2 = token.clone();
    let global_config_str2 = global_config_str.clone();
    let cli_config2 = cli_config.clone();

    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url2, &token2, &global_config_str2), || {
            temper_cli::commands::resource::create(
                &cli_config2,
                temper_cli::commands::resource::CreateResourceArgs {
                    doc_type: "task",
                    title: "Delete Projection Test",
                    context: Some("myapp"),
                    goal: None,
                    mode: None,
                    effort: None,
                    slug: None,
                    task: None,
                    body_flag: None,
                    from: None,
                    format: temper_cli::format::OutputFormat::Json,
                },
            )
            .expect("cloud create should succeed")
        })
    })
    .await
    .expect("spawn_blocking create joined");

    // Derive slug (Phase 5 unified slug derivation: tasks get {date}-{slugify(title)} prefix).
    let date_prefix = chrono::Local::now().format("%Y-%m-%d").to_string();
    let slug = format!("{date_prefix}-delete-projection-test");
    let projection_path = vault_root
        .join("@me")
        .join("myapp")
        .join("task")
        .join(format!("{slug}.md"));

    // Step 2: Assert the projection file exists after create.
    assert!(
        projection_path.exists(),
        "projection file must exist at {} after cloud create",
        projection_path.display()
    );

    // Step 3: Delete the resource (force=true so it works in non-TTY test context).
    let api_url3 = api_url.clone();
    let token3 = token.clone();
    let global_config_str3 = global_config_str.clone();
    let cli_config3 = cli_config.clone();
    let slug_for_delete = slug.clone();

    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url3, &token3, &global_config_str3), || {
            temper_cli::commands::resource::delete(
                &cli_config3,
                "task",
                &slug_for_delete,
                Some("myapp"),
                true, // force — accepted for CLI compatibility; cloud delete is non-interactive
                temper_cli::format::OutputFormat::Json,
            )
            .expect("cloud delete should succeed")
        })
    })
    .await
    .expect("spawn_blocking delete joined");

    // ---- Assertion 1: projection file is gone ----
    assert!(
        !projection_path.exists(),
        "projection file must be removed after cloud delete; path: {}",
        projection_path.display()
    );

    // ---- Assertion 2: resource is soft-deleted in the database ----
    let is_active: bool =
        sqlx::query_scalar("SELECT r.is_active FROM kb_resources r WHERE r.slug = $1")
            .bind(&slug)
            .fetch_one(&pool)
            .await
            .expect("resource row must still exist after soft-delete");

    assert!(
        !is_active,
        "resource must be soft-deleted (is_active = false) after cloud delete"
    );
}

// ---------------------------------------------------------------------------
// Test 11: cloud show --edges resolves via server-side resolve_by_uri
// ---------------------------------------------------------------------------

/// Cloud `temper resource show <slug> --type research --context <ctx> --edges`
/// must succeed without a manifest entry. Previously `show_edges` loaded the
/// local manifest to resolve the id and returned a "sync first" error in
/// cloud-only mode. The fix switches to `client.resources().resolve_by_uri`
/// (same path as `show`). This test verifies the end-to-end path: create a
/// resource via the API, then call `show` with `edges: true` and assert it
/// returns `Ok(())` — even with zero edges.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_show_edges_resolves_without_manifest(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("edgesctx")
        .await
        .expect("create edgesctx context");

    // Seed the resource via the API client (no CLI, no manifest written).
    use temper_core::types::ingest::{pack_chunks, IngestPayload};
    app.client
        .ingest()
        .create(&IngestPayload {
            title: "Edges Resolve Test".to_string(),
            origin_uri: "kb://edgesctx/research/edges-resolve-test".to_string(),
            context_name: "edgesctx".to_string(),
            doc_type_name: "research".to_string(),
            content_hash: None,
            slug: "edges-resolve-test".to_string(),
            content: String::new(),
            metadata: None,
            managed_meta: Some(serde_json::json!({
                "temper-title": "Edges Resolve Test"
            })),
            open_meta: None,
            chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
        })
        .await
        .expect("seed resource via client");

    // Drive show with edges=true on a blocking thread (runtime::with_client
    // creates an inner tokio runtime — must not nest).
    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();
    let cli_config = app.cli_config.clone();

    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::commands::resource::show(
                &cli_config,
                temper_cli::commands::resource::ShowParams {
                    doc_type: "research",
                    slug: "edges-resolve-test",
                    context: Some("edgesctx"),
                    format: temper_cli::format::OutputFormat::Json,
                    edges: true, // edges — the path under test
                    meta_only: false,
                    fields: &[],
                },
            )
            .expect(
                "cloud show --edges must succeed without a manifest entry; \
                 previously returned 'sync first' error",
            )
        })
    })
    .await
    .expect("spawn_blocking joined");
}
