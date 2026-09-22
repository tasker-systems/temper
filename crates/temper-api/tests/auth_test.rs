#![cfg(feature = "test-db")]

mod common;

use serde_json::Value;
use sqlx::PgPool;

/// GET /api/profile without an Authorization header must return 401.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_missing_auth_returns_401(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let resp = app
        .client
        .get(app.url("/api/profile"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "missing auth header must return 401"
    );
}

/// GET /api/profile with an expired JWT must return 401.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_expired_jwt_returns_401(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let token = common::generate_expired_jwt("expired-user-sub", "expired@example.com");

    let resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status().as_u16(), 401, "expired JWT must return 401");
}

/// GET /api/profile with a valid JWT for a brand-new user must:
/// - return 200
/// - auto-provision a profile
/// - set display_name to the email prefix
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_valid_jwt_auto_provisions_profile(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let sub = format!("test-sub-{}", uuid::Uuid::new_v4());
    let email = format!("autoprovision-{}@example.com", uuid::Uuid::new_v4());
    let token = common::generate_test_jwt(&sub, &email);

    let resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "valid JWT must return 200; body: {}",
        resp.text().await.unwrap_or_default()
    );

    let resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("second request failed");

    let body: Value = resp.json().await.expect("expected JSON body");

    // display_name should be derived from the email prefix.
    let expected_display = email.split('@').next().unwrap();
    assert_eq!(
        body["display_name"], expected_display,
        "display_name should be email prefix"
    );
    assert_eq!(
        body["email"].as_str().unwrap_or(""),
        email,
        "email field should match"
    );
}

// --- one-trust-domain: the accepted-audience set is one definition, shared by both doors ---
//
// These three pin `require_auth` accepting the same audience set the MCP middleware accepts
// (`AuthConfig::accepted_audiences`). The MCP-audience witness is the clause's test: the same
// validated bearer token authorizes identically at either door, and door choice is never an
// authorization input. Before the set was shared, `require_auth` validated the API audience
// alone and these tokens took a 401 here.

/// An instance running with a dedicated MCP audience accepts an `mcp_audience` token at the
/// HTTP door — the same token the MCP middleware already accepts.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_mcp_audience_token_authenticates_at_the_api_door(pool: PgPool) {
    const MCP_AUDIENCE: &str = "https://test.example/mcp";
    let app = common::setup_test_app_with_config(pool, |config| {
        config.auth.mcp_audience = MCP_AUDIENCE.to_string();
    })
    .await;

    let sub = format!("mcp-aud-sub-{}", uuid::Uuid::new_v4());
    let email = format!("mcp-aud-{}@example.com", uuid::Uuid::new_v4());
    let token = common::generate_test_jwt_with_audience(&sub, &email, MCP_AUDIENCE);

    let resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "an mcp_audience token must authenticate at the HTTP door; body: {}",
        resp.text().await.unwrap_or_default()
    );
}

/// Widening to the MCP audience must not drop the API audience: with the two distinct, a
/// token carrying the API audience still authenticates (no-door-regression).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_api_audience_token_still_authenticates_when_mcp_audience_is_distinct(pool: PgPool) {
    let app = common::setup_test_app_with_config(pool, |config| {
        config.auth.mcp_audience = "https://test.example/mcp".to_string();
    })
    .await;

    let sub = format!("api-aud-sub-{}", uuid::Uuid::new_v4());
    let email = format!("api-aud-{}@example.com", uuid::Uuid::new_v4());
    let token = common::generate_test_jwt_with_audience(&sub, &email, common::TEST_AUDIENCE);

    let resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "the API audience must stay accepted when an MCP audience is configured"
    );
}

/// Parity is not promiscuity: a token naming a third audience is refused at the HTTP door,
/// exactly as the MCP middleware refuses it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unknown_audience_is_still_refused_at_the_api_door(pool: PgPool) {
    let app = common::setup_test_app_with_config(pool, |config| {
        config.auth.mcp_audience = "https://test.example/mcp".to_string();
    })
    .await;

    let sub = format!("other-aud-sub-{}", uuid::Uuid::new_v4());
    let email = format!("other-aud-{}@example.com", uuid::Uuid::new_v4());
    let token = common::generate_test_jwt_with_audience(&sub, &email, "https://other.example");

    let resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "an audience neither door accepts must be refused at the HTTP door"
    );
}

/// Auto-provisioned profile must have a "default" context.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_auto_provisioned_profile_has_default_context(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let sub = format!("test-sub-{}", uuid::Uuid::new_v4());
    let email = format!("defaultctx-{}@example.com", uuid::Uuid::new_v4());
    let token = common::generate_test_jwt(&sub, &email);

    // Trigger auto-provisioning
    let resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);
    // D11: born Denied. Approve so the gated /api/contexts route admits this caller.
    common::fixtures::approve_standing_by_email(&app.pool, &email).await;

    // Check that the "default" context was created
    let resp = app
        .client
        .get(app.url("/api/contexts"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("contexts request failed");
    assert_eq!(resp.status().as_u16(), 200);

    let body: Vec<Value> = resp.json().await.expect("expected JSON array");
    let has_default = body.iter().any(|c| c["name"] == "default");
    assert!(
        has_default,
        "auto-provisioned profile must have a 'default' context; got: {:?}",
        body.iter().map(|c| c["name"].clone()).collect::<Vec<_>>()
    );
}
