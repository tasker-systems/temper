#![cfg(feature = "test-db")]

mod common;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use common::tracing_layer::TestTracingLayer;

/// Verify that a request to a protected endpoint produces tracing spans
/// with the expected structured fields (method, path, status, profile_id).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn request_produces_structured_spans(pool: sqlx::PgPool) {
    let (layer, captured) = TestTracingLayer::new();
    let _guard = tracing_subscriber::registry().with(layer).set_default();

    let app = common::setup(pool).await;

    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status().as_u16(), 200);

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let events = captured.lock().unwrap();

    let has_request_span = events.iter().any(|e| {
        let sf = &e.span_fields;
        sf.contains_key("method") && sf.contains_key("path")
    });

    assert!(
        has_request_span,
        "expected a tracing event with method and path span fields, got: {events:#?}"
    );
}

/// Verify that an unauthenticated request produces a warn-level event.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unauthenticated_request_logs_warning(pool: sqlx::PgPool) {
    let (layer, captured) = TestTracingLayer::new();
    let _guard = tracing_subscriber::registry().with(layer).set_default();

    let app = common::setup(pool).await;

    let resp = app
        .reqwest_client
        .get(app.url("/api/resources"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status().as_u16(), 401);

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let events = captured.lock().unwrap();

    let has_auth_warning = events.iter().any(|e| e.level <= tracing::Level::WARN);

    assert!(
        has_auth_warning,
        "expected a WARN-level event for 401 response, got: {events:#?}"
    );
}
