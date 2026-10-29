#![cfg(feature = "test-db")]
//! The drain-span gate for the shape-reconcile drain (task 01a15663).
//!
//! `drain_span_test.rs` holds the shared field vocabulary to the region drain's spans. This file
//! holds it to the shape drain's, so an operator query written against the vocabulary finds the
//! shape drain too, and holds the job span to the one number the drain has for its backlog:
//! `backlog_remaining`. One test, own file, for that file's reason: it installs a process-global
//! subscriber and tracer provider.

use opentelemetry_sdk::trace::InMemorySpanExporter;
use sqlx::PgPool;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::ContextId;
use temper_core::types::workflow_job::{AnchorJobPayload, DispatchType, Persona};
use temper_services::services::drain_span::{DRAIN_DISPATCH_FIELDS, DRAIN_JOB_FIELDS};
use temper_services::services::{shape_service, workflow_job_service};

/// Seed a profile plus its three surface emitter entities, then a context owned by it.
///
/// Copied from `drain_span_test.rs` rather than shared: `crates/temper-services/tests/` has no
/// `common` module, and each integration-test file is its own binary. Keep the two in step.
async fn seed_profile_with_context(pool: &PgPool, email: &str) -> (uuid::Uuid, uuid::Uuid) {
    let profile_id = uuid::Uuid::now_v7();
    let local = email.split('@').next().unwrap_or("test-user");
    let handle = format!("{local}-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind(email)
        .bind(email)
        .execute(pool)
        .await
        .expect("seed profile");
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb)",
        )
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    }
    let context_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,'temper','temper')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id)
}

fn attr(span: &opentelemetry_sdk::trace::SpanData, key: &str) -> Option<String> {
    span.attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| kv.value.to_string())
}

// FAILS IF: the shape drain's spans do not carry the shared drain vocabulary, or its job span does
// not report how much backlog the anchor has left.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_shape_drain_reports_the_shared_vocabulary_and_its_backlog(pool: PgPool) {
    let exporter = InMemorySpanExporter::default();
    assert!(
        temper_telemetry::export::install_test_provider(exporter.clone()),
        "a provider was already installed — this test owns the process"
    );
    let layer = temper_telemetry::export::test_export_layer()
        .expect("the layer must exist once a provider is installed");
    tracing_subscriber::registry().with(layer).init();

    let (profile, context) = seed_profile_with_context(&pool, "shapespan@example.com").await;
    let emitter: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_entities WHERE profile_id = $1 AND name LIKE '%@cli'",
    )
    .bind(profile)
    .fetch_one(&pool)
    .await
    .expect("cli emitter");
    // Enqueue directly: this test is about what the drain reports, and an anchor with no shape has
    // an empty backlog, which is a value the job span must still report.
    workflow_job_service::enqueue_anchor(
        &pool,
        HomeAnchor::Context(ContextId::from(context)),
        Persona::Shape.as_str(),
        DispatchType::ShapeReconcile.as_str(),
        AnchorJobPayload { emitter },
    )
    .await
    .expect("enqueue");

    shape_service::dispatch_tick(&pool, None)
        .await
        .expect("a drain pass");

    temper_telemetry::force_flush_spans();
    let spans = exporter.get_finished_spans().expect("exporter readable");
    let named = |name: &str| {
        spans.iter().find(|s| s.name == name).unwrap_or_else(|| {
            panic!(
                "the drain emitted no `{name}` span. Spans seen: {:?}",
                spans.iter().map(|s| &s.name).collect::<Vec<_>>()
            )
        })
    };
    let tick = named("shape_reconcile_dispatch");
    let job = named("shape_reconcile_job");

    for field in DRAIN_DISPATCH_FIELDS {
        assert!(
            attr(tick, field).is_some(),
            "`{field}` is in DRAIN_DISPATCH_FIELDS but `shape_reconcile_dispatch` does not carry it"
        );
    }
    for field in DRAIN_JOB_FIELDS {
        assert!(
            attr(job, field).is_some(),
            "`{field}` is in DRAIN_JOB_FIELDS but `shape_reconcile_job` does not carry it"
        );
    }
    assert_eq!(attr(job, "outcome").as_deref(), Some("completed"));
    assert_eq!(
        attr(job, "backlog_remaining").as_deref(),
        Some("0"),
        "the job span reports the anchor's remaining backlog, here none"
    );
    assert_eq!(
        attr(job, "backlog_remaining_capped").as_deref(),
        Some("false")
    );
}
