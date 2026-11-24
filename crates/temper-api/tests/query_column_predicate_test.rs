#![cfg(feature = "test-db")]
//! **A resource property predicate on `created` or `updated` is refused through `POST /api/query`,
//! and the refusal keeps its kind to the caller.**
//!
//! Both are columns of every resource and never property rows, so a predicate on either matched
//! nothing: the door answered `200` with an `empty` stage, which a caller asking "created after X"
//! reads as "none". Asserted at the HTTP door because the kind is what a caller matches on, and the
//! door is where it could be lost.

mod common;

use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;

/// One returned `list-resources-with` stage over `ctx`, carrying one resource predicate.
fn plan(ctx: uuid::Uuid, key: &str, op: Value) -> Value {
    json!({
        "stages": [
            { "name": "rows", "act": "list-resources-with",
              "inputs": [ { "from": "caller", "as": "bound",
                            "ids": { "kind": "context", "ids": [ctx] } } ],
              "resource_filter": { "properties": [ { "key": key, "op": op } ] } }
        ],
        "outcome": { "returns": [ { "stage": "rows", "with": [] } ] }
    })
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_predicate_on_created_or_updated_is_refused_by_name(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("column-pred-{}@example.com", uuid::Uuid::new_v4());
    let (profile, ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);
    let event = common::seed_genesis_event(&app.pool, profile, ctx).await;
    common::seed_resource(&app.pool, ctx, profile, event, "A resource", "concept").await;

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

    // Control: the same plan on an open key runs, so the refusal below is the field, not the plan.
    let (status, body) = post(plan(ctx, "date", json!({ "op": "has_key" }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    for field in ["created", "updated"] {
        let compare = json!({ "op": "compare", "direction": "gte", "value": "2000-01-01" });
        let (status, body) = post(plan(ctx, field, compare)).await;
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
        assert_eq!(refusals[0]["stage"], "rows");
        let detail = refusals[0]["detail"].as_str().expect("a detail");
        assert!(
            detail.starts_with(&format!("`{field}` is not a property: it is"))
                && detail.contains("a column of every resource"),
            "the detail names the field and where it lives: {detail}"
        );
    }
}
