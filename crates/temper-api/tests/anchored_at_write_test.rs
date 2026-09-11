#![cfg(feature = "test-db")]
//! The `anchored-at` write action — the key-carrying, edge-owner facet write.
//!
//! Witnesses for the write leg's three behaviors: structural validation at the shared
//! dispatch (`DbBackend::set_facet` — refusals that append no ledger event), insert-if-not-live
//! assertion (a repeated assert of a live address acks the existing row id), and the
//! one-row-per-(endpoint, block) landing with the object value stored verbatim.
//!
//! The validation probes STRUCTURE only, and the happy path is what bites on that: the block
//! halves below name NO row — they are fabricated uuids — and the writes succeed anyway. A
//! write-time existence probe would fail these tests.

mod common;

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::types::facet_requests::ANCHORED_AT_PROPERTY_KEY;
use temper_core::types::graph;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, EdgeId, ProfileId, ResourceId};
use temper_core::types::property_owner::PropertyOwner;
use temper_services::backend::DbBackend;
use temper_workflow::operations::{AssertRelationship, Backend, CreateResource, SetFacet, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;

// ─── fixtures ────────────────────────────────────────────────────────────────

fn create_cmd(context: Uuid, slug: &str) -> CreateResource {
    CreateResource {
        idempotency_key: None,
        slug: slug.to_string(),
        doctype: "research".to_string(),
        home: HomeAnchor::Context(ContextId::from(context)),
        title: format!("anchored-at {slug}"),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        origin_uri: Some(format!("test://anchored-{slug}-{}", Uuid::new_v4())),
        chunks_packed: None,
        content_hash: None,
        goal: None,
        act: Default::default(),
        origin: Surface::ApiHttp,
    }
}

/// A profile with one context, two resources homed there, and one `derived_from` edge
/// source → target — the shape every test below shares.
async fn edge_fixture(pool: &PgPool) -> (DbBackend, Uuid, Uuid, Uuid) {
    let email = format!("anchored-{}@example.com", Uuid::new_v4());
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
    let edge = backend
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
        .expect("assert the edge")
        .value;
    (backend, source, target, Uuid::from(edge))
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

async fn anchored_row_count(pool: &PgPool, edge: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE owner_table = 'kb_edges' AND owner_id = $1 \
            AND property_key = 'anchored-at'",
    )
    .bind(edge)
    .fetch_one(pool)
    .await
    .expect("anchored-at row count")
}

// ─── the happy path ──────────────────────────────────────────────────────────

/// One anchored-at row per (endpoint, block), the object value stored verbatim, edge-owned.
///
/// The block uuids name NO rows — deliberately. A write-time existence probe over the block
/// would refuse these writes, so a passing assert is the bite proving validation is structural
/// only (existence and fold state are the read contract's to state).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_happy_path_assert_lands_one_verbatim_row_per_endpoint_and_block(pool: PgPool) {
    let (backend, source, target, edge) = edge_fixture(&pool).await;

    // Fabricated block halves: no kb_content_blocks rows anywhere carry these ids.
    let source_block = Uuid::now_v7();
    let target_block = Uuid::now_v7();

    let source_ack = backend
        .set_facet(anchored_cmd(
            edge,
            anchored_values("source", source, source_block),
        ))
        .await
        .expect("a structurally valid source-side anchor must land");
    let target_ack = backend
        .set_facet(anchored_cmd(
            edge,
            anchored_values("target", target, target_block),
        ))
        .await
        .expect("a structurally valid target-side anchor must land");

    assert_eq!(
        source_ack.value.len(),
        1,
        "one keyed assert writes exactly one row: {source_ack:?}"
    );
    assert_ne!(
        source_ack.value[0], target_ack.value[0],
        "two addresses are two rows, never one"
    );

    let stored: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT owner_table, property_value FROM kb_properties \
          WHERE owner_id = $1 AND property_key = 'anchored-at' AND NOT is_folded \
          ORDER BY property_value->>'address'",
    )
    .bind(edge)
    .fetch_all(&pool)
    .await
    .expect("stored anchored-at rows");

    assert_eq!(
        stored,
        vec![
            (
                "kb_edges".to_string(),
                anchored_values("source", source, source_block)
            ),
            (
                "kb_edges".to_string(),
                anchored_values("target", target, target_block)
            ),
        ],
        "the value lands verbatim — the object the caller sent, under the edge's ownership"
    );
}

