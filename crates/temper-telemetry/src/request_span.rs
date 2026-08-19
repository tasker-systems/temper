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
//! `latency_ms`. Those are the convention `internal/development/span-field-conventions.md` describes and
//! `tests/e2e/tests/logging_test.rs` gates, and they are unchanged — which is what makes this a
//! change of *mechanism* rather than of contract. `latency_ms` in particular has to survive: it is
//! the meter the exporter's own cost is measured with.

use std::future::{poll_fn, Future};
use std::panic::AssertUnwindSafe;
use std::pin::pin;
use std::task::Poll;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use tracing::{Instrument, Span};

/// End the span and drain the queue — the ordering the module exists to own.
///
/// Extracted because it now runs on **two** paths (a returned response and a propagating panic) and
/// the ordering is the whole invariant: `drop` before flush, because until the span closes there is
/// nothing queued to flush. Two copies of an invariant is how an invariant decoheres.
async fn end_span_and_flush(span: Span) {
    // Dropping the last handle closes the tracing span, which is what makes `tracing-opentelemetry`
    // end the OpenTelemetry span and hand it to the processor.
    drop(span);

    let flush = crate::export::flush_within_budget().await;
    if !flush.is_zero() {
        tracing::debug!(flush_ms = flush.as_millis() as u64, "span flush");
    }
}

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

    // ## Why the handler is polled inside `catch_unwind`
    //
    // A panicking handler unwinds through this `await`. `drop(span)` and the flush below are ordinary
    // statements, and unwinding skips ordinary statements — so the span was queued (the local *is*
    // dropped during unwind) but nothing drained the queue. It rode out on a later request's flush, or
    // was lost when the sandbox froze. The trace missing was the one for the request that failed.
    //
    // The obvious fix — a `Drop` scope guard — is not available: `flush_within_budget` is `async` and
    // `Drop::drop` cannot await. Blocking inside `drop` on a Tokio worker would be worse than the bug.
    //
    // So the unwind is caught, the flush runs, and the unwind is **resumed** below. The panic still
    // propagates exactly as it did, so nothing a caller can observe changes. This is deliberately not
    // `CatchPanicLayer`, which would turn a panic into a 500 and change the surface's behaviour — a
    // decision that belongs to the surfaces, not to a telemetry seam.
    //
    // Hand-rolled rather than `futures_util::FutureExt::catch_unwind`: no temper crate depends on
    // `futures` directly, and this is the whole of what that combinator does. The future is never
    // polled again after a panic — the `Ready` arm below ends the poll loop — which is the one rule
    // that makes catching an unwind mid-poll sound.
    //
    // ## The inner block is load-bearing — do not flatten it
    //
    // The instrumented future holds its **own clone** of the span. Previously it was a temporary
    // (`next.run(request).instrument(span.clone()).await`) dropped at the end of the await expression,
    // leaving `span` as the only handle — which is what makes `drop(span)` actually close the span.
    //
    // Binding it to a local for `poll_fn` keeps it alive to the end of the function, so its clone
    // outlives `end_span_and_flush(span)`: the span never closes, nothing is queued, and the flush
    // exports nothing. That is not hypothetical — it broke the *healthy* path the moment this
    // middleware was rewritten, and `panicking_request_still_flushes`'s baseline assertion is what
    // caught it. The block scopes the future so it is dropped before `span` is used.
    let outcome = {
        let mut instrumented = pin!(next.run(request).instrument(span.clone()));
        poll_fn(|cx| {
            match std::panic::catch_unwind(AssertUnwindSafe(|| instrumented.as_mut().poll(cx))) {
                Ok(Poll::Pending) => Poll::Pending,
                Ok(Poll::Ready(response)) => Poll::Ready(Ok(response)),
                Err(payload) => Poll::Ready(Err(payload)),
            }
        })
        .await
    };

    let response = match outcome {
        Ok(response) => response,
        Err(payload) => {
            // The span carries no `status`: there is no response, and inventing one would make a
            // panic indistinguishable from a handler that returned 500. `latency_ms` stays, so the
            // panic's timing is comparable with every other request.
            //
            // But the span's *status* is unambiguously ERROR — a panic is the clearest server failure
            // there is, and this is the one place the two axes must not be conflated: no HTTP `status`
            // (there was no response), yet `otel.status_code = ERROR` (the span failed). Without this,
            // the request most worth a health signal is the one with no error status at all.
            span.record("otel.status_code", "ERROR");
            tracing::error!(
                parent: &span,
                latency_ms = started.elapsed().as_millis() as u64,
                "panic"
            );
            end_span_and_flush(span).await;
            std::panic::resume_unwind(payload);
        }
    };

    let status = response.status();
    let latency_ms = started.elapsed().as_millis() as u64;

    // `parent: &span` rather than entering it: the event belongs to this request's span, and
    // entering here would be a second way to say so that only works while the span is current.
    //
    // Emitted **before** `drop(span)`, necessarily: an event recorded after the span closes cannot be
    // attached to it, and this event is the convention `internal/development/span-field-conventions.md`
    // describes and `tests/e2e/tests/logging_test.rs` gates.
    // ## The error rule is the OTel HTTP server semantic convention, and the choice is deliberate
    //
    // `otel.status_code = ERROR` iff the response is **5xx**. Not `status >= 400`. A 4xx is a correct
    // judgment the service rendered about a request — bad input, an unauthorized caller, a resource
    // that is not there — not a failure of the service to function. Marking 4xx `ERROR` would make an
    // *Error Rate by Service* panel track how often clients send bad requests and how often the authz
    // gates *work*, which is worse than no signal: it makes a healthy system read as broken and trains
    // the reader to ignore the panel.
    //
    // temper sharpens this. Several authorization denials are rendered as `404` on purpose, so a probe
    // cannot become an existence oracle (`ScopedAuthority`, the `CONTEXT_REFUSAL` family). Those 404s
    // are the gate *doing its job*; counting them as errors would invert the signal exactly where it
    // matters most. So the line is drawn at 5xx — the range that means "the service failed to fulfil a
    // well-formed request" — and nowhere else.
    //
    // Success is left `UNSET`, never `OK`: the OTel convention reserves `OK` for an explicit
    // application affirmation, and a 2xx server response is not one. `UNSET` + `ERROR` is a complete
    // basis for a RED error rate (`ERROR / total`); a third `OK` bucket would add nothing and misuse
    // the status. This mirrors the `is_server_error()` split the log level already makes — one rule,
    // two consumers.
    //
    // Recorded before `drop(span)`, necessarily: `tracing-opentelemetry` applies `otel.status_code`
    // from the recorded value, and a value recorded after the span closes cannot reach it.
    if status.is_server_error() {
        span.record("otel.status_code", "ERROR");
        tracing::error!(parent: &span, status = status.as_u16(), latency_ms, "response");
    } else {
        tracing::info!(parent: &span, status = status.as_u16(), latency_ms, "response");
    }

    // ## Why the flush cost is its own event, and `latency_ms` is not it
    //
    // The guide used to name `latency_ms` as the meter for the exporter's own cost — *"the cost is the
    // before/after difference in a number we already have."* That difference is **structurally zero**:
    // `latency_ms` is taken above, and the flush can only run after `drop(span)`, because until the span
    // closes there is nothing queued to flush. No ordering fixes that; the flush is genuinely not part
    // of the span it flushes.
    //
    // It is, however, part of what the **client** waits for — the response is returned after this line —
    // so the cost is real and had to become visible some other way. `flush_ms` is that way, and a
    // caller's observed latency is `latency_ms + flush_ms`.
    //
    // Worth naming the pattern: an instrument that is real, on the real path, and blind to the one
    // thing it was cited for is the same shape as an exporter whose test could not reach a transport.
    // Per-flush timing stays at `debug`: it is one extra line **per request**, and the servers already
    // emit a `response` event per request — doubling that at `info` would be a real log bill for a
    // distribution nobody reads most days. The condition worth seeing by default is *the budget being
    // exceeded*, which `flush_within_budget` warns about, because that one means spans were dropped.
    //
    // `RUST_LOG=debug` is now a safe way to sample this distribution on a live deployment: since both
    // stacks filter per-layer, raising the log level no longer widens what is exported (or billed).
    // That was not true before — a subscriber-wide filter meant asking for detail also shipped it.
    //
    // **This call is the reason the module exists** — see `end_span_and_flush`, which owns the
    // `drop`-then-flush ordering for both this path and the panicking one.
    end_span_and_flush(span).await;

    response
}
