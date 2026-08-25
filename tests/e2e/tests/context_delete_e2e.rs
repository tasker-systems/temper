#![cfg(feature = "test-db")]
//! Context-delete e2e: the dependents guard, and the destructive act itself.
//!
//! `DELETE /api/contexts/{id}` is the one context mutation that is genuinely irreversible — unlike
//! `rename`/`reassign`, there is no `is_active` to flip back. Its whole claim is the refusal:
//! a context still homing a live resource must not silently strand it, so the endpoint 409s with a
//! `doc_type` breakdown and only actually deletes once nothing is left attached. Ingest uses empty
//! content (no body → no embed) so this runs on plain `cargo make test-e2e` without `test-embed`.
//! This file proves
//! that ordering end to end — refuse while attached, move it out via `resource update
//! --context-to`, then succeed — through both the client and the `temper` binary, and separately
//! proves the empty-context happy path never has to fight the guard at all.
//!
//! Modeled on `context_rename_e2e.rs` (its `provision` / `root_bootstrap_first_admin` / `cli_*`
//! idiom); the gate itself (`ContextAdminAuthority`) is exercised generically at the unit tier and
//! by `context_rename_e2e.rs`'s two-dialect test, so this file does not re-prove it — it proves the
//! part that is new: the dependents guard and the hard delete.
//!
//! No embedding work is inspected here, so this runs on plain `cargo make test-e2e`.

mod common;

use reqwest::StatusCode;
use serde_json::Value;
use uuid::Uuid;

use temper_workflow::types::resource::ResourceUpdateRequest;

/// Provision a profile by hitting an authed endpoint (auto-provision on first request), then
/// approve it — a fresh principal is born Denied (D11), and this test exercises the delete
/// endpoint's own gate, not the front door.
async fn provision(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("json");
    let pid: Uuid = body["id"].as_str().expect("id").parse().expect("uuid");
    common::approve(&app.pool, pid).await;
    pid
}

/// The irreducible operator root step: configure gating + mint first admin.
async fn root_bootstrap_first_admin(pool: &sqlx::PgPool, admin_id: Uuid) {
    sqlx::query(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system','Temper System') \
         ON CONFLICT (slug) DO UPDATE SET name=EXCLUDED.name",
    )
    .execute(pool)
    .await
    .expect("team");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug='temper-system' WHERE id=1")
        .execute(pool)
        .await
        .expect("gating");
    common::approved_admin(pool, admin_id).await;
}

/// `POST /api/ingest` homed at `context_ref`, as `token`. Returns `(status, resource_id, body)` —
/// the body rides along so a caller can print the server's actual error on an unexpected status
/// instead of just the bare status code.
async fn ingest(
    app: &common::E2eTestApp,
    token: &str,
    context_ref: &str,
    slug: &str,
) -> (StatusCode, Option<Uuid>, Value) {
    let resp = app
        .reqwest_client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "title": format!("ctx-delete {slug}"),
            "origin_uri": format!("test://context-delete-e2e/{}", Uuid::new_v4()),
            "context_ref": context_ref,
            "doc_type_name": "research",
            "slug": slug,
            "content": "",
        }))
        .send()
        .await
        .expect("ingest request");
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    let id = body["id"].as_str().and_then(|s| s.parse().ok());
    (status, id, body)
}

