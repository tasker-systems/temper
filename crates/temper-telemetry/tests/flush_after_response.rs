//! With an `AfterResponse` from the host, a request's span flush runs after its response, not before.
//!
//! The property is the one that matters to a client: the response does not wait on the telemetry
//! backend. So the test holds the handed-off work instead of running it. If the response arrives
//! and nothing has reached the collector, the flush was not on the response path. Running the held
//! work must then deliver the span, which proves the flush was handed off rather than dropped.
//!
//! The batch processor's periodic export is pushed far past the test's lifetime, so only an
//! explicit flush can reach the collector.

mod common;

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::Request;
use axum::routing::get;
use axum::Router;
use temper_telemetry::after_response::AfterResponseFuture;
use temper_telemetry::AfterResponse;
use tower::ServiceExt;

#[test]
fn the_flush_runs_after_the_response_and_still_delivers_the_span() {
    let sink = common::Sink::start();
    let endpoint = sink.endpoint();

    temp_env::with_vars(
        [
            ("OTEL_EXPORTER_OTLP_ENDPOINT", Some(endpoint.as_str())),
            ("OTEL_SERVICE_NAME", Some("after-response-test")),
            ("OTEL_SDK_DISABLED", None),
            ("RUST_LOG", Some("info")),
            ("OTEL_BSP_SCHEDULE_DELAY", Some("600000")),
        ],
        || {
            temper_telemetry::init_server_logging();

            let held: Arc<Mutex<Vec<AfterResponseFuture>>> = Arc::default();
            let host = Arc::clone(&held);
            let after_response = AfterResponse::new(move |future| {
                host.lock().expect("held-work lock").push(future);
            });

            let app = Router::new()
                .route("/fine", get(|| async { "ok" }))
                .layer(axum::middleware::from_fn(|request, next| {
                    temper_telemetry::traced_request(request, next, |request: &Request<Body>| {
                        tracing::info_span!("http_request", path = %request.uri().path())
                    })
                }));

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build a runtime");

            let mut request = Request::builder()
                .uri("/fine")
                .body(Body::empty())
                .expect("build request");
            request.extensions_mut().insert(after_response);

            let response = rt
                .block_on(app.oneshot(request))
                .expect("the request completes");
            assert_eq!(response.status(), 200);

            sink.expect_silence(
                "the response came back after an export: with an AfterResponse the flush must be \
                 handed to the host, not run before the response",
            );

            let work: Vec<_> = std::mem::take(&mut *held.lock().expect("held-work lock"));
            assert_eq!(work.len(), 1, "exactly one flush is handed to the host");
            rt.block_on(async {
                for future in work {
                    future.await;
                }
            });

            sink.expect_request(
                "running the handed-off flush must deliver the request's span — if it does not, the \
                 flush was dropped rather than deferred",
            );
        },
    );
}
