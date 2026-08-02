//! Shared telemetry seam for temper's deployables.
//!
//! Five things live here:
//!
//! - [`init`] — how a temper process logs. One seam for the five binaries that used to configure
//!   `tracing_subscriber` themselves, and where the export layer attaches.
//! - [`export`] — span export over OTLP: what temper decides that the OpenTelemetry SDK does not.
//! - [`request_span`] — the request root span, owned so its lifetime can be ended deliberately.
//! - [`link`] — joining a *trusted* caller's trace, by link, after authentication. The half of the
//!   trust decision that only an authenticated request gets.
//! - [`propagate`] — writing our trace context onto an *outbound* request. The mirror of [`link`],
//!   and what gives a receiving surface's link an id that actually exists in the backend.
//! - This module — reading whatever correlation context arrives on an inbound request and stamping
//!   it onto that request's root span. The *extraction* half of W3C trace context.
//!
//! ## Extraction landed before export, and stays a field afterwards
//!
//! `docs/development/span-field-conventions.md` recorded the gap extraction closed: *"No W3C trace
//! context. Nothing extracts or propagates `traceparent`, so spans do not yet join across
//! deployables."* A trace id in a JSON log line is worth having on its own — it is what lets a Slack
//! mention's three hops be grepped into one story.
//!
//! The exporter has since landed, and **that did not promote the extracted context to a parent**.
//! It stays exactly what it was: a field. See the trust boundary below — this is the decided
//! behaviour, not an unfinished step.
//!
//! ## What is deliberately *not* here
//!
//! - **Synthesis.** When no `traceparent` arrives, the fields stay empty rather than being filled
//!   with a locally-minted id. Minting root ids is the tracer provider's job, and doing it here
//!   would manufacture ids that no exported span agrees with.
//!
//! ## Inbound trace context is caller-supplied, and that is a trust boundary
//!
//! Any client can set `traceparent`, so the recorded `trace_id` is attacker-choosable on the public
//! surface. Parsing rather than copying bounds the damage to a *well-formed* id — 32 hex characters
//! and nothing else can reach a log field, so this is not a log-forging vector — but a caller can
//! still place its requests under a trace id of its choosing.
//!
//! As a log field that is tolerable, and it is what every OTel deployment does by default at the
//! edge. It would stop being merely a field the moment the extracted context became a real
//! **parent**: at that point an unauthenticated caller could graft spans onto someone else's trace.
//!
//! **That is decided, and the answer is no.** Temper accepts `traceparent` only from itself
//! (decision `019f95ff-e216-7dd1-b2aa-a49d20b1cd6c`):
//!
//! 1. **Never parent from inbound context** — every root span roots its own trace, every surface,
//!    unconditionally.
//! 2. **Join a trusted caller's trace with an OTel span *link*, recorded post-auth**, not by
//!    parenting. Implemented in [`link`], which is also where "one of ours" is defined and
//!    defended.
//! 3. **Never let an inbound `sampled` flag drive sampling.** Record it; do not obey it. It is the
//!    one consequence with a bill attached — honoring it lets anyone force every span to export.
//! 4. **Field extraction below stays as it is**, on all surfaces including unauthenticated ones.
//!    Fields are inert, and they are what makes a 401 debuggable.
//!
//! Refusing untrusted context costs nothing, which is why the rule can be absolute: every trace
//! worth joining is one temper sent itself (agent → internal → MCP, steward → dispatch, UI → API).
//! An MCP client's trace is not one we want to be a child of.
//!
//! The alternative — trusting inbound context on the signed/internal surfaces only — was rejected on
//! maintenance grounds, not security ones: it is an allowlist that must stay correct as routes move,
//! and `create_app` mounts the internal routes too, so it could not even be stated per-router. The
//! accepted cost of links is that a linked trace is navigable but is not a single waterfall.
//!
//! **The constraint any counter-proposal must answer:** a span's parent is fixed at creation, and
//! the root span is built before auth runs. So "extract after auth" can yield a field or
//! a link, never a parent.
//!
//! ## The Vercel headers, and one name that must not be reused
//!
//! `vercel_runtime` reads `x-vercel-internal-invocation-id` off every inbound request
//! (`vercel_runtime-2.1.1/src/lib.rs`, in `run`'s connection handler) to tag its IPC log messages.
//! It reads without removing, so the header is still on the request when it reaches our layer.
//!
//! That id is Vercel's **function-invocation** grain. temper already has an `invocation_id` — the
//! agent-run envelope carried in `ActContext` and asserted by
//! `temper_services::backend::ACT_SPAN_FIELDS`. They are unrelated concepts that would silently
//! merge under one field name, so Vercel's is recorded as `vercel_invocation_id` and never as
//! `invocation_id`.

