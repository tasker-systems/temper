#![cfg(feature = "test-db")]

//! The steward tick's `cap` at the wire: a non-positive cap is a 400, and a cap past `i32` is
//! clamped, not wrapped into a negative `LIMIT` that Postgres answers with a 500.

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

const MACHINE_CLIENT: &str = "steward-cap-agent";

/// A registered machine with approved standing; returns its bearer. The steward runs as one.
async fn approved_machine(pool: &PgPool) -> String {
    let profile = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, $2, $2, NULL, '{}')",
    )
    .bind(profile)
    .bind(MACHINE_CLIENT)
    .execute(pool)
    .await
    .expect("seed machine profile");
    sqlx::query(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ($1, 'test', $2, $2)",
    )
    .bind(MACHINE_CLIENT)
    .bind(profile)
    .execute(pool)
    .await
    .expect("seed machine registration");
    sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1, 'approved')")
        .bind(profile)
        .execute(pool)
        .await
        .expect("approve machine");
    common::generate_machine_jwt(MACHINE_CLIENT)
}

async fn dispatch(app: &common::TestApp, token: &str, body: Value) -> (u16, Value) {
    let resp = app
        .client
        .post(app.url("/api/steward/dispatch"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .expect("request");
    let status = resp.status().as_u16();
    (status, resp.json().await.unwrap_or(Value::Null))
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_non_positive_steward_cap_is_a_bad_request(pool: PgPool) {
    let token = approved_machine(&pool).await;
    let app = common::setup_test_app(pool).await;
    for cap in [0, -1] {
        let (status, body) = dispatch(&app, &token, json!({ "cap": cap })).await;
        assert_eq!(status, 400, "cap {cap}: {body}");
        assert!(
            body["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("cap must be at least 1")),
            "cap {cap}: {body}"
        );
    }
}

/// `2147483648 as i32` is `-2147483648`. Before the clamp this body reached Postgres as a negative
/// `LIMIT` and answered 500.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_steward_cap_past_i32_is_clamped_not_a_server_error(pool: PgPool) {
    let token = approved_machine(&pool).await;
    let app = common::setup_test_app(pool).await;
    let (status, body) = dispatch(&app, &token, json!({ "cap": 2_147_483_648_i64 })).await;
    assert_eq!(status, 200, "{body}");
}
