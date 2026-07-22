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

/// Provision a profile WITHOUT approving it — so it stays born-`Denied` (D11). `/api/profile` is on
/// the auth-only router, so a denied principal can still reach it and get JIT-provisioned.
async fn provision_unapproved(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("json");
    body["id"].as_str().expect("id").parse().expect("uuid")
}

/// The current `kb_principal_standing.state` for a profile (`None` ⇒ no row).
async fn standing_of(pool: &sqlx::PgPool, profile_id: Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT state FROM kb_principal_standing WHERE profile_id = $1")
        .bind(profile_id)
        .fetch_optional(pool)
        .await
        .expect("standing query")
}

/// POST one of the standing acts as the app-token admin, asserting `200 OK`.
async fn admin_act(app: &common::E2eTestApp, profile_id: Uuid, verb: &str, body: Option<Value>) {
    let mut req = app
        .reqwest_client
        .post(app.url(&format!("/api/access/admin/principals/{profile_id}/{verb}")))
        .header("Authorization", format!("Bearer {}", app.token));
    if let Some(body) = body {
        req = req.json(&body);
    }
    let resp = req.send().await.expect("admin act");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "{verb} should succeed for an admin"
    );
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

    // First admin demotes the second via the CLI (the manual governance twin of `promote`).
    let out = common::run_temper_cli(&app, &["admin", "demote", &second_id.to_string()])
        .await
        .expect("cli demote");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The demoted user is no longer a system admin: admin settings → 403. Governance is gone; the
    // standing (access) it was granted on promotion is untouched — demotion is governance-only.
    let resp = app
        .reqwest_client
        .get(app.url("/api/access/admin/settings"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("demoted admin settings");
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "a demoted admin no longer governs"
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

    // POST demote → 403. Service-gated (F-3), so the gate fires the same as the handler-gated ones.
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/admin/demote"))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&json!({"profile_id": admin_id}))
        .send()
        .await
        .expect("demote");
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

    // The standing acts (Task 13) are admin-only too. A valid revoke body is sent so the JSON
    // extractor succeeds and it is the handler's admin gate — not a 4xx from a missing body — that
    // produces the 403.
    for (verb, body) in [
        ("approve", None),
        ("revoke", Some(json!({"reason": "no"}))),
        ("deactivate", None),
        ("reactivate", None),
    ] {
        let mut req = app
            .reqwest_client
            .post(app.url(&format!("/api/access/admin/principals/{admin_id}/{verb}")))
            .header("Authorization", format!("Bearer {second_token}"));
        if let Some(body) = body {
            req = req.json(&body);
        }
        let resp = req.send().await.expect("standing act");
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{verb} must be admin-only"
        );
    }
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn admin_can_approve_a_born_denied_principal(pool: sqlx::PgPool) {
    // D14 — Approve is legal from `Denied`. A machine (or any principal admitted directly rather
    // than via a join request) is born `Denied` and can never `Request`; without Approve-from-Denied
    // the entire direct-grant surface is a dead end.
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    let subject_token = common::generate_second_user_jwt();
    let subject_id = provision_unapproved(&app, &subject_token).await;
    assert_eq!(
        standing_of(&pool, subject_id).await.as_deref(),
        Some("denied"),
        "a fresh principal is born Denied (D11)"
    );

    admin_act(&app, subject_id, "approve", None).await;

    assert_eq!(
        standing_of(&pool, subject_id).await.as_deref(),
        Some("approved"),
        "a direct approve admits a born-Denied principal (D14)"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reactivate_restores_the_prior_state_not_approved(pool: sqlx::PgPool) {
    // Spec §5. A principal deactivated while `Revoked` must come back `Revoked`, not `Approved` —
    // the failure mode is silently upgrading someone during a deactivation round-trip. This drives
    // the full admin chain (approve → revoke → deactivate → reactivate) through the real endpoints.
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    let subject_token = common::generate_second_user_jwt();
    let subject_id = provision(&app, &subject_token).await; // born Denied, then approved by provision

    admin_act(
        &app,
        subject_id,
        "revoke",
        Some(json!({"reason": "policy"})),
    )
    .await;
    assert_eq!(
        standing_of(&pool, subject_id).await.as_deref(),
        Some("revoked")
    );

    admin_act(&app, subject_id, "deactivate", None).await;
    assert_eq!(
        standing_of(&pool, subject_id).await.as_deref(),
        Some("deactivated")
    );

    admin_act(&app, subject_id, "reactivate", None).await;
    assert_eq!(
        standing_of(&pool, subject_id).await.as_deref(),
        Some("revoked"),
        "reactivation restores the prior state, it does not guess (§5)"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn revoke_requires_a_reason(pool: sqlx::PgPool) {
    // `--reason` is required, and that friction is a feature (D15): a later review's reviewer needs
    // it. A missing body is a 4xx (the JSON extractor rejects it), never a silent reasonless revoke.
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;
    let subject_id = provision(&app, &common::generate_second_user_jwt()).await;

    let resp = app
        .reqwest_client
        .post(app.url(&format!("/api/access/admin/principals/{subject_id}/revoke")))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("revoke without body");
    assert!(
        resp.status().is_client_error(),
        "a reasonless revoke is refused, got {}",
        resp.status()
    );
    // Standing is untouched — the refused request wrote nothing.
    assert_eq!(
        standing_of(&pool, subject_id).await.as_deref(),
        Some("approved")
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cli_admin_access_round_trips_approve_and_revoke(pool: sqlx::PgPool) {
    // The CLI surface end to end: arg parsing → AdminClient → API → standing. Full MCP+API+CLI
    // parity is always intended, so the operator's actual command is exercised, not just the HTTP.
    let app = common::setup(pool.clone()).await;
    let admin_id = provision(&app, &app.token).await;
    root_bootstrap_first_admin(&pool, admin_id).await;

    let subject_id = provision_unapproved(&app, &common::generate_second_user_jwt()).await;
    assert_eq!(
        standing_of(&pool, subject_id).await.as_deref(),
        Some("denied")
    );

    let out = common::run_temper_cli(
        &app,
        &["admin", "access", "approve", &subject_id.to_string()],
    )
    .await
    .expect("cli approve");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        standing_of(&pool, subject_id).await.as_deref(),
        Some("approved")
    );

    let out = common::run_temper_cli(
        &app,
        &[
            "admin",
            "access",
            "revoke",
            &subject_id.to_string(),
            "--reason",
            "cli test",
        ],
    )
    .await
    .expect("cli revoke");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        standing_of(&pool, subject_id).await.as_deref(),
        Some("revoked")
    );
}
