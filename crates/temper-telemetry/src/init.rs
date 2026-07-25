//! One place where a temper process decides how it logs.
//!
//! Before this module there were five `tracing_subscriber::fmt()` calls — `api/axum.rs`,
//! `api/mcp.rs`, `api/internal.rs`, `crates/temper-api/src/main.rs`, and
//! `crates/temper-cli/src/main.rs`. Four were byte-identical and the fifth deliberately was not.
//! Nothing linked them, so a change to how temper logs had to be made four times and remembered
//! not to be made a fifth.
//!
//! ## Two shapes, because there are two audiences
//!
//! [`init_server_logging`] is for the long-lived server processes: JSON on stdout, defaulting to
//! `info`. Their stdout *is* the log stream — Vercel and every container runtime collect it.
//!
//! [`init_cli_logging`] is for the `temper` binary: human-readable on **stderr**, defaulting to
//! `warn`. The CLI's stdout is reserved for machine-readable JSON/TOON so `temper … | jq` stays
//! clean, and ONNX Runtime's `ort` INFO chatter would otherwise interleave with a command's output
//! and break parsing. That divergence is a contract, not drift — which is why it lives here as a
//! named variant with a test behind it, rather than as a comment in one `main` that a future
//! consolidation could tidy away.
//!
//! ## Why `registry()` and layers rather than the `fmt()` builder
//!
//! `tracing_subscriber::fmt()` returns a builder that finishes into a whole subscriber; there is no
//! seam to add anything beside it. The OTLP exporter under goal
//! `019f9404-2a4e-7530-8744-92ae4ab6d83e` is a `tracing-opentelemetry` **layer**, so a `fmt()`-based
//! seam would have to be rewritten to accept it. Composing over `Registry` from the start means that
//! increment adds one `.with(…)` to each stack below and changes nothing else. `Registry` is also
//! what gives the exporter its span storage.
//!
//! The output is unchanged by that choice, and `stacks_match_the_fmt_builder_they_replaced` in this
//! module's tests is the proof rather than the claim: it renders the same event through both the old
//! builder and the new stack and compares, modulo the timestamp.
//!
//! ## Both stacks export; only the CLI asks first
//!
//! The servers export whenever an OTLP endpoint is configured — a deployment's environment is set
//! deliberately, per project. The CLI runs on someone's laptop where `OTEL_EXPORTER_OTLP_ENDPOINT`
//! may already point at an unrelated collector, so it needs a second, temper-owned switch
//! (`TEMPER_CLI_TRACE`) before it will send anything. See `export::cli_export_opted_in`.
//!
//! An earlier revision of this module asserted the CLI must *never* export, and framed it as settled:
//! *"that is a decision with its own consequences, not an omission to tidy up."* It was not settled —
//! it had never reached the person who owns it, who did want CLI opt-in. That is why the paragraph
//! above explains a constraint (an ambient endpoint variable) instead of forbidding a direction, and
//! it is worth remembering as a shape to avoid: a comment records an author's reasoning, and cannot
//! stand in for a decision the author was not the one to make. Tracked as task
//! `019f97a8-cbdf-72b0-b4f2-f1996288a2f5`.

use tracing::Subscriber;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// Default filter for the server processes when `RUST_LOG` says nothing.
const SERVER_DEFAULT_FILTER: &str = "info";

/// Default filter for the CLI when `RUST_LOG` says nothing.
///
/// Quieter than the servers on purpose: a person running one command wants their output, not a
/// narration of it. `RUST_LOG=info temper …` opts back in.
const CLI_DEFAULT_FILTER: &str = "warn";

/// Filter for the CLI's export layer, independent of what the fmt layer shows.
///
/// `info` because that is the grain the request spans are declared at — `root_span!` builds
/// `info_span!` on both server surfaces, and temper-client's outbound span matches them. Exporting
/// `debug` from a CLI run would ship a large volume of spans nobody asked to pay for; exporting at
/// `warn` (the fmt default) would ship none at all.
const EXPORT_FILTER: &str = "info";

/// Resolve `RUST_LOG`, falling back to `default` when it is unset or unparseable.
///
/// An unparseable `RUST_LOG` falls back rather than failing: refusing to start a server because
/// someone fat-fingered a log directive would be a worse failure than logging at the default.
fn env_filter(default: &str) -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default))
}

