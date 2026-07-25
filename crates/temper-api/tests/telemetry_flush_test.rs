#![cfg(feature = "test-db")]
//! The flush-ordering gate.
//!
//! One test, in its own file on purpose: it installs a **process-global** tracing subscriber and a
//! process-global tracer provider, neither of which can be installed twice. Each integration-test
//! file is its own binary, and nextest gives each test its own process, so this one owns both.
//!
//! ## What is actually being verified, and why reading the code was not enough
//!
//! A batch processor can only export a span that has **ended**. So the flush is correct only if the
//! request's root span is closed by the time it runs — and that is a property of who *owns* the
//! span, not of where a layer sits.
//!
//! The first attempt used `tower_http`'s `TraceLayer` with the flush in an outermost middleware,
//! reasoning that the last `.layer` is the outermost and therefore runs after the span closes. That
//! reasoning was wrong, and this test is what proved it: the exporter held **zero** spans when the
//! response reached the client, while a manual flush one statement later produced one.
//!
//! The cause is that `TraceLayer` clones its span into the *response body*
//! (`tower-http-0.7.0/src/trace/body.rs:25`), so the span outlives every middleware in the chain
//! and no layer position can ever see it closed. `temper_telemetry::request_span` now owns the span
//! and drops it explicitly before flushing.
//!
//! The failure this guards against is the worst kind: everything looks healthy, requests are fast,
//! the exporter is configured, and no spans ever arrive.
//!
//! ## Why the provider batches
//!
//! With a `SimpleSpanProcessor` a span exports when it closes, so this test would pass under either
//! ordering and prove nothing. `install_test_provider` uses a **batch** processor — the same one
//! production uses — so a span that is never flushed stays in the queue and never reaches the
//! exporter. That is what gives the assertion below the ability to fail.

mod common;

use opentelemetry_sdk::trace::InMemorySpanExporter;
use sqlx::PgPool;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn flush_ordering_exports_the_request_span(pool: PgPool) {
    let exporter = InMemorySpanExporter::default();
    assert!(
        temper_telemetry::export::install_test_provider(exporter.clone()),
        "a provider was already installed — this test owns the process"
    );
    let layer = temper_telemetry::export::test_export_layer()
        .expect("the layer must exist once a provider is installed");
    tracing_subscriber::registry().with(layer).init();

    let app = common::setup_test_app(pool).await;

    // A public route: this is about the transport layers, not about auth.
    let response = app
        .client
        .get(app.url("/api/health"))
        .send()
        .await
        .expect("health request");
    assert_eq!(response.status().as_u16(), 200);

    // No sleep, no retry, deliberately. The flush happens inside the middleware *before* the
    // response is written to the socket, so by the time the client has a response the export has
    // already happened. If this needs a sleep to pass, the flush is not where it claims to be.
    let exported = exporter
        .get_finished_spans()
        .expect("in-memory exporter readable");

    assert!(
        exported.iter().any(|span| span.name == "http_request"),
        "the request's root span was never exported. Either the flush runs while the root span is \
         still open — so it drains a queue the span has not reached — or it is not wired at all. \
         Exported spans: {:?}",
        exported.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}
