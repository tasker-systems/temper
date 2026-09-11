//! Witnesses for the edge facets read's resolution + verdict — the anchored-at row-grain
//! read. Each live `anchored-at` row's address resolves through the block read's three-state
//! contract: `live` (the verdict computed against the block's live, uncorrected attribution —
//! corrected rows excluded, carried rows corroborating), `folded` (the gated disposition
//! envelope, null verdict), `absent` (null verdict, stated identically for no-such-block and
//! unreadable-home), and every non-anchored-at row renders both fields null, never absent.
//!
//! Attribution rides the real body-update path (`BodyUpdate.sources` — the same writer the
//! content PATCH serves); corrected and carried rows are minted substrate-direct, exactly the
//! rows the correction and redistribution writers produce; the fold rides the real
//! whole-body-replace geometry so the envelope carries a genuine `resource_reblocked` map.
//! The divergent witness is the named bite: nothing is rewritten on either side, and the
//! disagreement renders by name.
#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::facet_requests::{
    AnchorAddressResolution, AnchorVerdict, EdgeFacetRow, ANCHORED_AT_PROPERTY_KEY,
};
use temper_core::types::graph;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, EdgeId, ProfileId, ResourceId};
use temper_core::types::property_owner::PropertyOwner;
use temper_core::types::provenance::{BlockFoldDisposition, ProvenanceSource};
use temper_services::backend::DbBackend;
use temper_services::services::edge_service::list_edge_facets;
use temper_workflow::operations::{
    AssertRelationship, Backend, BodyUpdate, CreateResource, SetFacet, Surface, UpdateResource,
};
use temper_workflow::types::managed_meta::ManagedMeta;

// ─── fixtures ────────────────────────────────────────────────────────────────

fn create_cmd(context: Uuid, slug: &str) -> CreateResource {
    CreateResource {
        idempotency_key: None,
        slug: slug.to_string(),
        doctype: "research".to_string(),
        home: HomeAnchor::Context(ContextId::from(context)),
        title: format!("anchored-at resolution {slug}"),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        origin_uri: Some(format!("test://anchor-res-{slug}-{}", Uuid::new_v4())),
        chunks_packed: None,
        content_hash: None,
        goal: None,
        act: Default::default(),
        origin: Surface::ApiHttp,
    }
}

