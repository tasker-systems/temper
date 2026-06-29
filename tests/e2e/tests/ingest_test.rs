#![cfg(feature = "test-db")]

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload};

/// Ingest a resource via the client, then verify it exists via list.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn ingest_creates_resource(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    // Ensure the test user's profile exists (auto-provisioned on first API call).
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    // Create a context owned by the test user so the ingest can resolve it.
    app.client
        .contexts()
        .create("e2e-test", None)
        .await
        .expect("context create failed");

    // Ingest a test resource.
    // content_hash must be a 64-char hex string (raw SHA-256, no prefix).
    let payload = IngestPayload {
        title: "E2E Test Document".to_string(),
        origin_uri: "test://e2e/ingest-test".to_string(),
        context_ref: "@me/e2e-test".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: Some(
            "e2e0test00000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        slug: "e2e-test-document".to_string(),
        content: "# E2E Test\n\nThis is a test document for e2e testing.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
        act: Default::default(),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest create failed");

    assert_eq!(resource.title, "E2E Test Document");
    assert_eq!(resource.origin_uri, "test://e2e/ingest-test");
    assert!(resource.is_active);

    // Verify it appears in resource list.
    let resources = app
        .client
        .resources()
        .list(&temper_workflow::types::resource::ResourceListParams {
            limit: Some(50),
            ..Default::default()
        })
        .await
        .expect("list resources failed");

    assert!(
        resources
            .rows
            .iter()
            .any(|r| r.origin_uri == "test://e2e/ingest-test"),
        "ingested resource not found in list"
    );
}
