#![cfg(feature = "test-db")]
//! The sensitivity sweep's span, and witness 2's telemetry half (build order 3a PR D).
//!
//! One test, own file, for `internal_call_health_span_test.rs`'s reason: it installs a
//! process-global tracing subscriber and tracer provider, neither of which can be installed twice.
//!
//! Three passes, each a state an operator must be able to tell apart:
//! 1. a tick that found a planted SSN: the span carries the declared vocabulary, and no span
//!    attribute or log line carries the value;
//! 2. an unset salt: the run fails as `salt_missing` and an ERROR event says so;
//! 3. the same unset salt while that job backs off, so nothing is claimable: the error still fires.
//!    Without it, an unconfigured deployment would go quiet between retries (Q44).

use std::io::Write;
use std::sync::{Arc, Mutex};

use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanData};
use sqlx::PgPool;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use uuid::Uuid;

use temper_services::services::sensitivity_sweep_service::{
    sweep, SENSITIVITY_SWEEP_FIELDS, SENSITIVITY_SWEEP_RUN_FIELDS,
};

const SALT: &[u8] = b"span-witness-salt-of-sixteen-plus";
const SSN: &str = "219-45-6789";

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn take(&self) -> String {
        String::from_utf8(std::mem::take(&mut *self.0.lock().unwrap())).unwrap()
    }
}

fn attr(span: &SpanData, key: &str) -> Option<String> {
    span.attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| kv.value.to_string())
}

fn the_sweep_span(exporter: &InMemorySpanExporter) -> SpanData {
    temper_telemetry::force_flush_spans();
    let spans = exporter.get_finished_spans().expect("exporter readable");
    exporter.reset();
    spans
        .into_iter()
        .find(|s| s.name == "sensitivity_sweep")
        .expect("the door emitted no `sensitivity_sweep` span")
}

async fn order(pool: &PgPool, surface: &str) {
    sqlx::query("SELECT workflow_job_enqueue_system('sensitivity', 'sensitivity-sweep', $1)")
        .bind(serde_json::json!({ "surface": surface, "budget": 1000 }))
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_sweep_span_carries_counts_and_never_the_content(pool: PgPool) {
    let exporter = InMemorySpanExporter::default();
    assert!(
        temper_telemetry::export::install_test_provider(exporter.clone()),
        "a provider was already installed — this test owns the process"
    );
    let layer = temper_telemetry::export::test_export_layer()
        .expect("the layer must exist once a provider is installed");
    let logs = Captured::default();
    let writer = logs.clone();
    tracing_subscriber::registry()
        .with(layer)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(move || writer.clone()),
        )
        .init();

    // ── Pass 1: a planted SSN, found.
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, 'test://span') RETURNING id",
    )
    .bind(format!("Payroll note for {SSN}"))
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE kb_resources SET updated = now() - interval '1 hour' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    order(&pool, "kb_resources.title").await;

    let summary = sweep(&pool, Some(SALT)).await.expect("sweep runs");
    assert!(
        summary.tick.as_ref().is_some_and(|t| t.new_findings >= 1),
        "the SSN must be found, or the absence below proves nothing: {summary:?}"
    );
    let span = the_sweep_span(&exporter);
    for field in SENSITIVITY_SWEEP_FIELDS
        .iter()
        .chain(SENSITIVITY_SWEEP_RUN_FIELDS.iter())
    {
        assert!(
            attr(&span, field).is_some(),
            "`{field}` is declared but the span does not carry it, so a query on it finds nothing"
        );
    }
    assert_eq!(attr(&span, "outcome").as_deref(), Some("scanned"));
    assert_eq!(
        attr(&span, "failure"),
        None,
        "a clean tick has no failure, not an empty one"
    );
    let found = logs.take();
    for needle in [SSN, "219456789", "6789"] {
        for kv in &span.attributes {
            assert!(
                !kv.value.to_string().contains(needle),
                "span attribute `{}` carries `{needle}`",
                kv.key
            );
        }
        for event in span.events.iter() {
            assert!(
                !format!("{event:?}").contains(needle),
                "a span event carries `{needle}`"
            );
        }
        assert!(
            !found.contains(needle),
            "a log line carries `{needle}`:\n{found}"
        );
    }

    // ── Pass 2: the salt is unset. The run fails, and the error names the variable.
    order(&pool, "kb_resources.origin_uri").await;
    sweep(&pool, None).await.expect("sweep runs");
    let span = the_sweep_span(&exporter);
    assert_eq!(attr(&span, "salt_configured").as_deref(), Some("false"));
    assert_eq!(attr(&span, "failure").as_deref(), Some("salt_missing"));
    let unset = logs.take();
    assert!(
        unset.contains("ERROR") && unset.contains("SENSITIVITY_SWEEP_SALT"),
        "an unset salt must raise an error naming the variable:\n{unset}"
    );

    // ── Pass 3: that job is backing off, so the claim finds nothing. Still loud.
    let summary = sweep(&pool, None).await.expect("sweep runs");
    assert!(
        !summary.claimed,
        "the failed job holds the single-flight slot while it backs off"
    );
    let span = the_sweep_span(&exporter);
    assert_eq!(attr(&span, "claimed").as_deref(), Some("false"));
    for field in SENSITIVITY_SWEEP_RUN_FIELDS {
        assert_eq!(
            attr(&span, field),
            None,
            "`{field}` must be absent when nothing ran"
        );
    }
    let quiet = logs.take();
    assert!(
        quiet.contains("ERROR") && quiet.contains("SENSITIVITY_SWEEP_SALT"),
        "with nothing claimable the unset salt must still raise its error:\n{quiet}"
    );
}