/// A profile with one context, three resources, and the `derived_from` edge source → target —
/// `third` is the unrelated block the divergent/corrected witnesses attribute instead of the
/// peer. The edge shape matches the write witnesses' (`leads_to` carrying the label; the
/// direction declarations key on the label, not the kind-shape).
async fn edge_fixture(pool: &PgPool) -> (DbBackend, Uuid, Uuid, Uuid, Uuid, Uuid) {
    let email = format!("anchor-res-{}@example.com", Uuid::new_v4());
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
    let third = Uuid::from(
        backend
            .create_resource(create_cmd(context, "third"))
            .await
            .expect("create third")
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
    (backend, profile, source, target, third, Uuid::from(edge))
}

/// The body update whose content path the content PATCH also serves; `sources` is the
/// block-grain attribution the create/update writer attaches.
async fn put_body(
    backend: &DbBackend,
    resource: Uuid,
    content: &str,
    sources: Vec<ProvenanceSource>,
) {
    backend
        .update_resource(UpdateResource {
            resource: ResourceId::from(resource),
            title: None,
            slug: None,
            body: Some(BodyUpdate {
                content: content.to_string(),
                content_hash: None,
                chunks_packed: None,
                sources,
                content_block: None,
            }),
            managed_meta: None,
            open_meta: None,
            open_meta_add: None,
            goal: None,
            move_to: None,
            context_ref: None,
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("body update lands");
}

/// A one-heading body: exactly one live block, so `live_blocks(...)[0]` is total.
const BODY_ONE: &str = "# Only\n\nThe only section's prose.\n";

/// The two-section body and its filler-append rewrite (cribbed from
/// `block_read_handler_test.rs`): the rewrite changes the alpha bytes — so the alpha
/// incumbent folds and its section re-creates — while beta survives byte-identical and is
/// kept. The real whole-body-replace geometry, not a synthetic fold.
const BODY_A_B: &str = "# Alpha\n\nAlpha body paragraph.\n## Beta\n\nBeta body paragraph.\n";

fn body_a_with_filler() -> String {
    let filler = "Brand new prose. ".repeat(120);
    format!("# Alpha\n\nAlpha body paragraph.\n\n{filler}\n## Beta\n\nBeta body paragraph.\n")
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

async fn anchor(backend: &DbBackend, edge: Uuid, endpoint: &str, resource: Uuid, block: Uuid) {
    backend
        .set_facet(SetFacet {
            owner: PropertyOwner::edge(EdgeId::from(edge)),
            property_key: Some(ANCHORED_AT_PROPERTY_KEY.to_string()),
            values: json!({
                "endpoint": endpoint,
                "address": format!("{resource}#{block}"),
            }),
            weight: 1.0,
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("the anchor lands");
}

async fn read_facets(pool: &PgPool, profile: Uuid, edge: Uuid) -> Vec<EdgeFacetRow> {
    list_edge_facets(pool, profile, edge)
        .await
        .expect("the facets read answers")
}

fn anchored_row(rows: &[EdgeFacetRow]) -> &EdgeFacetRow {
    rows.iter()
        .find(|r| r.property_key == ANCHORED_AT_PROPERTY_KEY)
        .expect("an anchored-at row is on the read")
}

/// Mint a corrected or carried attribution row substrate-direct — exactly the rows the
/// correction and redistribution writers produce (a `resource`-kind row on the block,
/// attributed through some existing ledger event).
async fn insert_provenance_row(
    pool: &PgPool,
    block: Uuid,
    source_id: Uuid,
    is_carried: bool,
    is_corrected: bool,
) {
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events ORDER BY created LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("an event exists to attribute through");
    sqlx::query(
        "INSERT INTO kb_block_provenance \
             (block_id, source_kind, source_id, contributed_by_event_id, accretion_seq, \
              is_carried, is_corrected) \
         VALUES ($1, 'resource', $2, $3, 99, $4, $5)",
    )
    .bind(block)
    .bind(source_id)
    .bind(event)
    .bind(is_carried)
    .bind(is_corrected)
    .execute(pool)
    .await
    .expect("the provenance row inserts");
}

async fn stored_anchor_value(pool: &PgPool, edge: Uuid) -> Value {
    let (value,): (Value,) = sqlx::query_as(
        "SELECT property_value FROM kb_properties \
          WHERE owner_table = 'kb_edges' AND owner_id = $1 \
            AND property_key = 'anchored-at' AND NOT is_folded",
    )
    .bind(edge)
    .fetch_one(pool)
    .await
    .expect("the stored anchored-at row");
    value
}

// ─── the verdict arm: the named disagreement state ───────────────────────────

/// THE named disagreement: the anchored block's live attribution names a third resource, the
/// edge's peer is elsewhere — the read renders `divergent` BY NAME and rewrites nothing:
/// neither the qualification's stored value nor a single attribution row moves.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_divergent_anchor_renders_divergent_and_rewrites_nothing(pool: PgPool) {
    let (backend, profile, source, target, third, edge) = edge_fixture(&pool).await;

    put_body(
        &backend,
        source,
        BODY_ONE,
        vec![ProvenanceSource::Resource(third)],
    )
    .await;
    let block = live_blocks(&pool, source).await[0];
    anchor(&backend, edge, "source", source, block).await;

    let value_before = stored_anchor_value(&pool, edge).await;
    let attribution_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_block_provenance WHERE block_id = $1")
            .bind(block)
            .fetch_one(&pool)
            .await
            .expect("attribution count");

    let rows = read_facets(&pool, profile, edge).await;
    let row = anchored_row(&rows);
    assert_eq!(
        row.address_resolution,
        Some(AnchorAddressResolution::Live),
        "the anchored block resolves live: {row:?}"
    );
    assert_eq!(
        row.verdict,
        Some(AnchorVerdict::Divergent),
        "the disagreement renders by name — the peer {target} is not among the rows: {row:?}"
    );

    assert_eq!(
        stored_anchor_value(&pool, edge).await,
        value_before,
        "the read rewrites no qualification"
    );
    let attribution_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_block_provenance WHERE block_id = $1")
            .bind(block)
            .fetch_one(&pool)
            .await
            .expect("attribution count");
    assert_eq!(
        attribution_after, attribution_before,
        "the read rewrites no attribution"
    );
}

/// The verdict runs against LIVE, uncorrected attribution only: a corrected (retracted) row
/// naming the peer is scarred testimony — certifying agreement with it would be silent
/// reconciliation inverted, so the read stays `divergent` on the uncorrected rows alone.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_corrected_attribution_row_never_corroborates(pool: PgPool) {
    let (backend, profile, source, target, third, edge) = edge_fixture(&pool).await;

    put_body(
        &backend,
        source,
        BODY_ONE,
        vec![ProvenanceSource::Resource(third)],
    )
    .await;
    let block = live_blocks(&pool, source).await[0];
    // The retracted testimony: it once named the peer, then was corrected away.
    insert_provenance_row(&pool, block, target, false, true).await;
    anchor(&backend, edge, "source", source, block).await;

    let rows = read_facets(&pool, profile, edge).await;
    let row = anchored_row(&rows);
    assert_eq!(row.address_resolution, Some(AnchorAddressResolution::Live));
    assert_eq!(
        row.verdict,
        Some(AnchorVerdict::Divergent),
        "the corrected row must not corroborate; only the uncorrected naming counts: {row:?}"
    );
}

/// Carried rows corroborate like direct ones: `is_carried` marks partial coverage, not false
/// testimony. Here the ONLY peer-naming row is a carried one — a read that discounted it
/// would render `divergent`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_carried_attribution_row_corroborates(pool: PgPool) {
    let (backend, profile, source, target, third, edge) = edge_fixture(&pool).await;

    put_body(
        &backend,
        source,
        BODY_ONE,
        vec![ProvenanceSource::Resource(third)],
    )
    .await;
    let block = live_blocks(&pool, source).await[0];
    insert_provenance_row(&pool, block, target, true, false).await;
    anchor(&backend, edge, "source", source, block).await;

    let rows = read_facets(&pool, profile, edge).await;
    let row = anchored_row(&rows);
    assert_eq!(row.address_resolution, Some(AnchorAddressResolution::Live));
    assert_eq!(
        row.verdict,
        Some(AnchorVerdict::Corroborated),
        "the carried row's naming corroborates: {row:?}"
    );
}

/// A block with no live attribution rows at all: `unattributed` — a named absence, never
/// readable as agreement or as `divergent`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unattributed_block_renders_unattributed(pool: PgPool) {
    let (backend, profile, source, _target, _third, edge) = edge_fixture(&pool).await;

    put_body(&backend, source, BODY_ONE, vec![]).await;
    let block = live_blocks(&pool, source).await[0];
    anchor(&backend, edge, "source", source, block).await;

    let rows = read_facets(&pool, profile, edge).await;
    let row = anchored_row(&rows);
    assert_eq!(row.address_resolution, Some(AnchorAddressResolution::Live));
    assert_eq!(
        row.verdict,
        Some(AnchorVerdict::Unattributed),
        "an absence of testimony is its own named state: {row:?}"
    );
}

// ─── direction: resolution-only renderings ───────────────────────────────────

/// The cross-side row is legitimate data and renders RESOLUTION-ONLY: a `derived_from` edge
/// anchored target-side gets a null verdict even though the block resolves live — the
/// attribution direction cannot corroborate it, and computing one anyway would invert
/// semantics. Null, never a computed negative.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_cross_side_anchor_renders_resolution_only(pool: PgPool) {
    let (backend, profile, source, target, _third, edge) = edge_fixture(&pool).await;

    // The target's block names the source (its own directional story) — attribution exists
    // and would support a verdict if one were declared; none is.
    put_body(
        &backend,
        target,
        BODY_ONE,
        vec![ProvenanceSource::Resource(source)],
    )
    .await;
    let block = live_blocks(&pool, target).await[0];
    anchor(&backend, edge, "target", target, block).await;

    let rows = read_facets(&pool, profile, edge).await;
    let row = anchored_row(&rows);
    assert_eq!(row.address_resolution, Some(AnchorAddressResolution::Live));
    assert_eq!(
        row.verdict, None,
        "cross-side renders resolution-only, never a computed negative: {row:?}"
    );
}

/// The OPEN declaration: an `express` edge carrying a non-`derived_from` label renders
/// resolution-only even when the block resolves live AND its attribution names the peer —
/// a direction declared under the wrong reading would aim corrections at true rows, so
/// until one is declared the verdict stays null.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_undeclared_direction_renders_resolution_only(pool: PgPool) {
    let (backend, profile, source, target, _third, _edge) = edge_fixture(&pool).await;

    let edge = backend
        .assert_relationship(AssertRelationship {
            source: ResourceId::from(source),
            target: ResourceId::from(target),
            target_table: Default::default(),
            edge_kind: graph::EdgeKind::Express,
            polarity: graph::Polarity::Forward,
            label: "derivation_source".to_string(),
            weight: 1.0,
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("assert the express edge")
        .value;
    let edge = Uuid::from(edge);

    put_body(
        &backend,
        source,
        BODY_ONE,
        vec![ProvenanceSource::Resource(target)],
    )
    .await;
    let block = live_blocks(&pool, source).await[0];
    anchor(&backend, edge, "source", source, block).await;

    let rows = read_facets(&pool, profile, edge).await;
    let row = anchored_row(&rows);
    assert_eq!(row.address_resolution, Some(AnchorAddressResolution::Live));
    assert_eq!(
        row.verdict, None,
        "attribution non-empty and peer-naming, yet no declared direction — null, never a \
         computed negative: {row:?}"
    );
}

// ─── the resolution arm: folded and absent ───────────────────────────────────

/// A fold never re-points, retires, or rewrites the qualification: the anchored block folds
/// under the real whole-body-replace geometry and the row renders `folded` with the block
/// read's gated envelope — the re-created section named as a located successor — and a null
/// verdict (the comparison the clause names is live-vs-live).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_folded_anchor_renders_the_gated_envelope_with_a_null_verdict(pool: PgPool) {
    let (backend, profile, source, _target, _third, edge) = edge_fixture(&pool).await;

    put_body(&backend, source, BODY_A_B, vec![]).await;
    let folded_block = live_blocks(&pool, source).await[0];
    anchor(&backend, edge, "source", source, folded_block).await;

    put_body(&backend, source, &body_a_with_filler(), vec![]).await;
    let after = live_blocks(&pool, source).await;
    assert!(
        !after.contains(&folded_block),
        "fixture: the anchored incumbent folded under the rewrite"
    );

    let rows = read_facets(&pool, profile, edge).await;
    let row = anchored_row(&rows);
    let Some(AnchorAddressResolution::Folded {
        folded_by_event_id,
        disposition,
    }) = row.address_resolution.clone()
    else {
        panic!("a folded anchor must render the folded envelope, got {row:?}")
    };
    assert_eq!(row.verdict, None, "a folded anchor states no verdict");
    let BlockFoldDisposition::Located { absorbers, carried } = disposition else {
        panic!("the re-created section is a located successor, got {disposition:?}")
    };
    assert!(
        absorbers.len() == 1 && carried.is_empty(),
        "the gated envelope names the re-created section: {absorbers:?}"
    );
    assert!(
        after.contains(&absorbers[0].block_id),
        "the named successor is a live block the caller can read"
    );
    let event_type: String = sqlx::query_scalar(
        "SELECT t.name FROM kb_events e \
         JOIN kb_event_types t ON t.id = e.event_type_id WHERE e.id = $1",
    )
    .bind(folded_by_event_id)
    .fetch_one(&pool)
    .await
    .expect("the fold event");
    assert_eq!(
        event_type, "resource_reblocked",
        "the envelope walked the row's own fold pointer to the reblocked event"
    );
}

/// An address that names no block anywhere — fabricated at the write (validation probes
/// structure only, never existence) — renders `absent` with a null verdict: an address you
/// cannot read tells you nothing about whether it would corroborate, and `absent` never
/// reads as `divergent`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_absent_address_renders_absent_with_a_null_verdict(pool: PgPool) {
    let (backend, profile, source, _target, _third, edge) = edge_fixture(&pool).await;

    anchor(&backend, edge, "source", source, Uuid::now_v7()).await;

    let rows = read_facets(&pool, profile, edge).await;
    let row = anchored_row(&rows);
    assert_eq!(
        row.address_resolution,
        Some(AnchorAddressResolution::Absent),
        "the refused face of the block read, stated by name: {row:?}"
    );
    assert_eq!(row.verdict, None, "absent states no verdict");
}