// ─── insert-if-not-live ──────────────────────────────────────────────────────

/// A repeated assert of a live address acks the EXISTING row id, appends nothing, and rewrites
/// nothing — the already-existed arm is a plain ack, so a retried loop converges instead of
/// noising.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_repeated_assert_of_a_live_address_acks_the_existing_row_id(pool: PgPool) {
    let (backend, source, _target, edge) = edge_fixture(&pool).await;
    let values = anchored_values("source", source, Uuid::now_v7());

    let first = backend
        .set_facet(anchored_cmd(edge, values.clone()))
        .await
        .expect("first assert lands");

    // The retry carries a different weight: an ack must not become a rewrite of the row it acks.
    let mut retry = anchored_cmd(edge, values);
    retry.weight = 0.5;
    let second = backend.set_facet(retry).await.expect("retry acks");

    assert_eq!(
        first.value, second.value,
        "the retry must ack the first row's id, not mint a twin: {first:?} vs {second:?}"
    );
    assert_eq!(
        anchored_row_count(&pool, edge).await,
        1,
        "one address, one live row"
    );

    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events \
          WHERE payload->>'property_key' = 'anchored-at' \
            AND payload#>>'{owner,id}' = $1",
    )
    .bind(edge.to_string())
    .fetch_one(&pool)
    .await
    .expect("anchored-at event count");
    assert_eq!(events, 1, "the ack arm appended no second ledger event");

    let weight: f64 = sqlx::query_scalar(
        "SELECT weight FROM kb_properties \
          WHERE owner_id = $1 AND property_key = 'anchored-at' AND NOT is_folded",
    )
    .bind(edge)
    .fetch_one(&pool)
    .await
    .expect("the acked row");
    assert_eq!(
        weight, 1.0,
        "the ack rewrote nothing — the row keeps its original assertion"
    );
}

// ─── structural validation: the refusal arms ─────────────────────────────────

/// Every malformed value shape is refused with a 400-class error and nothing written — the
/// error body is the record; an in-transaction refusal appends no ledger event.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn malformed_anchored_at_values_are_refused_with_nothing_written(pool: PgPool) {
    let (backend, source, _target, edge) = edge_fixture(&pool).await;
    let block = Uuid::now_v7();

    let probes = vec![
        ("a non-object value", serde_json::json!("not an object")),
        (
            "a missing endpoint",
            serde_json::json!({ "address": "x#y" }),
        ),
        (
            "a missing address",
            serde_json::json!({ "endpoint": "source" }),
        ),
        (
            "an extra key",
            serde_json::json!({
                "endpoint": "source",
                "address": format!("{source}#{block}"),
                "note": "smuggled"
            }),
        ),
        (
            "an unknown endpoint word",
            serde_json::json!({
                "endpoint": "middle",
                "address": format!("{source}#{block}"),
            }),
        ),
        (
            "an address with no '#'",
            serde_json::json!({ "endpoint": "source", "address": source.to_string() }),
        ),
        (
            "an address with two '#'",
            serde_json::json!({
                "endpoint": "source",
                "address": format!("{source}#{block}#extra"),
            }),
        ),
        (
            "a non-uuid resource half",
            serde_json::json!({
                "endpoint": "source",
                "address": format!("not-a-uuid#{block}"),
            }),
        ),
        (
            "a non-uuid block half",
            serde_json::json!({
                "endpoint": "source",
                "address": format!("{source}#also-not-a-uuid"),
            }),
        ),
        (
            "a non-canonical (uppercase) resource half",
            serde_json::json!({
                "endpoint": "source",
                "address": format!("{}#{block}", source.to_string().to_uppercase()),
            }),
        ),
        (
            "a braced uuid half",
            serde_json::json!({
                "endpoint": "source",
                "address": format!("{{{source}}}#{block}"),
            }),
        ),
    ];

    for (name, values) in probes {
        let refused = backend
            .set_facet(anchored_cmd(edge, values))
            .await
            .expect_err(&format!("{name} must be refused"));
        assert!(
            matches!(refused, TemperError::BadRequest(_)),
            "{name} must refuse as a bad request, not write: {refused:?}"
        );
    }

    assert_eq!(
        anchored_row_count(&pool, edge).await,
        0,
        "no refusal arm may have written anything"
    );
}

