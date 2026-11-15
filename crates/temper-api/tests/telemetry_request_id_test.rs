#![cfg(feature = "test-db")]
//! The request id is the operator's join key: what a caller quotes from `x-request-id` must be the
//! `request_id` on the `http_request` root span the refusal was logged under.
//!
//! The outer service mints the id before routing, and the root span is built inside routing, so the
//! join holds only if the id reaches the span as a request extension and the span records it. Both
//! halves are checked against what the exporter received, through the real router. Its own test
//! binary, because the provider and the subscriber are process-global (see `telemetry_link_test`).

mod common;

use opentelemetry_sdk::trace::InMemorySpanExporter;
use sqlx::PgPool;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

fn attribute(span: &opentelemetry_sdk::trace::SpanData, key: &str) -> Option<String> {
    span.attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| kv.value.as_str().into_owned())
}

// FAILS IF: the root span does not carry the `request_id` the response returned (the extension is
// planted after routing, or the span never records it), or it records the caller's own
// `x-request-id` unbounded, or as the request's id.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_root_span_carries_the_returned_request_id(pool: PgPool) {
    let exporter = InMemorySpanExporter::default();
    assert!(
        temper_telemetry::export::install_test_provider(exporter.clone()),
        "a provider was already installed — this test owns the process"
    );
    let layer = temper_telemetry::export::test_export_layer()
        .expect("the layer must exist once a provider is installed");
    tracing_subscriber::registry().with(layer).init();

    let app = common::setup_test_app(pool).await;
    let inbound = "c".repeat(300);
    let response = app
        .client
        .get(app.url("/api/profile"))
        .header("x-request-id", &inbound)
        .send()
        .await
        .expect("request sent");
    assert_eq!(response.status().as_u16(), 401);
    let returned = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .expect("x-request-id returned")
        .to_string();

    let spans = exporter.get_finished_spans().expect("exporter readable");
    let span = spans
        .iter()
        .find(|s| attribute(s, "path").as_deref() == Some("/api/profile"))
        .unwrap_or_else(|| panic!("no root span for /api/profile: {spans:#?}"));
    assert_eq!(
        attribute(span, "request_id").as_deref(),
        Some(returned.as_str()),
        "the span's request_id is not the one returned: {span:#?}"
    );
    assert_eq!(
        attribute(span, "inbound_request_id").as_deref(),
        Some(&inbound[..128]),
        "the caller's id is kept bounded to 128 bytes: {span:#?}"
    );
}