/// Wire-level status + body of `DELETE /api/contexts/{context}` as `token`.
async fn delete_status(
    app: &common::E2eTestApp,
    token: &str,
    context_id: Uuid,
) -> (StatusCode, Value) {
    let resp = app
        .reqwest_client
        .delete(app.url(&format!("/api/contexts/{context_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("delete request");
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    (status, body)
}

/// Run the real `temper` binary's `context delete <ref>`, returning `(success, combined output)`.
async fn cli_delete(app: &common::E2eTestApp, token: &str, context_id: Uuid) -> (bool, String) {
    let config_toml = toml::to_string(&app.config).expect("serialize test TemperConfig");
    let config_path = app.vault_dir.path().join("test-temper-config.toml");
    std::fs::write(&config_path, config_toml).expect("write test config");

    let bare_uuid = context_id.to_string();
    let out = common::run_temper_cli_with_token(
        &app.base_url(),
        token,
        &config_path,
        &["context", "delete", &bare_uuid],
    )
    .await
    .expect("spawn temper");

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = if stdout.trim().is_empty() {
        stderr
    } else {
        stdout
    };
    (out.status.success(), combined)
}

/// The headline behavior: a context homing a live resource refuses to delete, names what is
/// attached, and only succeeds once the resource has been moved out — proved over both HTTP (the
/// 409's shape) and the CLI (the message a real operator sees).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn delete_is_refused_while_attached_and_succeeds_once_the_resource_moves(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_id = provision(&app, &app.token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    let source = app
        .client
        .contexts()
        .create("Delete Me", None)
        .await
        .expect("admin creates the source context");
    let source_ref = format!("{}/{}", source.owner_ref, source.slug);

    let destination = app
        .client
        .contexts()
        .create("Elsewhere", None)
        .await
        .expect("admin creates the destination context");
    let destination_ref = format!("{}/{}", destination.owner_ref, destination.slug);

    let (ing, resource_id, ing_body) = ingest(&app, &app.token, &source_ref, "attached").await;
    assert_eq!(
        ing,
        StatusCode::OK,
        "admin authors into their own context: {ing_body:?}"
    );
    let resource_id = resource_id.expect("ingest returns the new resource id");

    // ── refused while attached ──────────────────────────────────────────────────────────────
    let (status, body) = delete_status(&app, &app.token, *source.id).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a context still homing a live resource must not silently strand it: {body:?}"
    );
    let message = body["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("dependent resource"),
        "the 409 names what is attached: {message}"
    );
    assert!(
        message.contains("research"),
        "the breakdown is by doc_type, so it names the actual type: {message}"
    );
    assert!(
        message.contains("--context-to"),
        "the refusal points at the fix, not just the problem: {message}"
    );

    // The CLI surfaces the same refusal to a real operator.
    let (cli_ok, cli_err) = cli_delete(&app, &app.token, *source.id).await;
    assert!(
        !cli_ok,
        "the CLI must fail while a resource is attached: {cli_err}"
    );
    assert!(
        cli_err.contains("dependent resource"),
        "the CLI carries the server's dependents message verbatim: {cli_err}"
    );

    // Nothing was deleted by the refused attempts.
    let still_there: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM kb_contexts WHERE id = $1)")
            .bind(*source.id)
            .fetch_one(&pool)
            .await
            .expect("existence check");
    assert!(still_there, "a refused delete changes nothing");

    // ── move the resource out, exactly the way the refusal told the operator to ────────────────
    let moved = app
        .client
        .resources()
        .update(
            resource_id,
            &ResourceUpdateRequest {
                context_to: Some(destination_ref.clone()),
                ..Default::default()
            },
        )
        .await
        .expect("move the resource to the destination context");
    assert_eq!(
        moved.context_ref.as_deref(),
        Some(destination_ref.as_str()),
        "the resource is now homed in the destination context"
    );

    // ── and now the delete succeeds ─────────────────────────────────────────────────────────
    let (status, body) = delete_status(&app, &app.token, *source.id).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "with nothing left attached, the delete goes through: {body:?}"
    );

    let gone: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM kb_contexts WHERE id = $1)")
        .bind(*source.id)
        .fetch_one(&pool)
        .await
        .expect("existence check");
    assert!(
        !gone,
        "the context row is actually gone — this is a hard delete"
    );

    // The moved resource is unaffected by its former home's deletion.
    let show = app
        .reqwest_client
        .get(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("show request")
        .status();
    assert_eq!(
        show,
        StatusCode::OK,
        "the resource survives — only its former (now-deleted) home is gone"
    );

    // A second delete of the same, now-absent context is the ordinary 404 — deletion is not
    // idempotent (there is nothing left to be a no-op about), but it must not 500.
    let (status, _) = delete_status(&app, &app.token, *source.id).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "deleting an already-deleted context reads the same as deleting one that never existed"
    );
}

/// The trivial path: a context with nothing homed in it deletes on the first try, both via HTTP
/// and via the CLI — the guard must never fire when there is nothing to guard.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_empty_context_deletes_immediately(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_id = provision(&app, &app.token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    let context = app
        .client
        .contexts()
        .create("Never Used", None)
        .await
        .expect("admin creates an empty context");

    let (status, body) = delete_status(&app, &app.token, *context.id).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "nothing is homed here, so the guard has nothing to refuse: {body:?}"
    );

    let get_status = app
        .reqwest_client
        .get(app.url(&format!("/api/contexts/{}", *context.id)))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("get request")
        .status();
    assert_eq!(
        get_status,
        StatusCode::NOT_FOUND,
        "the deleted context is no longer readable"
    );

    // Same shape, through the CLI, on a second empty context.
    let context2 = app
        .client
        .contexts()
        .create("Also Never Used", None)
        .await
        .expect("admin creates a second empty context");
    let (cli_ok, cli_out) = cli_delete(&app, &app.token, *context2.id).await;
    assert!(
        cli_ok,
        "the CLI delete must succeed on an empty context: {cli_out}"
    );
}