/// A cogmap side carries no blocks, so anchoring it is refused even when the address's resource
/// half names the map correctly.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_cogmap_side_anchor_is_refused(pool: PgPool) {
    use temper_substrate::blob_store::InMemoryBlobStore;
    use temper_substrate::writes;

    let email = format!("anchored-cogmap-{}@example.com", Uuid::new_v4());
    let (profile, context) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let telos = Uuid::from(
        DbBackend::new(pool.clone(), ProfileId::from(profile))
            .create_resource(create_cmd(context, "map-telos"))
            .await
            .expect("create the map's telos resource")
            .value
            .id,
    );

    let cogmap: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind("anchored-at-cogmap-side")
    .bind(telos)
    .fetch_one(&pool)
    .await
    .expect("seed a cogmap");
    common::fixtures::grant_cogmap_write(&pool, cogmap, profile).await;

    let blob = {
        let hash = "9f2b0d4c1a7e5f83b6c9d0e2f4a6b8c0d2e4f6a8b0c2d4e6f8a0b2c4d6e8f0ab".to_string();
        let pathname = temper_substrate::blob_store::blob_pathname(&hash);
        let store = InMemoryBlobStore::default().with_object(pathname.clone());
        let emitter =
            writes::resolve_emitter(&pool, ProfileId::from(profile), Surface::ApiHttp.marker())
                .await
                .expect("emitter resolves");
        writes::commit_blob(
            &pool,
            &store,
            writes::CommitBlobParams {
                id: temper_core::types::ids::BlobId::from(Uuid::now_v7()),
                home: temper_substrate::payloads::AnchorRef::context(ContextId::from(context)),
                owner: ProfileId::from(profile),
                originator: None,
                content_hash: hash,
                content_type: "image/png".to_string(),
                content_bytes: 18,
                max_bytes: 10 * 1024 * 1024,
                allowlist: ["image/png".to_string()].as_slice(),
                emitter,
            },
        )
        .await
        .expect("blob commits")
    };

    // The map IS the edge's source (the legacy cogmap-sourced shape); minted substrate-direct
    // because no door asserts this shape anymore, exactly as the fold-gate witness does.
    let emitter =
        writes::resolve_emitter(&pool, ProfileId::from(profile), Surface::ApiHttp.marker())
            .await
            .expect("emitter resolves");
    let edge = temper_substrate::writes::assert_anchored_edge_with(
        &pool,
        temper_substrate::writes::AssertAnchoredEdgeParams {
            source: temper_substrate::payloads::AnchorRef {
                table: temper_substrate::payloads::AnchorTable::Cogmaps,
                id: cogmap,
            },
            target: temper_substrate::payloads::AnchorRef::blob(blob),
            kind: temper_substrate::affinity::EdgeKind::LeadsTo,
            polarity: temper_substrate::payloads::EdgePolarity::Forward,
            label: Some("derivation_source"),
            weight: 1.0,
            home: temper_substrate::events::EdgeHome::Context(ContextId::from(context)),
            emitter,
        },
        temper_substrate::events::EventContext::default(),
    )
    .await
    .expect("the cogmap-sourced edge asserts");
    let edge = edge.uuid();

    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let refused = backend
        .set_facet(anchored_cmd(
            edge,
            anchored_values("source", cogmap, Uuid::now_v7()),
        ))
        .await
        .expect_err("a cogmap-side anchor must be refused");

    assert!(
        matches!(refused, TemperError::BadRequest(_)),
        "the refusal names the side, as a bad request: {refused:?}"
    );
    assert_eq!(anchored_row_count(&pool, edge).await, 0, "nothing written");
}

