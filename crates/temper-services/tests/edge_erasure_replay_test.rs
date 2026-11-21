#![cfg(feature = "test-db")]
//! Replay of the per-edge erasure unit `edge_erasure_execute` (20261201000060) under the per-item
//! erasure deferral of `temper_substrate::replay::replay`.
//!
//! One span holds both per-item erasure authorities: a `resource_erased` and two `edge_erased`
//! records appended in one transaction under one request reference, the span a person act that
//! runs both units would leave. Replay projects each record at its position and runs each body at
//! the span's end; the projections it rebuilds must equal the live ones byte for byte. The world is
//! built through the real write paths and the units are called by SQL directly, since no act calls
//! `edge_erasure_execute` yet.
//! TODO(person-act step): once the person act calls the unit, replay a span that act wrote.

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::property_owner::PropertyOwner;
use temper_services::test_support;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::{fire, fire_with, EdgeHome, EventContext, SeedAction};
use temper_substrate::ids::{ContextId, EdgeId, EntityId, ResourceId};
use temper_substrate::payloads::{AgentAuthorship, AnchorRef, ConfidenceBand, EdgePolarity};
use temper_substrate::replay;
use temper_substrate::writes::{self, CreateParams};

struct World {
    profile: Uuid,
    emitter: Uuid,
    home: ContextId,
}

/// A profile, its `@web` emitter and an `@me` context, which `_erasure_estate_contexts(profile)`
/// returns. The handle is the full id, so same-millisecond UUIDv7 handles cannot collide.
async fn world(pool: &PgPool) -> World {
    let profile = Uuid::now_v7();
    let handle = format!("user-{profile}");
    test_support::profile::insert_with_id(pool, profile, &handle).await;
    let emitter = test_support::entity::insert(pool, profile, &format!("{handle}@web")).await;
    let home = test_support::context::insert_personal(pool, profile, "notes", "notes").await;
    World {
        profile,
        emitter,
        home: ContextId::from(home),
    }
}

/// A context of a team that is not the person's personal team: outside the estate.
async fn team_context(pool: &PgPool) -> ContextId {
    let team = test_support::team::insert(pool, &format!("team-{}", Uuid::now_v7())).await;
    ContextId::from(test_support::context::insert_team(pool, team, "shared", "shared").await)
}

async fn resource(pool: &PgPool, w: &World, home: ContextId, title: &str) -> ResourceId {
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: &format!("test://{}", Uuid::now_v7()),
            body: "a body",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner: ProfileId::from(w.profile),
            originator: ProfileId::from(w.profile),
            emitter: EntityId::from(w.emitter),
            properties: &[],
            chunks: None,
            sources: Vec::new(),
        },
        EventContext::default(),
    )
    .await
    .expect("create through the real path")
}

fn authored(reasoning: &str) -> EventContext {
    EventContext {
        authorship: Some(AgentAuthorship {
            reasoning: Some(reasoning.into()),
            confidence: ConfidenceBand::Probable,
            rationale: None,
            persona: None,
            model: None,
        }),
        invocation: None,
        correlation: None,
    }
}

/// An edge homed in the world's `@me` context, asserted with authorship reasoning on its event.
async fn edge(
    pool: &PgPool,
    w: &World,
    src: ResourceId,
    tgt: ResourceId,
    label: &str,
    reasoning: &str,
) -> EdgeId {
    let mut conn = pool.acquire().await.expect("acquire");
    fire_with(
        &mut conn,
        SeedAction::RelationshipAssert {
            src: AnchorRef::resource(src),
            tgt: AnchorRef::resource(tgt),
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some(label),
            weight: 1.0,
            home: EdgeHome::Context(w.home),
            emitter: EntityId::from(w.emitter),
        },
        authored(reasoning),
    )
    .await
    .expect("assert the edge")
    .relationship()
    .expect("an edge id")
}

