#![cfg(feature = "test-db")]
//! The `property_retracted` correction affordance — row-grain retraction of an edge-owned
//! facet, addressed by `property_id` and bound to the edge.
//!
//! Witnesses for the named bite states: a retracted row is re-assertable as a FRESH row
//! (insert-if-not-live sees the folded row as not-live), a foreign-edge retraction refuses
//! indistinguishably from a missing id (assert the error bodies are EQUAL, not just both 404),
//! an already-retracted id refuses again, the facets read returns live rows only, and an
//! unauthorized caller gets the gate's own refusal — never a 404 that would act as an
//! existence oracle.
//!
//! Backend-level, like the write leg's sibling file (`anchored_at_write_test.rs`): every
//! surface dispatches through `DbBackend`, so the shared dispatch is the layer that bites.

mod common;

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::types::facet_requests::ANCHORED_AT_PROPERTY_KEY;
use temper_core::types::graph;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, EdgeId, ProfileId, PropertyId, ResourceId};
use temper_core::types::property_owner::PropertyOwner;
use temper_services::backend::DbBackend;
use temper_workflow::operations::{
    AssertRelationship, Backend, CreateResource, RetractFacet, SetFacet, Surface,
};
use temper_workflow::types::managed_meta::ManagedMeta;

// ─── fixtures ────────────────────────────────────────────────────────────────

fn create_cmd(context: Uuid, slug: &str) -> CreateResource {
    CreateResource {
        idempotency_key: None,
        slug: slug.to_string(),
        doctype: "research".to_string(),
        home: HomeAnchor::Context(ContextId::from(context)),
        title: format!("anchored-retract {slug}"),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        origin_uri: Some(format!("test://retract-{slug}-{}", Uuid::new_v4())),
        chunks_packed: None,
        content_hash: None,
        goal: None,
        act: Default::default(),
        origin: Surface::ApiHttp,
    }
}

/// A profile with one context, two resources homed there, and TWO `derived_from` edges —
/// source→target and target→source — so the foreign-edge arm has a real second edge to aim at.
/// Returns `(backend, profile, source, target, edge_a, edge_b)`.
#[allow(clippy::type_complexity)]
async fn two_edge_fixture(pool: &PgPool) -> (DbBackend, Uuid, Uuid, Uuid, Uuid, Uuid) {
    let email = format!("retract-{}@example.com", Uuid::new_v4());
    let (profile, context) = common::fixtures::create_test_profile_with_context(pool, &email).await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let source = Uuid::from(
        backend
            .create_resource(create_cmd(context, "src"))
            .await
            .expect("create source")
            .value
            .id,
    );
    let target = Uuid::from(
        backend
            .create_resource(create_cmd(context, "tgt"))
            .await
            .expect("create target")
            .value
            .id,
    );
    let edge_a = Uuid::from(
        backend
            .assert_relationship(AssertRelationship {
                source: ResourceId::from(source),
                target: ResourceId::from(target),
                target_table: Default::default(),
                edge_kind: graph::EdgeKind::LeadsTo,
                polarity: graph::Polarity::Forward,
                label: "derived_from".to_string(),
                weight: 1.0,
                act: Default::default(),
                origin: Surface::ApiHttp,
            })
            .await
            .expect("assert edge a")
            .value,
    );
    let edge_b = Uuid::from(
        backend
            .assert_relationship(AssertRelationship {
                source: ResourceId::from(target),
                target: ResourceId::from(source),
                target_table: Default::default(),
                edge_kind: graph::EdgeKind::LeadsTo,
                polarity: graph::Polarity::Forward,
                label: "derived_from".to_string(),
                weight: 1.0,
                act: Default::default(),
                origin: Surface::ApiHttp,
            })
            .await
            .expect("assert edge b")
            .value,
    );
    (backend, profile, source, target, edge_a, edge_b)
}

fn anchored_cmd(edge: Uuid, values: serde_json::Value) -> SetFacet {
    SetFacet {
        owner: PropertyOwner::edge(EdgeId::from(edge)),
        property_key: Some(ANCHORED_AT_PROPERTY_KEY.to_string()),
        values,
        weight: 1.0,
        act: Default::default(),
        origin: Surface::ApiHttp,
    }
}

fn anchored_values(endpoint: &str, resource: Uuid, block: Uuid) -> serde_json::Value {
    serde_json::json!({
        "endpoint": endpoint,
        "address": format!("{resource}#{block}"),
    })
}

fn retract_cmd(edge: Uuid, property_id: Uuid) -> RetractFacet {
    RetractFacet {
        edge_handle: EdgeId::from(edge),
        property_id: PropertyId::from(property_id),
        act: Default::default(),
        origin: Surface::ApiHttp,
    }
}

/// The live anchored-at rows of one edge, as the facets read returns them (the service the
/// read surface dispatches to — the ledger distinguishes, the product read does not).
async fn live_facet_rows(pool: &PgPool, profile: Uuid, edge: Uuid) -> Vec<Uuid> {
    let rows = temper_services::services::edge_service::list_edge_facets(pool, profile, edge)
        .await
        .expect("list the edge's facets");
    rows.iter().map(|r| r.property_id).collect()
}