/// The server logging stack: JSON records, one per line, over `writer`.
///
/// Split from [`init_server_logging`] so tests can drive it with a capture writer and an explicit
/// filter. `init_server_logging` supplies the two production arguments and installs it.
///
/// The `export_layer` call is the whole of what the OTLP increment added here — exactly the one
/// `.with(…)` the `Registry`-over-`fmt()` decision was made to buy. `Option<Layer>` implements
/// `Layer`, so the disabled case needs no branch: when no endpoint is configured this is `None` and
/// the stack is what it always was.
fn server_stack<W>(writer: W, filter: EnvFilter) -> impl Subscriber + Send + Sync
where
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().json().with_writer(writer))
        .with(crate::export::export_layer())
}

/// The CLI logging stack: human-readable records over `writer`, plus opt-in span export.
///
/// Split from [`init_cli_logging`] for the same reason as [`server_stack`].
///
/// ## The filters are per-layer here, and that is load-bearing
///
/// [`server_stack`] puts one `EnvFilter` in front of every layer because its two consumers want the
/// same level. The CLI's two do not: `filter` defaults to `warn` ([`CLI_DEFAULT_FILTER`]) to keep
/// stderr quiet, while a span worth exporting is `info`. A subscriber-wide `warn` starves the export
/// layer of exactly the spans it exists to send.
///
/// **Measured, not reasoned about** — `tests/cli_export_filter.rs` drives both arrangements: a
/// subscriber-wide `warn` filter exports **0** spans, while the per-layer arrangement below exports
/// the request span. That test exists because the last time this goal reasoned about layer
/// composition instead of measuring it, the reasoning was wrong (the flush that exported zero spans,
/// PR #535).
///
/// So the fmt layer keeps whatever `RUST_LOG` resolved — `RUST_LOG=info temper …` still opts into
/// verbose logging exactly as before — and the export layer carries its own fixed [`EXPORT_FILTER`].
/// The two are independent on purpose: turning on tracing must not make the CLI chatty, and turning
/// on verbose logging must not change what gets exported (or billed).
fn cli_stack<W>(writer: W, filter: EnvFilter) -> impl Subscriber + Send + Sync
where
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_filter(filter),
        )
        .with(crate::export::cli_export_layer().with_filter(EnvFilter::new(EXPORT_FILTER)))
}

/// Install the server logging stack as this process's global subscriber.
///
/// For `api/axum.rs`, `api/mcp.rs`, `api/internal.rs`, and `temper-api`'s own `main`. Panics if a
/// subscriber is already installed, which is the same failure the five `fmt().init()` calls this
/// replaced would have produced.
pub fn init_server_logging() {
    server_stack(std::io::stdout, env_filter(SERVER_DEFAULT_FILTER)).init();
}

