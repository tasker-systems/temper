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

    // Pre-create the local context dir so `resolve_context_with_fallback`
    // doesn't redirect to "default" (matches `tests/common/mod.rs::create_goal`).
    std::fs::create_dir_all(app.vault_dir.path().join("@me").join("myapp"))
        .expect("pre-create context dir");

    // Step 1: create a goal locally and publish to the server. This gives
    // us a vault file with `temper-id`, a manifest entry, and a real
    // server row.
    let cli_config_create = cli_config.clone();
    let api_url_create = api_url.clone();
    let auth_path_create = auth_path_string.clone();
    let global_config_create = global_config_string.clone();
    let goal_title = "delete-target";
    let slug = temper_cli::vault::slugify(goal_title);
    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(
            [
                ("TEMPER_API_URL", Some(api_url_create.as_str())),
                ("TEMPER_AUTH_PATH", Some(auth_path_create.as_str())),
                ("TEMPER_GLOBAL_CONFIG", Some(global_config_create.as_str())),
                ("TEMPER_VAULT_STATE", Some("local")),
                ("TEMPER_TOKEN", None),
            ],
            || {
                temper_cli::commands::resource::create(
                    &cli_config_create,
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

/// After soft-delete, the same `(slug, context)` should be free for reuse.
///
/// Regression guard for vault task
/// `2026-05-03-soft-deleted-resources-should-not-block-slug-reuse`. Before the
/// `kb_resources_slug_kb_context_id_active_unique` partial-index migration,
/// soft-deleted rows kept their slug reserved, so recreate-with-same-slug
/// returned 409 Conflict ("Resource already exists").
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn slug_freed_for_reuse_after_soft_delete(pool: sqlx::PgPool) {
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

    // Pre-create the local context dir so `resolve_context_with_fallback`
    // doesn't redirect to "default" (matches `tests/common/mod.rs::create_goal`).
    std::fs::create_dir_all(app.vault_dir.path().join("@me").join("myapp"))
        .expect("pre-create context dir");

    // ── Step 1: create a goal and publish to the server. ─────────────────
    let cli_config_create = cli_config.clone();
    let api_url_1 = api_url.clone();
    let auth_path_1 = auth_path_string.clone();
    let global_config_1 = global_config_string.clone();
    let first_title = "reuse-target";
    let first_slug = temper_cli::vault::slugify(first_title);
    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(
            [
                ("TEMPER_API_URL", Some(api_url_1.as_str())),
                ("TEMPER_AUTH_PATH", Some(auth_path_1.as_str())),
                ("TEMPER_GLOBAL_CONFIG", Some(global_config_1.as_str())),
                ("TEMPER_VAULT_STATE", Some("local")),
                ("TEMPER_TOKEN", None),
            ],
            || {
                temper_cli::commands::resource::create(
                    &cli_config_create,
                    "goal",
                    first_title,
                    Some("myapp"),
                    None,
                    None,
                    None,
                    None,
                    None,
                    "text",
                )
                .expect("first goal create + publish")
            },
        )
    })
    .await
    .expect("spawn_blocking joined");

    // Capture the first row's UUID for later soft-delete verification.
    let owner = cli_config.owner_for_context("myapp");
    let first_path = app
        .vault_dir
        .path()
        .join(&owner)
        .join("myapp")
        .join("goal")
        .join(format!("{first_slug}.md"));
    let raw = std::fs::read_to_string(&first_path).expect("read first goal file");
    let fm =
        temper_core::frontmatter::Frontmatter::try_from(raw.as_str()).expect("parse frontmatter");
    let first_temper_id = fm
        .value()
        .get("temper-id")
        .and_then(|v| v.as_str())
        .expect("first temper-id must be in frontmatter")
        .to_string();
    let first_uuid = uuid::Uuid::parse_str(&first_temper_id).expect("first temper-id parses");

    // ── Step 2: delete via the CLI subcommand. ────────────────────────────
    let cli_config_delete = cli_config.clone();
    let slug_for_delete = first_slug.clone();
    let api_url_2 = api_url.clone();
    let auth_path_2 = auth_path_string.clone();
    let global_config_2 = global_config_string.clone();
    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(
            [
                ("TEMPER_API_URL", Some(api_url_2.as_str())),
                ("TEMPER_AUTH_PATH", Some(auth_path_2.as_str())),
                ("TEMPER_GLOBAL_CONFIG", Some(global_config_2.as_str())),
                ("TEMPER_VAULT_STATE", Some("local")),
                ("TEMPER_TOKEN", None),
            ],
            || {
                temper_cli::commands::resource::delete(
                    &cli_config_delete,
                    "goal",
                    &slug_for_delete,
                    Some("myapp"),
                    /* force */ true,
                )
                .expect("delete should succeed")
            },
        )
    })
    .await
    .expect("spawn_blocking joined");

    // Sanity: first row is soft-deleted.
    let first_active: bool = sqlx::query_scalar("SELECT is_active FROM kb_resources WHERE id = $1")
        .bind(first_uuid)
        .fetch_one(&pool)
        .await
        .expect("first row must still exist after soft-delete");
    assert!(!first_active, "first row should be soft-deleted");

    // ── Step 3: recreate a goal with the same title → same slug. ─────────
    let cli_config_recreate = cli_config.clone();
    let api_url_3 = api_url.clone();
    let auth_path_3 = auth_path_string.clone();
    let global_config_3 = global_config_string.clone();
    let second_title = "reuse-target";
    let second_slug = temper_cli::vault::slugify(second_title);
    tokio::task::spawn_blocking(move || {
        temp_env::with_vars(
            [
                ("TEMPER_API_URL", Some(api_url_3.as_str())),
                ("TEMPER_AUTH_PATH", Some(auth_path_3.as_str())),
                ("TEMPER_GLOBAL_CONFIG", Some(global_config_3.as_str())),
                ("TEMPER_VAULT_STATE", Some("local")),
                ("TEMPER_TOKEN", None),
            ],
            || {
                temper_cli::commands::resource::create(
                    &cli_config_recreate,
                    "goal",
                    second_title,
                    Some("myapp"),
                    None,
                    None,
                    None,
                    None,
                    None,
                    "text",
                )
                .expect("recreate with same slug must succeed after soft-delete")
            },
        )
    })
    .await
    .expect("spawn_blocking joined");

    assert_eq!(
        second_slug, first_slug,
        "recreated goal should use the same slug"
    );

    // ── Step 4: verify two server rows now exist for (slug, context):
    //   - the first one with is_active = false
    //   - the new one with is_active = true and a different UUID
    let rows: Vec<(uuid::Uuid, bool)> = sqlx::query_as(
        "SELECT r.id, r.is_active \
         FROM kb_resources r \
         JOIN kb_contexts c ON c.id = r.kb_context_id \
         WHERE r.slug = $1 AND c.name = $2 \
         ORDER BY r.created",
    )
    .bind(&first_slug)
    .bind("myapp")
    .fetch_all(&pool)
    .await
    .expect("query rows");

    assert_eq!(rows.len(), 2, "expected two rows for slug after recreate");
    assert_eq!(rows[0].0, first_uuid, "first row preserved by soft-delete");
    assert!(!rows[0].1, "first row stays soft-deleted");
    assert_ne!(rows[1].0, first_uuid, "second row has a new UUID");
    assert!(rows[1].1, "second row is active");
}
