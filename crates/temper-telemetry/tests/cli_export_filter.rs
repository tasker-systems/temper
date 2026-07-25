//! The CLI stack exports request spans **without** becoming chatty on stderr.
//!
//! Two independent properties, and the whole reason `cli_stack` filters per-layer rather than
//! subscriber-wide. They pull in opposite directions:
//!
//! - export needs `info` spans to reach the OTel layer,
//! - the CLI's stderr contract needs `info` to reach *no* fmt output, since stdout belongs to
//!   `temper … | jq` and stderr is for things a person should read.
//!
//! Measured rather than reasoned about. The arrangement `cli_stack` had before this test — one
//! `EnvFilter` in front of every layer — exports **zero** spans at the CLI's default `warn`, which is
//! asserted below so nobody folds the per-layer filters back into one. That failure mode is silent:
//! export appears configured, the endpoint resolves, "span export on" is logged, and nothing arrives.
//!
//! Sibling of PR #535's flush test, and it exists for the same reason: this goal has already shipped
//! one correct-looking layer arrangement that exported nothing.
//!
//! ## Why this is one long test and not three short ones
//!
//! Same reason `crates/temper-api/tests/telemetry_link_test.rs` gives: the provider is a process-global
//! `OnceLock` and installs once. nextest gives each test its own process and `cargo test` does not, so
//! separate `#[test]` functions each installing a provider pass under the runner this repo uses and fail
//! under the one someone reaches for by hand (`cargo test -p temper-telemetry`). A gate that silently
//! degrades depending on the runner is worse than one that is simply longer to read.

use opentelemetry_sdk::trace::InMemorySpanExporter;
use std::sync::{Arc, Mutex};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::{EnvFilter, Layer};

/// Captures fmt output so the stderr-quietness half can be asserted rather than assumed.
#[derive(Clone, Default)]
struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl CaptureWriter {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().expect("capture buffer poisoned").clone())
            .expect("tracing output is UTF-8")
    }
}

impl std::io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("capture buffer poisoned")
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'w> tracing_subscriber::fmt::MakeWriter<'w> for CaptureWriter {
    type Writer = Self;
    fn make_writer(&'w self) -> Self::Writer {
        self.clone()
    }
}

/// One process, one `PROVIDER` (a `OnceLock`), so both arrangements share the installed provider and
/// read the same exporter. Layers are per-subscriber, which is what makes that sound.
#[test]
fn cli_stack_exports_info_spans_while_stderr_stays_quiet() {
    let exporter = InMemorySpanExporter::default();
    assert!(
        temper_telemetry::export::install_test_provider(exporter.clone()),
        "provider must install exactly once per process"
    );

    // ── The arrangement `cli_stack` uses: filter scoped to fmt, export layer carrying its own. ──
    let fmt_out = CaptureWriter::default();
    {
        let subscriber = tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(fmt_out.clone())
                    .with_filter(EnvFilter::new("warn")),
            )
            .with(
                temper_telemetry::export::test_export_layer().with_filter(EnvFilter::new("info")),
            );
        let _guard = tracing::subscriber::set_default(subscriber);

        drop(tracing::info_span!(
            "http_request",
            request = "GET /api/resources"
        ));
        tracing::warn!("a warning a person should see");
    }
    temper_telemetry::force_flush_spans();

    let exported: Vec<String> = exporter
        .get_finished_spans()
        .expect("exporter readable")
        .iter()
        .map(|s| s.name.to_string())
        .collect();
    assert!(
        exported.iter().any(|n| n == "http_request"),
        "the CLI stack must export the request span; exported: {exported:?}"
    );

    let stderr = fmt_out.contents();
    assert!(
        stderr.contains("a warning a person should see"),
        "warn-level output must still reach stderr; got: {stderr:?}"
    );
    assert!(
        !stderr.contains("/api/resources"),
        "turning on export must not make the CLI chatty — an info span leaked into fmt output: \
         {stderr:?}"
    );

    // ── The arrangement to never go back to: one subscriber-wide filter in front of both. ──
    let before = exporter
        .get_finished_spans()
        .expect("exporter readable")
        .len();
    {
        let subscriber = tracing_subscriber::registry()
            .with(EnvFilter::new("warn"))
            .with(temper_telemetry::export::test_export_layer());
        let _guard = tracing::subscriber::set_default(subscriber);
        drop(tracing::info_span!(
            "http_request",
            request = "GET /api/starved"
        ));
    }
    temper_telemetry::force_flush_spans();
    let after = exporter
        .get_finished_spans()
        .expect("exporter readable")
        .len();

    assert_eq!(
        before, after,
        "a subscriber-wide `warn` filter starves the export layer — this is why cli_stack filters \
         per-layer, and folding them back into one would silently export nothing"
    );

    // Same process, same provider — see the module docs on why this is a call and not its own `#[test]`.
    injection_reads_the_span_it_is_called_from();
}

/// Injection reads the span it is called from — not an ambient or parent one.
///
/// The property that makes `inject_trace_context` usable: `temper-client` calls it inside the
/// `.instrument(span)` body precisely so the receiver links to *that request's* span. Calling it a few
/// lines earlier, while building the `RequestBuilder`, would compile and would inject *something* —
/// whatever happened to be current — and the resulting trace would look joined while pointing at the
/// wrong span. That is strictly worse than not injecting, and it is invisible without this assertion.
///
/// Asserted by *distinctness* rather than by naming an id: two sibling spans must yield two different
/// parent ids, a nested span must yield the innermost one, and no span at all must yield nothing. Those
/// three together pin "reads the current span" without needing to reach into the OTel context to learn
/// what the id should be.
///
/// A real tracer is required for a span to have a context worth injecting, which is why this lives here
/// — beside the provider — rather than in `propagate`'s own unit tests.
fn injection_reads_the_span_it_is_called_from() {
    let layer =
        temper_telemetry::export::test_export_layer().expect("the caller installed the provider");
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    fn parent_id_now() -> Option<String> {
        let mut headers = http::HeaderMap::new();
        temper_telemetry::inject_trace_context(&mut headers);
        let raw = headers.get("traceparent")?.to_str().ok()?.to_string();
        // Field 3 of `version-traceid-parentid-flags`.
        raw.split('-').nth(2).map(str::to_string)
    }

    assert!(
        parent_id_now().is_none(),
        "outside any span there is nothing to inject, so no header may be written"
    );

    let (first, second) = {
        let a = tracing::info_span!("outbound_a");
        let _ea = a.enter();
        let first = parent_id_now().expect("inside a span, a traceparent must be injected");

        // Nested: the *innermost* span is the one an outbound call belongs to.
        let b = tracing::info_span!("outbound_b");
        let _eb = b.enter();
        let second = parent_id_now().expect("inside a nested span, a traceparent must be injected");
        (first, second)
    };

    assert_ne!(
        first, second,
        "two different spans injected the same parent id, so injection is not reading the current \
         span — a receiver would link every request to whichever span happened to be outermost"
    );
    assert!(
        parent_id_now().is_none(),
        "both spans have exited, so there is again nothing to inject"
    );
}
