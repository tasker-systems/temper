#![cfg(feature = "test-db")]
//! The sensitivity sweep's spans, and witness 2's telemetry half (build order 3a PR D).
//!
//! One test, own file, for `internal_call_health_span_test.rs`'s reason: it installs a
//! process-global tracing subscriber and tracer provider, neither of which can be installed twice.
//!
//! Each pass is a state an operator must be able to tell apart from a healthy, quiet sweep:
//! 1. a tick that fails with the salt set, and the failed job then blocking the slot;
//! 2. a tick that finds a planted SSN: the spans carry the declared vocabulary, and no span
//!    attribute, span event or log line carries the value;
//! 3. an unset salt, ticked and then with nothing claimable: loud both times (Q44);
//! 4. a tick whose statement raises after its claim committed: a span and an error, not silence.

use std::io::Write;
use std::sync::{Arc, Mutex};

use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanData};
use sqlx::PgPool;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use uuid::Uuid;

use temper_services::services::sensitivity_sweep_service::{
    sweep, SENSITIVITY_SWEEP_CALL_FIELDS, SENSITIVITY_SWEEP_FIELDS, SENSITIVITY_SWEEP_RUN_FIELDS,
};

const SALT: &[u8] = b"span-witness-salt-of-thirty-two-plus-bytes";
const SSN: &str = "219-45-6789";
/// The value and its digits. Not a shorter fragment: log lines carry timestamps, and
/// `…:51.456789Z` holds `6789`.
const NEEDLES: [&str; 2] = [SSN, "219456789"];

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

/// Every span the pass emitted, then a clean exporter for the next pass.
fn spans(exporter: &InMemorySpanExporter) -> Vec<SpanData> {
    temper_telemetry::force_flush_spans();
    let spans = exporter.get_finished_spans().expect("exporter readable");
    exporter.reset();
    spans
}

fn the_call(spans: &[SpanData]) -> &SpanData {
    let calls: Vec<_> = spans
        .iter()
        .filter(|s| s.name == "sensitivity_sweep_call")
        .collect();
    assert_eq!(calls.len(), 1, "one call span per call");
    calls[0]
}

fn ticks(spans: &[SpanData]) -> Vec<&SpanData> {
    spans
        .iter()
        .filter(|s| s.name == "sensitivity_sweep")
        .collect()
}

/// An ERROR-level log line carrying `message`: the level and the message on one line, so an event
/// demoted to INFO does not pass.
fn has_error(logs: &str, message: &str) -> bool {
    logs.lines()
        .any(|line| line.contains(" ERROR ") && line.contains(message))
}

fn assert_carries_none(spans: &[SpanData], logs: &str) {
    for needle in NEEDLES {
        for span in spans {
            for kv in &span.attributes {
                assert!(
                    !kv.value.to_string().contains(needle),
                    "span `{}` attribute `{}` carries `{needle}`",
                    span.name,
                    kv.key
                );
            }
            for event in span.events.iter() {
                assert!(
                    !format!("{event:?}").contains(needle),
                    "a span event carries `{needle}`"
                );
            }
        }
        assert!(
            !logs.contains(needle),
            "a log line carries `{needle}`:\n{logs}"
        );
    }
}

async fn order(pool: &PgPool, surface: &str) {
    sqlx::query("SELECT workflow_job_enqueue_system('sensitivity', 'sensitivity-sweep', $1)")
        .bind(serde_json::json!({ "surface": surface, "budget": 1000 }))
        .execute(pool)
        .await
        .unwrap();
}

/// Let the backing-off job be claimed now.
async fn release(pool: &PgPool) {
    sqlx::query(
        "UPDATE kb_workflow_jobs SET next_visible_at = now() WHERE persona = 'sensitivity'",
    )
    .execute(pool)
    .await
    .unwrap();
}

