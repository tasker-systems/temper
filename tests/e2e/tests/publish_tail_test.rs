#![cfg(feature = "test-db")]

//! End-to-end coverage for the CLI publish-tail policy
//! (`temper_cli::actions::runtime::publish_local_write_best_effort`).
//!
//! This is the right testing layer for publish-tail behavior:
//! `commands::resource::create` (and other Local-mode creator/update surfaces)
//! dispatches through `VaultBackend` which calls `push_one_resource` with a
//! real client from disk auth + global config to post to the API. Unit tests in
//! `temper-cli/tests/` deliberately skip publishing (no token configured
//! via `TEMPER_AUTH_PATH` isolation); the real publish path is verified
//! here against an in-process Axum server backed by a real Postgres test
//! database.
//!
//! Auth wiring uses `TEMPER_AUTH_PATH` to point both `DiskTokenStore` and
//! the device_id loader (`load_auth` → `resolve_auth_path()`) at a per-
//! test auth.json — exercising the unified resolver across every reader.
//!
//! Env vars set per test:
//!   - `TEMPER_API_URL` — overrides `config.cloud.api_url`
//!   - `TEMPER_AUTH_PATH` — disk auth file written with the test JWT
//!     (publish path) or a non-existent file (no-token path)
//!   - `TEMPER_GLOBAL_CONFIG` — points at non-existent path so config
//!     defaults take effect (no developer config file leakage)

mod common;

use chrono::{Duration, Local, Utc};
use temper_client::auth::{Provider, StoredAuth};
use temper_core::frontmatter::Frontmatter;

/// Write a `StoredAuth` JSON to `path` so `DiskTokenStore::at(path)` and the
/// uniform path resolver (used by `load_device_id`) find real credentials.
/// Mirrors the shape `temper auth login` produces.
fn write_auth_json(path: &std::path::Path, jwt: &str) {
    let auth = StoredAuth {
        provider: Provider::Auth0 {
            domain: "test".to_string(),
        },
        access_token: jwt.to_string().into(),
        refresh_token: None,
        expires_at: Utc::now() + Duration::hours(1),
        profile_id: None,
        device_id: Some("e2e-publish-tail-device".to_string()),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create auth dir");
    }
    let bytes = serde_json::to_vec(&auth).expect("serialize StoredAuth");
    std::fs::write(path, bytes).expect("write auth.json");
}

