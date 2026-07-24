//! Shared telemetry seam for temper's deployables.
//!
//! Two things live here:
//!
//! - [`init`] — how a temper process logs. One seam for the five binaries that used to configure
//!   `tracing_subscriber` themselves, and the place the OTLP exporter will attach.
//! - This module — reading whatever correlation context arrives on an inbound request and stamping
//!   it onto that request's root span. That is the *extraction* half of W3C trace context, the half
//!   that needs no exporter, no vendor, and no dependency set.
//!
//! ## Why extraction lands before export
//!
//! `docs/development/span-field-conventions.md` recorded the gap this closes: *"No W3C trace
//! context. Nothing extracts or propagates `traceparent`, so spans do not yet join across
//! deployables."* A trace id in today's JSON log lines is worth having before anything is exported —
//! it is what lets a Slack mention's three hops be grepped into one story. When the exporter lands
//! (`temper-telemetry`'s next increment, under goal
//! `019f9404-2a4e-7530-8744-92ae4ab6d83e`), the same extracted context becomes the actual parent
//! rather than a field, and nothing here is thrown away.
//!
//! ## What is deliberately *not* here
//!
//! - **Propagation.** Nothing injects `traceparent` on outbound calls yet. `tracestate` is
//!   consequently not read at all — vendor state exists to be forwarded, and reading it before
//!   there is anywhere to forward it to would be storage with no reader.
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
//! edge. It stops being merely a field when an exporter lands and the extracted context could become
//! a real **parent**: at that point an unauthenticated caller could graft spans onto someone else's
//! trace.
//!
//! **That is decided, and the answer is no.** Temper accepts `traceparent` only from itself
//! (decision `019f95ff-e216-7dd1-b2aa-a49d20b1cd6c`):
//!
//! 1. **Never parent from inbound context** — every root span roots its own trace, every surface,
//!    unconditionally.
//! 2. **Join a trusted caller's trace with an OTel span *link*, recorded post-auth**, not by
//!    parenting.
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
//! `TraceLayer` builds the root span before auth runs. So "extract after auth" can yield a field or
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

pub mod init;

pub use init::{init_cli_logging, init_server_logging};

use http::HeaderMap;
use tracing::Span;

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

/// W3C `traceparent` header name.
const TRACEPARENT: &str = "traceparent";
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
fn header_str<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
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
}
