//! POST /api/ingest rejects a caller-invented managed_meta key.
//!
//! `managed_meta` is a closed, temper-owned vocabulary (deny_unknown_fields on
//! `ManagedMeta`). A key the typed struct does not name must 400 with a hint
//! pointing the caller at `open_meta`, not silently migrate tiers.
#![cfg(feature = "test-db")]

mod common;

use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn ingest_rejects_unknown_managed_key(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("meta-reject-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let sub = format!("test|{profile_id}");
    let token = common::generate_test_jwt(&sub, &email);

    let body = json!({
        "title": "reject me",
        "origin_uri": format!("test://meta-reject-{}", uuid::Uuid::new_v4()),
        "context_ref": context_id.to_string(),
        "doc_type_name": "task",
        "slug": "reject-me",
        "content": "body",
        "managed_meta": { "not-a-managed-key": "boom" },
        "open_meta": {}
    });

    let resp = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .expect("ingest request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "an unknown managed key must be rejected with 400"
    );
    let text = resp.text().await.expect("error body");
    assert!(
        text.contains("open_meta"),
        "error must point the caller at open_meta, got: {text}"
    );
}

/// A create with NO `managed_meta` key at all must succeed: the Property
/// vocabulary is entirely optional and smart-defaulted server-side. The
/// doc-type default (`temper-stage` → `backlog` for a task) is applied before
/// schema validation, so `managed_meta` is never caller-required (spec P2.3).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn ingest_with_no_managed_meta_succeeds_with_defaults(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("nomm-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    // Note the absence of any `managed_meta` key.
    let resp = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "title": "No managed meta",
            "origin_uri": format!("test://nomm-{}", uuid::Uuid::new_v4()),
            "context_ref": context_id.to_string(),
            "doc_type_name": "task",
            "slug": "no-managed-meta",
            "content": "body"
        }))
        .send()
        .await
        .expect("ingest request failed");
    let status = resp.status().as_u16();
    let created: serde_json::Value = resp.json().await.expect("ingest response JSON");
    assert_eq!(
        status, 200,
        "a create with no managed_meta must succeed; body: {created}"
    );
    let resource_id = created["id"].as_str().expect("id in ingest response");

    // The server-side default `temper-stage: backlog` is visible via GET meta.
    let meta: serde_json::Value = app
        .client
        .get(app.url(&format!("/api/resources/{resource_id}/meta")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("GET meta failed")
        .json()
        .await
        .expect("meta JSON");
    assert_eq!(
        meta["managed_meta"]["temper-stage"], "backlog",
        "default stage must be applied server-side; meta: {meta}"
    );
}

/// `temper-updated` and `temper-source` left the managed vocabulary in Phase 2
/// (they are `KeyFate::Die` system keys, not Property metadata). A caller that
/// smuggles them into `managed_meta` must be rejected at the type boundary, not
/// silently carried (spec P2.1 risk closure).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn ingest_rejects_temper_updated_and_source(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("sys-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    for key in ["temper-updated", "temper-source"] {
        let resp = app
            .client
            .post(app.url("/api/ingest"))
            .header("Authorization", format!("Bearer {token}"))
            .json(&json!({
                "title": "sys",
                "origin_uri": format!("test://sys-{}", uuid::Uuid::new_v4()),
                "context_ref": context_id.to_string(),
                "doc_type_name": "task",
                "slug": "sys",
                "content": "b",
                "managed_meta": { key: "x" }
            }))
            .send()
            .await
            .expect("ingest request failed");
        assert_eq!(
            resp.status().as_u16(),
            400,
            "{key} left the managed vocabulary and must be rejected as a non-managed key"
        );
    }
}