/// CLI publish-tail end-to-end: `commands::resource::create` writes a goal
/// locally via `VaultBackend`, then the publish tail (`push_one_resource`)
/// pushes it to the test server. Verify the file on disk now carries the
/// canonical `temper-id` the server assigned (proves the publish
/// round-tripped, not just that the local write succeeded).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn local_mode_create_publishes_to_server(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    // Pre-flight + create the "myapp" context the CLI Config expects.
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");
    app.client
        .contexts()
        .create("myapp")
        .await
        .expect("create myapp context");

    // Disk auth at a tmp path inside the test vault dir. Both the disk
    // token store (via TEMPER_AUTH_PATH) and `load_device_id` (via the
    // unified resolver) read from this file — no env-var auth needed.
    let auth_path = app.vault_dir.path().join(".temper/auth.json");
    write_auth_json(&auth_path, &app.token);

    // Non-existent config path so load_cloud_config() returns defaults
    // (no developer ~/.config/temper/config.toml leakage).
    let global_config = app.vault_dir.path().join("no-such-config.toml");

    // Run the CLI creator with env wiring scoped to this test only.
    // temp_env::with_vars serializes against other env-mutating tests.
    // The CLI creator is sync and builds its own tokio runtime internally
    // (`runtime::with_client`), so we must run it on a blocking thread
    // — nesting tokio runtimes panics with "cannot start a runtime from
    // within a runtime".
    let cli_config = app.cli_config.clone();
    let api_url = format!("http://{}", app.addr);
    let auth_path_string = auth_path.to_str().unwrap().to_string();
    let global_config_string = global_config.to_str().unwrap().to_string();
    // Pre-create the local context dir so `resolve_context_with_fallback`
    // doesn't redirect to "default" (matches `tests/common/mod.rs::create_goal`).
    std::fs::create_dir_all(app.vault_dir.path().join("@me").join("myapp"))
        .expect("pre-create context dir");
    // Goals use a plain slugified title — no date prefix. The slug is determined
    // by `commands::resource::create`'s slug-derivation branch for DocType::Goal.
    let goal_title = "publish-tail-goal";
    let slug = temper_cli::vault::slugify(goal_title);
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
                temper_cli::commands::resource::create(
                    &cli_config,
                    "goal",
                    goal_title,
                    Some("myapp"),
                    None,
                    None,
                    None,
                    None,
                    None,
                    "text",
                )
                .expect("goal create + publish")
            },
        )
    })
    .await
    .expect("spawn_blocking joined");

    // The on-disk file should now have `temper-id` (server-canonical UUIDv7),
    // not `temper-provisional-id` — proof the publish tail completed.
    let owner = app.cli_config.owner_for_context("myapp");
    let goal_path = app
        .vault_dir
        .path()
        .join(&owner)
        .join("myapp")
        .join("goal")
        .join(format!("{slug}.md"));
    assert!(
        goal_path.exists(),
        "expected goal file at {}",
        goal_path.display()
    );
    let raw = std::fs::read_to_string(&goal_path).expect("read goal file");
    assert!(
        raw.contains("temper-id:"),
        "expected temper-id on goal frontmatter after publish; got:\n{raw}"
    );
    assert!(
        !raw.contains("temper-provisional-id:"),
        "provisional id should have been rewritten to canonical id; got:\n{raw}"
    );

    // Also verify the resource is visible via the server API — the
    // primary contract of "publish to server" being exercised.
    let fm = Frontmatter::parse_file(&goal_path).expect("parse goal frontmatter");
    let yaml = fm.value();
    let temper_id = yaml
        .get("temper-id")
        .and_then(|v| v.as_str())
        .expect("temper-id must be a string in YAML");
    let id_uuid = uuid::Uuid::parse_str(temper_id).expect("temper-id parses as UUID");
    let row: (String,) =
        sqlx::query_as("SELECT slug FROM kb_resources WHERE id = $1 AND is_active")
            .bind(id_uuid)
            .fetch_one(&app.pool)
            .await
            .expect("server-side resource lookup");
    assert_eq!(row.0, slug, "server slug must match local slug");
}

