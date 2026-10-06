//! An https collector that answers 3xx must not be able to move the export — and the
//! `OTEL_EXPORTER_OTLP_HEADERS` credential with it — somewhere else, plaintext included. reqwest's
//! default policy follows up to 10 redirects and keeps custom header names, so the exporter's
//! client is built with redirects off (`export::exporter_http_client`).

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;

/// A collector that answers every request `307` to `location`, reporting each request it saw.
fn redirecting_collector(location: String) -> (u16, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let port = listener
        .local_addr()
        .expect("listener has an address")
        .port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 8192];
            let read = stream.read(&mut buf).unwrap_or(0);
            let response = format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n"
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            if tx
                .send(String::from_utf8_lossy(&buf[..read]).to_string())
                .is_err()
            {
                return;
            }
        }
    });
    (port, rx)
}

#[test]
fn a_redirecting_collector_does_not_move_the_export() {
    let elsewhere = common::Sink::start();
    let (port, seen) = redirecting_collector(format!("{}/v1/traces", elsewhere.endpoint()));
    let endpoint = format!("http://127.0.0.1:{port}");

    temp_env::with_vars(
        [
            ("OTEL_EXPORTER_OTLP_ENDPOINT", Some(endpoint.as_str())),
            ("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", None),
            ("OTEL_EXPORTER_OTLP_HEADERS", Some("x-api-key=s3cret")),
            ("OTEL_SERVICE_NAME", Some("no-redirect-test")),
            ("OTEL_SDK_DISABLED", None),
            ("RUST_LOG", Some("info")),
        ],
        || {
            temper_telemetry::init_server_logging();
            drop(tracing::info_span!(
                "http_request",
                request = "GET /api/health"
            ));
            temper_telemetry::force_flush_spans();
        },
    );

    seen.recv_timeout(common::EXPORT_WAIT)
        .expect("the export never reached the configured collector");
    elsewhere.expect_silence(
        "the exporter followed the collector's redirect, credential header and all",
    );
}
