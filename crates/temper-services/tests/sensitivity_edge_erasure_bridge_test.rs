#![cfg(feature = "test-db")]
//! The sweep's digest expiry for an edge erased by `edge_erasure_execute` (20261201000060).
//!
//! The edge is stranded: homed in a context of the person's estate, between resources no erasure
//! reaches. A finding on its label or on a property it owns is therefore never held by
//! `sensitivity.erased_place_findings`' erased-endpoint arms; the arms for an edge an
//! `edge_erased` record names are what start its window. Findings are inserted directly, as the
//! sweep would store them, so the witness does not depend on a detector matching the fixture text.

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::property_owner::PropertyOwner;
use temper_services::test_support;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::{fire, EdgeHome, EventContext, SeedAction};
use temper_substrate::ids::{ContextId, EdgeId, EntityId, ResourceId};
use temper_substrate::payloads::{AnchorRef, EdgePolarity};
use temper_substrate::writes::{self, CreateParams};

const A_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

// ── fixture helpers (duplicated per file, per this suite's convention) ──

struct World {
    profile: Uuid,
    emitter: Uuid,
    home: ContextId,
}

/// A profile, its `@web` emitter and an `@me` context, which `_erasure_estate_contexts(profile)`
/// returns.
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

async fn resource(pool: &PgPool, w: &World, title: &str) -> ResourceId {
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: &format!("test://{}", Uuid::now_v7()),
            body: "a body",
            doc_type: "research",
            home: AnchorRef::context(w.home),
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

/// A labelled edge homed in the world's context, with one facet, through the real write paths.
async fn faceted_edge(pool: &PgPool, w: &World, src: ResourceId, tgt: ResourceId) -> EdgeId {
    let mut conn = pool.acquire().await.expect("acquire");
    let e = fire(
        &mut conn,
        SeedAction::RelationshipAssert {
            src: AnchorRef::resource(src),
            tgt: AnchorRef::resource(tgt),
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("jane roe told us"),
            weight: 1.0,
            home: EdgeHome::Context(w.home),
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .expect("assert the edge")
    .relationship()
    .expect("an edge id");
    fire(
        &mut conn,
        SeedAction::FacetSet {
            owner: PropertyOwner::Edge { id: e },
            values: &json!({ "node_label": "jane roe" }),
            weight: 1.0,
            emitter: EntityId::from(w.emitter),
        },
    )
    .await
    .expect("set a facet on the edge");
    e
}

/// One stored finding; `path` is set exactly when the surface is jsonb.
async fn finding(
    pool: &PgPool,
    surface: &str,
    target_table: &str,
    target: Uuid,
    path: Option<&str>,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO sensitivity.findings (surface, target_table, target_id, path, content_hash, \
           detector_id, detector_version, category, severity, match_count) \
         VALUES ($1, $2, $3, $4, $5, 'us_ssn_delimited', 1, 'national_id', 4, 1) RETURNING id",
    )
    .bind(surface)
    .bind(target_table)
    .bind(target)
    .bind(path)
    .bind(A_HASH)
    .fetch_one(pool)
    .await
    .expect("store a finding")
}

async fn erased_place_findings(pool: &PgPool) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT finding_id FROM sensitivity.erased_place_findings()")
        .fetch_all(pool)
        .await
        .unwrap()
}

/// FAILS IF `sensitivity.erased_place_findings` holds a finding on an erased edge's label or on a
/// property it owns only when an endpoint is an erased resource: after the unit erases a stranded
/// edge neither finding is held, so `expire_erased_fingerprints` never starts their window. Also
/// fails if either is held while the edge is live.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_erased_edges_label_and_facet_findings_are_erased_places(pool: PgPool) {
    let w = world(&pool).await;
    let op = world(&pool).await;
    let a = resource(&pool, &w, "alpha").await;
    let b = resource(&pool, &w, "beta").await;
    let e = faceted_edge(&pool, &w, a, b).await;
    let facet_row: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_properties \
          WHERE owner_table = 'kb_edges' AND owner_id = $1 AND property_key = 'facet' \
            AND NOT is_folded",
    )
    .bind(e.uuid())
    .fetch_one(&pool)
    .await
    .expect("the edge's facet row");

    let on_label = finding(&pool, "kb_edges.label", "kb_edges", e.uuid(), None).await;
    let on_facet = finding(
        &pool,
        "kb_properties.property_value",
        "kb_properties",
        facet_row,
        Some("/node_label"),
    )
    .await;

    let held = erased_place_findings(&pool).await;
    assert!(
        !held.contains(&on_label) && !held.contains(&on_facet),
        "a live edge's findings are not erased places"
    );

    sqlx::query("SELECT edge_erasure_execute($1, $2, $3, $4, $5)")
        .bind(e.uuid())
        .bind(w.profile)
        .bind(op.profile)
        .bind(op.emitter)
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await
        .expect("the unit erases the stranded edge");

    let held = erased_place_findings(&pool).await;
    assert!(held.contains(&on_label), "the label finding is held");
    assert!(held.contains(&on_facet), "the facet finding is held");

    sqlx::query("SELECT sensitivity_expire_erased_fingerprints()")
        .execute(&pool)
        .await
        .unwrap();
    let clocked: Vec<Uuid> =
        sqlx::query_scalar("SELECT finding_id FROM sensitivity.erased_closures")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        clocked.contains(&on_label) && clocked.contains(&on_facet),
        "both findings' windows started; clocked {clocked:?}"
    );
}
