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
    let cli_command = details["cli_command"]
        .as_str()
        .expect("cli_command should be present");
    assert_advertised_command_is_runnable(cli_command);
}

/// Asserts the remedy the server just put on the wire is a command the CLI can
/// actually run.
///
/// `is_some()` was the whole assertion here once, and under it the 403 shipped
/// advertising `temper team join --message "..."` — a real command that takes a
/// positional invitation token and has no `--message`, so a user following the
/// error accepted an invite instead of requesting access. Presence was never the
/// property worth checking; runnability is. This is the end-to-end half of the
/// pin (temper-cli's `access_gate` unit test covers the constant itself), and it
/// is the only place the *wire value* is checked against the *real parser*.
fn assert_advertised_command_is_runnable(advertised: &str) {
    use clap::Parser;

    let argv = shlex::split(advertised)
        .unwrap_or_else(|| panic!("403 advertises `{advertised}`, which is not shell-splittable"));

    if let Err(err) = temper_cli::cli::Cli::try_parse_from(&argv) {
        panic!("403 advertises `{advertised}`, but the CLI cannot parse it: {err}");
    }
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

    // `enable_invite_only` above created the `temper-system` gating team; resolve
    // it by slug and add the second user as a watcher (substrate `kb_team_members`
    // is keyed on `(team_id, profile_id)` — no surrogate id / `joined_at`).
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role)
         SELECT id, $1, 'watcher' FROM kb_teams WHERE slug = 'temper-system'
         ON CONFLICT (team_id, profile_id) DO NOTHING",
    )
    .bind(second_id)
    .execute(&pool)
    .await
    .expect("add second user as watcher");
    // D11: gating-team membership no longer confers `has_system_access` (which reads standing). Grant
    // the watcher approved standing — the front door — WITHOUT governance, so it reaches gated routes
    // yet is still blocked from the admin surface below.
    common::approve(&pool, second_id).await;

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

    // Check the audit trail. Post-WS6-collapse, admin/operational events are
    // firewalled OUT of the cognition ledger `kb_events` (which is now
    // cognition-only: entity emitters, no `profile_id` column, and carries no
    // `join_request.*` event types). The durable audit for the join-request
    // lifecycle lives on the `kb_join_requests` row itself — status plus
    // reviewer attribution (`reviewed_by_profile_id` / `reviewed_at`). See
    // `crates/temper-api/src/services/access_service.rs:4-9` (a dedicated
    // admin-event sink is a future deliverable). The query repoints there: the
    // submit→approve lifecycle is recorded as the request reaching `approved`
    // with the reviewing admin attributed and a decision timestamp stamped.
    let (status, reviewer, reviewed): (String, Option<uuid::Uuid>, bool) = sqlx::query_as(
        "SELECT status::text, reviewed_by_profile_id, (reviewed_at IS NOT NULL) \
         FROM kb_join_requests \
         WHERE requesting_profile_id = $1 ORDER BY created DESC LIMIT 1",
    )
    .bind(second_id)
    .fetch_one(&pool)
    .await
    .expect("join-request audit row must exist for the lifecycle");

    assert_eq!(
        status, "approved",
        "the submit→approve lifecycle must be recorded on the request row"
    );
    assert_eq!(
        reviewer,
        Some(admin_id),
        "approval must attribute the reviewing admin (audit trail)"
    );
    assert!(reviewed, "approval must stamp reviewed_at (audit trail)");
}

// ---------------------------------------------------------------------------
// The 403 guidance block is one stream: stderr
// ---------------------------------------------------------------------------

