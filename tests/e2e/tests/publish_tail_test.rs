#![cfg(feature = "test-db")]

//! End-to-end coverage for the CLI publish-tail policy
//! (`temper_cli::actions::runtime::publish_local_write_best_effort`).
//!
//! This is the right testing layer for publish-tail behavior:
//! `actions::task::create` (and the other seven Local-mode creator/update
//! sites) calls into `runtime::with_client` which builds a real client
//! from disk auth + global config and posts to the API. Unit tests in
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

use chrono::{Duration, Utc};
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

/// CLI publish-tail end-to-end: `actions::goal::create` writes a goal
/// locally, then the publish tail (`publish_local_write_best_effort`)
/// pushes it through `with_client` → `push_one_resource` to the test
/// server. Verify the file on disk now carries the canonical `temper-id`
/// the server assigned (proves the publish round-tripped, not just that
/// the local write succeeded).
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
    let slug: String = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(
            [
                ("TEMPER_API_URL", Some(api_url.as_str())),
                ("TEMPER_AUTH_PATH", Some(auth_path_string.as_str())),
                ("TEMPER_GLOBAL_CONFIG", Some(global_config_string.as_str())),
                ("TEMPER_VAULT_STATE", Some("local")),
                ("TEMPER_TOKEN", None),
            ],
            || {
                temper_cli::actions::goal::create(&cli_config, "myapp", "publish-tail-goal", None)
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
    let slug: String = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(
            [
                ("TEMPER_API_URL", Some(api_url.as_str())),
                ("TEMPER_AUTH_PATH", Some(auth_path_string.as_str())),
                ("TEMPER_GLOBAL_CONFIG", Some(global_config_string.as_str())),
                ("TEMPER_VAULT_STATE", Some("local")),
                ("TEMPER_TOKEN", None),
            ],
            || {
                temper_cli::actions::goal::create(&cli_config, "myapp", "no-token-goal", None)
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
