#![cfg(feature = "artifact-tests")]
//! Chunk D acceptance invariant: **per-act agent authorship is invisible to the affinity / region
//! projection by construction.** Authorship rides `kb_events.metadata`; the affinity math reads only
//! the projection tables (`kb_edges` here). So an edge asserted *with* graded authorship and the same
//! edge asserted *without* it produce a byte-identical edge projection and an identical affinity —
//! while the authored act, and only it, carries the authorship in `kb_events.metadata`.
//!
//! (06-18 plan §arch, carried verbatim: "authorship rides kb_events.metadata, NOT the event payload —
//! so it is invisible to projections (and thus affinity math) by construction.")

use sqlx::{PgPool, Row};
use temper_substrate::affinity::{affinity, Edge, EdgeKind, Lens};
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AgentAuthorship, AnchorRef, ConfidenceBand, EdgePolarity};
use temper_substrate::writes::{self, AssertParams, CreateParams};
use uuid::Uuid;

mod common;

async fn system_profile(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn system_entity(pool: &PgPool, profile: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
        .bind(profile)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Create two resources and assert a `leads_to` edge between them under the given act context.
/// Returns the (src, tgt, edge_id).
async fn build_edge_pair(
    pool: &PgPool,
    home: ContextId,
    owner: ProfileId,
    emitter: EntityId,
    tag: &str,
    ctx: EventContext,
) -> (ResourceId, ResourceId, Uuid) {
    let src_title = format!("{tag}-src");
    let tgt_title = format!("{tag}-tgt");
    let src = writes::create_resource(
        pool,
        CreateParams {
            sources: vec![],
            title: &src_title,
            origin_uri: &src_title,
            body: "body",
            doc_type: "research",
            home: AnchorRef::context(home),
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
        pool,
        CreateParams {
            sources: vec![],
            title: &tgt_title,
            origin_uri: &tgt_title,
            body: "body",
            doc_type: "research",
            home: AnchorRef::context(home),
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
        pool,
        AssertParams {
            src,
            tgt,
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("depends_on"),
            weight: 0.8,
            home,
            emitter,
        },
        ctx,
    )
    .await
    .expect("assert edge");
    (src, tgt, edge.uuid())
}

/// Load an edge's affinity inputs (kind/weight/label) from the projection table and its asserting
/// event's metadata, then compute the affinity over its endpoints.
async fn project_and_affinity(
    pool: &PgPool,
    src: ResourceId,
    tgt: ResourceId,
    edge_id: Uuid,
) -> (f64, serde_json::Value, String, f64) {
    let row = sqlx::query(
        "SELECT e.edge_kind::text AS kind, e.weight, e.label, ev.metadata
           FROM kb_edges e
           JOIN kb_events ev ON ev.id = e.asserted_by_event_id
          WHERE e.id = $1",
    )
    .bind(edge_id)
    .fetch_one(pool)
    .await
    .expect("load edge projection");
    let kind_text: String = row.get("kind");
    let weight: f64 = row.get("weight");
    let metadata: serde_json::Value = row.get("metadata");
    let edge = Edge {
        src,
        tgt,
        kind: EdgeKind::from_sql(&kind_text).expect("known edge kind"),
        weight,
        label: row.get("label"),
    };
    let aff = affinity(src, tgt, &[edge], &[], &Lens::telos_default());
    (aff, metadata, kind_text, weight)
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn authorship_is_invisible_to_affinity_projection(pool: PgPool) {
    common::seed_system(&pool).await;
    let profile = system_profile(&pool).await;
    let entity = EntityId::from(system_entity(&pool, profile).await);
    let owner = ProfileId::from(profile);
    let home = ContextId::from(
        common::insert_context(&pool, "kb_profiles", profile, "inv-proj", "Invisibility")
            .await
            .expect("context"),
    );

    // Edge A: asserted WITH graded authorship (no invocation needed — authorship rides alone).
    let authored_ctx = EventContext {
        authorship: Some(AgentAuthorship {
            reasoning: Some("these co-vary".to_string()),
            confidence: ConfidenceBand::Confident,
            rationale: None,
            persona: Some("steward".to_string()),
            model: None,
        }),
        invocation: None,
        correlation: None,
    };
    let (a_src, a_tgt, a_edge) =
        build_edge_pair(&pool, home, owner, entity, "authored", authored_ctx).await;

    // Edge B: the same edge with NO authorship.
    let (b_src, b_tgt, b_edge) =
        build_edge_pair(&pool, home, owner, entity, "plain", EventContext::default()).await;

    let (a_aff, a_meta, a_kind, a_weight) = project_and_affinity(&pool, a_src, a_tgt, a_edge).await;
    let (b_aff, b_meta, b_kind, b_weight) = project_and_affinity(&pool, b_src, b_tgt, b_edge).await;

    // The edge PROJECTION (the affinity inputs) is byte-identical regardless of authorship.
    assert_eq!(a_kind, b_kind, "edge kind identical");
    assert_eq!(a_weight, b_weight, "edge weight identical");
    assert_eq!(
        a_aff, b_aff,
        "affinity is identical with/without authorship: {a_aff} vs {b_aff}"
    );

    // …yet the authored act, and ONLY it, carries the authorship in kb_events.metadata.
    assert_eq!(
        a_meta["confidence"], "confident",
        "authored edge act carries authorship metadata: {a_meta}"
    );
    assert_eq!(
        a_meta["reasoning"], "these co-vary",
        "authored edge act carries reasoning: {a_meta}"
    );
    assert_eq!(
        b_meta,
        serde_json::json!({}),
        "the un-authored edge act has empty metadata (no leak into the projection input either): {b_meta}"
    );
}
