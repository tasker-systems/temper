#![cfg(feature = "test-db")]
//! **A `resource_filter` field the server does not declare is refused through `POST /api/query`,
//! and the refusal keeps its kind to the caller.**
//!
//! `created_at` for `created` used to parse and narrow nothing: the door answered `200` with every
//! resource in the bound, a wider answer than the one asked, and no sign of why. Asserted at the
//! HTTP door because the kind is what a caller matches on and the door is where it could be lost:
//! a deserializer refusal here would answer `422 UNREADABLE_PLAN` without naming the field, since a
//! stage is an untagged enum.

mod common;

use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;

/// One returned `list-resources-with` stage over `ctx`, carrying `resource_filter` as given.
fn plan(ctx: uuid::Uuid, resource_filter: Value) -> Value {
    json!({
        "stages": [
            { "name": "rows", "act": "list-resources-with",
              "inputs": [ { "from": "caller", "as": "bound",
                            "ids": { "kind": "context", "ids": [ctx] } } ],
              "resource_filter": resource_filter }
        ],
        "outcome": { "returns": [ { "stage": "rows", "with": [] } ] }
    })
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unknown_resource_filter_field_is_refused_by_name(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("unknown-field-{}@example.com", uuid::Uuid::new_v4());
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

    // Control: the field spelled as declared runs, so the refusal below is the name.
    let (status, body) = post(plan(
        ctx,
        json!({ "created": { "from": "2000-01-01T00:00:00Z" } }),
    ))
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    for typo in ["created_at", "update"] {
        let (status, body) = post(plan(
            ctx,
            json!({ typo: { "from": "2000-01-01T00:00:00Z" } }),
        ))
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "`{typo}` must be refused, never ignored: {body}"
        );
        assert_eq!(body["error"]["code"], temper_core::error::PLAN_REFUSED_CODE);
        let refusals = body["error"]["details"]["refusals"]
            .as_array()
            .expect("the refusals ride the details");
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert_eq!(refusals[0]["reason"], "unknown_filter_field");
        assert_eq!(refusals[0]["stage"], "rows");
        let detail = refusals[0]["detail"].as_str().expect("a detail");
        assert!(
            detail.starts_with(&format!("`resource_filter` has no field named \"{typo}\";")),
            "the detail names the field: {detail}"
        );
        assert!(
            detail.contains("`created`") && detail.contains("`updated`"),
            "the detail lists the declared fields: {detail}"
        );
    }
}
