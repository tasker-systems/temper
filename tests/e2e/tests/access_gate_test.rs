#![cfg(feature = "test-db")]

mod common;

use reqwest::StatusCode;
use serde_json::Value;

/// Helper: pre-flight a user by hitting GET /api/profile, returning the profile JSON.
async fn preflight(app: &common::E2eTestApp, token: &str) -> Value {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight request failed");

    assert_eq!(resp.status(), StatusCode::OK, "preflight should succeed");
    resp.json::<Value>().await.expect("preflight json parse")
}

/// Extract profile UUID from a profile response.
fn profile_id(profile: &Value) -> uuid::Uuid {
    profile["id"]
        .as_str()
        .expect("profile id missing")
        .parse::<uuid::Uuid>()
        .expect("profile id parse")
}

// ---------------------------------------------------------------------------
// 1. Open mode allows all authenticated users
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn open_mode_allows_all_authenticated_users(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    preflight(&app, &app.token).await;

    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// 2. Entitlements in open mode
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn entitlements_in_open_mode(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    let profile = preflight(&app, &app.token).await;

    assert_eq!(profile["entitlements"]["system_access"], true);
    assert_eq!(profile["entitlements"]["is_admin"], false);
    assert_eq!(profile["entitlements"]["join_request_status"], Value::Null);
}

// ---------------------------------------------------------------------------
// 3. System settings does not leak gating_team_slug
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn system_settings_no_slug_leak(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    preflight(&app, &app.token).await;

    let resp = app
        .reqwest_client
        .get(app.url("/api/access/settings"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("json parse");

    assert!(body.get("access_mode").is_some(), "should have access_mode");
    assert!(
        body.get("gating_team_slug").is_none(),
        "should NOT have gating_team_slug"
    );
}

// ---------------------------------------------------------------------------
// 4. Invite-only blocks non-members
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn invite_only_blocks_non_members(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_profile = preflight(&app, &app.token).await;
    let admin_id = profile_id(&admin_profile);

    common::enable_invite_only(&pool, admin_id).await;

    // Second user who is NOT a member
    let second_token = common::generate_second_user_jwt();
    preflight(&app, &second_token).await;

    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: Value = resp.json().await.expect("json parse");
    assert_eq!(body["error"]["code"], "SYSTEM_ACCESS_REQUIRED");
}

// ---------------------------------------------------------------------------
// 4b. Enriched 403 contains access details (no join request)
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn enriched_403_contains_access_details(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_profile = preflight(&app, &app.token).await;
    let admin_id = profile_id(&admin_profile);

    common::enable_invite_only(&pool, admin_id).await;

    let second_token = common::generate_second_user_jwt();
    preflight(&app, &second_token).await;

    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: Value = resp.json().await.expect("json parse");

    // Top-level error structure
    assert_eq!(body["error"]["code"], "SYSTEM_ACCESS_REQUIRED");
    assert!(
        body["error"]["message"].as_str().is_some(),
        "error.message should be present"
    );

    // Details
    let details = &body["error"]["details"];
    assert_eq!(details["access_mode"], "invite_only");
    assert!(
        details["email"].as_str().is_some(),
        "email should be present"
    );
    assert_eq!(
        details["join_request_status"],
        Value::Null,
        "join_request_status should be null when never requested"
    );
    assert!(
        details["request_url"].as_str().is_some(),
        "request_url should be present"
    );
    assert!(
        details["cli_command"].as_str().is_some(),
        "cli_command should be present"
    );
}

// ---------------------------------------------------------------------------
// 4c. Enriched 403 shows pending join request status
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn enriched_403_shows_pending_join_request_status(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_profile = preflight(&app, &app.token).await;
    let admin_id = profile_id(&admin_profile);

    common::enable_invite_only(&pool, admin_id).await;

    let second_token = common::generate_second_user_jwt();
    preflight(&app, &second_token).await;

    // Second user submits a join request
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&serde_json::json!({
            "source": "cli",
            "message": "Please let me in"
        }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Now hit a gated endpoint — should get 403 with pending status
    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: Value = resp.json().await.expect("json parse");

    assert_eq!(body["error"]["code"], "SYSTEM_ACCESS_REQUIRED");
    assert_eq!(
        body["error"]["details"]["join_request_status"], "pending",
        "should reflect pending join request"
    );
    assert_eq!(body["error"]["details"]["access_mode"], "invite_only");
}

// ---------------------------------------------------------------------------
// 5. Auth-only routes bypass gate
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn auth_only_routes_bypass_gate(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_profile = preflight(&app, &app.token).await;
    let admin_id = profile_id(&admin_profile);

    common::enable_invite_only(&pool, admin_id).await;

    let second_token = common::generate_second_user_jwt();
    preflight(&app, &second_token).await;

    // /api/profile should still work
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::OK);

    // /api/access/settings should still work
    let resp = app
        .reqwest_client
        .get(app.url("/api/access/settings"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::OK);

    // /api/access/requests/me should still work
    let resp = app
        .reqwest_client
        .get(app.url("/api/access/requests/me"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::OK);
}

// ---------------------------------------------------------------------------
// 6. Join request approval lifecycle
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn join_request_approval_lifecycle(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_profile = preflight(&app, &app.token).await;
    let admin_id = profile_id(&admin_profile);

    common::enable_invite_only(&pool, admin_id).await;

    let second_token = common::generate_second_user_jwt();
    preflight(&app, &second_token).await;

    // Second user submits a join request
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&serde_json::json!({
            "source": "cli",
            "message": "Please let me in"
        }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::CREATED);
    let request: Value = resp.json().await.expect("json parse");
    let request_id = request["id"].as_str().expect("request id");
    assert_eq!(request["status"], "pending");

    // Second user checks status — pending
    let resp = app
        .reqwest_client
        .get(app.url("/api/access/requests/me"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::OK);
    let own_request: Value = resp.json().await.expect("json parse");
    assert_eq!(own_request["status"], "pending");

    // Entitlements show pending
    let profile = preflight(&app, &second_token).await;
    assert_eq!(profile["entitlements"]["system_access"], false);
    assert_eq!(profile["entitlements"]["join_request_status"], "pending");

    // Still blocked on gated routes
    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Admin lists pending requests
    let resp = app
        .reqwest_client
        .get(app.url("/api/access/admin/requests"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::OK);
    let pending: Vec<Value> = resp.json().await.expect("json parse");
    assert!(
        pending.iter().any(|r| r["id"].as_str() == Some(request_id)),
        "admin should see the pending request"
    );

    // Admin approves
    let resp = app
        .reqwest_client
        .patch(app.url(&format!("/api/access/admin/requests/{request_id}")))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&serde_json::json!({
            "status": "approved",
            "decision_note": "Welcome aboard"
        }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::OK);
    let approved: Value = resp.json().await.expect("json parse");
    assert_eq!(approved["status"], "approved");

    // Second user can now access gated routes
    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::OK);

    // Entitlements show approved
    let profile = preflight(&app, &second_token).await;
    assert_eq!(profile["entitlements"]["system_access"], true);
    assert_eq!(profile["entitlements"]["join_request_status"], "approved");
}

// ---------------------------------------------------------------------------
// 7. Join request rejection allows resubmit
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn join_request_rejection_allows_resubmit(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_profile = preflight(&app, &app.token).await;
    let admin_id = profile_id(&admin_profile);

    common::enable_invite_only(&pool, admin_id).await;

    let second_token = common::generate_second_user_jwt();
    preflight(&app, &second_token).await;

    // Submit
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&serde_json::json!({ "source": "cli" }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::CREATED);
    let request: Value = resp.json().await.expect("json parse");
    let request_id = request["id"].as_str().unwrap();

    // Admin rejects
    let resp = app
        .reqwest_client
        .patch(app.url(&format!("/api/access/admin/requests/{request_id}")))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&serde_json::json!({ "status": "rejected" }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::OK);

    // User can submit a new request
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&serde_json::json!({ "source": "cli", "message": "Try again" }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::CREATED);
}

// ---------------------------------------------------------------------------
// 8. Withdraw allows resubmit
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn join_request_withdraw_allows_resubmit(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_profile = preflight(&app, &app.token).await;
    let admin_id = profile_id(&admin_profile);

    common::enable_invite_only(&pool, admin_id).await;

    let second_token = common::generate_second_user_jwt();
    preflight(&app, &second_token).await;

    // Submit
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&serde_json::json!({ "source": "cli" }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Withdraw
    let resp = app
        .reqwest_client
        .delete(app.url("/api/access/requests/me"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Resubmit
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&serde_json::json!({ "source": "web", "message": "Changed my mind" }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::CREATED);
}

// ---------------------------------------------------------------------------
// 9. Non-admin blocked from admin endpoints
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_admin_blocked_from_admin_endpoints(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_profile = preflight(&app, &app.token).await;
    let admin_id = profile_id(&admin_profile);

    common::enable_invite_only(&pool, admin_id).await;

    // Add second user as a watcher (has system access, but NOT admin)
    let second_token = common::generate_second_user_jwt();
    let second_profile = preflight(&app, &second_token).await;
    let second_id = profile_id(&second_profile);

    sqlx::query(
        "INSERT INTO kb_team_members (id, team_id, profile_id, role, joined_at)
         VALUES (gen_random_uuid(), $1::uuid, $2, 'watcher', now())
         ON CONFLICT (team_id, profile_id) DO NOTHING",
    )
    .bind(uuid::Uuid::parse_str(common::TEMPER_SYSTEM_TEAM_ID).unwrap())
    .bind(second_id)
    .execute(&pool)
    .await
    .expect("add second user as watcher");

    // Watcher can access gated routes
    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::OK);

    // But blocked from admin endpoints
    let resp = app
        .reqwest_client
        .get(app.url("/api/access/admin/requests"))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: Value = resp.json().await.expect("json parse");
    assert_eq!(body["error"]["code"], "FORBIDDEN");
}

// ---------------------------------------------------------------------------
// 10. Duplicate pending request returns conflict
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn duplicate_pending_request_returns_conflict(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_profile = preflight(&app, &app.token).await;
    let admin_id = profile_id(&admin_profile);

    common::enable_invite_only(&pool, admin_id).await;

    let second_token = common::generate_second_user_jwt();
    preflight(&app, &second_token).await;

    // First submit succeeds
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&serde_json::json!({ "source": "cli" }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Second submit should return CONFLICT
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&serde_json::json!({ "source": "cli" }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

// ---------------------------------------------------------------------------
// 11. Request in open mode returns bad request
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn request_in_open_mode_returns_bad_request(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    preflight(&app, &app.token).await;

    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&serde_json::json!({ "source": "cli" }))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// 12. Audit events written for lifecycle
// ---------------------------------------------------------------------------

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn audit_events_written_for_lifecycle(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_profile = preflight(&app, &app.token).await;
    let admin_id = profile_id(&admin_profile);

    common::enable_invite_only(&pool, admin_id).await;

    let second_token = common::generate_second_user_jwt();
    let second_profile = preflight(&app, &second_token).await;
    let second_id = profile_id(&second_profile);

    // Submit
    let resp = app
        .reqwest_client
        .post(app.url("/api/access/requests"))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&serde_json::json!({ "source": "cli" }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::CREATED);
    let request: Value = resp.json().await.expect("json parse");
    let request_id = request["id"].as_str().unwrap();

    // Admin approves
    let resp = app
        .reqwest_client
        .patch(app.url(&format!("/api/access/admin/requests/{request_id}")))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&serde_json::json!({ "status": "approved" }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::OK);

    // Check audit events
    let submitted_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kb_events WHERE profile_id = $1 AND event_type = 'join_request.submitted'",
    )
    .bind(second_id)
    .fetch_one(&pool)
    .await
    .expect("query submitted events");

    assert!(
        submitted_count >= 1,
        "should have join_request.submitted event"
    );

    let approved_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM kb_events WHERE profile_id = $1 AND event_type = 'join_request.approved'",
    )
    .bind(second_id)
    .fetch_one(&pool)
    .await
    .expect("query approved events");

    assert!(
        approved_count >= 1,
        "should have join_request.approved event"
    );
}
