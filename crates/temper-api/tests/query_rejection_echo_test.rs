#![cfg(feature = "test-db")]
//! `/api/query` reads its body through `CompositionBody`, whose rejection never repeats more than
//! a typo's worth of what the caller sent.
//!
//! Asserted through the HTTP door because the extractor's own tests cannot see the wiring: a
//! handler that went back to plain `Json<Composition>` would leave them green while the door
//! echoed the caller's string in full again.

mod common;

use sqlx::PgPool;

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_query_door_does_not_echo_an_unreadable_body(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("query-echo-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, _) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    // A string where the stage list belongs: serde's `invalid type` message quotes it whole.
    let huge = "z".repeat(100_000);
    let body = format!(r#"{{"outcome":{{"returns":[{{"stage":"a"}}]}},"stages":"{huge}"}}"#);

    let resp = app
        .client
        .post(app.url("/api/query"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .expect("query request failed");

    let status = resp.status();
    let text = resp.text().await.expect("rejection body");
    assert_eq!(
        status, 422,
        "an unreadable body is refused by the deserializer, not by auth or a later gate: {text:.200}"
    );
    assert!(
        text.len() < 2048,
        "the door repeated {} bytes of a 100 KB string",
        text.len()
    );
}
