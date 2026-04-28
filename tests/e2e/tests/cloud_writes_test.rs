#![cfg(feature = "test-db")]

//! End-to-end coverage for cloud-mode write paths.
//!
//! Drives `TEMPER_VAULT_STATE=cloud` plus per-test `TEMPER_TOKEN` /
//! `TEMPER_API_URL` / `TEMPER_GLOBAL_CONFIG` against an in-process Axum
//! server backed by a real Postgres test database. No vault directory
//! is touched for the cloud-mode write paths under test — the cloud
//! branches in `commands::resource::create` / `update` / `list` /
//! `show` and the `sync_cmd::run` cloud guard are exercised end to end.
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
) -> [(&'static str, Option<&'a str>); 5] {
    [
        ("TEMPER_API_URL", Some(api_url)),
        ("TEMPER_TOKEN", Some(token)),
        ("TEMPER_GLOBAL_CONFIG", Some(global_config)),
        ("TEMPER_VAULT_STATE", Some("cloud")),
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
                "session",
                "Cloud Round-Trip Session",
                Some("myapp"),
                None, // goal
                None, // mode
                None, // effort
                None, // slug override
                None, // body_flag (default body generated)
                "text",
            )
            .expect("cloud create should succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    // ---- Assertion 1: resource row exists ----
    let slug = "cloud-round-trip-session";
    let (doc_type_name, context_name): (String, String) = sqlx::query_as(
        "SELECT dt.name, c.name
         FROM kb_resources r
         JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
         JOIN kb_contexts c ON c.id = r.kb_context_id
         WHERE r.slug = $1 AND r.is_active",
    )
    .bind(slug)
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
    .bind(slug)
    .fetch_one(&pool)
    .await
    .expect("manifest row must exist after cloud create");

    let obj = managed_meta
        .as_object()
        .expect("managed_meta must be a JSON object");
    assert_eq!(
        obj.get("title").and_then(|v| v.as_str()),
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

    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url2, &token2, &global_config_str2), || {
            temper_cli::commands::resource::show(
                &cli_config2,
                "session",
                slug,
                Some("myapp"),
                "text",
                false, // edges
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
            "title": "Meta-Only Update Test",
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
                    title: None,
                    tags: &[],
                    aliases: &[],
                    relates_to: &[],
                    references: &[],
                    depends_on: &[],
                    extends: &[],
                    preceded_by: &[],
                    derived_from: &[],
                    stage: Some("done"),
                    mode: None,
                    effort: None,
                    goal: None,
                    seq: None,
                    branch: None,
                    pr: None,
                    status: None,
                    body: None,
                },
            )
            .expect("cloud meta-only update must succeed")
        })
    })
    .await
    .expect("spawn_blocking joined");

    // ---- Assertion 1: stage is "done" ----
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
        obj.get("temper-stage").and_then(|v| v.as_str()),
        Some("done"),
        "temper-stage must be 'done' after meta-only update; got: {managed_meta}"
    );

    // ---- Assertion 2: title preserved ----
    assert_eq!(
        obj.get("title").and_then(|v| v.as_str()),
        Some("Meta-Only Update Test"),
        "title must be preserved after meta-only update; got: {managed_meta}"
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
            "title": "Body+Meta Update Test",
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
                    title: None,
                    tags: &[],
                    aliases: &[],
                    relates_to: &[],
                    references: &[],
                    depends_on: &[],
                    extends: &[],
                    preceded_by: &[],
                    derived_from: &[],
                    stage: Some("done"),
                    mode: None,
                    effort: None,
                    goal: None,
                    seq: None,
                    branch: None,
                    pr: None,
                    status: None,
                    body: Some(body_flag),
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
        obj.get("temper-stage").and_then(|v| v.as_str()),
        Some("done"),
        "temper-stage must be 'done' after body+meta update; got: {managed_meta}"
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
            "title": "Body-Only Update Test",
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
// Test 5: chunk dedupe short-circuit skips unchanged bodies
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
                "session",
                "Chunk Dedup Test",
                Some("myapp"),
                None,
                None,
                None,
                None,
                Some(body_flag),
                "text",
            )
            .expect("cloud create for dedup test")
        })
    })
    .await
    .expect("spawn_blocking joined");

    // Count kb_chunks after first create.
    let slug = "chunk-dedup-test";
    let chunk_count_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kb_chunks c
         JOIN kb_resources r ON r.id = c.resource_id
         WHERE r.slug = $1 AND c.is_current",
    )
    .bind(slug)
    .fetch_one(&pool)
    .await
    .expect("chunk count before second write");

    // Re-send identical body via PATCH — should short-circuit.
    let cli_config2 = app.cli_config.clone();
    let api_url3 = api_url.clone();
    let token3 = token.clone();
    let global_config_str3 = global_config_str.clone();

    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url3, &token3, &global_config_str3), || {
            temper_cli::commands::resource::update(
                &cli_config2,
                &temper_cli::commands::resource::UpdateParams {
                    slug,
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
    .bind(slug)
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
// Test 6: sync run returns cloud-mode error message
// ---------------------------------------------------------------------------

/// Cloud `temper sync run` returns the exact redirect error string instead of
/// attempting a sync (which would fail without a local vault).
///
/// The error must contain the canonical redirect phrase:
/// "cloud mode has no local vault to sync"
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_sync_run_redirects_with_message(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // No profile/context pre-flight needed — the guard fires before any I/O.

    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let token = app.token.clone();
    let global_config_str = global_config.to_str().unwrap().to_string();

    let result = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(cloud_env(&api_url, &token, &global_config_str), || {
            temper_cli::commands::sync_cmd::run(
                &[], // contexts (empty = all)
                "text",
            )
        })
    })
    .await
    .expect("spawn_blocking joined");

    // Must be an Err whose message contains the cloud redirect phrase.
    let err = result.expect_err("sync run must fail with cloud-mode redirect error");
    let err_str = err.to_string();
    assert!(
        err_str.contains("cloud mode has no local vault to sync"),
        "error message must contain redirect phrase; got: {err_str}"
    );
    assert!(
        err_str.contains("temper resource create"),
        "error message must mention 'temper resource create'; got: {err_str}"
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
            managed_meta: Some(serde_json::json!({"title": format!("Cloud Only {i}")})),
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
                    format: "text",
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
