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
use opentelemetry::trace::TraceContextExt;
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
    inject_context(&span.context(), headers)
}

/// Inject a context's trace parent, **if that context describes a span that will be exported**.
///
/// ## The sampling gate, and why the module's stated invariant needed it
///
/// The docs above say a `traceparent` naming a span that was never recorded is worse than no header,
/// and cite `injects_nothing_without_a_provider`. That test pins only the *no provider* case, where the
/// `SpanContext` is invalid. There is a second way to name an unexported span, and it produces a
/// perfectly **valid** `SpanContext`: the span was sampled *out*.
///
/// Measured with an `AlwaysOff` sampler before this gate existed:
///
/// ```text
/// injected: 00-53f03b1f569809a3769651197aeb94b2-5ea27fc7c0490c25-00
/// spans actually exported: 0
/// ```
///
/// The receiving side links unconditionally — `link::inbound_span_context` never consults the sampled
/// bit — so every one of those becomes a link to a span no backend holds. Dormant under the default
/// `ParentBased(AlwaysOn)` sampler, and live the moment an operator sets `OTEL_TRACES_SAMPLER`, which
/// `docs/guides/open-telemetry-setup.md` explicitly offers as the cost-control knob. At
/// `traceidratio=0.1` that is 90% of temper's inter-service links pointing at nothing, while the guide
/// promises the opposite.
///
/// Split from [`inject_from`] so the gate is testable against a constructed context, without needing to
/// install a provider with a particular sampler.
fn inject_context(cx: &opentelemetry::Context, headers: &mut HeaderMap) {
    if !cx.span().span_context().is_sampled() {
        return;
    }
    TraceContextPropagator::new().inject_context(cx, &mut HeaderInjector(headers));
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

    /// A span that was sampled **out** has a perfectly valid `SpanContext` and will never be exported.
    /// Injecting for it manufactures the dangling link this module exists to prevent — see
    /// [`inject_context`].
    #[test]
    fn a_context_that_will_not_be_exported_injects_nothing() {
        use opentelemetry::trace::{
            SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState,
        };

        let unsampled = opentelemetry::Context::new().with_remote_span_context(SpanContext::new(
            TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736").expect("valid trace id"),
            SpanId::from_hex("00f067aa0ba902b7").expect("valid span id"),
            // The only difference from the test above: the sampled bit is clear.
            TraceFlags::default(),
            true,
            TraceState::default(),
        ));

        let mut headers = HeaderMap::new();
        inject_context(&unsampled, &mut headers);

        assert!(
            headers.is_empty(),
            "injected a traceparent for a span that will not be exported, so the receiver will link \
             to an id no backend holds: {headers:?}"
        );
    }
}