/// The operator's act on a deployment that opts in (Q52, Q53): every seeded detector is off until
/// someone turns it on, and these witnesses are about what an enabled detector does.
async fn enable_seeded_detectors(pool: &PgPool) {
    sqlx::query("SELECT sensitivity.enable_detectors(1, 'temper')")
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_sweep_spans_tell_every_stalled_state_from_a_quiet_one(pool: PgPool) {
    enable_seeded_detectors(&pool).await;
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

    // ── Pass 0: the deployment has not opted in (Q52, Q53). The salt is unset and an order waits,
    //    which an opted-in call would claim and fail loudly on. This one claims nothing, writes
    //    nothing and raises nothing: the call span says it was off, and that is all.
    order(&pool, "kb_resources.title").await;
    logs.take();
    spans(&exporter);
    let summary = sweep(&pool, None, false).await.expect("sweep answers");
    assert!(summary.ticks.is_empty(), "{summary:?}");
    assert!(!summary.answer().enabled);
    let emitted = spans(&exporter);
    assert!(
        ticks(&emitted).is_empty(),
        "a call that is off ticks nothing"
    );
    let call = the_call(&emitted);
    assert_eq!(attr(call, "enabled").as_deref(), Some("false"));
    assert_eq!(attr(call, "ended").as_deref(), Some("disabled"));
    let off = logs.take();
    assert!(
        !off.contains(" ERROR "),
        "a deployment that has not opted in raises nothing:\n{off}"
    );
    let (runs, claimed): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM sensitivity.runs), \
                (SELECT count(*) FROM kb_workflow_jobs \
                  WHERE persona = 'sensitivity' AND status <> 'pending')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (runs, claimed),
        (0, 0),
        "a call that is off wrote or claimed"
    );

    // ── Pass 1: the store refuses the finding, so the tick fails with the salt set; the failed job
    //    then holds the slot. Witness 14's trigger, which quotes the value in its message.
    sqlx::query(&format!(
        "CREATE FUNCTION witness_boom() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'cannot store {SSN}'; END $$"
    ))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TRIGGER boom BEFORE INSERT ON sensitivity.findings \
         FOR EACH ROW EXECUTE FUNCTION witness_boom()",
    )
    .execute(&pool)
    .await
    .unwrap();
    order(&pool, "kb_resources.title").await;
    // The setup's own statements quote the value (the trigger's message); judge only the sweep's.
    logs.take();
    spans(&exporter);
    let summary = sweep(&pool, Some(SALT), true).await.expect("sweep runs");
    assert_eq!(summary.ticks.len(), 1, "{summary:?}");
    let emitted = spans(&exporter);
    let failed = logs.take();
    let tick = ticks(&emitted)[0];
    assert_eq!(attr(tick, "failure").as_deref(), Some("scan_failed"));
    assert!(
        has_error(&failed, "sensitivity sweep tick failed"),
        "a failed tick with the salt set must raise its own error:\n{failed}"
    );
    let call = the_call(&emitted);
    assert_eq!(attr(call, "ended").as_deref(), Some("slot_blocked"));
    assert_eq!(attr(call, "slot_failure").as_deref(), Some("scan_failed"));
    assert!(
        has_error(&failed, "holds the single-flight slot"),
        "a blocked slot stalls every surface and must say so:\n{failed}"
    );
    assert_carries_none(&emitted, &failed);

    // ── Pass 2: the store accepts it; the SSN is found.
    sqlx::query("DROP TRIGGER boom ON sensitivity.findings")
        .execute(&pool)
        .await
        .unwrap();
    release(&pool).await;
    let summary = sweep(&pool, Some(SALT), true).await.expect("sweep runs");
    assert!(
        summary.ticks.iter().any(|t| t.new_findings >= 1),
        "the SSN must be found, or the absence below proves nothing: {summary:?}"
    );
    let emitted = spans(&exporter);
    let found = logs.take();
    let finder = ticks(&emitted)
        .into_iter()
        .find(|s| attr(s, "new_findings").is_some_and(|n| n != "0"))
        .expect("the finding tick's span");
    for field in SENSITIVITY_SWEEP_FIELDS
        .iter()
        .chain(SENSITIVITY_SWEEP_RUN_FIELDS.iter())
    {
        assert!(
            attr(finder, field).is_some(),
            "`{field}` is declared but the tick span does not carry it"
        );
    }
    assert_eq!(attr(finder, "outcome").as_deref(), Some("scanned"));
    assert_eq!(
        attr(finder, "failure"),
        None,
        "a clean tick has no failure, not an empty one"
    );
    let call = the_call(&emitted);
    for field in SENSITIVITY_SWEEP_CALL_FIELDS {
        assert!(
            attr(call, field).is_some(),
            "`{field}` is declared but the call span does not carry it"
        );
    }
    assert_eq!(attr(call, "ended").as_deref(), Some("idle"));
    assert!(
        !found.contains(" ERROR "),
        "a healthy call raises nothing:\n{found}"
    );
    assert_carries_none(&emitted, &found);

    // ── Pass 3: the salt is unset. The tick fails, and the error names the variable.
    let summary = sweep(&pool, None, true).await.expect("sweep runs");
    assert_eq!(summary.ticks.len(), 1);
    let emitted = spans(&exporter);
    assert_eq!(
        attr(ticks(&emitted)[0], "failure").as_deref(),
        Some("salt_missing")
    );
    assert_eq!(
        attr(the_call(&emitted), "salt_configured").as_deref(),
        Some("false")
    );
    let unset = logs.take();
    assert!(
        has_error(&unset, "SENSITIVITY_SWEEP_SALT is unset"),
        "an unset salt must raise an error naming the variable:\n{unset}"
    );

    // ── That job is backing off, so the claim finds nothing. Still loud.
    let summary = sweep(&pool, None, true).await.expect("sweep runs");
    assert!(summary.ticks.is_empty());
    let emitted = spans(&exporter);
    assert!(
        ticks(&emitted).is_empty(),
        "no tick span when nothing was claimed"
    );
    assert_eq!(
        attr(the_call(&emitted), "ended").as_deref(),
        Some("slot_blocked")
    );
    let quiet = logs.take();
    assert!(
        has_error(&quiet, "SENSITIVITY_SWEEP_SALT is unset"),
        "with nothing claimable the unset salt must still raise its error:\n{quiet}"
    );

    // ── Pass 4: the tick statement itself raises after the claim committed.
    sqlx::query("UPDATE kb_workflow_jobs SET status = 'dead' WHERE persona = 'sensitivity'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "ALTER FUNCTION sensitivity_sweep_tick(uuid, uuid, bytea, interval, int) RENAME TO tick_gone",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(sweep(&pool, Some(SALT), true).await.is_err());
    let emitted = spans(&exporter);
    let raised = ticks(&emitted);
    assert_eq!(raised.len(), 1, "the claimed tick still reports");
    assert_eq!(attr(raised[0], "errored").as_deref(), Some("true"));
    assert_eq!(attr(raised[0], "ticked").as_deref(), Some("false"));
    assert!(attr(raised[0], "run_id").is_some());
    for field in SENSITIVITY_SWEEP_RUN_FIELDS {
        assert_eq!(
            attr(raised[0], field),
            None,
            "`{field}` must be absent when no tick returned"
        );
    }
    the_call(&emitted);
    let errored = logs.take();
    assert!(
        has_error(&errored, "raised after its claim committed"),
        "an errored tick must be loud:\n{errored}"
    );
}
