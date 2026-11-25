#![cfg(feature = "test-db")]
//! **`resource_filter.created` and `.updated` narrow a selection through `POST /api/query`, and an
//! inverted range is refused by name.**
//!
//! Asserted at the HTTP door because the wire is where a range could be dropped: `ResourceFilter`
//! admits unknown fields, so a door that did not know `created` would accept the plan and answer
//! the unnarrowed set.

mod common;

use reqwest::StatusCode;
use serde_json::{json, Value};

/// One returned `list-resources-with` stage over `ctx`, narrowed by `filter`.
fn plan(ctx: uuid::Uuid, filter: Value) -> Value {
    json!({
        "stages": [
            { "name": "rows", "act": "list-resources-with",
              "inputs": [ { "from": "caller", "as": "bound",
                            "ids": { "kind": "context", "ids": [ctx] } } ],
              "resource_filter": filter }
        ],
        "outcome": { "returns": [ { "stage": "rows", "with": [] } ] }
    })
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_time_range_narrows_at_either_end_and_an_inverted_one_is_refused(pool: sqlx::PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("time-range-{}@example.com", uuid::Uuid::new_v4());
    let (profile, ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);
    let event = common::seed_genesis_event(&app.pool, profile, ctx).await;
    // Three resources created on 1, 2 and 3 November, and updated in the reverse order.
    let mut ids = Vec::new();
    for (day, title) in [(1, "first"), (2, "second"), (3, "third")] {
        let id = common::seed_resource(&app.pool, ctx, profile, event, title, "concept").await;
        sqlx::query(
            "UPDATE kb_resources SET created = make_timestamptz(2026, 11, $2, 0, 0, 0, 'UTC'), \
             updated = make_timestamptz(2026, 11, 4 - $2, 0, 0, 0, 'UTC') WHERE id = $1",
        )
        .bind(id)
        .bind(day)
        .execute(&app.pool)
        .await
        .expect("pin the clocks");
        ids.push(id.to_string());
    }

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
    let titles = |body: &Value| -> Vec<String> {
        let produced = &body["returned"]["rows"]["produced"];
        let hits = produced["hits"]
            .as_array()
            .or_else(|| produced["resources"]["hits"].as_array())
            .unwrap_or_else(|| panic!("the rows stage's hits: {body}"));
        let mut out: Vec<String> = hits
            .iter()
            .map(|h| {
                h["resource"]["title"]
                    .as_str()
                    .expect("a title")
                    .to_string()
            })
            .collect();
        out.sort();
        out
    };

    for (case, filter, expected) in [
        (
            "created, from only (inclusive)",
            json!({ "created": { "from": "2026-11-02T00:00:00Z" } }),
            vec!["second", "third"],
        ),
        (
            "created, until only (exclusive)",
            json!({ "created": { "until": "2026-11-02T00:00:00Z" } }),
            vec!["first"],
        ),
        (
            "created, both, with an offset read as UTC",
            json!({ "created": { "from": "2026-11-02T09:00:00+09:00",
                                 "until": "2026-11-03T00:00:00Z" } }),
            vec!["second"],
        ),
        (
            "updated, from only",
            json!({ "updated": { "from": "2026-11-03T00:00:00Z" } }),
            vec!["first"],
        ),
    ] {
        let (status, body) = post(plan(ctx, filter)).await;
        assert_eq!(status, StatusCode::OK, "{case}: {body}");
        assert_eq!(titles(&body), expected, "{case}: {body}");
    }

    // The trace echoes each applied end, in UTC at the store's precision, and nothing for an end
    // left open: a nanosecond `from` is echoed as the microsecond it ran at.
    let (status, body) = post(plan(
        ctx,
        json!({ "created": { "from": "2026-11-02T09:00:00.123456789+09:00" } }),
    ))
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let echoed: Vec<(String, String)> = body["returned"]["rows"]["narrowed_by"]
        .as_array()
        .unwrap_or_else(|| panic!("narrowed_by: {body}"))
        .iter()
        .filter(|n| n["key"].as_str().is_some_and(|k| k.starts_with("created_")))
        .map(|n| {
            (
                n["key"].as_str().unwrap().to_string(),
                n["value"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        echoed,
        vec![(
            "created_from".to_string(),
            "2026-11-02T00:00:00.123456+00:00".to_string()
        )],
        "{body}"
    );

    // A bare date and an offset-less timestamp do not parse: the plan is unreadable, never run.
    for unparsed in ["2026-11-02", "2026-11-02T00:00:00"] {
        let (status, body) = post(plan(ctx, json!({ "created": { "from": unparsed } }))).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "`{unparsed}`: {body}"
        );
        assert_eq!(
            body["error"]["code"], "UNREADABLE_PLAN",
            "`{unparsed}`: {body}"
        );
    }

    // Inverted: refused by name, never answered empty.
    let (status, body) = post(plan(
        ctx,
        json!({ "updated": { "from": "2026-11-03T00:00:00Z", "until": "2026-11-02T00:00:00Z" } }),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], temper_core::error::PLAN_REFUSED_CODE);
    let refusals = body["error"]["details"]["refusals"]
        .as_array()
        .expect("the refusals ride the details");
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    assert_eq!(refusals[0]["reason"], "inverted_range");
    assert_eq!(refusals[0]["stage"], "rows");
    assert!(
        refusals[0]["detail"]
            .as_str()
            .expect("a detail")
            .starts_with("`resource_filter.updated` runs from "),
        "{refusals:?}"
    );
}
