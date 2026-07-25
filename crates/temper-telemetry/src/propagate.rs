//! Injecting temper's own trace context onto an outbound request — the mirror of [`crate::link`].
//!
//! [`crate::link`] rebuilds a caller's `SpanContext` *from* a header; this writes ours *into* one. The
//! two live side by side because they are the same knowledge pointed in opposite directions, and
//! splitting propagator details across crates is how the two halves drift.
//!
//! ## This is the half that makes a link have a target
//!
//! Decision `019f95ff-e216-7dd1-b2aa-a49d20b1cd6c` rule 1: a receiving surface never parents from
//! inbound context — it roots its own trace and *links* to the caller. A link names a `(trace_id,
//! span_id)` pair, so it is only navigable if a span with that id **actually exists in the same
//! backend**. Injection is what puts it there. Without this module the receiving side's links point at
//! ids no exporter ever emitted.
//!
//! ## Why the standard propagator rather than formatting the header ourselves
//!
//! [`crate::TraceParent`] can already parse the format, so writing one back looks like four `format!`
//! arguments. Two reasons not to:
//!
//! - **`tracestate` is vendor state that exists to be forwarded.** `TraceContextPropagator` carries it
//!   for free and correctly; a hand-rolled `traceparent` would silently drop it. `crate::lib`'s "what
//!   is deliberately not here" noted `tracestate` was unread because there was nowhere to forward it
//!   to — this is that somewhere.
//! - **The sampled flag must come from the real sampling decision**, not from a guess. The propagator
//!   reads it off the live `SpanContext`.
//!
//! ## When it does nothing, which is most of the time
//!
//! No OTel layer installed (any CLI without `TEMPER_CLI_TRACE`, every unit test, every server with no
//! endpoint configured) means the current span has no valid `SpanContext`, and the propagator writes
//! **no headers at all**. That is the correct outcome rather than a degraded one: a `traceparent`
//! naming a span that was never recorded is worse than no header, because the receiver would link to
//! it. `injects_nothing_without_a_provider` pins that.

use http::{HeaderMap, HeaderName, HeaderValue};
use opentelemetry::propagation::{Injector, TextMapPropagator};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Adapter letting the propagator write into an [`http::HeaderMap`].
///
/// Silently drops a pair whose name or value is not a legal header — unreachable for the W3C keys the
/// propagator emits, and the alternative (panicking inside an outbound request path) would trade a
/// request for a telemetry detail.
///
/// **Empty values are dropped rather than sent.** `TraceContextPropagator` always calls `set` for
/// `tracestate`, so a context with no vendor state yields `tracestate: ""` — a header carrying no
/// information, on every request temper makes. W3C makes `tracestate` optional, so omitting it is
/// correct rather than lossy, and it keeps the common case to exactly one injected header.
struct HeaderInjector<'a>(&'a mut HeaderMap);

impl Injector for HeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if value.is_empty() {
            return;
        }
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            self.0.insert(name, value);
        }
    }
}

/// Write the **current** span's trace context into `headers`, if there is one.
///
/// Call it from inside the span that represents the outbound request, so the id the receiver links to
/// is that request's span rather than its parent. In temper-client that means inside the
/// `.instrument(span)` body, not while building the `RequestBuilder`.
///
/// A no-op when no OpenTelemetry layer is installed or the current span is not recording.
pub fn inject_trace_context(headers: &mut HeaderMap) {
    inject_from(&Span::current(), headers);
}

/// [`inject_trace_context`] against an explicit span, so a test can drive it without entering one.
pub fn inject_from(span: &Span, headers: &mut HeaderMap) {
    TraceContextPropagator::new().inject_context(&span.context(), &mut HeaderInjector(headers));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TRACEPARENT;

    /// The common case, and the one where a wrong answer is actively harmful: with no provider there
    /// is no recorded span, so there must be no header. A `traceparent` here would make every
    /// receiving surface link to a span that does not exist.
    #[test]
    fn injects_nothing_without_a_provider() {
        let subscriber = tracing_subscriber::registry();
        let _guard = tracing::subscriber::set_default(subscriber);

        let span = tracing::info_span!("http_request");
        let mut headers = HeaderMap::new();
        inject_from(&span, &mut headers);

        assert!(
            headers.is_empty(),
            "injected headers with no tracer installed: {headers:?}"
        );
    }

    /// Whatever we inject must be readable by the code on the other side. Asserting against
    /// [`crate::TraceParent::parse`] rather than a regex ties the two halves together: a propagator
    /// upgrade that changed the wire format would fail here rather than at a receiver that goes quiet.
    #[test]
    fn what_it_injects_is_what_the_extractor_accepts() {
        use opentelemetry::trace::{
            SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState,
        };

        let subscriber = tracing_subscriber::registry();
        let _guard = tracing::subscriber::set_default(subscriber);

        // A span context stood up directly, so this test needs no exporter and no provider — it is
        // about the wire format, not about export.
        let ctx = opentelemetry::Context::new().with_remote_span_context(SpanContext::new(
            TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736").expect("valid trace id"),
            SpanId::from_hex("00f067aa0ba902b7").expect("valid span id"),
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        ));

        let mut headers = HeaderMap::new();
        TraceContextPropagator::new().inject_context(&ctx, &mut HeaderInjector(&mut headers));

        let raw = headers
            .get(TRACEPARENT)
            .expect("a sampled context must produce a traceparent")
            .to_str()
            .expect("header is ASCII");
        let parsed = crate::TraceParent::parse(raw).expect("our own extractor must accept it");

        assert_eq!(parsed.trace_id(), "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(parsed.parent_id(), "00f067aa0ba902b7");
        assert!(parsed.sampled());

        // An empty `tracestate` is dropped, not sent: the propagator sets it unconditionally, and a
        // valueless header on every outbound request is noise. See `HeaderInjector`.
        assert!(
            !headers.contains_key("tracestate"),
            "an empty tracestate was injected: {headers:?}"
        );
    }
}
