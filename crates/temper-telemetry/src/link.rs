//! Joining a trusted caller's trace — by **link**, after authentication.
//!
//! The other half of decision `019f95ff-e216-7dd1-b2aa-a49d20b1cd6c`. [`crate::record_inbound_trace_context`]
//! implements rules 1, 3 and 4 (extract the caller's context as inert *fields*, never as a parent,
//! never letting its `sampled` flag reach our sampler). This module implements rule 2:
//!
//! > *"Join a trusted caller's trace with an OTel span **link**, recorded after authentication."*
//!
//! ## What "one of ours" means, and why it is verification rather than identity
//!
//! A link is attached when the request has passed an **authentication gate**: a JWT verified against
//! our IdP's JWKS for our issuer and audience, or an HMAC signature over the exact request body
//! keyed on a secret only temper's own services hold. Nothing narrower, and nothing about *which*
//! principal it turned out to be.
//!
//! Two consequences worth stating, because both are deliberate:
//!
//! - **The trust event is the token verifying, not the profile resolving.** temper-api's
//!   `require_auth` continues past `decode` into `authenticate_token`, which can still refuse a
//!   deactivated profile; temper-mcp splits those steps across two files. Anchoring on verification
//!   makes the rule one sentence at all four gates instead of four slightly different sentences, and
//!   a 401 that is still joined to the caller's trace is *more* debuggable, which is the same
//!   argument rule 4 makes for keeping extraction on unauthenticated surfaces.
//! - **Machine principals and humans are treated alike.** A `client_credentials` token from a
//!   registered machine and a human's token both verify against the same JWKS; "the mention flow's
//!   three hops" is precisely a mix of the two, so distinguishing them here would break the case
//!   this exists to serve.
//!
//! The rejected alternative is on record in `crate`'s module docs: an allowlist of *surfaces* is an
//! enumeration that must stay correct as routes move, and `create_app` mounts the internal routes
//! too, so it could not even be stated per-router. Authentication is a property each request carries
//! with it.
//!
//! ## The residual risk, and why it is inert
//!
//! An authenticated principal can send any `traceparent`, so it can make **its own** span carry a
//! link to a trace id that is not its own. That is the whole exposure, and each thing it is *not*
//! matters:
//!
//! - **Not a parent.** Rule 1 is unconditional and is enforced by construction — nothing here
//!   touches the span's parent, which is fixed at creation and long since fixed by the time auth
//!   runs.
//! - **Not a sampling lever**, and this one deserves its citation because `Sampler::should_sample`
//!   *does* take links. It never sees ours: `tracing_opentelemetry::layer()` defaults
//!   `context_activation: true` (`tracing-opentelemetry-0.33.0/src/layer.rs:668`), so entering a
//!   span starts the OpenTelemetry span (`on_enter` → `with_started_cx` → `start_cx`, `:1206-1222`,
//!   `:1077-1096`) — and the root span is entered by `traced_request`'s `.instrument()` long before
//!   any auth gate runs. A link added afterwards takes the `OtelDataState::Context` arm of
//!   `add_link` (`src/span_ext.rs:330-336`), which mutates a span whose sampling decision was taken
//!   at entry. Belt and braces: the default `ParentBased` sampler reads only the parent context
//!   anyway. Rule 3 holds, and `link_survives_without_becoming_a_parent_or_a_sampler_input` pins the
//!   observable half against a real tracer.
//! - **Not unbounded content.** [`crate::TraceParent::parse`] admits 32 and 16 hex characters and
//!   the all-zero sentinels are rejected, so the worst payload is a well-formed id.
//!
//! What is left is a spurious edge in a trace viewer, drawn from a span the caller already owns,
//! costing one authenticated principal's own trace some noise. Weighed against a mention flow whose
//! three hops are navigable as one story, that is the trade the decision took.

use http::HeaderMap;
use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

use crate::{header_str, TraceParent, TRACEPARENT};

/// Link `span` to the caller's trace, if the caller sent one.
///
/// Call this from an authentication gate, once that gate has verified the request and before it
/// hands off — see the module docs for what counts. Every other request reaches this function's
/// absence, which is the point: an unauthenticated request keeps its inert *fields* and gains no
/// link.
///
/// `span` is the request's root span. The call sites pass `Span::current()`, which resolves to the
/// root span because the root instruments the whole downstream future — the same deferred seam
/// `profile_id` has always used, one gate further in.
///
/// A no-op when no `traceparent` arrived, when it is malformed, or when no OpenTelemetry layer is
/// installed (`add_link` on a span with no `WithContext` subscriber does nothing). So this is safe
/// to call unconditionally: local dev and CI run with export off and pay nothing.
pub fn link_trusted_caller(span: &Span, headers: &HeaderMap) {
    let Some(caller) = inbound_span_context(headers) else {
        return;
    };
    span.add_link(caller);
}

