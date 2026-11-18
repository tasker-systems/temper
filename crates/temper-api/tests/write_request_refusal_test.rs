//! A resource write whose shape no act can honour is refused by the server, by name.
//!
//! The server is the only enforcing copy of request shaping: the CLI and MCP used to refuse these
//! before sending, while a direct HTTP caller's request was quietly reshaped instead. Sources with
//! no body were dropped, a `content_block` with no `content` was ignored, and a payload naming both
//! homes went to the cogmap. Each case asserts the 400 names the field, and that nothing was
//! written.
#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn auth(pool: &PgPool, tag: &str) -> (String, Uuid) {
    let email = format!("shape-{tag}-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    (token, context_id)
}

/// How many live resources carry this title: the proof that a refused create wrote nothing.
async fn resources_titled(pool: &PgPool, title: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_resources WHERE title = $1")
        .bind(title)
        .fetch_one(pool)
        .await
        .expect("count resources")
}

async fn post_ingest(app: &common::TestApp, token: &str, body: Value) -> (u16, String) {
    let resp = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .expect("ingest request");
    let status = resp.status().as_u16();
    (status, resp.text().await.unwrap_or_default())
}

fn a_source() -> Value {
    json!([{ "kind": "resource", "value": Uuid::now_v7() }])
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_ingest_with_sources_and_no_body_is_refused_by_name(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, ctx) = auth(&pool, "sources").await;
    let title = format!("sources-no-body-{}", Uuid::new_v4());

    let (status, body) = post_ingest(
        &app,
        &token,
        json!({
            "title": title,
            "origin_uri": "",
            "context_ref": ctx.to_string(),
            "doc_type_name": "research",
            "content": "",
            "sources": a_source(),
        }),
    )
    .await;

    assert_eq!(
        status, 400,
        "sources with no body must be refused; body: {body}"
    );
    assert!(body.contains("sources supplied without content"), "{body}");
    assert_eq!(
        resources_titled(&pool, &title).await,
        0,
        "nothing is written"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_ingest_naming_two_homes_is_refused_by_name(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, ctx) = auth(&pool, "homes").await;
    let title = format!("two-homes-{}", Uuid::new_v4());

    let (status, body) = post_ingest(
        &app,
        &token,
        json!({
            "title": title,
            "origin_uri": "",
            "context_ref": ctx.to_string(),
            "home_cogmap_id": Uuid::now_v7(),
            "doc_type_name": "research",
            "content": "",
        }),
    )
    .await;

    assert_eq!(status, 400, "two homes must be refused; body: {body}");
    assert!(body.contains("mutually exclusive"), "{body}");
    assert_eq!(
        resources_titled(&pool, &title).await,
        0,
        "nothing is written"
    );
}

/// A bodyless create with no sources stays an ordinary create: the refusals above are not a
/// new requirement to send a body.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_ingest_with_no_body_and_no_sources_still_creates(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, ctx) = auth(&pool, "plain").await;

    let (status, body) = post_ingest(
        &app,
        &token,
        json!({
            "title": format!("plain-{}", Uuid::new_v4()),
            "origin_uri": "",
            "context_ref": ctx.to_string(),
            "doc_type_name": "research",
            "content": "",
        }),
    )
    .await;

    assert_eq!(status, 200, "{body}");
}

/// Create a bodyless resource to patch, returning its id.
async fn bodyless_resource(app: &common::TestApp, token: &str, ctx: Uuid) -> String {
    let (status, body) = post_ingest(
        app,
        token,
        json!({
            "title": format!("patch-target-{}", Uuid::new_v4()),
            "origin_uri": "",
            "context_ref": ctx.to_string(),
            "doc_type_name": "research",
            "content": "",
        }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body).expect("create JSON");
    v["id"].as_str().expect("id").to_owned()
}

async fn patch(app: &common::TestApp, token: &str, id: &str, body: Value) -> (u16, String) {
    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .expect("patch request");
    let status = resp.status().as_u16();
    (status, resp.text().await.unwrap_or_default())
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_patch_with_sources_and_no_content_is_refused_by_name(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, ctx) = auth(&pool, "patch-sources").await;
    let id = bodyless_resource(&app, &token, ctx).await;

    let (status, body) = patch(&app, &token, &id, json!({ "sources": a_source() })).await;

    assert_eq!(
        status, 400,
        "sources with no content must be refused; body: {body}"
    );
    assert!(body.contains("sources supplied without content"), "{body}");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_patch_with_a_content_block_and_no_content_is_refused_by_name(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, ctx) = auth(&pool, "patch-block").await;
    let id = bodyless_resource(&app, &token, ctx).await;

    let (status, body) = patch(
        &app,
        &token,
        &id,
        json!({ "content_block": Uuid::now_v7(), "title": "renamed" }),
    )
    .await;

    assert_eq!(
        status, 400,
        "content_block with no content must be refused; body: {body}"
    );
    assert!(
        body.contains("content_block supplied without content"),
        "{body}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_ingest_with_an_empty_doc_type_is_refused_by_name(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, ctx) = auth(&pool, "doctype").await;
    let title = format!("empty-doctype-{}", Uuid::new_v4());

    let (status, body) = post_ingest(
        &app,
        &token,
        json!({
            "title": title,
            "origin_uri": "",
            "context_ref": ctx.to_string(),
            "doc_type_name": "",
            "content": "",
        }),
    )
    .await;

    assert_eq!(
        status, 400,
        "an empty doc type must be refused; body: {body}"
    );
    assert!(body.contains("doc_type must be non-empty"), "{body}");
    assert_eq!(
        resources_titled(&pool, &title).await,
        0,
        "nothing is written"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_patch_converting_to_an_empty_doc_type_is_refused_by_name(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, ctx) = auth(&pool, "type-to-empty").await;
    let id = bodyless_resource(&app, &token, ctx).await;

    let (status, body) = patch(&app, &token, &id, json!({ "type_to": "" })).await;

    assert_eq!(
        status, 400,
        "an empty type_to must be refused; body: {body}"
    );
    assert!(body.contains("doc_type must be non-empty"), "{body}");
}

/// A conversion to a name no schema knows is accepted, as a create of that type is: the caller's
/// doc-type vocabulary is its own. The doors refused it before sending; the server never did, and
/// that is the rule now on every door.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_patch_converting_to_an_unrecognized_doc_type_is_accepted(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, ctx) = auth(&pool, "type-to-open").await;
    let id = bodyless_resource(&app, &token, ctx).await;

    let (status, body) = patch(&app, &token, &id, json!({ "type_to": "field-note" })).await;

    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body).expect("patch JSON");
    assert_eq!(v["doc_type_name"], "field-note", "{body}");
}
