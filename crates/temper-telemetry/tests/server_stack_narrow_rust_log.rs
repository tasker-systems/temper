//! A narrowed `RUST_LOG` must not silently turn off span export on the servers.
//!
//! ## The bug this pins
//!
//! `server_stack` used to put one `EnvFilter` in front of every layer, including the export layer.
//! Measured against a real exporter, one span per case:
//!
//! | `RUST_LOG` | spans exported |
//! |---|---|
//! | unset (⇒ `info`) | 1 |
//! | `warn` | **0** |
//! | `temper_api=info` | **0** |
//!
//! The third row is the dangerous one: the level is `info`, the operator did nothing that looks wrong,
//! and export is dead — because a root span's target is `temper_telemetry`, not `temper_api`. Quieting
//! a chatty deployment is an ordinary act, and it silently disabled the trace export it has no visible
//! relationship to.
//!
//! The reverse direction is a data-egress problem rather than a blindness one: a subscriber-wide filter
//! also means `RUST_LOG=debug` *widens* what is exported to a third-party vendor — sqlx's
//! `statements_level` defaults to `Debug`, so query text and bound values would ride out with it. What
//! is logged and what is exported must be independent in both directions.
//!
//! ## Why this drives `init_server_logging` rather than `server_stack`
//!
//! Because `server_stack` is private, and the test that *did* cover this built a replica of it out of
//! the same layers — which an adversarial review showed proves nothing: reverting the real function to
//! the broken arrangement left the whole suite green. This test calls the public seam that production
//! calls, so the only way to make it pass is to fix the code that ships.

mod common;

#[test]
fn a_narrowed_rust_log_does_not_disable_span_export() {
    let sink = common::Sink::start();
    let endpoint = sink.endpoint();

    temp_env::with_vars(
        [
            ("OTEL_EXPORTER_OTLP_ENDPOINT", Some(endpoint.as_str())),
            ("OTEL_SERVICE_NAME", Some("server-stack-filter-test")),
            ("OTEL_SDK_DISABLED", None),
            // The whole point: quiet logs, and export must survive it.
            ("RUST_LOG", Some("warn")),
        ],
        || {
            temper_telemetry::init_server_logging();

            // The grain `root_span!` builds. Under the old subscriber-wide `warn` this span was never
            // recorded at all, so nothing reached the exporter.
            drop(tracing::info_span!("http_request", path = "/api/resources"));
            temper_telemetry::force_flush_spans();
        },
    );

    let request = sink.expect_request(
        "RUST_LOG=warn silently disabled span export — the fmt filter is starving the export layer, \
         which is the exact arrangement `server_stack` was changed away from",
    );
    assert!(
        request.starts_with("POST /v1/traces"),
        "expected an OTLP trace POST, got: {request:?}"
    );
}
