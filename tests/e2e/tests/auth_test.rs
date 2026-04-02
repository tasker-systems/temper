#![cfg(feature = "test-db")]

mod common;

use reqwest::StatusCode;

/// Request without auth header returns 401.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn no_auth_returns_401(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Request with expired JWT returns 401.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn expired_jwt_returns_401(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    let expired = common::generate_expired_jwt("expired-user", "expired@test.example.com");

    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {expired}"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Request with valid JWT succeeds (200).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn valid_jwt_returns_200(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), StatusCode::OK);
}