/// One anchor asserted on `edge`, its row id returned.
async fn assert_anchor(backend: &DbBackend, edge: Uuid, endpoint: &str, half: Uuid) -> Uuid {
    let ack = backend
        .set_facet(anchored_cmd(
            edge,
            anchored_values(endpoint, half, Uuid::now_v7()),
        ))
        .await
        .expect("the anchor asserts");
    assert_eq!(ack.value.len(), 1, "one keyed assert, one row");
    Uuid::from(ack.value[0])
}

// ─── the act: fold + stamp, payload owner-shaped, trail blind ─────────────────

/// A retraction folds the row in place — it persists, `is_folded` set, `last_event_id`
/// stamped to the retraction event — and the event's payload is owner-shaped with NO
/// `edge_id` key, so the element trail stays blind to property lifecycle exactly as it is
/// for `property_asserted` (that trail matches `payload->>'edge_id'` only).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_retraction_folds_the_row_and_stamps_the_retraction_event(pool: PgPool) {
    let (backend, _profile, source, _target, edge, _edge_b) = two_edge_fixture(&pool).await;
    let property_id = assert_anchor(&backend, edge, "source", source).await;

    backend
        .retract_facet(retract_cmd(edge, property_id))
        .await
        .expect("the retraction lands");

    let row: (bool, Uuid) =
        sqlx::query_as("SELECT is_folded, last_event_id FROM kb_properties WHERE id = $1")
            .bind(property_id)
            .fetch_one(&pool)
            .await
            .expect("the retracted row persists");
    assert!(row.0, "the row is folded, never deleted");

    let (event_name, payload): (String, serde_json::Value) = sqlx::query_as(
        "SELECT et.name, e.payload FROM kb_events e \
          JOIN kb_event_types et ON et.id = e.event_type_id \
         WHERE et.name = 'property_retracted'",
    )
    .fetch_one(&pool)
    .await
    .expect("exactly one retraction event");
    assert_eq!(event_name, "property_retracted");
    assert_eq!(
        row.1,
        sqlx::query_scalar::<_, Uuid>(
            "SELECT e.id FROM kb_events e \
              JOIN kb_event_types et ON et.id = e.event_type_id \
             WHERE et.name = 'property_retracted'",
        )
        .fetch_one(&pool)
        .await
        .expect("the retraction event id"),
        "the row's last_event_id is the retraction event"
    );
    assert_eq!(
        payload["owner"]["table"], "kb_edges",
        "owner-shaped payload, edge owner"
    );
    assert_eq!(payload["owner"]["id"], edge.to_string().as_str());
    assert_eq!(payload["property_id"], property_id.to_string().as_str());
    assert!(
        payload.get("edge_id").is_none(),
        "no edge_id spelling — the element trail must stay blind to property lifecycle: {payload}"
    );
}

// ─── re-assertability: the fold frees the address ─────────────────────────────

/// Retract then re-assert the SAME address: the re-assertion mints a FRESH row with a NEW id
/// and fresh authorship — it inherits nothing. The insert-if-not-live pre-check and the
/// partial unique index both see the folded row as not-live.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_retracted_address_is_re_assertable_as_a_fresh_row(pool: PgPool) {
    let (backend, _profile, source, _target, edge, _edge_b) = two_edge_fixture(&pool).await;
    let address = anchored_values("source", source, Uuid::now_v7());

    let first = Uuid::from(
        backend
            .set_facet(anchored_cmd(edge, address.clone()))
            .await
            .expect("first assert lands")
            .value[0],
    );
    backend
        .retract_facet(retract_cmd(edge, first))
        .await
        .expect("the retraction lands");

    let second = Uuid::from(
        backend
            .set_facet(anchored_cmd(edge, address))
            .await
            .expect("re-asserting the retracted address lands")
            .value[0],
    );
    assert_ne!(
        first, second,
        "the re-assertion mints a fresh row — it inherits nothing from the folded one"
    );

    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE owner_table = 'kb_edges' AND owner_id = $1",
    )
    .bind(edge)
    .fetch_one(&pool)
    .await
    .expect("row count for the edge");
    assert_eq!(total, 2, "the folded row persists beside its successor");

    let live = live_facet_rows(&pool, _profile, edge).await;
    assert_eq!(
        live,
        vec![second],
        "exactly the fresh row is live — the product read distinguishes nothing further"
    );
}

// ─── the three refusals render ONE 404 ────────────────────────────────────────

