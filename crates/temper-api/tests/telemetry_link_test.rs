#![cfg(feature = "test-db")]
//! The span-link gate: decision `019f95ff` rule 2, end to end.
//!
//! *"Join a trusted caller's trace with an OTel span **link**, recorded after authentication."*
//!
//! `temper_telemetry::link`'s own tests pin what `link_trusted_caller` does when it is called. They
//! cannot pin the part that carries the security claim — that it is called **only** for a request
//! that authenticated. That is a property of where the call sits in the middleware stack, so it can
//! only be checked by driving real requests through the real router and reading what the exporter
//! received.
//!
//! ## Why both halves are one test
//!
//! The provider and the subscriber are process-global and install once. nextest gives each test its
//! own process, but `cargo test` does not, and a gate that silently degrades under one runner is
//! worse than one that is simply harder to read. Contrasting the two requests in one body is also
//! the honest shape of the claim: the same header, the same route shape, the same everything except
//! a bearer token — and exactly one of them gets linked.

mod common;

use opentelemetry_sdk::trace::InMemorySpanExporter;
use sqlx::PgPool;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// The W3C spec's canonical example, so the expected ids are checkable against the standard.
const SPEC_TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
const SPEC_TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";

/// Find the exported root span for one request, identified by its `path` attribute.
///
/// Matching on `path` rather than taking the first span is load-bearing here for the same reason it
/// is in the e2e convention gate: several requests run through this process, and a link assertion
/// that reads the wrong request's span passes for someone else's reasons.
fn exported_span(
    exporter: &InMemorySpanExporter,
    path: &str,
) -> opentelemetry_sdk::trace::SpanData {
    let spans = exporter
        .get_finished_spans()
        .expect("in-memory exporter readable");
    spans
        .iter()
        .find(|span| {
            span.attributes
                .iter()
                .any(|kv| kv.key.as_str() == "path" && kv.value.as_str() == path)
        })
        .unwrap_or_else(|| {
            panic!(
                "no exported span for path `{path}`; exported: {:?}",
                spans
                    .iter()
                    .map(|s| (&s.name, s.attributes.clone()))
                    .collect::<Vec<_>>()
            )
        })
        .clone()
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn only_an_authenticated_caller_gets_its_trace_linked(pool: PgPool) {
    let exporter = InMemorySpanExporter::default();
    assert!(
        temper_telemetry::export::install_test_provider(exporter.clone()),
        "a provider was already installed — this test owns the process"
    );
    let layer = temper_telemetry::export::test_export_layer()
        .expect("the layer must exist once a provider is installed");
    tracing_subscriber::registry().with(layer).init();

    let email = format!("link-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, _context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    let app = common::setup_test_app(pool).await;

    // Authenticated, carrying a caller's trace context.
    let authenticated = app
        .client
        .get(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .header("traceparent", SPEC_TRACEPARENT)
        .send()
        .await
        .expect("authenticated request");
    assert_eq!(authenticated.status().as_u16(), 200);

    // The same header, on a route behind the same auth gate, with no bearer at all.
    let anonymous = app
        .client
        .get(app.url("/api/profile"))
        .header("traceparent", SPEC_TRACEPARENT)
        .send()
        .await
        .expect("anonymous request");
    assert_eq!(anonymous.status().as_u16(), 401);

    // No sleep: the flush runs inside the transport layer before the response is written, which is
    // what `telemetry_flush_test` exists to guarantee.
    let linked = exported_span(&exporter, "/api/resources");
    let link = linked.links.iter().next().unwrap_or_else(|| {
        panic!("the authenticated request's root span carries no link: {linked:#?}")
    });
    assert_eq!(
        link.span_context.trace_id().to_string(),
        SPEC_TRACE_ID,
        "the link does not point at the caller's trace: {linked:#?}"
    );
    assert_ne!(
        linked.span_context.trace_id().to_string(),
        SPEC_TRACE_ID,
        "our span was rooted INSIDE the caller's trace — decision 019f95ff rule 1 forbids parenting \
         from inbound context, and a link must not become one: {linked:#?}"
    );

    let unlinked = exported_span(&exporter, "/api/profile");
    assert!(
        unlinked.links.is_empty(),
        "an UNAUTHENTICATED request had its trace joined. Rule 2 says a link is recorded after \
         authentication; a link attached at span construction (or from a layer outside the auth \
         middleware) lets any caller graft onto our traces: {unlinked:#?}"
    );
    // Non-vacuity: the anonymous request must still have carried the inbound context as inert
    // *fields* (rule 4), otherwise the assertion above could pass simply because the header was
    // dropped somewhere upstream and nothing was ever available to link.
    assert!(
        unlinked
            .attributes
            .iter()
            .any(|kv| kv.key.as_str() == "trace_id" && kv.value.as_str() == SPEC_TRACE_ID),
        "the anonymous request did not even record `trace_id`, so the empty-links assertion above \
         proves nothing about auth — it would pass with extraction broken: {unlinked:#?}"
    );
}