/// A blob side carries no blocks, so anchoring it is refused even when the address's resource
/// half names the blob correctly.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_blob_side_anchor_is_refused(pool: PgPool) {
    use temper_core::types::blob::{BlobRelationAssertRequest, BlobRelationDirection};
    use temper_core::types::ids::BlobId;
    use temper_substrate::blob_store::InMemoryBlobStore;
    use temper_substrate::writes;

    let email = format!("anchored-blob-{}@example.com", Uuid::new_v4());
    let (profile, context) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let resource = Uuid::from(
        DbBackend::new(pool.clone(), ProfileId::from(profile))
            .create_resource(create_cmd(context, "peer"))
            .await
            .expect("create the relate peer")
            .value
            .id,
    );

    let blob: BlobId = {
        let hash = "8f2b0d4c1a7e5f83b6c9d0e2f4a6b8c0d2e4f6a8b0c2d4e6f8a0b2c4d6e8f0ab".to_string();
        let pathname = temper_substrate::blob_store::blob_pathname(&hash);
        let store = InMemoryBlobStore::default().with_object(pathname.clone());
        let emitter =
            writes::resolve_emitter(&pool, ProfileId::from(profile), Surface::ApiHttp.marker())
                .await
                .expect("emitter resolves");
        writes::commit_blob(
            &pool,
            &store,
            writes::CommitBlobParams {
                id: temper_core::types::ids::BlobId::from(Uuid::now_v7()),
                home: temper_substrate::payloads::AnchorRef::context(ContextId::from(context)),
                owner: ProfileId::from(profile),
                originator: None,
                content_hash: hash,
                content_type: "image/png".to_string(),
                content_bytes: 18,
                max_bytes: 10 * 1024 * 1024,
                allowlist: ["image/png".to_string()].as_slice(),
                emitter,
            },
        )
        .await
        .expect("blob commits")
    };

    // resource → blob, the production-real blob-ended shape (relate's blob_as_target act).
    let ack = temper_services::services::blob_service::relate_blob(
        &pool,
        ProfileId::from(profile),
        blob,
        &BlobRelationAssertRequest {
            direction: BlobRelationDirection::BlobAsTarget,
            peer_table: "kb_resources".to_string(),
            peer_id: resource,
            edge_kind: graph::EdgeKind::LeadsTo,
            polarity: graph::Polarity::Forward,
            label: "derivation_source".to_string(),
            weight: 1.0,
            act: Default::default(),
        },
        Default::default(),
        Surface::ApiHttp,
    )
    .await
    .expect("relate creates the blob-endpoint edge");
    let edge = ack.edge_handle;

    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let refused = backend
        .set_facet(anchored_cmd(
            edge,
            anchored_values("target", Uuid::from(blob), Uuid::now_v7()),
        ))
        .await
        .expect_err("a blob-side anchor must be refused");

    assert!(
        matches!(refused, TemperError::BadRequest(_)),
        "the refusal names the side, as a bad request: {refused:?}"
    );
    assert_eq!(anchored_row_count(&pool, edge).await, 0, "nothing written");
}

/// The address's resource half must BE the named endpoint — the target's id under
/// `endpoint: "source"` (and the reverse) is a mis-anchored qualification, refused.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_address_whose_resource_half_is_not_the_named_endpoint_is_refused(pool: PgPool) {
    let (backend, source, target, edge) = edge_fixture(&pool).await;

    for (endpoint, half) in [("source", target), ("target", source)] {
        let refused = backend
            .set_facet(anchored_cmd(
                edge,
                anchored_values(endpoint, half, Uuid::now_v7()),
            ))
            .await
            .expect_err("the mismatched half must be refused");
        assert!(
            matches!(refused, TemperError::BadRequest(_)),
            "{endpoint}-side anchor naming the other endpoint must refuse: {refused:?}"
        );
    }

    assert_eq!(anchored_row_count(&pool, edge).await, 0, "nothing written");
}

