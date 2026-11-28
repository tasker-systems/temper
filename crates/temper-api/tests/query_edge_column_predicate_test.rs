#![cfg(feature = "test-db")]
//! **An edge property predicate on an edge column is refused through `POST /api/query`, and the
//! refusal keeps its kind to the caller.**
//!
//! `created`, `edge_kind`, `label`, `polarity` and `weight` are columns of every edge and never
//! edge property rows, so a predicate on one matched nothing: the door answered `200` with an empty
//! walk, which a caller asking "edges created after X" reads as "none". The edge-side sibling of
//! `query_column_predicate_test.rs`, asserted at the HTTP door for the same reason: the kind is
//! what a caller matches on, and the door is where it could be lost.

mod common;

use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;

/// One returned `follow-from` stage seeded by `seed`, carrying one edge predicate.
fn plan(seed: uuid::Uuid, key: &str, op: Value) -> Value {
    json!({
        "stages": [
            { "name": "walk", "act": "follow-from",
              "inputs": [ { "from": "caller", "as": "seed",
                            "ids": { "kind": "resource", "ids": [seed] } } ],
              "edge_filter": { "properties": [ { "key": key, "op": op } ] } }
        ],
        "outcome": { "returns": [ { "stage": "walk", "with": [] } ] }
    })
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_edge_predicate_on_an_edge_column_is_refused_by_name(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("edge-column-pred-{}@example.com", uuid::Uuid::new_v4());
    let (profile, ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);
    let event = common::seed_genesis_event(&app.pool, profile, ctx).await;
    let seed = common::seed_resource(&app.pool, ctx, profile, event, "A seed", "concept").await;

    let post = |body: Value| {
        let app = &app;
        let token = token.clone();
        async move {
            let resp = app
                .client
                .post(app.url("/api/query"))
                .header("Authorization", format!("Bearer {token}"))
                .json(&body)
                .send()
                .await
                .expect("POST /api/query");
            let status = resp.status();
            (status, resp.json::<Value>().await.expect("a JSON body"))
        }
    };

    // Control: the same walk on an edge key that is no column runs, so the refusal below is the
    // field, not the plan. This is also the answer a column predicate used to get: `200`, empty.
    let (status, body) = post(plan(seed, "since", json!({ "op": "has_key" }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    for (field, reached) in [
        ("created", "No filter narrows an edge by it"),
        ("edge_kind", "Narrow by it with `edge_filter.edge_kinds`"),
        ("label", "Narrow by it with `edge_filter.labels`"),
        ("polarity", "No filter narrows an edge by it"),
        ("weight", "No filter narrows an edge by it"),
    ] {
        let compare = json!({ "op": "compare", "direction": "gte", "value": "2000-01-01" });
        let (status, body) = post(plan(seed, field, compare)).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "`{field}` must be refused, never answered empty: {body}"
        );
        assert_eq!(body["error"]["code"], temper_core::error::PLAN_REFUSED_CODE);
        let refusals = body["error"]["details"]["refusals"]
            .as_array()
            .expect("the refusals ride the details");
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert_eq!(refusals[0]["reason"], "column_not_a_property");
        assert_eq!(refusals[0]["stage"], "walk");
        let detail = refusals[0]["detail"].as_str().expect("a detail");
        assert!(
            detail.starts_with(&format!("`{field}` is read as a property here, but it is"))
                && detail.contains("a column of every edge")
                && detail.contains(reached),
            "the detail names the field and where it lives: {detail}"
        );
        assert!(
            detail.contains("`edge_filter.properties`"),
            "the detail names the container: {detail}"
        );
    }
}
