//! Integration test — a caller-supplied `correlation` reaches `kb_events.correlation_id` through the
//! real `DbBackend` (task 019f4912, P3 of the temper-rb goal).
//!
//! This is the production caller's level: `ActContext.correlation` → `EventContext.correlation` →
//! `<mutation_fn>(…, p_correlation)` → `_event_append(p_correlation => …)`. Before this task the last
//! hop did not exist — `_event_append` accepted `p_correlation` but no mutation function forwarded it,
//! so every event in the ledger self-rooted.
//!
//! The three properties under test are the task's acceptance criteria:
//!   1. Two writes issued under the same correlation share one `kb_events.correlation_id`.
//!   2. A write with no correlation self-roots (`correlation_id = id`) — unchanged from before.
//!   3. Correlation and invocation are independent grains: an act may carry one without the other.
//!
//! ONNX-free (bring-your-own packed chunks), so it runs under plain `cargo make test-db`.
#![cfg(feature = "test-db")]

use sqlx::PgPool;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, CorrelationId, ProfileId, ResourceId};
use temper_core::types::ingest::{pack_chunks, PackedChunk};
use temper_services::backend::DbBackend;
use temper_workflow::operations::Surface;
use temper_workflow::operations::{Backend, CreateResource, UpdateResource};
use temper_workflow::types::managed_meta::ManagedMeta;

/// Seed a substrate profile + a profile-owned `temper` context (the minimum the write path's
/// `resolve_emitter` + visibility gate require). Mirrors `segmented_backend_test.rs`'s inlined
/// fixture — each test-target crate keeps its own copy so it has no cross-target harness dependency.
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
    for surface in ["web", "cli", "mcp", "sdk"] {
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

fn one_chunk_packed(text: &str, hash_seed: &str) -> String {
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: text.to_owned(),
        content_hash: format!("{hash_seed:0>64}"),
        embedding: vec![0.1_f32; 768],
    };
    pack_chunks(&[chunk]).expect("pack chunk")
}

fn create_cmd(slug: &str, context: uuid::Uuid, act: ActContext) -> CreateResource {
    CreateResource {
        slug: slug.to_string(),
        doctype: "research".to_string(),
        home: HomeAnchor::Context(ContextId::from(context)),
        title: slug.to_string(),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        goal: None,
        origin_uri: Some(format!("test://{slug}")),
        chunks_packed: Some(one_chunk_packed("segment", "aa")),
        content_hash: None,
        act,
        origin: Surface::ApiHttp,
    }
}

fn update_cmd(resource: ResourceId, title: &str, act: ActContext) -> UpdateResource {
    UpdateResource {
        resource,
        title: Some(title.to_string()),
        slug: None,
        body: None,
        managed_meta: None,
        open_meta: None,
        goal: None,
        move_to: None,
        context_ref: None,
        act,
        origin: Surface::ApiHttp,
    }
}

/// Every `kb_events` row this resource anchored, as `(id, correlation_id, invocation_id)`.
type EventRow = (uuid::Uuid, Option<uuid::Uuid>, Option<uuid::Uuid>);

async fn events_for(pool: &PgPool, resource: ResourceId) -> Vec<EventRow> {
    sqlx::query_as::<_, EventRow>(
        "SELECT e.id, e.correlation_id, e.invocation_id \
           FROM kb_events e \
          WHERE e.payload->>'resource_id' = $1::text \
          ORDER BY e.occurred_at, e.id",
    )
    .bind(resource.uuid().to_string())
    .fetch_all(pool)
    .await
    .expect("read events")
}

/// Acceptance criterion 1 — the load-bearing one. Two *separate* writes (a create, then an update)
/// issued under one caller-minted correlation land in the ledger sharing a single
/// `correlation_id`. This is the Puma-request-plus-Sidekiq-job shape: distinct acts of writing,
/// one act of intent, stitched by a bare UUID that outlives any credential.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn two_writes_under_one_correlation_share_a_correlation_id(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "corr-shared@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    let corr = CorrelationId::new();
    let act = ActContext {
        correlation: Some(corr),
        ..Default::default()
    };

    let created = backend
        .create_resource(create_cmd("shared-thread", context, act.clone()))
        .await
        .expect("create under correlation")
        .value;
    let resource = created.id;

    backend
        .update_resource(update_cmd(resource, "shared thread retitled", act.clone()))
        .await
        .expect("update under the same correlation");

    let events = events_for(&pool, resource).await;
    assert!(
        events.len() >= 2,
        "expected at least a created + an updated event, got {}",
        events.len()
    );
    for (id, correlation_id, _) in &events {
        assert_eq!(
            *correlation_id,
            Some(corr.uuid()),
            "event {id} did not carry the caller's correlation"
        );
    }
    // And it is genuinely *shared*, not merely present: one distinct value across the whole act.
    let distinct: std::collections::HashSet<_> = events.iter().map(|(_, c, _)| *c).collect();
    assert_eq!(distinct.len(), 1, "the act must be one correlation thread");
}

/// Acceptance criterion 2 — an unsupplied correlation self-roots, byte-identical to the behavior
/// before correlation was threadable. This is the regression guard on `_event_append`'s
/// `COALESCE(p_correlation, v_ev)` root-event convention: passing NULL must not become "no
/// correlation" (a NULL column), and must not borrow some other event's root.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_write_without_correlation_self_roots(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "corr-absent@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    let created = backend
        .create_resource(create_cmd("self-rooted", context, ActContext::default()))
        .await
        .expect("create with no act context")
        .value;

    let events = events_for(&pool, created.id).await;
    assert!(!events.is_empty(), "the create must emit an event");
    for (id, correlation_id, _) in &events {
        assert_eq!(
            *correlation_id,
            Some(*id),
            "event {id} must be its own correlation root"
        );
    }
}

/// Acceptance criterion 3 — correlation is act-grain, invocation is run-grain, and neither implies
/// the other. A correlation supplied with no invocation writes `correlation_id` and leaves
/// `invocation_id` NULL. (The converse — invocation without correlation — is the self-rooting case
/// already covered by `nonauthored_act_correlation.rs` at the substrate layer.)
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn correlation_rides_without_an_invocation(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "corr-solo@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    let corr = CorrelationId::new();
    let created = backend
        .create_resource(create_cmd(
            "solo-correlation",
            context,
            ActContext {
                correlation: Some(corr),
                ..Default::default()
            },
        ))
        .await
        .expect("create with correlation only")
        .value;

    let events = events_for(&pool, created.id).await;
    assert!(!events.is_empty(), "the create must emit an event");
    for (id, correlation_id, invocation_id) in &events {
        assert_eq!(*correlation_id, Some(corr.uuid()), "event {id} correlation");
        assert!(
            invocation_id.is_none(),
            "event {id} must carry no invocation — correlation does not imply a run"
        );
    }
}