// ─── the keyed action's owner and key contracts ──────────────────────────────

/// The keyed write is EDGE-owned: a resource-owner write carrying a property key is refused
/// even when the value would pass anchored-at's shape.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_keyed_write_on_a_resource_owner_is_refused(pool: PgPool) {
    let (backend, source, _target, _edge) = edge_fixture(&pool).await;

    let refused = backend
        .set_facet(SetFacet {
            owner: PropertyOwner::resource(ResourceId::from(source)),
            property_key: Some(ANCHORED_AT_PROPERTY_KEY.to_string()),
            values: anchored_values("source", source, Uuid::now_v7()),
            weight: 1.0,
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect_err("a keyed write on a resource owner must be refused");

    assert!(
        matches!(refused, TemperError::BadRequest(_)),
        "the refusal names the owner contract: {refused:?}"
    );
}

/// The clustering `facet` key rides the facet verb (one row per inner key); a keyed single-row
/// write under it would land the whole-object pre-grain shape the projector layer retired.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_keyed_write_cannot_ride_the_clustering_facet_key(pool: PgPool) {
    let (backend, _source, _target, edge) = edge_fixture(&pool).await;

    let refused = backend
        .set_facet(SetFacet {
            owner: PropertyOwner::edge(EdgeId::from(edge)),
            property_key: Some("facet".to_string()),
            values: serde_json::json!({"status": "open"}),
            weight: 1.0,
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect_err("a keyed write under 'facet' must be refused");

    assert!(
        matches!(refused, TemperError::BadRequest(_)),
        "the refusal names the key's owner verb: {refused:?}"
    );

    let any_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties WHERE owner_table = 'kb_edges' AND owner_id = $1",
    )
    .bind(edge)
    .fetch_one(&pool)
    .await
    .expect("row count for the edge");
    assert_eq!(any_rows, 0, "nothing written under any key");
}

/// The keyed write admits exactly the declared key: a key-carrying edge write under any
/// OTHER key is refused — the vocabulary is one key (`anchored-at`), and an arbitrary
/// edge-owned row would mint vocabulary the read side never declared.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_keyed_write_under_any_other_key_is_refused(pool: PgPool) {
    use temper_core::types::facet_requests::ANCHORED_AT_PROPERTY_KEY as ANCHORED;

    let (backend, source, _target, edge) = edge_fixture(&pool).await;

    for key in ["anchored at", "anchored_at", "span", "ANCHORED-AT"] {
        let refused = backend
            .set_facet(SetFacet {
                owner: PropertyOwner::edge(EdgeId::from(edge)),
                property_key: Some(key.to_string()),
                values: serde_json::json!({"status": "open"}),
                weight: 1.0,
                act: Default::default(),
                origin: Surface::ApiHttp,
            })
            .await
            .expect_err(&format!(
                "key {key:?} must be refused — the vocabulary is one key"
            ));
        assert!(
            matches!(refused, TemperError::BadRequest(_)),
            "key {key:?} must be refused: {refused:?}"
        );
    }

    // The one admitted key still lands — the vocabulary is closed, not the write.
    let landed = backend
        .set_facet(SetFacet {
            owner: PropertyOwner::edge(EdgeId::from(edge)),
            property_key: Some(ANCHORED.to_string()),
            values: serde_json::json!({
                "endpoint": "source",
                "address": format!("{source}#{}", Uuid::now_v7()),
            }),
            weight: 1.0,
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("the declared key still writes");
    assert!(!landed.value.is_empty(), "the keyed write lands its row id");
}