pub mod export;
pub mod init;
pub mod link;
pub mod propagate;
pub mod redact;
pub mod request_span;

pub use export::{force_flush_spans, set_service_name, shutdown_telemetry};
#[cfg(feature = "test-support")]
pub use init::init_server_logging_with_writer;
pub use init::{init_cli_logging, init_server_logging};
pub use link::link_trusted_caller;
pub use propagate::inject_trace_context;
pub use request_span::traced_request;

use http::HeaderMap;
use tracing::Span;

/// Re-exported so [`root_span!`] can path to it as `$crate::tracing`, which works regardless of
/// what the calling crate happens to have in scope.
pub use tracing;

/// Build a request root span with the full field set, and stamp inbound trace context onto it.
///
/// ## Why a macro and not a function taking the name
///
/// Not a style choice. `tracing` puts a span's name into a `static Metadata` initializer, so a name
/// that arrives as a parameter fails to compile — `error[E0435]: attempt to use a non-constant
/// value in a constant`. And the callsite is static *per macro invocation*, so even a `const` name
/// would give every surface one shared callsite under one name. Two span names genuinely require
/// two expansion sites; a macro is what lets the sites differ while the *contents* are written once.
///
/// ## What this exists to stop
///
/// The name is the only thing that legitimately differs between `http_request` and `mcp_request`.
/// Everything else — the three request fields, the six deferred fields, and the
/// [`record_inbound_trace_context`] call — was copy-pasted into both surfaces, which meant adding a
/// field to one and forgetting the other compiled cleanly and passed every test. [`ROOT_TRACE_FIELDS`]
/// declared the names for the *gate*, but nothing tied the gate's list to what the constructors
/// actually built. Now one expansion builds both, and `macro_declares_every_root_trace_field` ties
/// it to the constant.
///
/// The span names stay deliberately distinct, and that rule had to be applied twice. temper-mcp's root
/// span is `mcp_request` rather than a second `http_request`; temper-client's outbound span was a third
/// until export made the collision visible, and is now `http_client_request`. Three things under one
/// name are unreadable once exported.
#[macro_export]
macro_rules! root_span {
    ($name:literal, $request:expr) => {{
        let request = &$request;
        let span = $crate::tracing::info_span!(
            $name,
            // `otel.kind` is a magic field `tracing-opentelemetry` consumes into the OTel span's
            // `SpanKind` (and strips from the attribute set) rather than a field we carry. Both
            // surfaces build inbound request spans through this macro, so both are `server`.
            //
            // Without this, `tracing-opentelemetry` defaults every span to `SpanKind::Internal`, and
            // Tempo's span-metrics and service-graph processors derive RED metrics and graph edges
            // only from `server`/`client` spans — so `http_request` and `mcp_request` were received
            // and stored fine yet excluded from `traces_spanmetrics_*` and the service graph, while
            // the surfaces whose entry spans already reported `server` showed up. This is the request
            // boundary; naming it as one is what puts these services back on the graph.
            otel.kind = "server",
            method = %request.method(),
            path = %$crate::redact::redact_path(request.uri().path()),
            version = ?request.version(),
            // Filled by the auth middleware once a token resolves to a profile — a validated token
            // is not yet a profile, so this cannot be known at construction.
            profile_id = $crate::tracing::field::Empty,
            // `ROOT_TRACE_FIELDS`, filled just below from whatever headers actually arrived.
            trace_id = $crate::tracing::field::Empty,
            parent_span_id = $crate::tracing::field::Empty,
            trace_sampled = $crate::tracing::field::Empty,
            vercel_id = $crate::tracing::field::Empty,
            vercel_invocation_id = $crate::tracing::field::Empty,
        );
        // Unlike the act ids — which arrive in the request *body* and so cannot be known here —
        // headers are in hand at construction, so this records onto the span just built rather than
        // via `Span::current()` further in, which is the trap the act-span convention exists to close.
        $crate::record_inbound_trace_context(&span, request.headers());
        span
    }};
}

