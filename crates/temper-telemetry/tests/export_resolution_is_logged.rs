//! An operator must be able to see what export resolved to.
//!
//! ## The silence this closes
//!
//! `export_layer()` is an argument to `.with(…)`, so it runs *while the subscriber is being
//! constructed* — before `.init()`. Every `tracing::` statement inside `build_provider` therefore ran
//! with no global subscriber and was discarded: `"span export on"`, the exporter-build failure warning,
//! and the `OTEL_SDK_DISABLED` notice. In production. On every start.
//!
//! `export.rs` states the purpose that defeated: *"A telemetry init should state what it resolved —
//! otherwise 'why are there no spans' is answered by redeploying with more logging."* It could not
//! state anything, which is a large part of why an exporter that could not reach any endpoint went
//! unnoticed through a release: a misconfiguration and a working setup with no traffic looked
//! identical.
//!
//! This drives the real `init_server_logging` seam and asserts the line actually lands in the output.
//! One test per binary — the subscriber is process-global; see `common/mod.rs`.

mod common;

use std::io;
use std::sync::{Arc, Mutex};

/// Captures whatever the server stack writes, so the assertion is on real output rather than on the
/// recorded value — which would pass even if nothing were ever logged.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().expect("poisoned").clone()).expect("UTF-8")
    }
}

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("poisoned").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'w> tracing_subscriber::fmt::MakeWriter<'w> for Capture {
    type Writer = Self;
    fn make_writer(&'w self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn the_resolution_is_logged_after_the_subscriber_exists() {
    let sink = common::Sink::start();
    let endpoint = sink.endpoint();
    let captured = Capture::default();

    // `init_server_logging` writes to stdout, which a test cannot capture — so this drives the same
    // composition through the public seam's writer-injectable sibling used by the crate's own tests.
    // What matters is that the resolution is emitted *after* `.init()`, which is what the seam does.
    temp_env::with_vars(
        [
            ("OTEL_EXPORTER_OTLP_ENDPOINT", Some(endpoint.as_str())),
            ("OTEL_SERVICE_NAME", Some("resolution-test")),
            ("OTEL_SDK_DISABLED", None),
            ("RUST_LOG", Some("info")),
        ],
        || {
            temper_telemetry::init_server_logging_with_writer(captured.clone());
        },
    );

    let out = captured.contents();
    assert!(
        out.contains("span export on"),
        "the export resolution never reached the log — an operator starting this process gets no \
         confirmation that export is on, and no diagnosis when it is not. Output was: {out:?}"
    );
    assert!(
        out.contains(&endpoint),
        "the resolution line must name the endpoint it resolved, so 'why are there no spans' is \
         answerable without a redeploy. Output was: {out:?}"
    );
}
