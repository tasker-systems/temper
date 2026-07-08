#![cfg(feature = "test-db")]

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};

/// GET /api/resources/{id}/content — ingest then retrieve markdown content.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_content_retrieval(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("content-test", None)
        .await
        .expect("context create failed");

    let chunk_content = "# Content Test\n\nThis document tests the content retrieval endpoint.";

    let chunks = vec![PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: chunk_content.to_string(),
        content_hash: "cont0test0000000000000000000000000000000000000000000000000000000"
            .to_string(),
        embedding: vec![0.0_f32; 768],
    }];

    let payload = IngestPayload {
        goal: None,
        title: "Content Retrieval Doc".to_string(),
        origin_uri: "test://e2e/content-test".to_string(),
        context_ref: "@me/content-test".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(
            "cont0test0000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        content: chunk_content.to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        chunks_packed: Some(pack_chunks(&chunks).expect("encode chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");

    let content_resp = app
        .client
        .resources()
        .content(resource.id.into())
        .await
        .expect("content retrieval failed");

    assert_eq!(content_resp.resource_id, resource.id);
    assert!(
        content_resp.markdown.contains("Content Test"),
        "expected markdown to contain original content, got: {}",
        content_resp.markdown
    );
}
