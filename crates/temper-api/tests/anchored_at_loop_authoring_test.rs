//! Witnesses for the authoring loop's coarse anchors (the edge-span spec §6) — the
//! `--sources-as-edges` phase of `temper resource create`
//! (`temper-cli/src/commands/resource.rs`, `write_source_edge_anchors`). Temper is
//! cloud-only, so the CLI loop's server-visible behavior IS this chain, driven here in the
//! loop's own order: a create whose body carries the sources (attribution rows land, the
//! create's re-block marks them), ONE gated `resource_block_provenance_select` (the loop's
//! follow-up read; the create response itself carries no block data), one keyed `anchored-at`
//! write per (endpoint=source, block) through the edge facet surface, then the rows read back
//! through `list_edge_facets`.
//!
//! The edge shape mirrors the loop's (`EdgeType::DerivedFrom.legacy_mapping()` =
//! leads_to/inverse carrying the label). The anchor-set expectation is computed from the same
//! read the loop consumes — differential: this file states the invariant, while the filter's
//! own unit tests live beside `anchor_blocks_for_target` in temper-cli. Carried rows are
//! minted substrate-direct, exactly as `anchored_at_resolution_test.rs` mints them.
#![cfg(feature = "test-db")]

mod common;

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::types::facet_requests::{
    AnchorAddressResolution, AnchorVerdict, EdgeFacetRow, ANCHORED_AT_PROPERTY_KEY,
};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, EdgeId, ProfileId, PropertyId, ResourceId};
use temper_core::types::property_owner::PropertyOwner;
use temper_core::types::provenance::{BlockProvenanceRow, ProvenanceSource};
use temper_services::backend::substrate_read::resource_block_provenance_select;
use temper_services::backend::DbBackend;
use temper_services::services::edge_service::list_edge_facets;
use temper_workflow::operations::{
    AssertRelationship, Backend, BodyUpdate, CreateResource, FoldRelationship, SetFacet, Surface,
};
use temper_workflow::types::managed_meta::ManagedMeta;

// ─── fixtures ────────────────────────────────────────────────────────────────

/// The two-section body: a create carrying it lands policy-partitioned into two blocks (the
/// partition witness is `write_path_policy_test.rs` w1 — exactly one re-block act rides the
/// create transaction). The one-section body is the single-block control.
const BODY_TWO_SECTIONS: &str = "# Alpha\n\nAlpha prose.\n\n# Beta\n\nBeta prose.\n";
const BODY_ONE_SECTION: &str = "Just some plain prose with no headings at all.\n";

async fn create_peer(backend: &DbBackend, context: Uuid, name: &str) -> Uuid {
    Uuid::from(
        backend
            .create_resource(CreateResource {
                idempotency_key: None,
                slug: format!("loop-peer-{name}"),
                doctype: "research".to_string(),
                home: HomeAnchor::Context(ContextId::from(context)),
                title: format!("loop-authored peer {name}"),
                body: None,
                managed_meta: ManagedMeta::default(),
                open_meta: None,
                origin_uri: Some(format!("test://loop-peer-{name}-{}", Uuid::new_v4())),
                chunks_packed: None,
                content_hash: None,
                goal: None,
                act: Default::default(),
                origin: Surface::ApiHttp,
            })
            .await
            .expect("create the peer")
            .value
            .id,
    )
}

/// The loop's created resource: a body-carrying create whose `sources` are resource-valued —
/// the wire shape the CLI create assembles when `--sources` is given.
async fn loop_created(backend: &DbBackend, context: Uuid, content: &str, cite: &[Uuid]) -> Uuid {
    Uuid::from(
        backend
            .create_resource(CreateResource {
                idempotency_key: None,
                slug: "loop-derived".to_string(),
                doctype: "research".to_string(),
                home: HomeAnchor::Context(ContextId::from(context)),
                title: "loop-authored derived".to_string(),
                body: Some(BodyUpdate {
                    content: content.to_string(),
                    content_hash: None,
                    chunks_packed: None,
                    sources: cite
                        .iter()
                        .map(|id| ProvenanceSource::Resource(*id))
                        .collect(),
                    content_block: None,
                }),
                managed_meta: ManagedMeta::default(),
                open_meta: None,
                origin_uri: Some(format!("test://loop-derived-{}", Uuid::new_v4())),
                chunks_packed: None,
                content_hash: None,
                goal: None,
                act: Default::default(),
                origin: Surface::ApiHttp,
            })
            .await
            .expect("create the loop's resource")
            .value
            .id,
    )
}

