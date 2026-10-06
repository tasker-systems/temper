//! The exporter sends to the URL `vet_endpoint` vetted — never one the SDK resolves for itself.
//!
//! The witness is a signal-specific value with surrounding whitespace, which a secret store can
//! leave behind. We trim it and vet it; the SDK's own env reading would not trim, would fail to
//! parse it as an `http::Uri`, and would fall back to `OTEL_EXPORTER_OTLP_ENDPOINT` — a URL nobody
//! vetted, and the one the collector credential would then ride.

mod common;

#[test]
fn a_padded_traces_endpoint_is_used_as_vetted_not_swapped_for_the_general_one() {
    let vetted = common::Sink::start();
    let general = common::Sink::start();
    let traces = format!(" {}/v1/traces\n", vetted.endpoint());

    temp_env::with_vars(
        [
            ("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", Some(traces.as_str())),
            (
                "OTEL_EXPORTER_OTLP_ENDPOINT",
                Some(general.endpoint().as_str()),
            ),
            ("OTEL_SERVICE_NAME", Some("vetted-url-test")),
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

    let request = vetted.expect_request("no export reached the vetted traces endpoint");
    assert!(request.starts_with("POST /v1/traces "), "{request}");
    general.expect_silence("the export went to the general endpoint instead of the vetted one");
}
