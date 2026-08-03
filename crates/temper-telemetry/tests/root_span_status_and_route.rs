//! The root span reports whether the request failed, and which route it was — the two things a RED
//! dashboard reads that a constant, statusless span cannot answer.
//!
//! ## What this gates, and why it is not the field-convention gate
//!
//! `tests/e2e/tests/logging_test.rs` asserts the *tracing* shape — that a root span exists and
//! carries `method`/`path`/trace-context. This asserts the *exported OTel* shape, which is a
//! different object: the span metrics a dashboard aggregates are generated from the exported span's
//! **name** and **status**, and both were blind on the three Rust surfaces —
//!
//! - **Status.** Every Rust span was `UNSET`; no `set_status` path existed, so an *Error Rate by
//!   Service* panel read a flat 0% no matter what failed. The rule implemented is the OTel HTTP
//!   server convention — 5xx (and a panic) are `ERROR`, everything else `UNSET` — and the load-bearing
//!   case is the negative one: a **4xx is not an error**, because temper renders several authorization
//!   denials as `404` on purpose, and counting those would make the error rate track the authz gate
//!   *working*.
//! - **Route.** Each surface emitted exactly one span name, with the route only in a `path` attribute
//!   that interpolates UUIDs (889 distinct values in 12h). The exported name is now
//!   `{method} {matched-route-template}`, bounded by the router's finite route set — UUIDs never
//!   appear in it.
//!
//! ## Why the exported span, through the real provider
//!
//! Asserting the recorded `otel.*` *fields* would only prove the values were written, not that
//! `tracing-opentelemetry` consumed them into the span's name and status — which is the whole claim.
//! So this installs the real batch provider (`install_test_provider`), drives real requests through
//! `traced_request` + the real `root_span!` macro, force-flushes, and reads what came out.

#![cfg(feature = "test-support")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use opentelemetry::trace::Status;
use opentelemetry_sdk::trace::InMemorySpanExporter;
use tower::ServiceExt;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

async fn ok() -> &'static str {
    "ok"
}
async fn thing() -> &'static str {
    "thing"
}
async fn boom() -> StatusCode {
    StatusCode::INTERNAL_SERVER_ERROR
}
/// A handler that refuses, the way temper's authz gates do — a `404` that is a decision, not a
/// failure. The rule under test must NOT mark this `ERROR`.
async fn deny() -> StatusCode {
    StatusCode::NOT_FOUND
}

/// A real router sharing the production `traced_request` + `root_span!` macro, so `MatchedPath` and
/// the `otel.*` fields are exercised exactly as they are in `temper-api`/`temper-mcp`.
fn app() -> Router {
    Router::new()
        .route("/ok", get(ok))
        .route("/boom", get(boom))
        .route("/deny", get(deny))
        .route("/things/{id}", get(thing))
        .layer(axum::middleware::from_fn(|request, next| {
            temper_telemetry::traced_request(request, next, |request: &Request<Body>| {
                temper_telemetry::root_span!("http_request", request)
            })
        }))
}

#[test]
fn root_span_reports_status_and_a_bounded_route() {
    // Pinned before `install_test_provider` builds the batch processor: a periodic sweep (default
    // 5s) would export the queue regardless of the per-request flush, so only an explicit flush
    // producing the spans keeps the assertion honest. Same reasoning as
    // `panicking_request_still_flushes`.
    temp_env::with_var("OTEL_BSP_SCHEDULE_DELAY", Some("600000"), || {
        let exporter = InMemorySpanExporter::default();
        assert!(
            temper_telemetry::export::install_test_provider(exporter.clone()),
            "the test provider must install once per process (one test per binary)"
        );
        let layer = temper_telemetry::export::test_export_layer().expect("provider installed");
        let _guard = tracing_subscriber::registry().with(layer).set_default();

        // Current-thread runtime so the request future runs on the thread the subscriber is set on;
        // the exported span is then built where the subscriber is active.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        for (uri, expected) in [
            ("/ok", 200u16),
            ("/boom", 500),
            ("/deny", 404),
            // A real resource-shaped UUID: it must not survive into the span name.
            ("/things/019f97a7-ad61-7e40-b325-73028060ac06", 200),
        ] {
            let resp = rt
                .block_on(app().oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()))
                .expect("request completes");
            assert_eq!(
                resp.status().as_u16(),
                expected,
                "unexpected status for {uri}"
            );
        }

        temper_telemetry::force_flush_spans();

        let spans = exporter.get_finished_spans().expect("finished spans");
        let names: Vec<String> = spans.iter().map(|s| s.name.to_string()).collect();
        let find = |name: &str| {
            spans
                .iter()
                .find(|s| s.name.as_ref() == name)
                .unwrap_or_else(|| panic!("no exported span named `{name}`; exported: {names:?}"))
        };

        // ── Route dimension: exported name is `{method} {route-template}`, bounded and UUID-free ──
        let param = find("GET /things/{id}");
        assert!(
            !param.name.contains("019f97a7"),
            "the resource UUID leaked into the span name `{}` — the route dimension must be the \
             matched template, not the filled path",
            param.name
        );

        // ── Status rule (OTel HTTP server convention) ──
        // 5xx → ERROR.
        assert!(
            matches!(find("GET /boom").status, Status::Error { .. }),
            "a 5xx must set span status ERROR, got {:?}",
            find("GET /boom").status
        );
        // 2xx → UNSET (never OK — the convention reserves OK for an explicit affirmation).
        assert_eq!(
            find("GET /ok").status,
            Status::Unset,
            "a 2xx must leave span status UNSET"
        );
        assert_eq!(find("GET /things/{id}").status, Status::Unset);
        // 4xx → UNSET. The decision the task exists to pin: temper's `404`-as-deny is the gate
        // working, not a service failure. Marking it ERROR would make the error rate track authz.
        assert_eq!(
            find("GET /deny").status,
            Status::Unset,
            "a 4xx — including temper's deliberate 404-as-deny — must NOT be ERROR; otherwise the \
             error rate measures how often the authorization gates refuse a caller"
        );
    });
}
