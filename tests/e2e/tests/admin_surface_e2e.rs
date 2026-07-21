#![cfg(feature = "test-db")]

mod common;

use reqwest::StatusCode;
use serde_json::{json, Value};
use uuid::Uuid;

/// Provision a profile by hitting an authed endpoint (auto-provision on first request).
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
    // D11: a fresh principal is born Denied. Approve so this actor clears the front door
    // and the ENDPOINT authz (ownership, admin-only, grants) is what the test exercises.
    let __pid: Uuid = body["id"].as_str().expect("id").parse().expect("uuid");
    common::approve(&app.pool, __pid).await;
    __pid
}

/// The irreducible 2-UPDATE operator root step: configure gating + mint first admin.
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
    sqlx::query("UPDATE kb_profiles SET system_access='admin' WHERE id=$1")
        .bind(admin_id)
        .execute(pool)
        .await
        .expect("promote first admin"); // trigger mints owner of temper-system
                                        // D11: is_system_admin reads governance, has_system_access reads standing; the column + gating
                                        // ownership above confer neither. Grant both so the bootstrapped admin can actually act.
    common::approved_admin(pool, admin_id).await;
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn admin_can_set_settings_and_promote_second_admin(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    let second_token = common::generate_second_user_jwt();
    let second_id = provision(&app, &second_token).await;

    root_bootstrap_first_admin(&pool, admin_id).await;

    // First admin sets an instance name via the CLI (runs as app.token).
    let out = common::run_temper_cli(
        &app,
        &[
            "admin",
            "settings",
            "--instance-name",
            "Acme Temper",
            "--format",
            "json",
        ],
    )
    .await
    .expect("cli settings");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let settings: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(settings["instance_name"], "Acme Temper");

    // Read-back round-trip via CLI (no flags ⇒ show).
    let out = common::run_temper_cli(&app, &["admin", "settings", "--format", "json"])
        .await
        .expect("cli show");
    let shown: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(shown["instance_name"], "Acme Temper");
    assert_eq!(shown["gating_team_slug"], "temper-system");

    // First admin promotes the second admin via the CLI (default = gating team).
    let out = common::run_temper_cli(
        &app,
        &[
            "admin",
            "promote",
            &second_id.to_string(),
            "--format",
            "json",
        ],
    )
    .await
    .expect("cli promote");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The second user is now a system admin: can read admin settings (200).
    let resp = app
        .reqwest_client
        .get(app.url("/api/access/admin/settings"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("second admin settings");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "promoted admin should have access"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_admin_is_forbidden_on_all_admin_endpoints(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    let second_token = common::generate_second_user_jwt();
    let second_id = provision(&app, &second_token).await;

    root_bootstrap_first_admin(&pool, admin_id).await;

    // Second user is a watcher member (has system access, NOT admin).
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) \
         SELECT id, $1, 'watcher' FROM kb_teams WHERE slug='temper-system' \
         ON CONFLICT (team_id, profile_id) DO NOTHING",
    )
    .bind(second_id)
    .execute(&pool)
    .await
    .expect("watcher");

    // GET admin settings → 403 FORBIDDEN.
    let resp = app
        .reqwest_client
        .get(app.url("/api/access/admin/settings"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("get");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: Value = resp.json().await.expect("json");
    assert_eq!(body["error"]["code"], "FORBIDDEN");

    // PATCH admin settings → 403.
    let resp = app
        .reqwest_client
        .patch(app.url("/api/access/admin/settings"))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&json!({"instance_name": "hijack"}))
        .send()
        .await
        .expect("patch");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // POST promote → 403.
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/admin/promote"))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&json!({"profile_id": admin_id}))
        .send()
        .await
        .expect("promote");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // GET admin requests → 403 (admin gate fires before list logic).
    let resp = app
        .reqwest_client
        .get(app.url("/api/access/admin/requests"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("get requests");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // PATCH admin request review → 403 (admin gate fires before request lookup).
    let resp = app
        .reqwest_client
        .patch(app.url(&format!("/api/access/admin/requests/{}", Uuid::new_v4())))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&json!({"status": "approved"}))
        .send()
        .await
        .expect("patch request");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
