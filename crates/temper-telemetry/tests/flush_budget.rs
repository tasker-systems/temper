//! A degraded vendor must not lengthen a response without bound.
//!
//! ## The cost this bounds, and why it only appeared now
//!
//! Before the exporter used a working HTTP client, the per-request flush did no I/O — it was a free
//! no-op, and the guide could honestly say *"an exporter that cannot reach its backend must never
//! lengthen a request."* Making export actually work made that sentence false: the flush became a
//! synchronous network round trip on the response path, and the SDK's own ceiling is a hardcoded,
//! non-configurable **5 seconds** (`opentelemetry_sdk-0.32.1/src/trace/span_processor.rs:476`).
//!
//! Measured before the fix, against an endpoint that accepts and never answers: **5.01s added to every
//! request**, indefinitely, with no circuit breaker. On a Vercel function with `maxDuration: 60` that
//! is 8% of the budget per request, paid to a backend that is not a dependency of serving traffic.
//!
//! So this asserts the property the guide claims: a dead vendor costs a bounded amount, once.
//!
//! ## What it does not assert
//!
//! That the span survives. It does not — that is the trade. `flush_within_budget` gives up on the
//! *wait*, not on the export; the blocking task runs on to the SDK's own timeout and the span rides out
//! on a later flush or is lost at freeze. Losing a span is the correct thing to trade for not stalling
//! a request, and stating it here keeps that from looking like an oversight later.

mod common;

use std::time::{Duration, Instant};

/// The budget is 500ms; allow generous slack for a loaded CI runner while staying far below the 5s
/// this exists to avoid. A regression that removed the timeout would land at ~5s and trip this by 3×.
const CEILING: Duration = Duration::from_millis(2500);

#[test]
fn a_dead_vendor_costs_a_bounded_wait_not_the_sdks_five_seconds() {
    let sink = common::Sink::start_silent();
    let endpoint = sink.endpoint();

    temp_env::with_vars(
        [
            ("OTEL_EXPORTER_OTLP_ENDPOINT", Some(endpoint.as_str())),
            ("OTEL_SERVICE_NAME", Some("flush-budget-test")),
            ("OTEL_SDK_DISABLED", None),
            ("RUST_LOG", Some("info")),
        ],
        || {
            temper_telemetry::init_server_logging();

            // `rt` only — a current-thread runtime is enough, and `spawn_blocking` works on it.
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build a runtime");

            // First request: a span is queued, and the flush finds a vendor that never answers.
            drop(tracing::info_span!("http_request", path = "/api/one"));
            let started = Instant::now();
            let reported = rt.block_on(temper_telemetry::export::flush_within_budget());
            let waited = started.elapsed();

            assert!(
                waited < CEILING,
                "a flush against an unresponsive endpoint waited {waited:?}, over the {CEILING:?} \
                 ceiling — the response path is exposed to the SDK's hardcoded 5s timeout, which is \
                 what FLUSH_BUDGET exists to cap"
            );
            assert!(
                reported <= waited + Duration::from_millis(50),
                "flush_within_budget reported {reported:?} but the caller waited {waited:?}; the \
                 returned duration is what the `flush_ms` field reports, so it must not understate \
                 the real cost"
            );

            // Second request, immediately after: the first flush is still in flight, so this one must
            // return at once rather than queue behind it. Without single-flight, N concurrent requests
            // each pay the sum of the ones ahead — measured at 16 concurrent requests against a 300ms
            // endpoint: median 1.226s for one round trip's worth of work.
            drop(tracing::info_span!("http_request", path = "/api/two"));
            let started = Instant::now();
            rt.block_on(temper_telemetry::export::flush_within_budget());
            let second = started.elapsed();

            assert!(
                second < Duration::from_millis(250),
                "a second flush while one is already in flight waited {second:?} — it should return \
                 immediately, because a flush drains the whole queue and a concurrent one has nothing \
                 of its own to do"
            );
        },
    );
}
