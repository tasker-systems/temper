#![cfg(feature = "test-db")]
//! The reconcile-channel health span gate.
//!
//! One test, own file, for `drain_span_test.rs`'s reason: it installs a **process-global** tracing
//! subscriber and tracer provider, neither of which can be installed twice.
//!
//! ## Why this gate is load-bearing rather than tidy
//!
//! The alert rule that pages an operator matches on `span.state` and reads `span.failure_cause` and
//! `span.failure_detail` out of the span. A rename on this side does not fail anything — it empties
//! the alert, silently, and the alert's whole job is to fire when something has gone quiet. An
//! observability mechanism that fails by going quiet cannot be allowed to fail by going quiet.
//!
//! ## What bites here, and what would not
//!
//! Asserting *"an `internal_call_health` span exists"* would be satisfied by any span of that name,
//! however wrongly filled. So the assertions are on **values** — the exact `state` string the alert
//! matches — and on the **absence** of the conditional fields, which is the property a reader cannot
//! get back once it is wrong: a `seconds_since_success` of 0 reads as *succeeded just now*, which is
//! the opposite of the truth for a channel that never has.

use opentelemetry_sdk::trace::InMemorySpanExporter;
use sqlx::PgPool;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use temper_services::services::internal_call_health_service::{
    check_internal_call_health, INTERNAL_CALL_HEALTH_CONDITIONAL_FIELDS,
    INTERNAL_CALL_HEALTH_FIELDS, RECONCILE_CHANNEL,
};

fn attr(span: &opentelemetry_sdk::trace::SpanData, key: &str) -> Option<String> {
    span.attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| kv.value.to_string())
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_health_span_carries_the_vocabulary_the_alert_queries(pool: PgPool) {
    let exporter = InMemorySpanExporter::default();
    assert!(
        temper_telemetry::export::install_test_provider(exporter.clone()),
        "a provider was already installed — this test owns the process"
    );
    let layer = temper_telemetry::export::test_export_layer()
        .expect("the layer must exist once a provider is installed");
    tracing_subscriber::registry().with(layer).init();

    // ── Pass 1: nothing recorded. The state a quiet deployment is in, and the one that must never
    //    reach the alert.
    check_internal_call_health(&pool).await.expect("check runs");
    temper_telemetry::force_flush_spans();
    let quiet = exporter.get_finished_spans().expect("exporter readable");
    let quiet = quiet
        .iter()
        .find(|s| s.name == "internal_call_health")
        .expect("the check emitted no `internal_call_health` span for a channel with no row");

    assert_eq!(
        attr(quiet, "state").as_deref(),
        Some("no_attempt_recorded"),
        "a channel with no row must be REPORTED, not omitted — a check that emits only for channels \
         it finds cannot report the one that has never written a row"
    );
    // THE BITE that matters most here: absent, not zero.
    for field in [
        "seconds_since_success",
        "failing_for_seconds",
        "failure_cause",
    ] {
        assert_eq!(
            attr(quiet, field),
            None,
            "`{field}` must be ABSENT when it has no value. A zero would read as a real \
             measurement and put a false floor under every aggregate built on it"
        );
    }

    // ── Pass 2: a sustained failure. The exact strings the alert rule matches on.
    sqlx::query(
        "INSERT INTO kb_internal_call_health
             (channel, last_success_at, last_failure_at, failing_since, consecutive_failures,
              failures_total, last_failure_cause, last_failure_detail)
         VALUES ($1, now() - interval '2 hours', now(), now() - interval '90 minutes', 4,
                 9, 'config_missing', 'INTERNAL_RECONCILE_URL')",
    )
    .bind(RECONCILE_CHANNEL)
    .execute(&pool)
    .await
    .expect("seed a sustained failure");

    exporter.reset();
    check_internal_call_health(&pool).await.expect("check runs");
    temper_telemetry::force_flush_spans();
    let spans = exporter.get_finished_spans().expect("exporter readable");
    let span = spans
        .iter()
        .find(|s| s.name == "internal_call_health")
        .expect("the check emitted no `internal_call_health` span");

    assert_eq!(
        attr(span, "state").as_deref(),
        Some("sustained"),
        "the alert rule matches `span.state = \"sustained\"` literally; any other spelling makes it \
         a rule that can never fire"
    );
    assert_eq!(
        attr(span, "channel").as_deref(),
        Some(RECONCILE_CHANNEL),
        "the operator queries group by `span.channel`"
    );
    assert_eq!(
        attr(span, "failure_cause").as_deref(),
        Some("config_missing"),
        "the cause is what tells an operator WHICH action to take, so it must survive to the span"
    );
    assert_eq!(
        attr(span, "failure_detail").as_deref(),
        Some("INTERNAL_RECONCILE_URL"),
        "`config_missing` without the variable's name tells an operator only that something is unset"
    );

    // ── The declared vocabulary is actually carried. A constant nothing asserts its consumers
    //    against prevents no drift at all — `drain_span.rs`'s own lesson, applied to its sibling.
    for field in INTERNAL_CALL_HEALTH_FIELDS {
        assert!(
            attr(span, field).is_some(),
            "`{field}` is in INTERNAL_CALL_HEALTH_FIELDS but no `internal_call_health` span carries \
             it, so the convention names a field the operator queries will never find"
        );
    }
    // Every conditional field HAS a value in this state, which is what makes this row the one that
    // can assert them at all — the quiet pass above asserts the other direction.
    for field in INTERNAL_CALL_HEALTH_CONDITIONAL_FIELDS {
        assert!(
            attr(span, field).is_some(),
            "`{field}` is in INTERNAL_CALL_HEALTH_CONDITIONAL_FIELDS and this row has a value for \
             it, so the span must carry it"
        );
    }
}