/// The edge in the loop's own shape — `EdgeType::DerivedFrom.legacy_mapping()`.
async fn assert_edge_like_the_loop(backend: &DbBackend, source: Uuid, target: Uuid) -> Uuid {
    let (edge_kind, polarity, label) =
        temper_workflow::types::graph::EdgeType::DerivedFrom.legacy_mapping();
    Uuid::from(
        backend
            .assert_relationship(AssertRelationship {
                source: ResourceId::from(source),
                target: ResourceId::from(target),
                target_table: Default::default(),
                edge_kind,
                polarity,
                label: label.to_string(),
                weight: 1.0,
                act: Default::default(),
                origin: Surface::ApiHttp,
            })
            .await
            .expect("assert the edge")
            .value,
    )
}

/// The full stage: a profile with one context, one cited peer, and the loop's created
/// resource citing it, plus the asserted `derived_from` edge created → peer.
async fn loop_fixture(pool: &PgPool, content: &str) -> (DbBackend, Uuid, Uuid, Uuid, Uuid) {
    let email = format!("loop-anchor-{}@example.com", Uuid::new_v4());
    let (profile, context) = common::fixtures::create_test_profile_with_context(pool, &email).await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let peer = create_peer(&backend, context, "cited").await;
    let created = loop_created(&backend, context, content, &[peer]).await;
    let edge = assert_edge_like_the_loop(&backend, created, peer).await;
    (backend, profile, created, peer, edge)
}

/// The loop's follow-up read — ONE gated call, exactly what `write_source_edge_anchors` makes
/// (the create response carries no block data, so this read IS how the loop learns the blocks).
async fn loop_read(pool: &PgPool, profile: Uuid, resource: Uuid) -> Vec<BlockProvenanceRow> {
    resource_block_provenance_select(pool, ProfileId::from(profile), resource)
        .await
        .expect("the gated provenance read answers")
}

/// The loop's anchor set for one target, computed from that read the way
/// `anchor_blocks_for_target` filters: `resource`-kind rows naming the target, `is_carried`
/// included, one block per address.
fn loop_anchor_set(rows: &[BlockProvenanceRow], target: Uuid) -> Vec<Uuid> {
    let mut blocks = Vec::new();
    for row in rows {
        if row.source_kind == "resource"
            && row.source_id == target
            && !blocks.contains(&row.block_id)
        {
            blocks.push(row.block_id);
        }
    }
    blocks
}

