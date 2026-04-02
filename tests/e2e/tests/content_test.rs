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
        .create("content-test")
        .await
        .expect("context create failed");

    let chunk_content = "# Content Test\n\nThis document tests the content retrieval endpoint.";

    let chunks = vec![PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        content: chunk_content.to_string(),
        content_hash: "cont0test0000000000000000000000000000000000000000000000000000000"
            .to_string(),
        embedding: vec![0.0_f32; 768],
    }];

    let payload = IngestPayload {
        title: "Content Retrieval Doc".to_string(),
        origin_uri: "test://e2e/content-test".to_string(),
        context_name: "content-test".to_string(),
        doc_type_name: "research".to_string(),
        resource_mode: "imported".to_string(),
        content_hash: "cont0test0000000000000000000000000000000000000000000000000000000"
            .to_string(),
        slug: "content-retrieval-doc".to_string(),
        mimetype: "text/markdown".to_string(),
        content: chunk_content.to_string(),
        metadata: None,
        chunks_packed: pack_chunks(&chunks).expect("encode chunks"),
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
        .content(resource.id)
        .await
        .expect("content retrieval failed");

    assert_eq!(content_resp.resource_id, resource.id);
    assert!(
        content_resp.markdown.contains("Content Test"),
        "expected markdown to contain original content, got: {}",
        content_resp.markdown
    );
}
