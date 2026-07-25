//! The exporter reaches a **real** HTTP endpoint — the gate the in-memory flush test cannot be.
//!
//! ## Why this exists
//!
//! PR #535 shipped OTLP export with a flush test that asserted spans reached an
//! `InMemorySpanExporter`. That proved the layer ordering and the flush seam, and it is still the
//! right test for those. But an in-memory exporter needs **no HTTP client and no async runtime**, so
//! it could not observe that the configured HTTP client was unusable — and it was:
//!
//! ```text
//! thread 'OpenTelemetry.Traces.BatchProcessor' panicked at …:
//! there is no reactor running, must be called from the context of a Tokio 1.x runtime
//! ```
//!
//! `BatchSpanProcessor` exports from a dedicated OS thread, and the **async** `reqwest` client needs an
//! ambient Tokio reactor on the thread driving the request. That thread has none — and being inside a
//! runtime *at the call site* does not help, which is what made the bug survive review: the servers are
//! Tokio processes throughout, so "we're in a runtime" was true and irrelevant. Every span was dropped,
//! and the only symptom was a `warn` from our own flush path saying spans may be lost.
//!
//! The fix is the `reqwest-blocking-client` feature (see `Cargo.toml`, where the reasoning lives beside
//! the dependency). This test is what makes the fix stick: it fails on the async client and passes on
//! the blocking one, so nobody can restore `reqwest-client` — the more natural-looking choice for an
//! async codebase — without a red build.
//!
//! ## Scope, stated so it is not mistaken for more
//!
//! A local socket is not a vendor. This asserts that a well-formed OTLP/HTTP POST leaves the process
//! and that the export thread does not die — the two things that were broken. Endpoint suffixes,
//! authentication headers, and whether a vendor accepts the payload are still the first live-vendor
//! run's job (goal `019f9404-2a4e-7530-8744-92ae4ab6d83e`, operator step 1).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

/// Accept exactly one request, answer `200`, and report its first bytes.
///
/// Hand-rolled rather than reaching for a mock-HTTP crate because the assertion is *"a real socket
/// received a real POST"* — a library that intercepts at a higher level would re-introduce the very
/// gap this test exists to close.
fn one_shot_http_sink() -> (u16, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let port = listener
        .local_addr()
        .expect("listener has an address")
        .port();
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let read = stream.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..read]).to_string();
            // Answer so the exporter sees success rather than a transport error.
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/x-protobuf\r\nContent-Length: 0\r\n\r\n",
            );
            let _ = stream.flush();
            let _ = tx.send(head);
        }
    });

    (port, rx)
}

#[test]
fn a_span_reaches_a_real_endpoint_over_otlp_http() {
    let (port, rx) = one_shot_http_sink();
    let endpoint = format!("http://127.0.0.1:{port}");

    temp_env::with_vars(
        [
            ("OTEL_EXPORTER_OTLP_ENDPOINT", Some(endpoint.as_str())),
            ("OTEL_SERVICE_NAME", Some("temper-telemetry-live-test")),
            // Keep the SDK's own diagnostics out of the assertion path.
            ("OTEL_SDK_DISABLED", None),
        ],
        || {
            // The production seam, not a replica: whatever `init_server_logging` composes is what
            // deploys, so a client misconfiguration has to show up here.
            temper_telemetry::init_server_logging();

            // `info`, because that is what the export layer admits; see `init::EXPORT_FILTER`.
            drop(tracing::info_span!("http_request", path = "/live/probe"));
            temper_telemetry::force_flush_spans();
        },
    );

    let request = rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_else(|e| {
            panic!(
            "no OTLP request reached the endpoint within 10s ({e}). The export thread most likely \
             died — check for `there is no reactor running` on the \
             `OpenTelemetry.Traces.BatchProcessor` thread, which means an async HTTP client is \
             configured where a blocking one is required (see Cargo.toml)."
        )
        });

    assert!(
        request.starts_with("POST /v1/traces"),
        "expected an OTLP/HTTP trace POST, got: {request:?}"
    );
    assert!(
        request.contains("application/x-protobuf"),
        "expected the protobuf encoding `http-proto` selects, got: {request:?}"
    );
}
