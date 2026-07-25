//! A panicking handler must still export its own span.
//!
//! ## Why this is the request you most want a trace for
//!
//! `traced_request` ends the root span and flushes *after* awaiting the handler. When the handler
//! panics, the await unwinds: `drop(span)` and the flush are ordinary statements, and unwinding skips
//! ordinary statements. The span itself is still *queued* — the `span` local is dropped during unwind,
//! which is what ends the OpenTelemetry span — but nothing drains the queue. It rides out on a later
//! request's flush, or is lost when the Vercel sandbox freezes. On a low-traffic function, "a later
//! request" may never come.
//!
//! So the 500 that woke someone up is precisely the trace that is missing, while every healthy request
//! around it exported fine. That asymmetry is the bug: it is not "we lose some spans", it is "we lose
//! the spans correlated with failure".
//!
//! Pointedly, PR #540 applied the *inverse* reasoning to the CLI — *"a command that **failed** is the
//! one someone wants to look at"* — and did not carry it to the server.
//!
//! ## Why the fix cannot be a `Drop` guard
//!
//! The obvious shape — a scope guard that flushes on unwind — does not exist, because
//! `flush_within_budget` is `async` and `Drop::drop` cannot await. Blocking inside `drop` on a Tokio
//! worker is worse than the bug.
//!
//! So the fix catches the unwind, flushes on both paths, and **resumes the unwind**. That last part is
//! what makes it a telemetry change rather than a behavioural one: the panic still propagates exactly
//! as before, so whatever the caller saw for a panicking handler, it still sees. This is deliberately
//! *not* `CatchPanicLayer`, which would convert a panic into a 500 and change what clients observe.
//!
//! ## Why a real Router
//!
//! The bug is in the ordering of instrument → respond → end span → flush, and only a real middleware
//! stack has that ordering. A test that called `flush_within_budget` itself would assert the flush
//! works, which was never in doubt.
//!
//! ## `OTEL_BSP_SCHEDULE_DELAY` is load-bearing, and the first version of this test was wrong without it
//!
//! `BatchSpanProcessor` also exports on a timer — `OTEL_BSP_SCHEDULE_DELAY`, defaulting to **5000ms**
//! (`opentelemetry_sdk-0.32/src/trace/span_processor.rs:55`). Written without pinning it, this test
//! **passed against the unfixed code**, taking 5.026s to do it: the span was queued by the unwind and
//! then swept up by the periodic export, which looks identical at the sink to a flush that worked.
//!
//! Racing that timer with a tight receive deadline would work and would be flaky. Pinning the delay
//! far beyond the test's lifetime removes the confound instead: with the timer unable to fire, the
//! **only** thing that can produce an export is an explicit flush, so a request arriving at the sink
//! is proof the flush ran on the unwinding path. The assertion means what it says only because of
//! this variable.

mod common;

use std::panic::AssertUnwindSafe;

use axum::body::Body;
use axum::http::Request;
use axum::routing::get;
use axum::Router;
use tower::ServiceExt;

/// A handler that panics. Named rather than a closure so it can declare a return type: a diverging
/// async block otherwise leans on never-type fallback, which `rust_2024_compatibility` denies.
async fn boom() -> &'static str {
    panic!("handler panicked on purpose")
}

/// Build the router both cases drive: one healthy route, one that panics, sharing the real
/// `traced_request` middleware.
fn app() -> Router {
    Router::new()
        .route("/fine", get(|| async { "ok" }))
        .route("/boom", get(boom))
        .layer(axum::middleware::from_fn(|request, next| {
            temper_telemetry::traced_request(request, next, |request: &Request<Body>| {
                tracing::info_span!("http_request", path = %request.uri().path())
            })
        }))
}

#[test]
fn a_panicking_handler_still_exports_its_span() {
    let sink = common::Sink::start();
    let endpoint = sink.endpoint();

    temp_env::with_vars(
        [
            ("OTEL_EXPORTER_OTLP_ENDPOINT", Some(endpoint.as_str())),
            ("OTEL_SERVICE_NAME", Some("panic-flush-test")),
            ("OTEL_SDK_DISABLED", None),
            ("RUST_LOG", Some("info")),
            // Ten minutes: far beyond this test's lifetime, so the periodic export can never fire and
            // an explicit flush is the only thing that can reach the sink. See the module docs — the
            // first version of this test passed against the unfixed code without it.
            ("OTEL_BSP_SCHEDULE_DELAY", Some("600000")),
        ],
        || {
            temper_telemetry::init_server_logging();

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build a runtime");

            // Baseline: a healthy request exports. If this ever fails, the test is broken rather than
            // the code, and the panic case below would be unreadable without it.
            let response = rt.block_on(async {
                app()
                    .oneshot(
                        Request::builder()
                            .uri("/fine")
                            .body(Body::empty())
                            .expect("build request"),
                    )
                    .await
                    .expect("healthy request completes")
            });
            assert_eq!(response.status(), 200, "the healthy route returns 200");
            sink.expect_request("a healthy request must export its span");

            // The panicking request. `block_on` propagates the handler's panic synchronously, so
            // `catch_unwind` here observes it without killing the test process.
            //
            // The hook is silenced only around this call: the panic is expected, and letting it print
            // a backtrace makes a passing run look like a failing one.
            let previous_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
                rt.block_on(async {
                    app()
                        .oneshot(
                            Request::builder()
                                .uri("/boom")
                                .body(Body::empty())
                                .expect("build request"),
                        )
                        .await
                })
            }));
            std::panic::set_hook(previous_hook);

            assert!(
                outcome.is_err(),
                "the panic must still propagate — the fix flushes and resumes the unwind, it does \
                 not swallow the panic. If this assertion fails, something turned the panic into a \
                 response and changed what callers observe."
            );

            sink.expect_request(
                "a panicking request must export its span before the unwind continues — this is the \
                 request most worth having a trace for, and the flush is skipped by unwinding unless \
                 it is explicitly run on that path",
            );
        },
    );
}