/// The root-span fields this crate records, declared once so the span constructors, the convention
/// gate, and the prose documentation cannot drift apart.
///
/// Every one of these is *deferred* — declared `tracing::field::Empty` at span creation and recorded
/// only if the corresponding header arrived. A field that stays empty is a real answer (nothing
/// upstream sent trace context), not a failure.
///
/// This is the request-level counterpart to `temper_services::backend::ACT_SPAN_FIELDS`, which
/// covers the act grain. The two sets are disjoint on purpose — see the module docs on
/// `vercel_invocation_id`.
pub const ROOT_TRACE_FIELDS: [&str; 5] = [
    "trace_id",
    "parent_span_id",
    "trace_sampled",
    "vercel_id",
    "vercel_invocation_id",
];

/// W3C `traceparent` header name. Shared with [`link`], which re-parses the same header once the
/// caller has authenticated.
pub(crate) const TRACEPARENT: &str = "traceparent";
/// Vercel's per-request id, surfaced on responses and in its own logs — the bridge from our trace
/// into Vercel's per-request view. Already used by hand at the steward hop
/// (`crates/temper-api/src/handlers/steward.rs`); this generalizes it to every request.
const X_VERCEL_ID: &str = "x-vercel-id";
/// Vercel's function-invocation id. See the module docs: NOT temper's `invocation_id`.
const X_VERCEL_INVOCATION_ID: &str = "x-vercel-internal-invocation-id";

/// A parsed W3C `traceparent`.
///
/// Constructed only by [`TraceParent::parse`], so a value of this type is a guarantee rather than a
/// hope: the ids are lowercase hex of the right length and are not the all-zero sentinel the spec
/// forbids. That is the point of parsing into a type instead of passing the raw header around and
/// re-checking it at each use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceParent {
    trace_id: String,
    parent_id: String,
    sampled: bool,
}

impl TraceParent {
    /// The 32-hex-character trace id — the value that is the same across every hop of one user
    /// action, and therefore the only field here worth grouping logs by.
    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    /// The 16-hex-character id of the span upstream that sent this request. Ours would be its child
    /// once a tracer provider exists.
    pub fn parent_id(&self) -> &str {
        &self.parent_id
    }

    /// The `sampled` flag (bit 0 of `trace-flags`) — upstream's statement of whether it recorded
    /// this trace. Retained because a sampling decision made downstream of one already taken
    /// upstream produces broken traces.
    pub fn sampled(&self) -> bool {
        self.sampled
    }

    /// Parse a `traceparent` header value, returning `None` for anything the spec calls invalid.
    ///
    /// Follows the W3C forward-compatibility rules rather than pinning the exact 55-character
    /// version-00 form:
    ///
    /// - version `ff` is invalid; other unknown versions parse as far as the known fields and the
    ///   remainder is ignored, which is what lets a future version reach us without going dark.
    /// - version `00` must have exactly four fields — a trailing field on `00` is invalid, not
    ///   forward-compatible.
    /// - the all-zero trace id and the all-zero parent id are both invalid sentinels.
    /// - hex is case-insensitive on the wire and normalized to lowercase here, so two spellings of
    ///   one trace never look like two traces in a log query.
    pub fn parse(value: &str) -> Option<Self> {
        let mut parts = value.split('-');
        let version = parts.next()?;
        let trace_id = parts.next()?;
        let parent_id = parts.next()?;
        let flags = parts.next()?;
        let has_extra = parts.next().is_some();

        if version.len() != 2 || !is_hex(version) || version == "ff" {
            return None;
        }
        // Only version 00 is fully specified; a later version may append fields, but 00 may not.
        if version == "00" && has_extra {
            return None;
        }
        if trace_id.len() != 32 || !is_hex(trace_id) || trace_id.bytes().all(|b| b == b'0') {
            return None;
        }
        if parent_id.len() != 16 || !is_hex(parent_id) || parent_id.bytes().all(|b| b == b'0') {
            return None;
        }
        if flags.len() != 2 || !is_hex(flags) {
            return None;
        }
        let flag_bits = u8::from_str_radix(flags, 16).ok()?;

        Some(Self {
            trace_id: trace_id.to_ascii_lowercase(),
            parent_id: parent_id.to_ascii_lowercase(),
            sampled: flag_bits & 0x01 != 0,
        })
    }
}