/// Retracting a property id that names edge A but is addressed to edge B renders the SAME
/// error SHAPE — not just same-status — as a fully missing id. No arm of the refusal may
/// distinguish a foreign row from an absent one: that would be an existence oracle over
/// property rows deployment-wide. Each body names only the inputs its own caller sent
/// (identical template, per-request ids), so neither discloses why it refused.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_foreign_edge_retraction_refuses_indistinguishably_from_a_missing_id(pool: PgPool) {
    let (backend, _profile, source, _target, edge_a, edge_b) = two_edge_fixture(&pool).await;
    let property_id = assert_anchor(&backend, edge_a, "source", source).await;
    let missing_id = Uuid::now_v7();

    let foreign = backend
        .retract_facet(retract_cmd(edge_b, property_id))
        .await
        .expect_err("the foreign-edge retraction must refuse");
    let missing = backend
        .retract_facet(retract_cmd(edge_b, missing_id))
        .await
        .expect_err("the missing-id retraction must refuse");

    let (TemperError::NotFound(foreign_body), TemperError::NotFound(missing_body)) =
        (&foreign, &missing)
    else {
        panic!("both refusals must render NotFound: {foreign:?} / {missing:?}")
    };
    let template =
        |pid: Uuid| format!("facet_retract: property {pid} is not a live facet of edge {edge_b}");
    assert_eq!(
        foreign_body,
        &template(property_id),
        "the foreign arm renders the one shape, naming only the caller's inputs"
    );
    assert_eq!(
        missing_body,
        &template(missing_id),
        "the missing arm renders the SAME shape — neither body says which arm fired"
    );
}

/// An already-retracted id refuses again — the idempotent refusal, not a second fold: the row
/// keeps pointing at the FIRST retraction event and no second event appends.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_already_retracted_id_refuses_again_without_a_second_fold(pool: PgPool) {
    let (backend, _profile, source, _target, edge, _edge_b) = two_edge_fixture(&pool).await;
    let property_id = assert_anchor(&backend, edge, "source", source).await;

    let first = backend
        .retract_facet(retract_cmd(edge, property_id))
        .await
        .expect("the first retraction lands");
    assert_eq!(Uuid::from(first.value), property_id);

    let again = backend
        .retract_facet(retract_cmd(edge, property_id))
        .await
        .expect_err("the second retraction must refuse, not re-fold");
    assert!(
        matches!(again, TemperError::NotFound(_)),
        "an already-retracted id is not a success and not a 5xx: {again:?}"
    );

    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e \
          JOIN kb_event_types et ON et.id = e.event_type_id \
         WHERE et.name = 'property_retracted'",
    )
    .fetch_one(&pool)
    .await
    .expect("retraction event count");
    assert_eq!(events, 1, "the refusal appended no second ledger event");

    let stamp: Uuid = sqlx::query_scalar("SELECT last_event_id FROM kb_properties WHERE id = $1")
        .bind(property_id)
        .fetch_one(&pool)
        .await
        .expect("the retracted row");
    let first_event: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e \
          JOIN kb_event_types et ON et.id = e.event_type_id \
         WHERE et.name = 'property_retracted'",
    )
    .fetch_one(&pool)
    .await
    .expect("the one retraction event");
    assert_eq!(
        stamp, first_event,
        "the row keeps its first retraction stamp"
    );
}

// ─── the read and the gate ────────────────────────────────────────────────────

/// The facets read returns live rows only: after retracting one of two anchors, the read
/// returns exactly the other row. The ledger distinguishes the retraction (a
/// `property_retracted` event, not a cascade); the product read does not.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_facets_read_returns_live_rows_only_after_a_retraction(pool: PgPool) {
    let (backend, profile, source, target, edge, _edge_b) = two_edge_fixture(&pool).await;

    let source_anchor = assert_anchor(&backend, edge, "source", source).await;
    let target_anchor = assert_anchor(&backend, edge, "target", target).await;
    let before = live_facet_rows(&pool, profile, edge).await;
    assert_eq!(
        before.len(),
        2,
        "both anchors are live before the retraction: {before:?}"
    );

    backend
        .retract_facet(retract_cmd(edge, source_anchor))
        .await
        .expect("the retraction lands");

    let after = live_facet_rows(&pool, profile, edge).await;
    assert_eq!(
        after,
        vec![target_anchor],
        "only the survivor is live — the retracted row is gone from the read"
    );
}

/// An unauthorized caller gets the gate's own refusal — `Forbidden` from
/// `check_edge_mutable`'s authority clauses — never a 404. A not-found here would let a
/// caller without standing probe which property ids exist on an edge.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unauthorized_caller_gets_the_gates_refusal_never_a_404(pool: PgPool) {
    let (backend, _profile, source, _target, edge, _edge_b) = two_edge_fixture(&pool).await;
    let property_id = assert_anchor(&backend, edge, "source", source).await;

    let stranger_email = format!("stranger-{}@example.com", Uuid::new_v4());
    let stranger = common::fixtures::create_test_profile(&pool, &stranger_email).await;
    let stranger_backend = DbBackend::new(pool.clone(), ProfileId::from(stranger));

    let denied = stranger_backend
        .retract_facet(retract_cmd(edge, property_id))
        .await
        .expect_err("a caller with no standing must be refused");
    assert!(
        matches!(denied, TemperError::Forbidden),
        "the gate's authority refusal, never a 404 that would act as an existence oracle: {denied:?}"
    );

    let folded: bool = sqlx::query_scalar("SELECT is_folded FROM kb_properties WHERE id = $1")
        .bind(property_id)
        .fetch_one(&pool)
        .await
        .expect("the anchored row");
    assert!(!folded, "the refusal wrote nothing");
}
