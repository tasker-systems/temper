#![cfg(feature = "test-db")]
//! The states a kind of work carries are reachable over HTTP.
//!
//! This is an **`enables`** change, and these are mechanism tests, not clause witnesses.
//! They establish that the derivation is reachable from the door the web surface speaks —
//! not that any surface presents the right vocabulary, which is the surface's own build.
//!
//! Each asks the running service rather than a function. The derivation's own behaviour is
//! pinned beside it in `temper_workflow::schema`; what only a wire test can establish is
//! that the route is mounted, that it is gated, and that the serialized body carries the
//! vocabulary rather than dropping it in a `Serialize` impl nobody checked.
//!
//! They bite against the pre-change state exactly: before this, `crates/temper-api/src/
//! routes.rs` served no schema route at all, so every request below returned 404 with a
//! valid token in hand — the condition that left the web surface able to satisfy
//! *"the states offered are the states the work carries"* only by keeping its own copy of
//! the vocabulary.

mod common;

use serde_json::Value;
use sqlx::PgPool;

/// The two states this task exists to make reachable, asked of a running service.
///
/// Set equality, not `contains`: a vocabulary that is a superset of the schema's is the
/// same defect as one that is a subset — it offers a state the work does not carry.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_task_s_stages_arrive_over_http_exactly_as_the_schema_declares_them(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("schema-reader-{}@test.com", uuid::Uuid::now_v7());
    let (profile, _ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    let resp = app
        .client
        .get(app.url("/api/schema/doc-types/task"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "the doc-type read must be mounted and reachable"
    );

    let body: Value = resp.json().await.expect("expected JSON body");
    assert_eq!(body["name"], "task");

    let stages: Vec<String> = body["enum_fields"]["temper-stage"]
        .as_array()
        .expect("enum_fields carries temper-stage over the wire")
        .iter()
        .map(|v| v.as_str().expect("a stage is a string").to_string())
        .collect();
    assert_eq!(
        stages,
        vec!["backlog", "in-progress", "done", "cancelled"],
        "a task's stages must be exactly the four `task.schema.json` declares"
    );

    // The negative half. A surface reading this answer must not be handed a state that
    // belongs to a different kind of work.
    assert!(
        body["enum_fields"].get("temper-status").is_none(),
        "a task carries no temper-status; the answer must not offer one: {}",
        body["enum_fields"]
    );
}

/// The other kind of work, so the answer is doc-type-specific rather than one shared list.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_goal_s_statuses_are_its_own_and_not_a_task_s(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("schema-reader-{}@test.com", uuid::Uuid::now_v7());
    let (profile, _ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    let resp = app
        .client
        .get(app.url("/api/schema/doc-types/goal"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);

    let body: Value = resp.json().await.expect("expected JSON body");
    let statuses: Vec<String> = body["enum_fields"]["temper-status"]
        .as_array()
        .expect("enum_fields carries temper-status over the wire")
        .iter()
        .map(|v| v.as_str().expect("a status is a string").to_string())
        .collect();
    assert_eq!(
        statuses,
        vec!["active", "completed", "paused", "cancelled"],
        "a goal's statuses must be exactly the four `goal.schema.json` declares"
    );
    assert!(
        body["enum_fields"].get("temper-stage").is_none(),
        "a goal carries no temper-stage; the answer must not offer one"
    );
}

/// The list, so a caller can find the doc types before describing one.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_doc_type_list_covers_every_kind_the_binary_knows(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("schema-reader-{}@test.com", uuid::Uuid::now_v7());
    let (profile, _ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    let resp = app
        .client
        .get(app.url("/api/schema/doc-types"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);

    let body: Value = resp.json().await.expect("expected JSON body");
    let rows = body.as_array().expect("the list is a JSON array");
    assert_eq!(
        rows.len(),
        temper_workflow::frontmatter::DocType::ALL.len(),
        "every doc-type variant must reach the wire"
    );

    let names: Vec<&str> = rows
        .iter()
        .map(|r| r["name"].as_str().expect("a name is a string"))
        .collect();
    assert!(names.contains(&"task") && names.contains(&"goal"));
    assert!(
        rows.iter().all(|r| r["has_schema"] == true),
        "every listed doc type embeds a schema"
    );
}

/// The open tier's recognized conventions, the third member of the same family.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_open_tier_conventions_are_reachable_from_the_same_door(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("schema-reader-{}@test.com", uuid::Uuid::now_v7());
    let (profile, _ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    let resp = app
        .client
        .get(app.url("/api/schema/open-meta"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status().as_u16(), 200);

    let body: Value = resp.json().await.expect("expected JSON body");
    assert!(
        body["schema"]["properties"].get("keywords").is_some(),
        "the recognized-conventions schema must reach the wire: {body}"
    );
    // The tier stays open — this is guidance, not a closed vocabulary, and a surface that
    // read it as closed would refuse keys the system accepts.
    assert_eq!(
        body["schema"]["additionalProperties"], true,
        "the open tier must publish itself as open"
    );
    let discouraged: Vec<&str> = body["discouraged_keys"]
        .as_array()
        .expect("discouraged_keys is an array")
        .iter()
        .map(|d| d["key"].as_str().expect("a key is a string"))
        .collect();
    assert!(
        discouraged.contains(&"slug") && discouraged.contains(&"title"),
        "discouraged keys must survive serialization: {discouraged:?}"
    );
}

/// The name comes from the caller, so an unrecognized one is the caller's mistake.
///
/// `DocType::from_str` refuses with `TemperError::Config`, which maps to a 500 — the
/// handler must not let that through. A 500 here would tell a caller the server broke when
/// in fact they asked for a type that does not exist.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unknown_doc_type_is_not_found_rather_than_an_internal_fault(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("schema-reader-{}@test.com", uuid::Uuid::now_v7());
    let (profile, _ctx) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);

    let resp = app
        .client
        .get(app.url("/api/schema/doc-types/widget"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        404,
        "an unknown doc-type name is a caller error, not a server fault"
    );
}

/// Caller-independence is a property of the answer, not a reason to publish it.
///
/// These three reads return the same bytes to everyone, which makes it easy to argue them
/// onto the unauthenticated surface beside `/api/health`. They are not there, and this is
/// the test that keeps them off it: a route that stopped requiring a token would go green
/// on every other test in this file.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_schema_read_requires_a_token(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    for path in [
        "/api/schema/doc-types",
        "/api/schema/doc-types/task",
        "/api/schema/open-meta",
    ] {
        let resp = app
            .client
            .get(app.url(path))
            .send()
            .await
            .expect("request failed");
        assert_eq!(
            resp.status().as_u16(),
            401,
            "{path} must refuse an unauthenticated caller"
        );
    }
}
