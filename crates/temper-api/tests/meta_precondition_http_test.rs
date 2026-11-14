//! Conditional meta writes on the wire: `PUT /api/resources/{id}/meta` and
//! `PATCH /api/resources/{id}` with an `expect`.
//!
//! The sequence itself (lock, read standing, comparison, order of refusals) is witnessed against
//! the backend in `temper-services/tests/meta_precondition_test.rs`. These pin what only HTTP can
//! show: the `412` envelope, the `422` for an unknown expectation member, and that the vestigial
//! hashes play no part.
#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;

struct Caller {
    token: String,
    resource: String,
}

async fn a_resource(app: &common::TestApp) -> Caller {
    let email = format!("guard-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    let resp = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "title": "Guarded",
            "origin_uri": format!("test://guard-{}", uuid::Uuid::new_v4()),
            "kb_context_id": context_id,
            "doc_type": "research",
        }))
        .send()
        .await
        .expect("create");
    let status = resp.status().as_u16();
    let created: Value = resp.json().await.expect("create JSON");
    assert_eq!(status, 200, "{created}");
    let who = Caller {
        token,
        resource: created["id"].as_str().expect("id").to_string(),
    };
    // Unconditional, as before this change: no `expect` at all.
    let (status, body) = send(
        app,
        &who,
        reqwest::Method::PATCH,
        &format!("/api/resources/{}", who.resource),
        json!({"open_meta": {"claim": "held"}}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    who
}

async fn send(
    app: &common::TestApp,
    who: &Caller,
    method: reqwest::Method,
    path: &str,
    body: Value,
) -> (u16, Value) {
    let resp = app
        .client
        .request(method, app.url(path))
        .header("Authorization", format!("Bearer {}", who.token))
        .json(&body)
        .send()
        .await
        .expect("request");
    let status = resp.status().as_u16();
    let text = resp.text().await.expect("body");
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

fn put_meta_body(who: &Caller, open: Value, expect: Value, hash: &str) -> Value {
    json!({
        "resource_id": who.resource,
        "managed_meta": {},
        "open_meta": open,
        "managed_hash": hash,
        "open_hash": hash,
        "expect": expect,
    })
}

/// **The refusal on the wire**: `412 PRECONDITION_FAILED`, a keys-only message, and the failed
/// expectations as `error.precondition`, a sibling of `details`, which is absent. On both doors.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_failed_expectation_is_a_412_with_the_precondition_beside_details(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let who = a_resource(&app).await;
    let meta = format!("/api/resources/{}/meta", who.resource);
    let patch = format!("/api/resources/{}", who.resource);
    let expected = json!({"error": {
        "code": "PRECONDITION_FAILED",
        "message": "1 precondition failed: open.claim",
        "precondition": {"failed": [
            {"tier": "open", "key": "claim", "expected": null, "current": "held"}
        ]},
    }});

    let (status, body) = send(
        &app,
        &who,
        reqwest::Method::PUT,
        &meta,
        put_meta_body(
            &who,
            json!({"claim": "mine"}),
            json!({"open": {"claim": null}}),
            "",
        ),
    )
    .await;
    assert_eq!((status, &body), (412, &expected));

    let (status, body) = send(
        &app,
        &who,
        reqwest::Method::PATCH,
        &patch,
        json!({"open_meta": {"claim": "mine"}, "expect": {"open": {"claim": null}}}),
    )
    .await;
    assert_eq!((status, &body), (412, &expected));
}

/// **An unknown expectation member is a `422`, never a silent drop**, and `expect.body` on
/// `PUT /meta` is one: a meta write's expectation has no body member.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unknown_expectation_member_is_a_422(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let who = a_resource(&app).await;
    let meta = format!("/api/resources/{}/meta", who.resource);
    for expect in [
        json!({"body": "bt1.x"}),
        json!({"open": {"claim": "held"}, "body": "bt1.x"}),
        json!({"opne": {"claim": "held"}}),
    ] {
        let (status, body) = send(
            &app,
            &who,
            reqwest::Method::PUT,
            &meta,
            put_meta_body(&who, json!({"claim": "mine"}), expect.clone(), ""),
        )
        .await;
        assert_eq!(status, 422, "PUT /meta with expect {expect}: {body}");
    }
    let (status, body) = send(
        &app,
        &who,
        reqwest::Method::PATCH,
        &format!("/api/resources/{}", who.resource),
        json!({"open_meta": {"claim": "mine"}, "expect": {"opne": {}}}),
    )
    .await;
    assert_eq!(status, 422, "PATCH with an unknown member: {body}");
}

/// **The vestigial hashes are ignored**: a holding guard lands whatever `managed_hash` and
/// `open_hash` say, and a failing one is refused whatever they say.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_hashes_play_no_part(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let who = a_resource(&app).await;
    let meta = format!("/api/resources/{}/meta", who.resource);
    let (status, body) = send(
        &app,
        &who,
        reqwest::Method::PUT,
        &meta,
        put_meta_body(
            &who,
            json!({"claim": "mine"}),
            json!({"open": {"claim": "held"}}),
            "garbage",
        ),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["open_meta"]["claim"], "mine");
    let (status, _) = send(
        &app,
        &who,
        reqwest::Method::PUT,
        &meta,
        put_meta_body(
            &who,
            json!({"claim": "again"}),
            json!({"open": {"claim": "held"}}),
            "garbage",
        ),
    )
    .await;
    assert_eq!(status, 412);
}
