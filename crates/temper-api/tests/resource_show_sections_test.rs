//! Witnesses for `GET /api/resources/{id}?sections=` — the additive section parameter
//! (beat G3a's B1): the derived embedding readiness rides the row the gate already
//! admitted, so no standalone status route is needed and none exists. Four faces:
//! the no-query answer is the incumbent shape (open-meta filled, `embedding_status`
//! ABSENT — not requested means absent, never null); `?sections=embedding-status`
//! fills it; the CSV form composes; an unknown name is a `400` naming the vocabulary.
//!
//! Driven over the REAL router (setup_test_app), not the service layer — the wire is
//! the thing under test here (the service-level shape agreement is
//! `resource_view_test.rs`'s).
#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// Create a resource in the test profile's context (no body — nothing here reads one).
async fn create_resource(app: &common::TestApp, token: &str, context_id: Uuid) -> String {
    let created: Value = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": context_id.to_string(),
            "doc_type": "research",
            "origin_uri": format!("test://show-sections-{}", Uuid::new_v4()),
            "title": "Show Sections Test",
            "slug": null
        }))
        .send()
        .await
        .expect("create failed")
        .json()
        .await
        .expect("create JSON");
    created["id"]
        .as_str()
        .expect("created resource id missing")
        .to_string()
}

async fn get_with_sections(
    app: &common::TestApp,
    token: &str,
    resource_id: &str,
    sections: Option<&str>,
) -> (u16, Value) {
    let path = match sections {
        Some(csv) => format!("/api/resources/{resource_id}?sections={csv}"),
        None => format!("/api/resources/{resource_id}"),
    };
    let resp = app
        .client
        .get(app.url(&path))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("resource GET failed");
    let status = resp.status().as_u16();
    let body = resp.json::<Value>().await.expect("resource GET JSON");
    (status, body)
}

/// CLAUSE: the default (no query) answer is the incumbent shape — the open tier filled,
/// `embedding_status` absent. Not-requested is ABSENT, never null: the key's omission is
/// what keeps the no-query shape byte-identical to the pre-`sections` door.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn no_query_answers_the_incumbent_shape_without_the_key(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let email = format!("show-sections-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    let resource_id = create_resource(&app, &token, context_id).await;

    let (status, body) = get_with_sections(&app, &token, &resource_id, None).await;
    assert_eq!(status, 200, "the incumbent GET answers 200: {body}");
    assert!(
        body.get("open_meta").is_some(),
        "open-meta stays the door's baseline: {body}"
    );
    assert!(
        body.get("embedding_status").is_none(),
        "a section never asked for is ABSENT, not null: {body}"
    );
}

/// CLAUSE: `?sections=embedding-status` fills the derived readiness on the gated row —
/// and a freshly created resource with no chunks is trivially `ready` (the empty-body
/// arm of `embedding_status_batch`).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_embedding_status_section_fills_the_derived_field(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let email = format!("show-sections-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    let resource_id = create_resource(&app, &token, context_id).await;

    let (status, body) =
        get_with_sections(&app, &token, &resource_id, Some("embedding-status")).await;
    assert_eq!(status, 200, "the additive section answers 200: {body}");
    assert!(
        ["ready", "pending", "failed"].contains(
            &body["embedding_status"]
                .as_str()
                .expect("embedding_status is one of the three states")
        ),
        "the derived readiness rides the gated row: {body}"
    );
}

/// CLAUSE: the CSV form composes — `open-meta,embedding-status` fills both (the shape
/// the MCP resources tools ask the door for, so the MCP response and this one carry the
/// same keys at the same depth).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_csv_form_fills_both_sections(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let email = format!("show-sections-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    let resource_id = create_resource(&app, &token, context_id).await;

    let (status, body) = get_with_sections(
        &app,
        &token,
        &resource_id,
        Some("open-meta,embedding-status"),
    )
    .await;
    assert_eq!(status, 200, "the CSV form answers 200: {body}");
    assert!(body.get("open_meta").is_some(), "{body}");
    assert!(body.get("embedding_status").is_some(), "{body}");
}

/// CLAUSE: an unknown section name is the caller's `400`, naming the vocabulary — the
/// parse happens at the door, so a typo is recoverable from the refusal alone and never
/// renders as the server's 500.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unknown_section_name_is_a_400_naming_the_vocabulary(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let email = format!("show-sections-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    let resource_id = create_resource(&app, &token, context_id).await;

    let (status, body) = get_with_sections(&app, &token, &resource_id, Some("bogus")).await;
    assert_eq!(status, 400, "an unknown section is the caller's 400: {body}");
    let message = body["error"]["message"].as_str().unwrap_or_default();
    for valid in ["body", "open-meta", "edges", "embedding-status"] {
        assert!(
            message.contains(valid),
            "the refusal names `{valid}` so the caller can recover from it alone: {body}"
        );
    }
}