/// The loop's keyed write — the request shape `write_source_edge_anchors` builds. Returns the
/// acked property ids (the write's insert-if-not-live ack arm).
async fn loop_anchor(
    backend: &DbBackend,
    edge: Uuid,
    created: Uuid,
    block: Uuid,
) -> Vec<PropertyId> {
    backend
        .set_facet(SetFacet {
            owner: PropertyOwner::edge(EdgeId::from(edge)),
            property_key: Some(ANCHORED_AT_PROPERTY_KEY.to_string()),
            values: serde_json::json!({
                "endpoint": "source",
                "address": format!("{created}#{block}"),
            }),
            weight: 1.0,
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("the anchor lands")
        .value
}

/// (id, seq) of the resource's LIVE blocks — read straight off the pool so the fixture's
/// partition shape is verified against the database, not the read's word for it.
async fn live_blocks(pool: &PgPool, resource: Uuid) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded ORDER BY seq, id",
    )
    .bind(resource)
    .fetch_all(pool)
    .await
    .expect("live blocks")
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

/// Every stored live anchored-at value on the edge — the values verbatim.
async fn stored_anchored_values(pool: &PgPool, edge: Uuid) -> Vec<serde_json::Value> {
    sqlx::query_scalar(
        "SELECT property_value FROM kb_properties \
          WHERE owner_table = 'kb_edges' AND owner_id = $1 \
            AND property_key = 'anchored-at' AND NOT is_folded",
    )
    .bind(edge)
    .fetch_all(pool)
    .await
    .expect("stored anchored-at rows")
}

async fn read_facets(pool: &PgPool, profile: Uuid, edge: Uuid) -> Vec<EdgeFacetRow> {
    list_edge_facets(pool, profile, edge)
        .await
        .expect("the facets read answers")
}

/// Mint a carried attribution row substrate-direct — exactly the row the redistribution
/// writer produces (a `resource`-kind row on the block, attributed through some existing
/// ledger event).
async fn insert_carried_provenance_row(pool: &PgPool, block: Uuid, source_id: Uuid) {
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events ORDER BY created LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("an event exists to attribute through");
    sqlx::query(
        "INSERT INTO kb_block_provenance \
             (block_id, source_kind, source_id, contributed_by_event_id, accretion_seq, \
              is_carried, is_corrected) \
         VALUES ($1, 'resource', $2, $3, 99, true, false)",
    )
    .bind(block)
    .bind(source_id)
    .bind(event)
    .execute(pool)
    .await
    .expect("the carried provenance row inserts");
}

// ─── the anchor set lands ────────────────────────────────────────────────────

/// A `--sources-as-edges` create whose source's attribution covers N blocks lands N
/// anchored-at rows on the asserted edge — one per (endpoint=source, block), no cap, no
/// primary-block heuristic. The anchor set is exactly the attribution-carrying block set,
/// computed from the one gated follow-up read.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_partitioned_create_anchors_every_block_carrying_the_sources_attribution(pool: PgPool) {
    let (backend, profile, created, peer, edge) = loop_fixture(&pool, BODY_TWO_SECTIONS).await;

    let live = live_blocks(&pool, created).await;
    assert!(
        live.len() >= 2,
        "fixture: the create landed policy-partitioned; got {} blocks",
        live.len()
    );

    let rows = loop_read(&pool, profile, created).await;
    let expected = loop_anchor_set(&rows, peer);
    assert_eq!(
        expected.len(),
        live.len(),
        "fixture grounding: the citation's attribution covers every live block — the anchor \
         set is the whole partition, never a subset"
    );

    for block in &expected {
        loop_anchor(&backend, edge, created, *block).await;
    }

    let stored = stored_anchored_values(&pool, edge).await;
    assert_eq!(
        stored.len(),
        expected.len(),
        "one row per block, nothing more: {stored:?}"
    );
    for block in &expected {
        let address = format!("{created}#{block}");
        assert!(
            stored
                .iter()
                .any(|v| v["address"] == address && v["endpoint"] == "source"),
            "the source-side anchor at {address} landed verbatim; stored: {stored:?}"
        );
    }
}

/// D-B4's bite at the loop grain: an `is_carried` attribution row's block is anchored like a
/// direct one. Here the ONLY row naming the edge's target is a carried one — a loop that
/// skipped carried rows would compute an EMPTY anchor set and silently qualify less than the
/// edge asserts. The carried row corroborates on read-back, too: partial coverage is not
/// false testimony.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_carried_attribution_row_is_anchored_like_a_direct_one(pool: PgPool) {
    let email = format!("loop-carried-{}@example.com", Uuid::new_v4());
    let (profile, context) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let source = create_peer(&backend, context, "edge-target").await;
    let third = create_peer(&backend, context, "cited").await;
    // The citation names THIRD only; the edge's target's sole attribution is minted carried.
    let created = loop_created(&backend, context, BODY_TWO_SECTIONS, &[third]).await;
    let edge = assert_edge_like_the_loop(&backend, created, source).await;

    let live = live_blocks(&pool, created).await;
    insert_carried_provenance_row(&pool, live[1], source).await;

    let rows = loop_read(&pool, profile, created).await;
    let expected = loop_anchor_set(&rows, source);
    assert_eq!(
        expected,
        vec![live[1]],
        "the anchor set is exactly the carried row's block — nothing else names the target"
    );

    loop_anchor(&backend, edge, created, live[1]).await;
    assert_eq!(
        anchored_row_count(&pool, edge).await,
        1,
        "one address, one row"
    );

    let facets = read_facets(&pool, profile, edge).await;
    let row = facets
        .iter()
        .find(|r| r.property_key == ANCHORED_AT_PROPERTY_KEY)
        .expect("the anchored-at row is on the read");
    assert_eq!(row.address_resolution, Some(AnchorAddressResolution::Live));
    assert_eq!(
        row.verdict,
        Some(AnchorVerdict::Corroborated),
        "the carried row's naming corroborates: {row:?}"
    );
}

// ─── warn-not-fatal: the failure the loop's arm absorbs ──────────────────────

