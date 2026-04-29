#![cfg(feature = "test-db")]

//! End-to-end coverage for `temper resource delete` in Local mode.
//!
//! Drives the full delete flow against an in-process Axum server backed
//! by a real Postgres test database:
//!
//! 1. Pre-flight + create a context + create a goal via the CLI's
//!    Local-mode publish-tail (so we have a real server-side row, a
//!    local vault file with `temper-id`, and a manifest entry).
//! 2. Call `temper_cli::commands::resource::delete` with the slug.
//! 3. Assert: vault file gone; manifest entry gone; server row
//!    soft-deleted (`is_active = false`).
//!
//! Sync-push missing-file error coverage lives in the unit test
//! `actions::sync::tests::vault_file_missing_err_includes_both_recovery_hints`
//! plus mechanical replacement of the three call sites in `actions::sync`;
//! exercising it through the full push pipeline would test sync
//! orchestration more than the delete contract this file targets.
//!
//! Auth wiring uses `TEMPER_AUTH_PATH` to point both `DiskTokenStore` and
//! the device_id loader at a per-test auth.json. Mirrors the proven
//! pattern from `publish_tail_test.rs`.

mod common;

use chrono::{Duration, Utc};
use temper_client::auth::{Provider, StoredAuth};

/// Write a `StoredAuth` JSON to `path` so the disk token store and the
/// uniform path resolver find real credentials.
fn write_auth_json(path: &std::path::Path, jwt: &str) {
    let auth = StoredAuth {
        provider: Provider::Auth0 {
            domain: "test".to_string(),
        },
        access_token: jwt.to_string().into(),
        refresh_token: None,
        expires_at: Utc::now() + Duration::hours(1),
        profile_id: None,
        device_id: Some("e2e-resource-delete-device".to_string()),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create auth dir");
    }
    let bytes = serde_json::to_vec(&auth).expect("serialize StoredAuth");
    std::fs::write(path, bytes).expect("write auth.json");
}

/// Local-mode `temper resource delete <slug>` end-to-end:
/// create-and-publish a goal → delete via the new subcommand → verify
/// (a) the vault file is gone, (b) the manifest entry is gone, and
/// (c) the server-side `kb_resources` row has `is_active = false`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn local_mode_delete_removes_file_and_soft_deletes_on_server(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

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

    let auth_path = app.vault_dir.path().join(".temper/auth.json");
    write_auth_json(&auth_path, &app.token);
    let global_config = app.vault_dir.path().join("no-such-config.toml");

    let cli_config = app.cli_config.clone();
    let api_url = format!("http://{}", app.addr);
    let auth_path_string = auth_path.to_str().unwrap().to_string();
    let global_config_string = global_config.to_str().unwrap().to_string();

    // Step 1: create a goal locally and publish to the server. This gives
    // us a vault file with `temper-id`, a manifest entry, and a real
    // server row.
    let cli_config_create = cli_config.clone();
    let api_url_create = api_url.clone();
    let auth_path_create = auth_path_string.clone();
    let global_config_create = global_config_string.clone();
    let slug: String = tokio::task::spawn_blocking(move || {
        temp_env::with_vars(
            [
                ("TEMPER_API_URL", Some(api_url_create.as_str())),
                ("TEMPER_AUTH_PATH", Some(auth_path_create.as_str())),
                ("TEMPER_GLOBAL_CONFIG", Some(global_config_create.as_str())),
                ("TEMPER_VAULT_STATE", Some("local")),
                ("TEMPER_TOKEN", None),
            ],
            || {
                temper_cli::actions::goal::create(
                    &cli_config_create,
                    "myapp",
                    "delete-target",
                    None,
                )
                .expect("goal create + publish")
            },
        )
    })
    .await
    .expect("spawn_blocking joined");

    // Confirm setup state on disk + server before deleting.
    let owner = cli_config.owner_for_context("myapp");
    let goal_path = app
        .vault_dir
        .path()
        .join(&owner)
        .join("myapp")
        .join("goal")
        .join(format!("{slug}.md"));
    assert!(
        goal_path.exists(),
        "expected goal file at {} before delete",
        goal_path.display()
    );

    // Capture the server-assigned id from frontmatter so we can verify
    // soft-delete state after.
    let raw = std::fs::read_to_string(&goal_path).expect("read goal file");
    let fm =
        temper_core::frontmatter::Frontmatter::try_from(raw.as_str()).expect("parse frontmatter");
    let temper_id = fm
        .value()
        .get("temper-id")
        .and_then(|v| v.as_str())
        .expect("temper-id must be in frontmatter post-publish");
    let resource_uuid = uuid::Uuid::parse_str(temper_id).expect("temper-id parses");

    let pre_active: bool = sqlx::query_scalar("SELECT is_active FROM kb_resources WHERE id = $1")
        .bind(resource_uuid)
        .fetch_one(&pool)
        .await
        .expect("server row must exist before delete");
    assert!(pre_active, "server row should be active before delete");

    // Step 2: delete via the new CLI subcommand.
    let cli_config_delete = cli_config.clone();
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
                temper_cli::commands::resource::delete(
                    &cli_config_delete,
                    "goal",
                    &slug,
                    Some("myapp"),
                    /* force */ true,
                )
                .expect("resource delete should succeed")
            },
        )
    })
    .await
    .expect("spawn_blocking joined");

    // Step 3a: vault file is gone.
    assert!(
        !goal_path.exists(),
        "expected goal file removed at {} after delete",
        goal_path.display()
    );

    // Step 3b: manifest entry is gone.
    let manifest_path = app.vault_dir.path().join(".temper/manifest.json");
    let manifest_raw =
        std::fs::read_to_string(&manifest_path).expect("manifest must exist after delete");
    assert!(
        !manifest_raw.contains(temper_id),
        "manifest must not contain the deleted resource's id; got:\n{manifest_raw}"
    );

    // Step 3c: server row is soft-deleted (is_active = false). Verify
    // the row still exists (UUID preserved) but no longer active.
    let post_active: bool = sqlx::query_scalar("SELECT is_active FROM kb_resources WHERE id = $1")
        .bind(resource_uuid)
        .fetch_one(&pool)
        .await
        .expect("server row must still exist (soft-delete preserves UUID)");
    assert!(
        !post_active,
        "server row must be soft-deleted (is_active = false) after delete"
    );
}
