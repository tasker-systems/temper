//! The request root span, owned rather than borrowed from `TraceLayer`.
//!
//! ## Why this is not `tower_http::trace::TraceLayer`
//!
//! It was, until the exporter needed the span to be *finished* at a moment we control.
//!
//! `TraceLayer` clones its span into the response body — `ResponseBody { …, span: Span }`
//! (`tower-http-0.7.0/src/trace/body.rs:25`) — so the root span stays open until the body is
//! dropped, which happens after every middleware in the chain has returned. That is the right
//! design for `TraceLayer`'s purpose: it wants `on_body_chunk` and `on_eos` to run inside the
//! span, and a streaming response is not "done" when the headers are.
//!
//! It is the wrong design for ours. A batch span processor only has a span to export once that span
//! has **ended**, and on Vercel the flush has to happen before the sandbox freezes. With the span's
//! lifetime tied to a body we do not own, there is no layer position from which a flush can see the
//! request's own span — measured, not assumed: with the flush in an outermost middleware the
//! exporter held **zero** spans when the response reached the client, and a manual flush one
//! statement later produced one.
//!
//! So the span is created here, held here, and dropped here — `drop(span)` is what ends the
//! OpenTelemetry span, and only then is there anything to flush. Owning the lifetime is the whole
//! point of the module.
//!
//! ## What was given up, and what was not
//!
//! Given up: `on_body_chunk` / `on_eos` (never used) and `DefaultOnFailure`'s classification, which
//! is replaced by an explicit server-error branch below — arguably clearer, since it says what it
//! logs instead of deferring to a classifier's notion of failure.
//!
//! Not given up: span names, the field set, and the `response` event with `status` and
//! `latency_ms`. Those are the convention `docs/development/span-field-conventions.md` describes and
//! `tests/e2e/tests/logging_test.rs` gates, and they are unchanged — which is what makes this a
//! change of *mechanism* rather than of contract. `latency_ms` in particular has to survive: it is
//! the meter the exporter's own cost is measured with.

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use tracing::{Instrument, Span};

/// Run `next` inside a root span built by `make_span`, then end the span and flush.
///
/// `make_span` rather than a span name, because the two surfaces build deliberately different root
/// spans — `http_request` and `mcp_request` — and `tracing`'s span name is part of its static
/// metadata, so it cannot be a runtime parameter. What is shared is everything that matters here:
/// the ordering of instrument → respond → **end the span** → flush.
///
/// The caller records inbound trace context inside `make_span`, where the headers are in hand, for
/// the same reason it always did.
pub async fn traced_request<F>(request: Request, next: Next, make_span: F) -> Response
where
    F: FnOnce(&Request) -> Span,
{
    let span = make_span(&request);
    let started = std::time::Instant::now();

    // The instrumented future holds its own clone; it is dropped when the await completes, leaving
    // `span` as the only handle.
    let response = next.run(request).instrument(span.clone()).await;

    let status = response.status();
    let latency_ms = started.elapsed().as_millis() as u64;

    // `parent: &span` rather than entering it: the event belongs to this request's span, and
    // entering here would be a second way to say so that only works while the span is current.
    if status.is_server_error() {
        tracing::error!(parent: &span, status = status.as_u16(), latency_ms, "response");
    } else {
        tracing::info!(parent: &span, status = status.as_u16(), latency_ms, "response");
    }

    // **This line is the reason the module exists.** Dropping the last handle closes the tracing
    // span, which is what makes `tracing-opentelemetry` end the OpenTelemetry span and hand it to
    // the processor. Until this runs there is nothing queued, and a flush would export nothing.
    drop(span);

    crate::export::force_flush_spans();

    response
}