/// Regression pin (post-Task-9 refactor): `commands::session::save` produces the
/// schema-correct wire shape after `build_managed_meta_for_create` was introduced.
///
/// Asserts four things that must hold after the helper consolidation:
/// 1. The on-disk frontmatter contains `title:` (new schema-correct field added by
///    Task 9) plus the structural fields (`temper-type: session`, `temper-context: myapp`).
/// 2. After the publish-tail completes, the file carries `temper-id:` (not just
///    `temper-provisional-id:`), proving the payload reached the server.
/// 3. The persisted `kb_resource_manifests.managed_meta` contains `title:` (Task 9's
///    schema-correct contribution) and the doc-type default `date:`. By design the
///    server strips tier-1 fields (`temper-type`, `temper-context`) from stored
///    managed_meta — they are implicit from the resource's doc_type and context rows.
/// 4. The `kb_resources` row has the correct `doc_type_name` and context slug,
///    confirming the ingest payload's context_name and doc_type_name routing.
///
/// If any of these fail, a future change drifted the local-mode wire output from the
/// schema-correct baseline established by Task 9.
#[cfg(feature = "test-embed")]
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn local_mode_session_create_wire_shape_regression(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Pre-flight: auto-provision the profile and create the context.
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");
    app.client
        .contexts()
        .create("myapp")
        .await
        .expect("create myapp context");

    // Disk auth wired at a tmp path inside the vault dir.
    let auth_path = app.vault_dir.path().join(".temper/auth.json");
    write_auth_json(&auth_path, &app.token);

    let global_config = app.vault_dir.path().join("no-such-config.toml");

    let cli_config = app.cli_config.clone();
    let api_url = format!("http://{}", app.addr);
    let auth_path_string = auth_path.to_str().unwrap().to_string();
    let global_config_string = global_config.to_str().unwrap().to_string();

    // Pre-create the local context dir so `resolve_context_with_fallback`
    // doesn't redirect to "default" (matches `tests/common/mod.rs::create_goal`).
    std::fs::create_dir_all(app.vault_dir.path().join("@me").join("myapp"))
        .expect("pre-create context dir");

    // Drive session::save on a blocking thread (it creates its own tokio runtime
    // internally via runtime::with_client — nesting would panic).
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
                temper_cli::commands::session::save(
                    &cli_config,
                    Some("Snapshot Test"),
                    Some("myapp"),
                    None, // stdin_content
                    None, // task
                    None, // state
                    "text",
                )
                .expect("session create + publish")
            },
        )
    })
    .await
    .expect("spawn_blocking joined");

    // ---- Assertion 1: on-disk frontmatter shape ----
    // Build the expected path: {vault_root}/@me/myapp/session/{today}-snapshot-test.md
    let today = Local::now().format("%Y-%m-%d").to_string();
    let session_slug = format!("{today}-snapshot-test");
    let vault = temper_core::vault::Vault::new(&app.cli_config.vault_root);
    let owner = app.cli_config.owner_for_context("myapp");
    let session_path = vault.doc_file(&owner, "myapp", "session", &session_slug);

    assert!(
        session_path.exists(),
        "expected session file at {}",
        session_path.display()
    );

    let raw = std::fs::read_to_string(&session_path).expect("read session file");

    // title: must be present (schema-correct field added by Task 9)
    assert!(
        raw.contains("title: Snapshot Test") || raw.contains("title: \"Snapshot Test\""),
        "expected 'title: Snapshot Test' in frontmatter; got:\n{raw}"
    );
    // structural fields
    assert!(
        raw.contains("temper-type: session"),
        "expected 'temper-type: session' in frontmatter; got:\n{raw}"
    );
    assert!(
        raw.contains("temper-context: myapp"),
        "expected 'temper-context: myapp' in frontmatter; got:\n{raw}"
    );

    // ---- Assertion 2: publish completed — canonical id present ----
    assert!(
        raw.contains("temper-id:"),
        "expected temper-id after publish; got:\n{raw}"
    );
    assert!(
        !raw.contains("temper-provisional-id:"),
        "provisional id should have been replaced by canonical id; got:\n{raw}"
    );

    // ---- Assertion 3: db managed_meta has schema-correct title field ----
    // Parse the canonical id from frontmatter to look up the manifest row.
    let fm = Frontmatter::parse_file(&session_path).expect("parse session frontmatter");
    let temper_id = fm
        .value()
        .get("temper-id")
        .and_then(|v| v.as_str())
        .expect("temper-id must be a string in YAML");
    let id_uuid = uuid::Uuid::parse_str(temper_id).expect("temper-id parses as UUID");

    let stored_managed_meta: serde_json::Value = sqlx::query_scalar!(
        "SELECT managed_meta FROM kb_resource_manifests WHERE resource_id = $1",
        id_uuid
    )
    .fetch_one(&pool)
    .await
    .expect("kb_resource_manifests lookup for session");

    let obj = stored_managed_meta
        .as_object()
        .expect("stored managed_meta must be a JSON object");

    // `title` is the schema-correct field added by Task 9's build_managed_meta_for_create.
    // It flows from local frontmatter → fm.managed_json() → IngestPayload → strip_system_fields
    // → apply_managed_defaults → stored in kb_resource_manifests. If this fails, Task 9
    // regressed.
    assert_eq!(
        obj.get("temper-title").and_then(|v| v.as_str()),
        Some("Snapshot Test"),
        "managed_meta must carry title: Snapshot Test (Task 9 schema-correct field); got: {stored_managed_meta}"
    );

    // `date` is the doc-type default for sessions, but Phase 6 / Migration A
    // established that it lives in open_meta, not managed_meta. The
    // `apply_open_defaults` helper writes it on the open side; managed_meta
    // therefore must NOT contain `date` for new ingests.
    assert!(
        !obj.contains_key("date"),
        "managed_meta must NOT contain 'date' (Phase 6 canonical: date lives in open_meta); got: {stored_managed_meta}"
    );

    // Cross-check: `date` IS present in the stored open_meta.
    let stored_open_meta: serde_json::Value = sqlx::query_scalar!(
        "SELECT open_meta FROM kb_resource_manifests WHERE resource_id = $1",
        id_uuid
    )
    .fetch_one(&pool)
    .await
    .expect("kb_resource_manifests open_meta lookup for session");
    let open_obj = stored_open_meta
        .as_object()
        .expect("stored open_meta must be a JSON object");
    assert!(
        open_obj.contains_key("date"),
        "open_meta must contain 'date' (session doc-type default lives here per Phase 6); got: {stored_open_meta}"
    );

    // By design, tier-1 system fields (temper-type, temper-context) are intentionally
    // stripped from stored managed_meta — they are encoded in the resource's doc_type and
    // context rows. This assertion documents that contract (guards against accidental
    // re-insertion).
    assert!(
        !obj.contains_key("temper-type"),
        "temper-type must NOT be stored in managed_meta (it is a tier-1 system field); got: {stored_managed_meta}"
    );
    assert!(
        !obj.contains_key("temper-context"),
        "temper-context must NOT be stored in managed_meta (it is a tier-1 system field); got: {stored_managed_meta}"
    );

    // ---- Assertion 4: kb_resources row has correct doc_type and context ----
    // The publish payload's doc_type_name and context_name are the real routing
    // contract; confirm they landed on the correct server-side record.
    let (doc_type_name, context_name): (String, String) = sqlx::query_as(
        "SELECT dt.name, c.name
         FROM kb_resources r
         JOIN kb_doc_types dt ON dt.id = r.kb_doc_type_id
         JOIN kb_contexts c ON c.id = r.kb_context_id
         WHERE r.id = $1",
    )
    .bind(id_uuid)
    .fetch_one(&pool)
    .await
    .expect("kb_resources doc_type + context lookup");

    assert_eq!(
        doc_type_name, "session",
        "resource must have doc_type 'session'; got: {doc_type_name}"
    );
    assert_eq!(
        context_name, "myapp",
        "resource must belong to context 'myapp'; got: {context_name}"
    );
}