fn is_hex(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Stamp whatever correlation context arrived on the request onto its root span.
///
/// Call this from the span constructor itself, with the span it just built — unlike the act ids
/// (which arrive in the request *body* and so genuinely cannot be known at span-creation time),
/// headers are in hand right there, and recording them anywhere else would reintroduce the
/// `Span::current()` trap the act-span convention exists to close.
///
/// Every record is conditional. A request that carries no trace context leaves all five fields
/// empty, which is the honest representation of "nothing upstream told us anything".
///
/// A `traceparent` that is present but malformed is logged at debug rather than recorded: it is a
/// distinct and interesting state from "absent", but it is a property of the sender, not of this
/// request, so it does not belong in the field set.
pub fn record_inbound_trace_context(span: &Span, headers: &HeaderMap) {
    if let Some(raw) = header_str(headers, TRACEPARENT) {
        match TraceParent::parse(raw) {
            Some(tp) => {
                span.record("trace_id", tp.trace_id());
                span.record("parent_span_id", tp.parent_id());
                span.record("trace_sampled", tp.sampled());
            }
            None => {
                tracing::debug!("inbound traceparent present but unparseable; ignoring");
            }
        }
    }

    if let Some(vercel_id) = header_str(headers, X_VERCEL_ID) {
        span.record("vercel_id", vercel_id);
    }
    if let Some(invocation) = header_str(headers, X_VERCEL_INVOCATION_ID) {
        span.record("vercel_invocation_id", invocation);
    }
}

/// Read a header as UTF-8, treating a non-UTF-8 value as absent.
pub(crate) fn header_str<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    headers.get(name)?.to_str().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical example from the W3C trace-context spec.
    const SPEC_EXAMPLE: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    #[test]
    fn parses_the_spec_example() {
        let tp = TraceParent::parse(SPEC_EXAMPLE).expect("spec example must parse");
        assert_eq!(tp.trace_id(), "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(tp.parent_id(), "00f067aa0ba902b7");
        assert!(tp.sampled());
    }

    #[test]
    fn unsampled_flag_is_read_from_bit_zero() {
        let tp = TraceParent::parse("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00")
            .expect("valid");
        assert!(!tp.sampled());

        // Bit 1 (`random-trace-id`) set, bit 0 clear: still not sampled. Masking, not truthiness.
        let tp = TraceParent::parse("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-02")
            .expect("valid");
        assert!(!tp.sampled());
    }

    /// Hex is case-insensitive on the wire. Without normalization the same trace arrives under two
    /// spellings and a log query grouped by `trace_id` splits one action into two.
    #[test]
    fn uppercase_hex_normalizes_to_lowercase() {
        let tp = TraceParent::parse("00-4BF92F3577B34DA6A3CE929D0E0E4736-00F067AA0BA902B7-01")
            .expect("valid");
        assert_eq!(tp.trace_id(), "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(tp.parent_id(), "00f067aa0ba902b7");
    }

    /// The spec's forward-compatibility rule: an unknown version may carry extra fields, and we
    /// must read the fields we know rather than rejecting the whole header.
    #[test]
    fn unknown_version_with_extra_fields_still_parses() {
        let tp =
            TraceParent::parse("01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-extra")
                .expect("unknown version must degrade gracefully, not go dark");
        assert_eq!(tp.trace_id(), "4bf92f3577b34da6a3ce929d0e0e4736");
    }

    /// …but version 00 is fully specified, so a trailing field there is malformed rather than
    /// forward-compatible.
    #[test]
    fn version_00_with_extra_fields_is_invalid() {
        assert!(TraceParent::parse(&format!("{SPEC_EXAMPLE}-extra")).is_none());
    }

    #[test]
    fn rejects_the_invalid_forms() {
        let cases = [
            ("", "empty"),
            ("00", "version only"),
            ("00-4bf92f3577b34da6a3ce929d0e0e4736", "no parent or flags"),
            (
                "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7",
                "no flags",
            ),
            (
                "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
                "version ff is reserved",
            ),
            (
                "0-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
                "short version",
            ),
            (
                "zz-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
                "non-hex version",
            ),
            (
                "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
                "all-zero trace id",
            ),
            (
                "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
                "all-zero parent id",
            ),
            (
                "00-4bf92f3577b34da6a3ce929d0e473-00f067aa0ba902b7-01",
                "short trace id",
            ),
            (
                "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902-01",
                "short parent id",
            ),
            (
                "00-4bf92f3577b34da6a3ce929d0e0e473g-00f067aa0ba902b7-01",
                "non-hex trace id",
            ),
            (
                "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-1",
                "short flags",
            ),
            (
                "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-zz",
                "non-hex flags",
            ),
        ];
        for (raw, why) in cases {
            assert!(
                TraceParent::parse(raw).is_none(),
                "expected `{raw}` to be rejected ({why})"
            );
        }
    }

    /// Mirrors `act_span_field_set_is_declared_once` in the e2e gate: the field set is a contract
    /// with the span constructors and the gate, so a rename has to be a deliberate edit here.
    #[test]
    fn root_trace_field_set_is_declared_once() {
        assert_eq!(
            ROOT_TRACE_FIELDS,
            [
                "trace_id",
                "parent_span_id",
                "trace_sampled",
                "vercel_id",
                "vercel_invocation_id",
            ]
        );
    }

    /// `invocation_id` is temper's act-envelope grain and must never be the home of Vercel's
    /// function-invocation id. Cheap, but it is the exact collision the module docs warn about.
    #[test]
    fn root_fields_do_not_collide_with_the_act_field_set() {
        assert!(!ROOT_TRACE_FIELDS.contains(&"invocation_id"));
        assert!(!ROOT_TRACE_FIELDS.contains(&"correlation_id"));
    }

    /// Ties [`ROOT_TRACE_FIELDS`] to the macro that actually builds the span.
    ///
    /// The constant already existed, but nothing connected it to the constructors — they spelled
    /// the fields out separately, in two surfaces, so the list could say one thing and the spans
    /// carry another. Now there is one expansion, and this asserts it declares every name the
    /// constant claims. A field added to the constant without the macro (or the reverse) fails here
    /// instead of silently producing spans the convention gate expects fields on.
    #[test]
    fn macro_declares_every_root_trace_field() {
        let subscriber = tracing_subscriber::registry();
        let _guard = tracing::subscriber::set_default(subscriber);

        let request = http::Request::builder()
            .method("GET")
            .uri("/api/health")
            .body(())
            .expect("request builds");

        let span = crate::root_span!("test_request", request);
        let metadata = span.metadata().expect("span is enabled under a registry");
        let declared: Vec<&str> = metadata.fields().iter().map(|f| f.name()).collect();

        for field in ROOT_TRACE_FIELDS {
            assert!(
                declared.contains(&field),
                "`{field}` is in ROOT_TRACE_FIELDS but the root_span! macro does not declare it, \
                 so the convention gate expects a field no span carries. Declared: {declared:?}"
            );
        }

        // The request-level fields are not in ROOT_TRACE_FIELDS (that constant is inbound trace
        // context only), but a root span without them is not a root span the convention describes.
        for field in ["method", "path", "version", "profile_id"] {
            assert!(declared.contains(&field), "root span lost `{field}`");
        }
    }

    /// The root span must export as `SpanKind::Server`, not the `Internal` default.
    ///
    /// Tempo's span-metrics and service-graph processors derive RED metrics and graph edges only
    /// from `server`/`client` spans, so a root span left at `tracing-opentelemetry`'s `Internal`
    /// default is received and stored fine yet silently absent from `traces_spanmetrics_*` and the
    /// service graph — exactly the symptom that took `temper-api` and `temper-mcp` off the graph
    /// while surfaces whose entry spans already reported `server` stayed on it. `otel.kind` is
    /// consumed by the OTel layer and stripped from the attribute set, so only an exported span can
    /// witness it — a `metadata().fields()` check like the one above would pass on a broken span.
    #[test]
    fn root_span_exports_as_server_kind() {
        use opentelemetry::trace::{SpanKind, TracerProvider as _};
        use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
        use tracing_subscriber::layer::SubscriberExt as _;

        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let tracer = provider.tracer("temper-test");
        let subscriber =
            tracing_subscriber::registry().with(tracing_opentelemetry::layer().with_tracer(tracer));

        tracing::subscriber::with_default(subscriber, || {
            let request = http::Request::builder()
                .method("GET")
                .uri("/api/health")
                .body(())
                .expect("request builds");
            // Dropped at the end of the closure, which ends the OTel span and hands it to the
            // exporter — the same lifetime `request_span` owns in production.
            drop(crate::root_span!("http_request", request));
        });

        provider.force_flush().expect("flush");
        let spans = exporter.get_finished_spans().expect("finished spans");
        let exported = spans.first().expect("the root span was exported");

        assert_eq!(
            exported.span_kind,
            SpanKind::Server,
            "the root span exported as {:?}, not Server — Tempo's span-metrics and service-graph \
             processors will exclude it. See the `otel.kind` field in `root_span!`.",
            exported.span_kind
        );
    }
}
