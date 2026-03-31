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