/// Rebuild the caller's `SpanContext` from its `traceparent`.
///
/// Re-parses the header rather than stashing the [`TraceParent`] the root span already parsed.
/// Parsing is a few length and hex checks over a 55-byte string, and the alternative — a request
/// extension carrying trace state from span construction to auth — would add a second place where
/// the caller's context lives, for a saving too small to measure.
///
/// `is_remote: true` is the honest value (this context was created in another process) and is
/// **only** consulted for links and parents; a link's remoteness reaches no sampler.
fn inbound_span_context(headers: &HeaderMap) -> Option<SpanContext> {
    let parsed = TraceParent::parse(header_str(headers, TRACEPARENT)?)?;
    Some(SpanContext::new(
        TraceId::from_hex(parsed.trace_id()).ok()?,
        SpanId::from_hex(parsed.parent_id()).ok()?,
        // Recorded, not obeyed — the flag describes the *linked* span, and rule 3 is satisfied by
        // the fact that nothing downstream reads it. Dropping it would lose the one piece of
        // information a viewer needs to explain why the linked trace has no spans of its own.
        TraceFlags::default().with_sampled(parsed.sampled()),
        true,
        TraceState::default(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
    use tracing_subscriber::layer::SubscriberExt as _;

    const INBOUND_TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
    const INBOUND_PARENT_ID: &str = "00f067aa0ba902b7";

    fn headers_with(traceparent: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "traceparent",
            traceparent.parse().expect("valid header value"),
        );
        headers
    }

    /// Drive one root span through a real tracer, doing what the transport layer does — build the
    /// span, record the inbound fields, then (optionally) link — and return the exported span.
    fn export_one(headers: &HeaderMap, link: bool) -> opentelemetry_sdk::trace::SpanData {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("temper-test")));

        tracing::subscriber::with_default(subscriber, || {
            let request = http::Request::builder()
                .method("GET")
                .uri("/api/resources")
                .body(())
                .expect("request builds");
            let span = crate::root_span!("http_request", request);
            // The span is entered before the link is added, mirroring production: the root span
            // instruments the downstream future, so it is live by the time an auth gate runs. This
            // is what makes the sampling decision already-taken when `add_link` lands.
            let _entered = span.enter();
            if link {
                link_trusted_caller(&span, headers);
            }
        });

        provider.force_flush().expect("flush");
        exporter
            .get_finished_spans()
            .expect("finished spans")
            .into_iter()
            .next()
            .expect("the root span was exported")
    }

    /// Rule 2, and the two rules it must not break on the way.
    ///
    /// One test rather than three because the interesting claim is the *conjunction*: a link that
    /// joined the caller's trace by quietly reparenting or resampling us would satisfy any of these
    /// assertions taken alone. Mutation-checked — deleting the `add_link` call fails the first
    /// assertion, and turning the link into a parent fails the last two.
    #[test]
    fn link_survives_without_becoming_a_parent_or_a_sampler_input() {
        let headers = headers_with(&format!(
            // `-01`: the caller says this trace is sampled. We record that on the link and must not
            // let it decide anything of ours.
            "00-{INBOUND_TRACE_ID}-{INBOUND_PARENT_ID}-01"
        ));
        let exported = export_one(&headers, true);

        let link = exported
            .links
            .iter()
            .next()
            .unwrap_or_else(|| panic!("no link was attached to the root span: {exported:#?}"));
        assert_eq!(
            link.span_context.trace_id().to_string(),
            INBOUND_TRACE_ID,
            "the link points at some other trace: {exported:#?}"
        );
        assert_eq!(
            link.span_context.span_id().to_string(),
            INBOUND_PARENT_ID,
            "the link points at some other span: {exported:#?}"
        );
        assert!(
            link.span_context.is_sampled(),
            "the caller's sampled flag was dropped from the link"
        );

        assert_ne!(
            exported.span_context.trace_id().to_string(),
            INBOUND_TRACE_ID,
            "linking moved our span INTO the caller's trace — that is parenting by another name, \
             and decision 019f95ff rule 1 forbids it: {exported:#?}"
        );
        assert!(
            !exported.parent_span_is_remote,
            "the linked context became a REMOTE PARENT, which hands the caller our sampling \
             decision — decision 019f95ff rule 3: {exported:#?}"
        );
    }

    /// The negative half of the same seam: without the post-auth call there is no link, so an
    /// unauthenticated request cannot be joined to anything.
    ///
    /// This is the assertion that makes "post-auth" mean something in *this* crate. The gate that
    /// proves it end-to-end — an unauthenticated request through the real router — lives in
    /// `temper-api/tests/telemetry_link_test.rs`, because only there is the auth middleware real.
    #[test]
    fn no_link_without_the_post_auth_call() {
        let headers = headers_with(&format!("00-{INBOUND_TRACE_ID}-{INBOUND_PARENT_ID}-01"));
        let exported = export_one(&headers, false);
        assert!(
            exported.links.is_empty(),
            "a span was linked without anything having authenticated the caller: {exported:#?}"
        );
    }

    /// A malformed `traceparent` yields no link, for the same reason it yields no fields: a link to
    /// a trace id nobody can have is worse than no link, because it looks like a real join.
    #[test]
    fn a_malformed_traceparent_links_nothing() {
        let exported = export_one(&headers_with("00-not-a-trace-parent"), true);
        assert!(
            exported.links.is_empty(),
            "a malformed traceparent produced a link: {exported:#?}"
        );
    }

    /// No `traceparent` at all is the common case — most callers are not instrumented — and it must
    /// be a silent no-op rather than an invalid `SpanContext` link. `add_link` itself drops invalid
    /// contexts (`tracing-opentelemetry-0.33.0/src/span_ext.rs:307`, the `cx.is_valid()` guard), so
    /// this pins our own early return rather than relying on that.
    #[test]
    fn no_traceparent_links_nothing() {
        assert!(inbound_span_context(&HeaderMap::new()).is_none());
        let exported = export_one(&HeaderMap::new(), true);
        assert!(exported.links.is_empty());
    }
}
