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
//! ## What it asserts, and what it deliberately does not
//!
//! It used to read one 4096-byte chunk and check the request line said `POST /v1/traces` with a
//! protobuf content type. That would have passed with an **empty body, a mangled protobuf, or the
//! resource attributes dropped** — so the only live-transport test in the repo could not see a
//! serialization or `service.name` regression. It now reads the whole body by `Content-Length` and
//! decodes it as an `ExportTraceServiceRequest`, asserting the span name and `service.name` survive
//! the round trip.
//!
//! Still out of scope, and stated so this is not mistaken for more: a local socket is not a vendor.
//! Endpoint suffixes, authentication headers, and whether a vendor *accepts* the payload are the
//! first live-vendor run's job (goal `019f9404-2a4e-7530-8744-92ae4ab6d83e`, operator step 1).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValueInner;
use prost::Message;

/// One captured request: its head (for the request line and headers) and its full body bytes.
struct Captured {
    head: String,
    body: Vec<u8>,
}

/// Accept exactly one request, answer `200`, and report the request line, headers, and **whole body**.
///
/// Hand-rolled rather than reaching for a mock-HTTP crate because the assertion is *"a real socket
/// received a real POST"* — a library that intercepts at a higher level would re-introduce the very
/// gap this test exists to close.
///
/// Reading to `Content-Length` rather than taking one `read()` is what makes the body decodable: a
/// single read returns whatever happened to arrive in the first segment, which for a payload split
/// across TCP segments is a truncated protobuf that fails to decode for reasons that have nothing to
/// do with the exporter.
fn one_shot_http_sink() -> (u16, mpsc::Receiver<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let port = listener
        .local_addr()
        .expect("listener has an address")
        .port();
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];

            // Read until the header terminator is in hand.
            let header_end = loop {
                if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break at + 4;
                }
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break buf.len(),
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            };

            let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
            let content_length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);

            let mut body = buf[header_end..].to_vec();
            while body.len() < content_length {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => body.extend_from_slice(&chunk[..n]),
                }
            }

            // Answer so the exporter sees success rather than a transport error.
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/x-protobuf\r\nContent-Length: 0\r\n\r\n",
            );
            let _ = stream.flush();
            let _ = tx.send(Captured { head, body });
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

    let captured = rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_else(|e| {
            panic!(
            "no OTLP request reached the endpoint within 10s ({e}). The export thread most likely \
             died — check for `there is no reactor running` on the \
             `OpenTelemetry.Traces.BatchProcessor` thread, which means an async HTTP client is \
             configured where a blocking one is required (see Cargo.toml)."
        )
        });

    let head = &captured.head;
    assert!(
        head.starts_with("POST /v1/traces"),
        "expected an OTLP/HTTP trace POST, got: {head:?}"
    );
    assert!(
        head.contains("application/x-protobuf"),
        "expected the protobuf encoding `http-proto` selects, got: {head:?}"
    );

    // The body is uncompressed here because the test sets no `OTEL_EXPORTER_OTLP_COMPRESSION`. Assert
    // it rather than assume it: with gzip on, the decode below would fail with a confusing protobuf
    // error rather than a legible one.
    assert!(
        !head.to_ascii_lowercase().contains("content-encoding: gzip"),
        "this test decodes the body directly, so it must not be compressed: {head:?}"
    );
    assert!(
        !captured.body.is_empty(),
        "the POST carried an empty body — the request line alone is not evidence that any span was \
         serialized, which is exactly what this test used to accept"
    );

    let decoded = ExportTraceServiceRequest::decode(captured.body.as_slice())
        .expect("the POSTed body decodes as an OTLP ExportTraceServiceRequest");

    let resource_spans = decoded
        .resource_spans
        .first()
        .expect("the export carries at least one ResourceSpans");

    // `service.name` rides on the Resource, not the span. Dropping it is the single most consequential
    // silent regression available here: every span still exports, and a vendor files them all under
    // "unknown_service", which looks like a vendor problem for as long as it takes to find.
    let service_name = resource_spans
        .resource
        .as_ref()
        .expect("ResourceSpans carries a Resource")
        .attributes
        .iter()
        .find(|kv| kv.key == "service.name")
        .and_then(|kv| kv.value.as_ref())
        .and_then(|v| match v.value.as_ref() {
            Some(AnyValueInner::StringValue(s)) => Some(s.as_str()),
            _ => None,
        })
        .expect("the Resource carries a string service.name");
    assert_eq!(
        service_name, "temper-telemetry-live-test",
        "OTEL_SERVICE_NAME must reach the wire as the Resource's service.name"
    );

    let span_names: Vec<&str> = resource_spans
        .scope_spans
        .iter()
        .flat_map(|scope| scope.spans.iter())
        .map(|span| span.name.as_str())
        .collect();
    assert!(
        span_names.contains(&"http_request"),
        "the exported payload must contain the span that was recorded; got {span_names:?}"
    );
}