/// The access-gate 403 must render **entirely on stderr**, leaving stdout clean.
///
/// temper defaults to JSON on a non-TTY stdout (how agents invoke it), so any
/// prose written there corrupts the document a caller parses. Before this gate
/// existed the block was split: `output::error` went to stderr while the
/// explanatory line, its spacing, and the `output::hint` remedy went to stdout
/// — so an agent redirecting stderr away got a stdout carrying neither valid
/// JSON nor the reason why.
///
/// This asserts both halves of the invariant, because either alone is passable
/// by a broken implementation: the guidance must be *present on stderr* (a
/// renderer that printed nothing would pass a stdout-only check) and *absent
/// from stdout* (the actual regression). Both strings below lived on stdout
/// before the fix, so this test goes red against it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn access_gate_403_renders_entirely_on_stderr(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_profile = preflight(&app, &app.token).await;
    common::enable_invite_only(&pool, profile_id(&admin_profile)).await;

    // A second, non-member user: authenticated, but gated out.
    let second_token = common::generate_second_user_jwt();
    preflight(&app, &second_token).await;

    let config_toml = toml::to_string(&app.config).expect("serialize test TemperConfig to TOML");
    let config_path = app.vault_dir.path().join("gate-stream-config.toml");
    std::fs::write(&config_path, config_toml).expect("write test config for CLI invocation");

    let output = common::run_temper_cli_with_token(
        &app.base_url(),
        &second_token,
        &config_path,
        &["resource", "list", "--type", "task"],
    )
    .await
    .expect("spawn the temper binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "a gated caller must exit non-zero; got success with stdout={stdout:?} stderr={stderr:?}"
    );

    // The guidance must actually be rendered — on stderr.
    assert!(
        stderr.contains("requires approved access"),
        "the 403 explanation must reach stderr; stderr={stderr:?}"
    );
    assert!(
        stderr.contains("To request access, run:"),
        "the remedy's lead-in must reach stderr; stderr={stderr:?}"
    );
    assert!(
        stderr.contains("temper auth request-access"),
        "the advertised remedy command must reach stderr; stderr={stderr:?}"
    );

    // ...and stdout must carry none of it. These are the exact lines that were
    // written to stdout before the fix.
    for leaked in [
        "To request access, run:",
        "temper auth request-access",
        "requires approved access",
    ] {
        assert!(
            !stdout.contains(leaked),
            "the 403 block leaked {leaked:?} onto stdout, which agents parse as JSON; \
             stdout={stdout:?}"
        );
    }
    assert!(
        stdout.trim().is_empty(),
        "a failed gated command must leave stdout empty for its parser; stdout={stdout:?}"
    );
}

// ---------------------------------------------------------------------------
// `temper init` renders the enriched 403 too
// ---------------------------------------------------------------------------

/// `temper init` must route its gated 403 through the enriched renderer.
///
/// This is the *highest-value* instance of the access guidance, not the lowest:
/// `init` is the first command a new user runs against an invite-only instance,
/// and it is exactly where "how do I request access?" needs answering. Both
/// `/api/contexts` routes are in `gated_routes()`, so a gated caller genuinely
/// receives the enriched 403 here.
///
/// Before the fix, `ensure_server_contexts`'s catch-all arm did
/// `TemperError::Api(format!("list contexts: {e}"))`. `ClientError::SystemAccessRequired`'s
/// `Display` is the bare string `"system access required"`, so the rich arm in
/// `main.rs` never fired and the user got no email, no join-request status, and
/// no remedy command. Unlike the `resource list` case there was no sibling path
/// that rendered correctly — the preceding arms match only `NotAuthenticated`
/// and `TokenExpired`.
///
/// Note the invocation: `ensure_server_contexts` is reachable only when an
/// instance is configured (`AuthChoice::None` short-circuits it), so this drives
/// `init` with `--instance-url`/`--idp temper-as` rather than a bare
/// `--no-interactive`. A bare init never builds a client at all and would make
/// this test green for the wrong reason.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn init_gated_403_renders_enriched_guidance_on_stderr(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_profile = preflight(&app, &app.token).await;
    common::enable_invite_only(&pool, profile_id(&admin_profile)).await;

    // A second, non-member user: authenticated, but gated out.
    let second_token = common::generate_second_user_jwt();
    preflight(&app, &second_token).await;

    let config_toml = toml::to_string(&app.config).expect("serialize test TemperConfig to TOML");
    let config_path = app.vault_dir.path().join("init-gate-config.toml");
    std::fs::write(&config_path, config_toml).expect("write test config for CLI invocation");

    let init_vault = app.vault_dir.path().join("init-gate-vault");
    let init_vault_arg = init_vault.to_string_lossy().to_string();
    let base_url = app.base_url();

    let output = common::run_temper_cli_with_token(
        &base_url,
        &second_token,
        &config_path,
        &[
            "init",
            "--no-interactive",
            &init_vault_arg,
            "--instance-url",
            &base_url,
            "--idp",
            "temper-as",
        ],
    )
    .await
    .expect("spawn the temper binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "a gated `init` must exit non-zero; got success with stdout={stdout:?} stderr={stderr:?}"
    );

    // The enriched guidance must actually be rendered — and on stderr.
    assert!(
        stderr.contains("requires approved access"),
        "the 403 explanation must reach stderr; stderr={stderr:?}"
    );
    assert!(
        stderr.contains("To request access, run:"),
        "the remedy's lead-in must reach stderr; stderr={stderr:?}"
    );
    assert!(
        stderr.contains("temper auth request-access"),
        "the advertised remedy command must reach stderr; stderr={stderr:?}"
    );

    // The flattened form is what this test exists to forbid. Asserting its
    // absence pins the *mechanism*, not just the symptom: if someone reverts to
    // `TemperError::Api(format!(...))` the enriched assertions above go red, but
    // this one names why.
    assert!(
        !stderr.contains("list contexts: system access required"),
        "the gated error must not be flattened to a bare string; stderr={stderr:?}"
    );

    for leaked in [
        "To request access, run:",
        "temper auth request-access",
        "requires approved access",
    ] {
        assert!(
            !stdout.contains(leaked),
            "the 403 block leaked {leaked:?} onto stdout, which agents parse as JSON; \
             stdout={stdout:?}"
        );
    }

    // A gated `init` renders no `InitSummary`, so stdout has no payload to carry
    // and must be completely empty. This also pins the config-already-exists
    // notice to stderr: it fires before the gate is reached, so before that line
    // moved to `dim_err` this assertion caught it.
    assert!(
        stdout.trim().is_empty(),
        "a failed gated `init` emits no payload, so stdout must stay empty; stdout={stdout:?}"
    );
}