// ─── the wire: both fields on every row, present even when null ──────────────

/// Over HTTP — the shape every skin ships — a plain facet row states BOTH new fields as
/// present-and-null (the sibling-author-fields rendering, never absent), while the anchored
/// row carries its resolution envelope and verdict as data.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_wire_states_both_fields_on_every_row(pool: PgPool) {
    let (backend, profile, source, target, _third, edge) = edge_fixture(&pool).await;

    let app = common::setup_test_app(pool.clone()).await;
    let email = format!("anchor-res-wire-{}@example.com", Uuid::new_v4());
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    put_body(
        &backend,
        source,
        BODY_ONE,
        vec![ProvenanceSource::Resource(target)],
    )
    .await;
    let block = live_blocks(&pool, source).await[0];

    // One plain facet row and one anchored-at row on the same edge.
    for body in [
        json!({ "values": { "status": "open" } }),
        json!({
            "values": {
                "endpoint": "source",
                "address": format!("{source}#{block}"),
            },
            "property_key": ANCHORED_AT_PROPERTY_KEY,
        }),
    ] {
        let resp = app
            .client
            .post(app.url(&format!("/api/relationships/{edge}/facets")))
            .header("Authorization", format!("Bearer {token}"))
            .json(&body)
            .send()
            .await
            .expect("facet POST failed");
        assert_eq!(
            resp.status().as_u16(),
            200,
            "fixture: the facet write lands; got {}",
            resp.text().await.unwrap_or_default()
        );
    }

    let resp = app
        .client
        .get(app.url(&format!("/api/relationships/{edge}/facets")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("facets GET failed");
    assert_eq!(resp.status().as_u16(), 200, "the read answers");
    let body: Value = resp.json().await.expect("facets JSON");
    let facets = body["facets"].as_array().expect("facets array");
    assert_eq!(facets.len(), 2, "one plain row and one anchored row");

    let plain = facets
        .iter()
        .find(|f| f["property_key"].as_str() == Some("facet"))
        .expect("the plain facet row");
    assert!(
        plain.get("address_resolution").is_some(),
        "present, never absent: {plain}"
    );
    assert_eq!(plain["address_resolution"], Value::Null);
    assert!(
        plain.get("verdict").is_some(),
        "present, never absent: {plain}"
    );
    assert_eq!(plain["verdict"], Value::Null);

    let anchored = facets
        .iter()
        .find(|f| f["property_key"].as_str() == Some(ANCHORED_AT_PROPERTY_KEY))
        .expect("the anchored-at row");
    assert_eq!(
        anchored["address_resolution"],
        json!({ "state": "live" }),
        "the resolution states the block read's own state name: {anchored}"
    );
    assert_eq!(
        anchored["verdict"], "corroborated",
        "the block's attribution names the peer: {anchored}"
    );
}