/// A failed anchor write refuses cleanly AFTER the create has committed — the exact state the
/// loop's warn-not-fatal arm absorbs (`write_source_edge_anchors` warns on this error and the
/// create stands). The fold-between-assert-and-anchor race is the natural failure:
/// `check_edge_mutable` refuses the stale edge as not-found, nothing is written, and the
/// committed resource remains.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_failed_anchor_write_after_the_commit_refuses_cleanly(pool: PgPool) {
    let (backend, _profile, created, _peer, edge) = loop_fixture(&pool, BODY_ONE_SECTION).await;
    let block = live_blocks(&pool, created).await.remove(0);

    backend
        .fold_relationship(FoldRelationship {
            edge_handle: EdgeId::from(edge),
            reason: Some("witness: the post-commit race".to_string()),
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("the edge folds after the create committed");

    let refused = backend
        .set_facet(SetFacet {
            owner: PropertyOwner::edge(EdgeId::from(edge)),
            property_key: Some(ANCHORED_AT_PROPERTY_KEY.to_string()),
            values: serde_json::json!({
                "endpoint": "source",
                "address": format!("{created}#{block}"),
            }),
            weight: 1.0,
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect_err("the stale edge must refuse the anchor, not write it");
    assert!(
        matches!(refused, TemperError::NotFound(_)),
        "the folded edge refuses as not-found — the arm's warning shape: {refused:?}"
    );

    assert_eq!(
        anchored_row_count(&pool, edge).await,
        0,
        "nothing written through the stale edge"
    );
    let resource_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_resources WHERE id = $1")
        .bind(created)
        .fetch_one(&pool)
        .await
        .expect("the created resource");
    assert_eq!(resource_rows, 1, "the committed create stands");
}

// ─── the retry converges ─────────────────────────────────────────────────────

/// A retried create converges instead of noising. The edge assert upserts on the active-edge
/// invariant, so the retry re-asserts the SAME edge; the loop's re-computed anchors — the same
/// addresses — ACK the existing rows: same property ids, no new rows, no new ledger events.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_retried_create_acks_its_anchors_instead_of_erroring(pool: PgPool) {
    let (backend, profile, created, peer, edge) = loop_fixture(&pool, BODY_TWO_SECTIONS).await;

    let expected = loop_anchor_set(&loop_read(&pool, profile, created).await, peer);
    let mut first_ids = Vec::new();
    for block in &expected {
        first_ids.extend(loop_anchor(&backend, edge, created, *block).await);
    }

    // The retry's edge assert: the active-edge upsert returns the SAME edge the anchors ride.
    let retried_edge = assert_edge_like_the_loop(&backend, created, peer).await;
    assert_eq!(
        retried_edge, edge,
        "the active-edge upsert hands back the same edge, never a twin"
    );

    let mut retried_ids = Vec::new();
    for block in &loop_anchor_set(&loop_read(&pool, profile, created).await, peer) {
        retried_ids.extend(loop_anchor(&backend, edge, created, *block).await);
    }
    assert_eq!(
        retried_ids, first_ids,
        "the ack returns the existing row ids, never fresh twins"
    );
    assert_eq!(
        anchored_row_count(&pool, edge).await,
        expected.len() as i64,
        "one address, one live row after the retry"
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
    assert_eq!(
        events,
        expected.len() as i64,
        "the ack arm appended no second ledger events"
    );
}

// ─── the loop authors what it knows ─────────────────────────────────────────

/// The clause's bite: the anchor set IS the attribution set, so every loop-authored row reads
/// back `live` with verdict `corroborated` — by construction, never by a verdict the loop
/// computed (the loop only writes rows; the read owns the verdict).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_loops_rows_read_back_live_and_corroborated(pool: PgPool) {
    let (backend, profile, created, peer, edge) = loop_fixture(&pool, BODY_TWO_SECTIONS).await;

    let expected = loop_anchor_set(&loop_read(&pool, profile, created).await, peer);
    for block in &expected {
        loop_anchor(&backend, edge, created, *block).await;
    }

    let facets = read_facets(&pool, profile, edge).await;
    let anchored: Vec<&EdgeFacetRow> = facets
        .iter()
        .filter(|r| r.property_key == ANCHORED_AT_PROPERTY_KEY)
        .collect();
    assert_eq!(
        anchored.len(),
        expected.len(),
        "every block the loop anchored is on the read"
    );
    for row in anchored {
        assert_eq!(
            row.address_resolution,
            Some(AnchorAddressResolution::Live),
            "the anchored block resolves live: {row:?}"
        );
        assert_eq!(
            row.verdict,
            Some(AnchorVerdict::Corroborated),
            "the anchored block's own attribution names the peer: {row:?}"
        );
    }
}
