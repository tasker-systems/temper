#![cfg(feature = "test-db")]
//! Witnesses for an edge's erasure trail and the ledger derivation over it
//! (20261201000050, `_edge_erasure_trail_scope` and `_edge_erasure_payload_redaction`).
//!
//! An edge's trail is its own events (`payload->>'edge_id'`), the property events it owns
//! (`payload->'owner'` naming `kb_edges`), and the re-assertions recorded before 20261117000030
//! whose minted `edge_id` never became a row (`_edge_phantom_reassertions`). The derivation runs
//! the loop `_resource_erasure_payload_redaction` runs, over that trail: an edge carries no remote
//! source, so its label, its facet marks and the authorship metadata of its events are what it
//! rewrites. Each world is built through the real write paths; only the historical re-assertion
//! is appended by SQL, as every version before 20261117000030 recorded one.

use std::collections::BTreeSet;

mod edge_erasure;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use temper_substrate::ids::EdgeId;

use edge_erasure::{asserting_event, authored, edge, facet, resource, world, World};

const LABEL: &str = "jane roe told us";
const REASONING: &str = "jane roe said so on the call";

/// A re-assertion as every version before 20261117000030 recorded one: the asserting event's
/// payload under a freshly minted `edge_id`, appended as written, then projected. The projector's
/// ON CONFLICT answers with the existing edge, so the event names an id with no row.
async fn historical_reassertion(pool: &PgPool, w: &World, e: EdgeId) -> Uuid {
    let asserted = asserting_event(pool, e).await;
    let payload: Value = sqlx::query_scalar(
        "SELECT jsonb_set(payload, '{edge_id}', to_jsonb($2::text)) FROM kb_events WHERE id = $1",
    )
    .bind(asserted)
    .bind(Uuid::now_v7())
    .fetch_one(pool)
    .await
    .expect("the re-assertion payload");
    let mut tx = pool.begin().await.unwrap();
    let ev: Uuid = sqlx::query_scalar(
        "SELECT _event_append('relationship_asserted', $1, $2->'home'->>'table', \
                              ($2->'home'->>'id')::uuid, $2)",
    )
    .bind(w.emitter)
    .bind(&payload)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let answered: Uuid = sqlx::query_scalar("SELECT _project_relationship_asserted($1, $2)")
        .bind(ev)
        .bind(&payload)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        answered,
        e.uuid(),
        "the projection reweights the existing edge"
    );
    let minted: Uuid = payload["edge_id"].as_str().unwrap().parse().unwrap();
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_edges WHERE id = $1")
        .bind(minted)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "the fixture must record a phantom");
    ev
}

struct Edged {
    edge: EdgeId,
    asserted: Uuid,
    facet: Uuid,
    phantom: Uuid,
    /// Every event of a second edge between other endpoints, with its own facet.
    other: BTreeSet<Uuid>,
}

async fn edged(pool: &PgPool) -> Edged {
    let w = world(pool).await;
    let a = resource(pool, &w, "a").await;
    let b = resource(pool, &w, "b").await;
    let c = resource(pool, &w, "c").await;

    let e = edge(pool, &w, a, b, LABEL, authored(REASONING)).await;
    let asserted = asserting_event(pool, e).await;
    let facet_ev = facet(
        pool,
        &w,
        e,
        json!({ "node_label": "jane roe", "status": "confided" }),
    )
    .await;
    let phantom = historical_reassertion(pool, &w, e).await;

    let o = edge(pool, &w, b, c, "another label", authored(REASONING)).await;
    let o_asserted = asserting_event(pool, o).await;
    let o_facet = facet(pool, &w, o, json!({ "node_label": "elsewhere" })).await;

    Edged {
        edge: e,
        asserted,
        facet: facet_ev,
        phantom,
        other: BTreeSet::from([o_asserted, o_facet]),
    }
}

/// FAILS IF the edge trail leaves out any of the three arms (its own event, the property events
/// it owns, its historical re-assertions), or reaches another edge's events.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_edge_trail_is_its_own_events_its_facets_and_its_phantom_reassertions(pool: PgPool) {
    let w = edged(&pool).await;

    let rows: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT event_id, event_type FROM _edge_erasure_trail_scope($1)")
            .bind(w.edge.uuid())
            .fetch_all(&pool)
            .await
            .expect("the edge trail");
    let trail: BTreeSet<Uuid> = rows.iter().map(|(id, _)| *id).collect();

    assert_eq!(rows.len(), trail.len(), "each event appears once");
    assert_eq!(
        trail,
        BTreeSet::from([w.asserted, w.facet, w.phantom]),
        "the trail is the asserting event, the facet event and the historical re-assertion"
    );
    assert!(
        trail.is_disjoint(&w.other),
        "no event of another edge is in this edge's trail"
    );
    for (id, ty) in &rows {
        let expected = if *id == w.facet {
            "property_asserted"
        } else {
            "relationship_asserted"
        };
        assert_eq!(ty, expected, "event {id} carries its type");
    }
}

/// FAILS IF the edge derivation leaves the label, a facet mark or the authorship reasoning of an
/// event on the edge's trail at its original text, or rewrites an event of another edge.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_edge_derivation_sentinels_label_and_facets(pool: PgPool) {
    let w = edged(&pool).await;

    let rows: Vec<(Uuid, Value, Value, Value)> = sqlx::query_as(
        "SELECT event_id, new_payload, new_metadata, paths \
           FROM _edge_erasure_payload_redaction($1)",
    )
    .bind(w.edge.uuid())
    .fetch_all(&pool)
    .await
    .expect("the edge derivation");
    let row = |id: Uuid| {
        rows.iter()
            .find(|r| r.0 == id)
            .unwrap_or_else(|| panic!("the derivation rewrites event {id}"))
    };

    let ids: BTreeSet<Uuid> = rows.iter().map(|r| r.0).collect();
    assert_eq!(
        ids,
        BTreeSet::from([w.asserted, w.facet, w.phantom]),
        "every event of the trail holds text to rewrite"
    );
    assert!(
        ids.is_disjoint(&w.other),
        "no event of another edge is rewritten"
    );

    let (_, payload, metadata, paths) = row(w.asserted);
    assert_eq!(payload["label"], json!("erased-label-1"));
    assert_eq!(metadata["reasoning"], json!("erased"));
    assert_eq!(
        metadata["confidence"],
        json!("probable"),
        "structural metadata stays"
    );
    assert!(paths.as_array().unwrap().contains(&json!("label")));
    assert!(paths
        .as_array()
        .unwrap()
        .contains(&json!("metadata.reasoning")));

    let (_, payload, metadata, paths) = row(w.phantom);
    assert_eq!(
        payload["label"],
        json!("erased-label-1"),
        "the re-assertion's label takes the same number as the assertion's"
    );
    assert!(
        metadata.get("reasoning").is_none(),
        "an event that carried no reasoning gains none"
    );
    assert_eq!(*paths, json!(["label"]));

    let (_, payload, _, paths) = row(w.facet);
    assert_eq!(
        payload["value"],
        json!({ "erased-facet-1": "erased", "erased-facet-2": "erased" }),
        "one erased-facet-<m> per original mark"
    );
    assert_eq!(
        payload["property_key"],
        json!("facet"),
        "the literal key stays"
    );
    assert_eq!(*paths, json!(["value"]));

    let dump =
        serde_json::to_string(&rows.iter().map(|r| (&r.1, &r.2)).collect::<Vec<_>>()).unwrap();
    for needle in [LABEL, REASONING, "jane roe", "confided"] {
        assert!(
            !dump.contains(needle),
            "the derivation still quotes {needle:?}"
        );
    }
}
