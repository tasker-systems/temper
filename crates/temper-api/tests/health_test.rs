#![cfg(feature = "test-db")]

mod common;

use serde_json::Value;
use sqlx::PgPool;

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_health_check(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let resp = app
        .client
        .get(app.url("/api/health"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status().as_u16(), 200, "expected 200 OK");

    let body: Value = resp.json().await.expect("expected JSON body");
    assert_eq!(body["status"], "ok", "status field should be 'ok'");
}
