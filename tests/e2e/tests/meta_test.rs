#![cfg(feature = "test-db")]

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload};
use temper_core::types::managed_meta::MetaUpdatePayload;

/// Ingest a resource, then update its meta via PUT /api/resources/:id/meta,
/// verifying the response and that title cascades to kb_resources.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_meta_cascades_title(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    // Ensure profile exists.
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    // Create a context for the test.
    app.client
        .contexts()
        .create("meta-test")
        .await
        .expect("context create failed");

    // Ingest a resource to get a manifest row.
    let payload = IngestPayload {
        title: "Meta Test Doc".to_string(),
        origin_uri: "test://e2e/meta-test".to_string(),
        context_name: "meta-test".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: "meta0test0000000000000000000000000000000000000000000000000000000"
            .to_string(),
        slug: "meta-test-doc".to_string(),
        content: "# Meta Test\n\nContent for meta testing.".to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: pack_chunks(&[]).expect("encode empty chunks"),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest create failed");

    assert_eq!(resource.title, "Meta Test Doc");

    // Build meta update payload with a new title in managed_meta.
    let managed_meta = serde_json::json!({
        "temper-type": "research",
        "title": "Updated Meta Title",
    });
    let open_meta = serde_json::json!({
        "tags": ["test", "meta"],
    });

    let meta_payload = MetaUpdatePayload {
        resource_id: resource.id,
        managed_meta,
        open_meta,
        managed_hash: "sha256:placeholder_managed_hash".to_string(),
        open_hash: "sha256:placeholder_open_hash".to_string(),
    };

    // PUT /api/resources/:id/meta via reqwest
    let resp = app
        .reqwest_client
        .put(app.url(&format!("/api/resources/{}/meta", resource.id)))
        .header("Authorization", format!("Bearer {}", app.token))
        .json(&meta_payload)
        .send()
        .await
        .expect("meta update request failed");

    assert_eq!(
        resp.status(),
        reqwest::StatusCode::OK,
        "expected 200, got {}",
        resp.status()
    );

    let body: serde_json::Value = resp.json().await.expect("parse response body");
    assert_eq!(body["updated"], true);
    assert_eq!(body["resource_id"], resource.id.to_string());

    // Verify title was cascaded to kb_resources.
    let fetched = app
        .client
        .resources()
        .get(resource.id)
        .await
        .expect("resource get after meta update failed");

    assert_eq!(
        fetched.title, "Updated Meta Title",
        "title should have been cascaded from managed_meta"
    );
}