/// A facet on the edge through the real write path.
async fn facet(pool: &PgPool, w: &World, e: EdgeId, values: Value) {
    let mut conn = pool.acquire().await.expect("acquire");
    fire(
        &mut conn,
        SeedAction::FacetSet {
            owner: PropertyOwner::Edge { id: e },
            values: &values,
            weight: 1.0,
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .expect("set a facet on the edge");
}

/// The edge's ordinary fold, through the real write path.
async fn fold(pool: &PgPool, w: &World, e: EdgeId) {
    let mut conn = pool.acquire().await.expect("acquire");
    fire(
        &mut conn,
        SeedAction::RelationshipFold {
            edge: e,
            reason: None,
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .expect("fold the edge");
}

/// The events of one kind carrying the request reference: (id, occurred_at).
async fn span_events(
    pool: &PgPool,
    kind: &str,
    request_ref: Uuid,
) -> Vec<(Uuid, chrono::DateTime<chrono::Utc>)> {
    sqlx::query_as(
        "SELECT ev.id, ev.occurred_at FROM kb_events ev \
           JOIN kb_event_types et ON et.id = ev.event_type_id \
          WHERE et.name = $1 AND ev.correlation_id = $2 \
          ORDER BY ev.id",
    )
    .bind(kind)
    .bind(request_ref)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// Reset the schema in the current database to a clean, un-seeded baseline (the erasure replay
/// suite's helper; the substrate's own lives in its test tree, which this crate cannot import).
async fn reset_namespace(pool: &PgPool) {
    use sqlx::Executor;
    pool.execute("DROP SCHEMA IF EXISTS sensitivity CASCADE; DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .await
        .expect("drop sensitivity, recreate public schema");
    temper_substrate::MIGRATOR
        .run(pool)
        .await
        .expect("re-apply the migration chain");
    pool.execute(
        "DO $$ DECLARE r record; BEGIN \
           FOR r IN SELECT tablename FROM pg_tables \
                     WHERE schemaname = 'public' AND tablename LIKE 'kb\\_%' \
           LOOP EXECUTE 'TRUNCATE TABLE ' || quote_ident(r.tablename) || ' RESTART IDENTITY CASCADE'; \
           END LOOP; END $$;",
    )
    .await
    .expect("truncate kb_ data tables to a seed-free baseline");
}

/// FAILS IF replay of a span holding a resource erasure and two edge erasures leaves any projection
/// table different from the one the units left: a missing `edge_erased` arm, say, or redaction rows
/// or edge bodies replay does not reproduce. It shows the span's projections are byte-identical;
/// nothing later in the span touches the edges, so it does not show at which position replay runs
/// an edge body.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_of_an_estate_edge_erasure_is_byte_identical(pool: PgPool) {
    let w = world(&pool).await;
    let op = world(&pool).await;
    test_support::grant_governance(&pool, op.profile).await;
    let team = team_context(&pool).await;

    // An estate resource with its own labelled edge, erased by the resource unit.
    let erased = resource(&pool, &w, w.home, "estate page").await;
    let neighbour = resource(&pool, &w, team, "team neighbour").await;
    edge(
        &pool,
        &w,
        erased,
        neighbour,
        "the estate page's own label",
        "the estate page's edge reasoning",
    )
    .await;

    // An estate-homed edge between two team resources: labelled, faceted, with reasoning.
    let a = resource(&pool, &w, team, "alpha").await;
    let b = resource(&pool, &w, team, "beta").await;
    let live = edge(
        &pool,
        &w,
        a,
        b,
        "jane roe told us",
        "jane roe said so on the call",
    )
    .await;
    facet(
        &pool,
        &w,
        live,
        json!({ "node_label": "jane roe", "status": "confided" }),
    )
    .await;

    // An estate-homed edge between two team resources, folded by the ordinary path first.
    let c = resource(&pool, &w, team, "gamma").await;
    let d = resource(&pool, &w, team, "delta").await;
    let folded = edge(
        &pool,
        &w,
        c,
        d,
        "another free label",
        "the folded edge's own reasoning",
    )
    .await;
    facet(&pool, &w, folded, json!({ "status": "folded first" })).await;
    fold(&pool, &w, folded).await;

    // One span: one transaction, one request reference, as one act running both units leaves it.
    let request_ref = Uuid::now_v7();
    let mut act = pool.begin().await.expect("begin the span");
    sqlx::query("SELECT resource_erasure_execute($1, $2, $3, $4, '{}')")
        .bind(erased.uuid())
        .bind(op.profile)
        .bind(op.emitter)
        .bind(request_ref)
        .execute(&mut *act)
        .await
        .expect("the resource unit erases the estate resource");
    for e in [live, folded] {
        sqlx::query("SELECT edge_erasure_execute($1, $2, $3, $4, $5)")
            .bind(e.uuid())
            .bind(w.profile)
            .bind(op.profile)
            .bind(op.emitter)
            .bind(request_ref)
            .execute(&mut *act)
            .await
            .expect("the edge unit erases the estate-homed edge");
    }
    act.commit().await.expect("commit the span");

    let resource_records = span_events(&pool, "resource_erased", request_ref).await;
    let edge_records = span_events(&pool, "edge_erased", request_ref).await;
    assert_eq!(resource_records.len(), 1, "one resource_erased in the span");
    assert_eq!(edge_records.len(), 2, "two edge_erased in the span");
    let at = resource_records[0].1;
    assert!(
        edge_records.iter().all(|(_, t)| *t == at),
        "the span's records share one occurred_at"
    );

    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    reset_namespace(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();
    assert_eq!(before.len(), after.len(), "the same projection tables");
    for ((table_a, x), (table_b, y)) in before.iter().zip(after.iter()) {
        assert_eq!(table_a, table_b);
        assert_eq!(x, y, "projection table {table_a} diverged under replay");
    }

    assert_eq!(
        span_events(&pool, "edge_erased", request_ref).await.len(),
        2,
        "the replayed ledger holds both edge_erased records"
    );
}
