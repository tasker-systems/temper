#![cfg(feature = "test-db")]
//! **A combinator stage's trace names its combinator, never `admit`, through `POST /api/query`.**
//!
//! A combinator (`union`, `intersect`, `difference`) runs no act, and its trace entry used to read
//! `"act": "admit"`, the registry's declared anti-act, so a reader of `act` saw the anti-act as the
//! thing that answered. Now `act` carries the combinator's wire name and a typed `combinator`
//! field carries the op; an act stage carries no `combinator` key. Asserted at the HTTP door,
//! because the trace is what a caller reads and the wire is where its spelling is fixed.

mod common;

use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;

/// Two selections over `ctx`, combined by `op`, and the combined set returned as rows.
fn plan(ctx: uuid::Uuid, op: &str) -> Value {
    let select = |name: &str, doc_type: &str| {
        json!({ "name": name, "act": "find-resources-with",
                "inputs": [ { "from": "caller", "as": "bound",
                              "ids": { "kind": "context", "ids": [ctx] } } ],
                "resource_filter": { "doc_type": [doc_type] } })
    };
    json!({
        "stages": [
            select("concepts", "concept"),
            select("tasks", "task"),
            { "name": "combined", "op": op, "inputs": ["concepts", "tasks"] },
            { "name": "rows", "act": "list-resources-with",
              "inputs": [ { "from": "upstream", "as": "bound", "stage": "combined" } ] }
        ],
        "outcome": { "returns": [ { "stage": "rows", "with": [] } ] }
    })
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_combinator_stage_names_its_combinator_never_admit(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("combinator-trace-{}@example.com", uuid::Uuid::new_v4());
    let (profile, ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);
    let event = common::seed_genesis_event(&app.pool, profile, ctx).await;
    common::seed_resource(&app.pool, ctx, profile, event, "A concept", "concept").await;
    common::seed_resource(&app.pool, ctx, profile, event, "A task", "task").await;

    for op in ["union", "intersect", "difference"] {
        let resp = app
            .client
            .post(app.url("/api/query"))
            .header("Authorization", format!("Bearer {token}"))
            .json(&plan(ctx, op))
            .send()
            .await
            .expect("POST /api/query");
        let status = resp.status();
        let body: Value = resp.json().await.expect("a JSON body");
        assert_eq!(status, StatusCode::OK, "`{op}`: {body}");

        let stages = body["trace"]["stages"]
            .as_array()
            .expect("the trace carries every stage");
        let stage = |name: &str| {
            stages
                .iter()
                .find(|s| s["stage"] == name)
                .unwrap_or_else(|| panic!("`{op}`: no trace entry for `{name}`: {stages:?}"))
        };

        let combined = stage("combined");
        assert_eq!(
            combined["act"], op,
            "`{op}`: a combinator stage names its combinator: {combined}"
        );
        assert_ne!(combined["act"], "admit", "`{op}`: never the anti-act");
        assert_eq!(
            combined["combinator"], op,
            "`{op}`: the typed handle names the same op: {combined}"
        );

        // An act stage names its act and carries no `combinator` key at all.
        for (name, act) in [
            ("concepts", "find-resources-with"),
            ("tasks", "find-resources-with"),
            ("rows", "list-resources-with"),
        ] {
            let s = stage(name);
            assert_eq!(s["act"], act, "`{op}`: {s}");
            assert!(
                s.get("combinator").is_none(),
                "`{op}`: an act stage carries no combinator: {s}"
            );
        }
    }
}
