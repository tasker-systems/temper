#![cfg(feature = "artifact-tests")]
//! Fast-follow to Deliverable 3a (generalized access-capability arc): close the CONTEXT-homed-edge
//! arm of `anchor_readable_by_profile`.
//!
//! D3a (`20260630000002`) wired explicit `kb_access_grants` READ rows into context-read
//! (`context_visible_to`) and homed-RESOURCE read (`resources_visible_to`), but its own scope note
//! flagged one gap: the `kb_contexts` arm of `anchor_readable_by_profile` — which `edges_visible_to`
//! gates each edge on via its HOME anchor — had no grant branch. So a profile granted read on a
//! context could see the context's resources yet NOT the edges among them: a false-negative, never a
//! leak. This test pins the fix (`20260701000004`): the context arm gains the same
//! `profile_explicit_grant(…, 'read', 'kb_contexts', …)` clause `context_visible_to` already carries,
//! so "a context you can read", "its resources", and "the edges among them" agree by construction.
//!
//! The edge is minted through the production write path (`writes::create_resource` +
//! `writes::assert_relationship_with`) rather than a hand-rolled insert, because `kb_edges` carries
//! FKs to `kb_events` — the synthetic-anchor trick the sibling read-wiring test uses does not reach
//! edges.

use sqlx::PgPool;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, EntityId, ProfileId};
use temper_substrate::payloads::{AnchorRef, EdgePolarity};
use temper_substrate::writes::{self, AssertParams, CreateParams};
use uuid::Uuid;

mod common;

async fn edge_visible(pool: &PgPool, profile: Uuid, edge: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM edges_visible_to($1) v WHERE v.edge_id=$2)")
        .bind(profile)
        .bind(edge)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn resource_visible(pool: &PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id=$2)",
    )
    .bind(profile)
    .bind(resource)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn grant_context_read(pool: &PgPool, context: Uuid, profile: Uuid, granter: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
         (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, $3)",
    )
    .bind(context)
    .bind(profile)
    .bind(granter)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn explicit_context_read_grant_confers_context_homed_edge(pool: PgPool) {
    common::seed_system(&pool).await;
    let sys_profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let sys_entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(sys_profile)
            .fetch_one(&pool)
            .await
            .unwrap();
    let owner = ProfileId::from(sys_profile);
    let emitter = EntityId::from(sys_entity);

    // A profile-owned context (owned by the system profile). The reader is neither owner nor a
    // member of any team — the ONLY path we will give them is an explicit context read-grant.
    let context = ContextId::from(
        common::insert_context(
            &pool,
            "kb_profiles",
            sys_profile,
            "edge-grant",
            "Edge Grant",
        )
        .await
        .expect("context"),
    );
    let reader = common::insert_profile(&pool, "edge_grant_reader").await;

    // Two resources homed in the context, and a context-homed edge between them.
    let src = writes::create_resource(
        &pool,
        CreateParams {
            sources: vec![],
            title: "eg-src",
            origin_uri: "temper://eg-src",
            body: "body",
            doc_type: "research",
            home: AnchorRef::context(context),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
        },
    )
    .await
    .expect("create src");
    let tgt = writes::create_resource(
        &pool,
        CreateParams {
            sources: vec![],
            title: "eg-tgt",
            origin_uri: "temper://eg-tgt",
            body: "body",
            doc_type: "research",
            home: AnchorRef::context(context),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
        },
    )
    .await
    .expect("create tgt");
    let edge = writes::assert_relationship_with(
        &pool,
        AssertParams {
            src,
            tgt,
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("depends_on"),
            weight: 0.8,
            home: context,
            emitter,
        },
        EventContext::default(),
    )
    .await
    .expect("assert edge");

    // Baseline: the reader has no path to either endpoint or the edge.
    assert!(
        !resource_visible(&pool, reader, src.uuid()).await,
        "no src read before grant"
    );
    assert!(
        !edge_visible(&pool, reader, edge.uuid()).await,
        "no edge read before grant"
    );

    grant_context_read(&pool, context.uuid(), reader, sys_profile).await;

    // The context read-grant confers BOTH endpoints AND the edge among them — the closed gap.
    // (Endpoints already flowed through D3a's `resources_visible_to` branch; the edge is the arm
    // this fast-follow adds to `anchor_readable_by_profile`.)
    assert!(
        resource_visible(&pool, reader, src.uuid()).await,
        "grant ⇒ src read"
    );
    assert!(
        resource_visible(&pool, reader, tgt.uuid()).await,
        "grant ⇒ tgt read"
    );
    assert!(
        edge_visible(&pool, reader, edge.uuid()).await,
        "grant ⇒ context-homed edge read (the fast-follow)"
    );

    // An ungranted profile still sees nothing — additive, no behavior change.
    let other = common::insert_profile(&pool, "edge_grant_other").await;
    assert!(!resource_visible(&pool, other, src.uuid()).await);
    assert!(!edge_visible(&pool, other, edge.uuid()).await);
}
