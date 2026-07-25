#![cfg(feature = "test-db")]
//! Injection closes the loop: what temper-client sends is what the server links to.
//!
//! `crates/temper-api/tests/telemetry_link_test.rs` proves the receiving half — that an authenticated
//! caller's `traceparent` becomes a link. It supplies that header by hand, so it cannot show that
//! anything temper *sends* is linkable. This test removes the hand: the caller is
//! `temper_client::TemperClient`, the same code path the `temper` CLI runs, and the header is whatever
//! `temper_telemetry::propagate` actually injected.
//!
//! That gap is the whole point of the work this test covers. Before it, the two halves were extraction
//! and linking on the receiving side (#529, #536) and nothing at all on the sending side — so every
//! link temper recorded pointed at a span id no exporter had ever emitted.
//!
//! ## What is asserted, and what deliberately lives elsewhere
//!
//! Everything here is read off the **server's** root span, because that is what is deterministic in
//! this process:
//!
//! 1. `parent_span_id` / `trace_id` are present at all — temper-client injected a `traceparent`, which
//!    before this work it never did.
//! 2. The root span carries a link, and `link.span_id == parent_span_id` — the two *independent* readers
//!    of that header agree. `record_inbound_trace_context` writes the fields before auth;
//!    `link_trusted_caller` writes the link after it. They could disagree and nothing else would notice.
//! 3. `link.trace_id == trace_id`, same reasoning.
//! 4. The server's root span **roots its own trace** — its own trace id differs from the caller's. This
//!    is decision `019f95ff-e216-7dd1-b2aa-a49d20b1cd6c` rule 1 (*"Never parent from inbound
//!    context"*) observed at the production caller rather than asserted about a header, and it is what
//!    would fail if someone "improved" injection into parenting.
//!
//! **What this test does *not* assert is that the injected id is temper-client's own span** rather than
//! some ambient one. That property is pinned by `injection_reads_the_span_it_is_called_from` in
//! `crates/temper-telemetry/tests/cli_export_filter.rs`, where it is deterministic. It was originally
//! asserted here, by finding temper-client's outbound span in the exported set — which **fails under a
//! `--workspace` test build**, where that span does not reach the exporter before the test reads it,
//! while the identical code exports correctly from the real CLI (verified against a live OTLP endpoint).
//! Rather than assert a build-dependent ordering, each half is asserted where it holds
//! deterministically. The unexplained ordering difference is filed as task
//! `019f97fc-e6fd-7710-9fa6-0d1dc7585a11` rather than papered over with a sleep — including the
//! evidence for why it is believed test-only, which is one measured configuration and therefore worth
//! someone confirming.

mod common;

use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanData};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Read a span attribute as a string, if present.
fn attr(span: &SpanData, key: &str) -> Option<String> {
    span.attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| kv.value.to_string())
}

/// The single exported span matching `predicate`, or a panic naming every span that was exported.
///
/// **Exactly one**, not the first. `common::setup` makes its own requests and both surfaces name their
/// root span `http_request`, so "the first match" would be a silent dependency on export order.
fn only_span(
    exporter: &InMemorySpanExporter,
    what: &str,
    predicate: impl Fn(&SpanData) -> bool,
) -> SpanData {
    let spans = exporter
        .get_finished_spans()
        .expect("in-memory exporter readable");
    let matches: Vec<&SpanData> = spans.iter().filter(|s| predicate(s)).collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one exported span for {what}, found {}; exported: {:#?}",
        matches.len(),
        spans
            .iter()
            .map(|s| (
                s.name.to_string(),
                s.span_context.trace_id().to_string(),
                s.links.iter().count(),
                s.attributes
                    .iter()
                    .map(|kv| format!("{}={}", kv.key, kv.value))
                    .collect::<Vec<_>>()
            ))
            .collect::<Vec<_>>()
    );
    matches[0].clone()
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_server_links_to_the_trace_context_temper_client_sent(pool: sqlx::PgPool) {
    let exporter = InMemorySpanExporter::default();
    assert!(
        temper_telemetry::export::install_test_provider(exporter.clone()),
        "a provider was already installed — this test owns the process"
    );
    let layer = temper_telemetry::export::test_export_layer()
        .expect("the layer must exist once a provider is installed");
    // No `EnvFilter`: this asserts injection, not filtering. What the CLI's own stack filters is
    // `crates/temper-telemetry/tests/cli_export_filter.rs`.
    tracing_subscriber::registry().with(layer).init();

    let app = common::setup(pool).await;

    // The production caller — the same path the `temper` CLI takes. `/api/profile` is the smallest
    // authenticated round trip.
    app.client
        .profile()
        .get()
        .await
        .expect("authenticated profile read through temper-client");

    // The server flushes inside its own transport layer before writing the response, so its span is
    // already exported by the time the call returns.
    temper_telemetry::force_flush_spans();

    // `common::setup` also hits `/api/profile`, through raw `reqwest`, which injects nothing — so the
    // presence of `parent_span_id` is itself what distinguishes our request from that one.
    let server_span = only_span(
        &exporter,
        "the server request that arrived carrying trace context",
        |s| attr(s, "parent_span_id").is_some(),
    );

    let injected_parent =
        attr(&server_span, "parent_span_id").expect("selected on this attribute being present");
    let injected_trace = attr(&server_span, "trace_id").unwrap_or_else(|| {
        panic!("a traceparent yielded parent_span_id but no trace_id: {server_span:#?}")
    });

    let link = server_span.links.iter().next().unwrap_or_else(|| {
        panic!(
            "the server recorded inbound trace context but added no link — extraction ran and \
             `link_trusted_caller` did not, even though the caller authenticated: {server_span:#?}"
        )
    });

    assert_eq!(
        link.span_context.span_id().to_string(),
        injected_parent,
        "the link and the recorded field disagree about which span sent this request; they read the \
         same header, before and after auth: {server_span:#?}"
    );
    assert_eq!(
        link.span_context.trace_id().to_string(),
        injected_trace,
        "the link and the recorded field disagree about the caller's trace: {server_span:#?}"
    );

    // Rule 1. The server joins the caller's trace by *link*; it must never adopt it as its own.
    assert_ne!(
        server_span.span_context.trace_id().to_string(),
        injected_trace,
        "the server's root span adopted the caller's trace id — inbound context became a parent, \
         which decision 019f95ff rule 1 forbids: {server_span:#?}"
    );
}
