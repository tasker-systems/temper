//! The CLI's opt-in gate, driven through the real seam.
//!
//! `TEMPER_CLI_TRACE` exists because `OTEL_EXPORTER_OTLP_ENDPOINT` is often already set in a
//! developer's shell for an unrelated project, and `temper` must not start shipping vault activity to
//! a collector configured for something else. That is a privacy promise, and until this test **nothing
//! asserted it** — an adversarial review inverted the gate so the CLI exported whenever an endpoint
//! existed, and all 31 tests stayed green, because `cli_export_layer` and `cli_export_opted_in` had
//! zero test references anywhere in the repo.
//!
//! This is the negative case, which is the one that carries the promise: **endpoint configured, opt-in
//! absent, nothing may leave the process.** The positive case is covered by `live_export_client.rs`
//! (server seam) and by the end-to-end CLI verification recorded on task
//! `019f97a7-ad61-7e40-b325-73028060ac06`.
//!
//! One test per binary — see `common/mod.rs` on why.

mod common;

#[test]
fn an_endpoint_alone_does_not_make_the_cli_export() {
    let sink = common::Sink::start();
    let endpoint = sink.endpoint();

    temp_env::with_vars(
        [
            // Exactly the situation the gate exists for: a developer's ambient collector.
            ("OTEL_EXPORTER_OTLP_ENDPOINT", Some(endpoint.as_str())),
            ("OTEL_SERVICE_NAME", Some("cli-gate-test")),
            ("OTEL_SDK_DISABLED", None),
            // The opt-in is ABSENT. Nothing else about this environment says "do not export".
            ("TEMPER_CLI_TRACE", None),
            // `info` so the span is enabled for the fmt layer too — this test must fail because of the
            // gate, not because the span was filtered out for an unrelated reason.
            ("RUST_LOG", Some("info")),
        ],
        || {
            temper_telemetry::init_cli_logging();

            drop(tracing::info_span!(
                "http_request",
                request = "GET /api/resources"
            ));
            // A no-op when no provider was installed, which is itself the assertion below.
            temper_telemetry::force_flush_spans();
        },
    );

    sink.expect_silence(
        "the CLI exported with no TEMPER_CLI_TRACE opt-in — an ambient OTEL_EXPORTER_OTLP_ENDPOINT \
         must never be sufficient on its own",
    );
}