// ---------------------------------------------------------------------------
// `init`'s payload stream carries a payload, and nothing else
// ---------------------------------------------------------------------------

/// A successful `temper init --no-interactive` must emit **only** its JSON
/// payload on stdout.
///
/// This is the same class as the tests above but the opposite shape: `init`
/// genuinely *has* a payload — `run_non_interactive` renders an `InitSummary`
/// and `println!`s it — so the fix is not "move everything to stderr" but "keep
/// the prose off the payload's stream". Before the fix `output::dim`'s
/// config-location notice and `output::success`'s closing line were written to
/// stdout on either side of that JSON document, so `--format json` produced
/// something no parser accepts.
///
/// Parsing the whole of stdout, rather than grepping for the prose strings, is
/// deliberate: it states the actual invariant (stdout *is* a JSON document) and
/// so catches any future prose line, not just the two that were there.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn init_emits_only_its_json_payload_on_stdout(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    preflight(&app, &app.token).await;

    let config_toml = toml::to_string(&app.config).expect("serialize test TemperConfig to TOML");
    let config_path = app.vault_dir.path().join("init-json-config.toml");
    std::fs::write(&config_path, config_toml).expect("write test config for CLI invocation");

    let init_vault = app.vault_dir.path().join("init-json-vault");
    let init_vault_arg = init_vault.to_string_lossy().to_string();

    let output = common::run_temper_cli_with_token(
        &app.base_url(),
        &app.token,
        &config_path,
        &[
            "--format",
            "json",
            "init",
            "--no-interactive",
            &init_vault_arg,
        ],
    )
    .await
    .expect("spawn the temper binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "init should succeed; stdout={stdout:?} stderr={stderr:?}"
    );

    let parsed: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!(
            "stdout must be exactly one JSON document, but did not parse ({e}); stdout={stdout:?}"
        )
    });
    assert!(
        parsed["vault_path"].is_string(),
        "the parsed payload should be the InitSummary; parsed={parsed:?}"
    );

    // The prose belongs on stderr — assert it is actually there, so this cannot
    // be satisfied by a build that simply stopped printing it.
    assert!(
        stderr.contains("Temper initialized successfully"),
        "the closing confirmation must reach stderr; stderr={stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// `auth request-access` is prose, and prose does not belong on stdout
// ---------------------------------------------------------------------------

/// `temper auth request-access` must leave stdout empty.
///
/// The success block here is *entirely* prose — a confirmation line, an
/// explanation, a hint, and a dimmed request id. `request_access` takes no
/// `fmt` parameter and never calls `format::render`, so there is no payload on
/// stdout to protect: on a non-TTY stdout (the agent default, JSON) the old code
/// emitted four lines of prose onto the stream a caller parses.
///
/// This one was initially judged correct-as-is during PR #486, on the reading
/// that it was payload-on-stdout / guidance-on-stderr. That reading was wrong.
/// "Payload on stdout, guidance on stderr" is only a defense when a payload
/// actually exists — check for the `fmt`/`render` call before invoking it.
///
/// The `blank()` was the tell: it was written to separate the hint from the
/// request id, but went to a different stream than the hint, so the spacing was
/// wrong on a terminal and the block fragmented on redirect.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn auth_request_access_leaves_stdout_empty(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_profile = preflight(&app, &app.token).await;
    common::enable_invite_only(&pool, profile_id(&admin_profile)).await;

    let second_token = common::generate_second_user_jwt();
    preflight(&app, &second_token).await;

    let config_toml = toml::to_string(&app.config).expect("serialize test TemperConfig to TOML");
    let config_path = app.vault_dir.path().join("request-access-config.toml");
    std::fs::write(&config_path, config_toml).expect("write test config for CLI invocation");

    let output = common::run_temper_cli_with_token(
        &app.base_url(),
        &second_token,
        &config_path,
        &["auth", "request-access"],
    )
    .await
    .expect("spawn the temper binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "requesting access as a gated user should succeed; stdout={stdout:?} stderr={stderr:?}"
    );

    // Present on stderr: a renderer that printed nothing at all would sail
    // through a stdout-only check, so assert both halves.
    assert!(
        stderr.contains("Access request submitted."),
        "the confirmation must reach stderr; stderr={stderr:?}"
    );
    assert!(
        stderr.contains("Request ID:"),
        "the request id must reach stderr; stderr={stderr:?}"
    );

    assert!(
        stdout.trim().is_empty(),
        "`auth request-access` has no payload — every line it prints is prose, so stdout \
         must stay empty for the parser; stdout={stdout:?}"
    );
}

