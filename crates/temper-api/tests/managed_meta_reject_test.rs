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