/// Install the CLI logging stack as this process's global subscriber.
///
/// **Logs go to stderr, never stdout.** See the module docs: stdout carries the command's
/// machine-readable output and anything else on it is a parsing bug for whoever piped us into `jq`.
///
/// Span export is off unless **both** `TEMPER_CLI_TRACE=true` and an OTLP endpoint are set. When it
/// is on, the caller owns draining it: a CLI process exits, so it must call
/// [`crate::shutdown_telemetry`] on **every** exit path — see `temper-cli`'s `main`, whose failure arm
/// ends in `std::process::exit`, which runs no destructors.
pub fn init_cli_logging() {
    cli_stack(std::io::stderr, env_filter(CLI_DEFAULT_FILTER)).init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::{Arc, Mutex};

    /// A `MakeWriter` that appends to a shared buffer, so a test can read what a stack rendered.
    #[derive(Clone, Default)]
    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl CaptureWriter {
        fn contents(&self) -> String {
            String::from_utf8(self.0.lock().expect("capture buffer poisoned").clone())
                .expect("tracing output is UTF-8")
        }
    }

    impl io::Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("capture buffer poisoned")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'w> MakeWriter<'w> for CaptureWriter {
        type Writer = Self;

        fn make_writer(&'w self) -> Self::Writer {
            self.clone()
        }
    }

    /// Render one event through `subscriber` and return what it wrote.
    ///
    /// `set_default` rather than `init`: the global subscriber can only be installed once per
    /// process, and these tests install several.
    fn render(subscriber: impl Subscriber + Send + Sync, capture: &CaptureWriter) -> String {
        let _guard = subscriber.set_default();
        tracing::error!(answer = 42, "the event");
        drop(_guard);
        capture.contents()
    }

    /// JSON output carries a timestamp that differs between two renders of the same event; the
    /// plain format leads with one. Strip it so a comparison is about everything else.
    fn without_timestamp(line: &str) -> String {
        if let Some(rest) = line.strip_prefix('{') {
            // JSON: drop the `"timestamp":"…",` member, which the formatter emits first.
            let after = rest
                .split_once(',')
                .map(|(_, after)| after)
                .unwrap_or(rest)
                .to_string();
            return format!("{{{after}");
        }
        // Plain: `2026-07-24T21:00:00.000000Z ERROR temper_telemetry::init: …`
        line.split_once(' ')
            .map_or(line, |(_, rest)| rest)
            .to_string()
    }

    /// The refactor's whole claim is "the output did not change". This asserts it differentially —
    /// the same event through the `fmt()` builder each `main` used to call and through the stack
    /// that replaced it — rather than by hand-writing what the output is expected to look like.
    #[test]
    fn stacks_match_the_fmt_builder_they_replaced() {
        let old_server = CaptureWriter::default();
        let old = tracing_subscriber::fmt()
            .json()
            .with_env_filter(EnvFilter::new("info"))
            .with_writer(old_server.clone())
            .finish();
        let old_out = render(old, &old_server);

        let new_server = CaptureWriter::default();
        let new_out = render(
            server_stack(new_server.clone(), EnvFilter::new("info")),
            &new_server,
        );

        assert_eq!(
            without_timestamp(&old_out),
            without_timestamp(&new_out),
            "server stack changed the JSON record shape"
        );

        let old_cli_buf = CaptureWriter::default();
        let old_cli = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new("warn"))
            .with_writer(old_cli_buf.clone())
            .finish();
        let old_cli_out = render(old_cli, &old_cli_buf);

        let new_cli_buf = CaptureWriter::default();
        let new_cli_out = render(
            cli_stack(new_cli_buf.clone(), EnvFilter::new("warn")),
            &new_cli_buf,
        );

        assert_eq!(
            without_timestamp(&old_cli_out),
            without_timestamp(&new_cli_out),
            "CLI stack changed the human-readable record shape"
        );
    }

    /// The two stacks are not interchangeable, and the difference is the CLI's output contract:
    /// a JSON record on the CLI's stream would be indistinguishable from the command's own JSON.
    #[test]
    fn the_cli_stack_does_not_emit_json() {
        let capture = CaptureWriter::default();
        let out = render(cli_stack(capture.clone(), EnvFilter::new("warn")), &capture);

        assert!(
            !out.trim_start().starts_with('{'),
            "CLI logs must stay human-readable, or they become ambiguous with `temper … | jq` \
             output: {out}"
        );
        assert!(
            out.contains("the event"),
            "the event was not rendered: {out}"
        );
    }

    /// …and the server stack does, because a container runtime parses it.
    #[test]
    fn the_server_stack_emits_one_json_object_per_event() {
        let capture = CaptureWriter::default();
        let out = render(
            server_stack(capture.clone(), EnvFilter::new("info")),
            &capture,
        );

        let line = out.lines().next().expect("one record was emitted");
        let parsed: serde_json::Value =
            serde_json::from_str(line).expect("server records must be valid JSON");
        assert_eq!(parsed["fields"]["message"], "the event");
        assert_eq!(parsed["fields"]["answer"], 42);
    }

    /// The defaults are the ones the five `main`s carried, and they are not the same default.
    /// A shared seam that quietly gave the CLI the servers' `info` would put `ort`'s chatter on a
    /// user's terminal for every embed.
    #[test]
    fn the_two_defaults_stay_distinct() {
        assert_eq!(SERVER_DEFAULT_FILTER, "info");
        assert_eq!(CLI_DEFAULT_FILTER, "warn");
    }
}