// ---------------------------------------------------------------------------
// 10. Whole-surface born-Denied guard (Task 11 step 5)
// ---------------------------------------------------------------------------

/// A principal's standing, resolved by the profile's email. `None` if there is no standing row.
async fn standing_of_email(pool: &sqlx::PgPool, email: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT s.state FROM kb_principal_standing s \
         JOIN kb_profiles p ON p.id = s.profile_id WHERE p.email = $1",
    )
    .bind(email)
    .fetch_optional(pool)
    .await
    .expect("standing by email")
}

/// D11 / Task 11 step 5 — the whole-surface born-Denied property. NO mint door, driven through the
/// real server under any actor, yields an `approved` principal: every door births `Denied`. This is
/// the belt-and-suspenders guard that a carelessly-added door — a new surface, a new registration
/// path, an admin-minted machine — can never silently confer access. It exercises the two live door
/// families end-to-end: a human OAuth first login, and a machine minted by the strongest actor there
/// is (a system admin).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn no_provision_path_under_any_actor_yields_approved(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Door 1 — human OAuth first login. A brand-new principal (NOT the centrally-approved app user).
    let fresh = common::generate_test_jwt("fresh-oauth-signup", "fresh-oauth@test.example.com");
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .bearer_auth(&fresh)
        .send()
        .await
        .expect("first login");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "first login provisions the profile"
    );
    assert_eq!(
        standing_of_email(&pool, "fresh-oauth@test.example.com")
            .await
            .as_deref(),
        Some("denied"),
        "human OAuth first login must birth Denied, never approved",
    );

    // Door 2 — machine issue, minted by the STRONGEST actor: a system admin. Even that yields no
    // access. Make the app principal an admin so it may mint a teamless machine (admin-only).
    let admin_id: uuid::Uuid =
        sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM kb_profiles WHERE email = $1")
            .bind("e2e@test.example.com")
            .fetch_one(&pool)
            .await
            .expect("app principal id");
    common::approved_admin(&pool, admin_id).await;

    let resp = app
        .reqwest_client
        .post(app.url("/api/machine-clients/issue"))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&serde_json::json!({
            "label": "born-denied probe",
            "owner_team_id": null,
            "teams": [],
            "grants": [],
        }))
        .send()
        .await
        .expect("issue request");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "a system admin may issue a teamless machine: {:?}",
        resp.text().await
    );

    // The just-minted machine — under the strongest possible minting authority — holds no approved
    // standing. No machine principal anywhere in this fresh instance is born approved.
    let any_machine_approved: bool = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS( \
           SELECT 1 FROM kb_machine_clients m \
           JOIN kb_principal_standing s ON s.profile_id = m.profile_id \
           WHERE s.state = 'approved')",
    )
    .fetch_one(&pool)
    .await
    .expect("machine approved check");
    assert!(
        !any_machine_approved,
        "no minted machine may be born approved — every door births Denied (D11)",
    );
}