/// No-token path: when the disk auth is absent, `publish_local_write_best_effort`
/// must `tracing::warn!` and return `Ok(None)` — file-creation contract
/// holds, no API call attempted, no failure surfaced. This is the
/// behaviour CLI unit tests rely on for isolation, but proving it
/// end-to-end (against a live server that would 401 a missing token)
/// is the right place to lock the contract down.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn local_mode_create_with_no_token_creates_file_and_skips_publish(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    // Auth path points at a non-existent file: disk store finds nothing.
    let auth_path = app.vault_dir.path().join(".temper/no-such-auth.json");
    let global_config = app.vault_dir.path().join("no-such-config.toml");
    let api_url = format!("http://{}", app.addr);
    let cli_config = app.cli_config.clone();

    let auth_path_string = auth_path.to_str().unwrap().to_string();
    let global_config_string = global_config.to_str().unwrap().to_string();
    // Pre-create the local context dir so `resolve_context_with_fallback`
    // doesn't redirect to "default" (matches `tests/common/mod.rs::create_goal`).
    std::fs::create_dir_all(app.vault_dir.path().join("@me").join("myapp"))
        .expect("pre-create context dir");
    let goal_title = "no-token-goal";
    let slug = temper_cli::vault::slugify(goal_title);
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
                temper_cli::commands::resource::create(
                    &cli_config,
                    "goal",
                    goal_title,
                    Some("myapp"),
                    None,
                    None,
                    None,
                    None,
                    None,
                    "text",
                )
                .expect("goal create succeeds even without auth")
            },
        )
    })
    .await
    .expect("spawn_blocking joined");

    // File exists locally...
    let owner = app.cli_config.owner_for_context("myapp");
    let goal_path = app
        .vault_dir
        .path()
        .join(&owner)
        .join("myapp")
        .join("goal")
        .join(format!("{slug}.md"));
    assert!(goal_path.exists(), "local file must exist on no-token path");

    // ...but the publish was skipped, so the file still carries
    // `temper-provisional-id` and the server has no record.
    let raw = std::fs::read_to_string(&goal_path).expect("read goal file");
    assert!(
        raw.contains("temper-provisional-id:"),
        "provisional id should remain when publish was skipped; got:\n{raw}"
    );
    assert!(
        !raw.contains("temper-id:"),
        "no temper-id should be assigned without a server round-trip; got:\n{raw}"
    );

    // Server-side: no resource with this slug exists.
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM kb_resources WHERE slug = $1 AND is_active")
            .bind(&slug)
            .fetch_one(&app.pool)
            .await
            .expect("count by slug");
    assert_eq!(
        count.0, 0,
        "no server-side resource expected on no-token path"
    );
}
