#![cfg(feature = "test-db")]
//! The drain-span gate (goal `019f9404`).
//!
//! One test, own file: it installs a **process-global** tracing subscriber and tracer provider,
//! neither of which can be installed twice. Each integration-test file is its own binary and nextest
//! gives each test its own process, so this one owns both — the same ownership
//! `crates/temper-api/tests/telemetry_flush_test.rs` documents.
//!
//! ## What bites here, and what would not
//!
//! Asserting *"a `region_job` span exists"* fails today against the **absence of the feature**,
//! which is a bite against nothing: any code emitting a span of that name satisfies it, however
//! wrongly. So the load-bearing assertion is on the **value** of `queue_wait_ms` against a
//! controlled enqueue-to-claim gap. It fails when the field is missing AND when it is computed
//! wrongly — and the second is the failure that actually ships, because a wrong-but-present number
//! is what an operator would build a dashboard on.
//!
//! The negative half — that none of this lands on the request ROOT span — is asserted for the reason
//! CLAUDE.md's span-field convention gives: with no child span, `Span::current().record(..)`
//! resolves to whatever is current and *appears* to work, right up until it silently doesn't.

use std::time::Duration;

use opentelemetry_sdk::trace::InMemorySpanExporter;
use sqlx::PgPool;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::ContextId;
use temper_core::types::workflow_job::{DispatchType, Persona, RegionJobPayload};
use temper_services::services::drain_span::{DRAIN_DISPATCH_FIELDS, DRAIN_JOB_FIELDS};
use temper_services::services::{region_service, workflow_job_service};

/// Seed a profile plus its three surface emitter entities, then a context owned by it.
///
/// Copied from `region_tick_off_request_test.rs` rather than shared: `crates/temper-services/tests/`
/// has no `common` module, and each integration-test file is its own binary. Keep the two in step.
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

/// This profile's `cli` emitter — the entity a queued region job attributes its settling to.
async fn cli_emitter(pool: &PgPool, profile: uuid::Uuid) -> uuid::Uuid {
    sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id = $1 AND name LIKE '%@cli'")
        .bind(profile)
        .fetch_one(pool)
        .await
        .expect("cli emitter")
}

fn attr(span: &opentelemetry_sdk::trace::SpanData, key: &str) -> Option<String> {
    span.attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| kv.value.to_string())
}

fn attr_i64(span: &opentelemetry_sdk::trace::SpanData, key: &str) -> Option<i64> {
    attr(span, key).and_then(|v| v.parse::<i64>().ok())
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_drained_job_reports_its_queue_wait_on_its_own_span(pool: PgPool) {
    let exporter = InMemorySpanExporter::default();
    assert!(
        temper_telemetry::export::install_test_provider(exporter.clone()),
        "a provider was already installed — this test owns the process"
    );
    let layer = temper_telemetry::export::test_export_layer()
        .expect("the layer must exist once a provider is installed");
    tracing_subscriber::registry().with(layer).init();

    let (profile, context) = seed_profile_with_context(&pool, "drainspan@example.com").await;
    let anchor = HomeAnchor::Context(ContextId::from(context));
    let emitter = cli_emitter(&pool, profile).await;

    // Enqueue directly rather than driving a create: this test is about what the DRAIN reports, and
    // the formation-clock machinery a real create needs would only add ways for it to fail for
    // reasons that are not this test's subject.
    workflow_job_service::enqueue_anchor(
        &pool,
        anchor,
        Persona::Region.as_str(),
        DispatchType::Materialize.as_str(),
        RegionJobPayload { emitter },
    )
    .await
    .expect("enqueue");

    // The controlled gap the value assertion bites on.
    tokio::time::sleep(Duration::from_millis(1_200)).await;

    region_service::dispatch_tick(&pool, Some(1))
        .await
        .expect("a drain pass");

    // Nothing here is behind an HTTP middleware, so no flush happens for us — a batch processor
    // holds unflushed spans forever and every assertion below would read an empty exporter.
    temper_telemetry::force_flush_spans();
    let spans = exporter.get_finished_spans().expect("exporter readable");

    let job = spans
        .iter()
        .find(|s| s.name == "region_job")
        .unwrap_or_else(|| {
            panic!(
                "the drain emitted no `region_job` span. Spans seen: {:?}",
                spans.iter().map(|s| &s.name).collect::<Vec<_>>()
            )
        });

    // ── THE BITE: a value, not an existence.
    let wait = attr_i64(job, "queue_wait_ms").unwrap_or_else(|| {
        panic!("`queue_wait_ms` absent or non-numeric on the `region_job` span")
    });
    assert!(
        wait >= 1_000,
        "queue_wait_ms = {wait}ms, but the job sat in the queue for at least 1200ms before the \
         drain claimed it — the field is present but computed wrong"
    );

    // Identity and outcome travel with it, or an operator cannot split the aggregate by anchor.
    assert_eq!(
        attr(job, "anchor_id").as_deref(),
        Some(context.to_string().as_str()),
        "the job span must name the anchor it settled"
    );
    assert_eq!(
        attr(job, "anchor_kind").as_deref(),
        Some("kb_contexts"),
        "both anchor kinds share this span, so the kind must be a dimension"
    );
    assert_eq!(
        attr(job, "outcome").as_deref(),
        Some("completed"),
        "a job the drain ran to completion reports `completed`"
    );

    // ── The declared vocabulary is actually carried. A constant nothing asserts its consumers
    //    against prevents no drift at all.
    let tick = spans
        .iter()
        .find(|s| s.name == "region_dispatch")
        .expect("the drain emitted no `region_dispatch` span");
    for field in DRAIN_DISPATCH_FIELDS {
        assert!(
            attr(tick, field).is_some(),
            "`{field}` is in DRAIN_DISPATCH_FIELDS but no `region_dispatch` span carries it, so the \
             convention names a field the operator queries will never find"
        );
    }
    for field in DRAIN_JOB_FIELDS {
        assert!(
            attr(job, field).is_some(),
            "`{field}` is in DRAIN_JOB_FIELDS but no `region_job` span carries it"
        );
    }

    // The backlog was read BEFORE the claim, so it must see the job this test queued. Read after the
    // loop it would be 0, and a drain falling behind would report an empty queue.
    assert_eq!(
        attr_i64(tick, "backlog_depth"),
        Some(1),
        "backlog_depth must be read before the claim loop — a 0 here means it is read after, and \
         reports the queue the tick just drained rather than the one it found"
    );

    // ── THE NEGATIVE, and the reason the job span exists at all.
    if let Some(root) = spans
        .iter()
        .find(|s| s.span_kind == opentelemetry::trace::SpanKind::Server)
    {
        for field in DRAIN_JOB_FIELDS {
            assert!(
                attr(root, field).is_none(),
                "`{field}` landed on the request ROOT span. Job fields belong on `region_job`; \
                 recording onto the root works only until something nests below it."
            );
        }
    }
}
