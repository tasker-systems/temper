#![cfg(feature = "test-db")]

mod common;

use serde_json::Value;
use sqlx::PgPool;
use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};

/// Round-trip test: ingest markdown with headings, read back via get_content,
/// verify heading markers (##, ###) are preserved in the reconstituted output.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_reconstitution_preserves_heading_markers(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let sub = format!("test-sub-{}", uuid::Uuid::new_v4());
    let email = format!("reconstitute-user-{}@example.com", uuid::Uuid::new_v4());
    let token = common::generate_test_jwt(&sub, &email);

    // First, hit /api/profile to auto-provision the user and their "default" context.
    let profile_resp = app
        .client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("profile request failed");
    assert_eq!(profile_resp.status().as_u16(), 200);

    // Build chunks that simulate what the CLI chunker produces for:
    //   "Preamble text.\n\n## Decision\n\nWe chose option B.\n\n### Rationale\n\nIt was simpler.\n\n## Implementation\n\nCode goes here."
    let fake_embedding = vec![0.0_f32; 768];
    let chunks = vec![
        PackedChunk {
            chunk_index: 0,
            header_path: String::new(),
            heading_depth: 0,
            content: "Preamble text.".to_string(),
            content_hash: "sha256:aaa".to_string(),
            embedding: fake_embedding.clone(),
        },
        PackedChunk {
            chunk_index: 1,
            header_path: "Decision".to_string(),
            heading_depth: 2,
            content: "We chose option B.".to_string(),
            content_hash: "sha256:bbb".to_string(),
            embedding: fake_embedding.clone(),
        },
        PackedChunk {
            chunk_index: 2,
            header_path: "Decision > Rationale".to_string(),
            heading_depth: 3,
            content: "It was simpler.".to_string(),
            content_hash: "sha256:ccc".to_string(),
            embedding: fake_embedding.clone(),
        },
        PackedChunk {
            chunk_index: 3,
            header_path: "Implementation".to_string(),
            heading_depth: 2,
            content: "Code goes here.".to_string(),
            content_hash: "sha256:ddd".to_string(),
            embedding: fake_embedding,
        },
    ];
    let chunks_packed = pack_chunks(&chunks).expect("pack_chunks failed");

    // Ingest via POST /api/ingest
    let origin_uri = format!("test://reconstitution-{}", uuid::Uuid::new_v4());
    let ingest_payload = IngestPayload {
        title: "Reconstitution Test Doc".to_string(),
        origin_uri,
        context_ref: "@me/default".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        slug: "reconstitution-test".to_string(),
        content: "Preamble text.\n\n## Decision\n\nWe chose option B.\n\n### Rationale\n\nIt was simpler.\n\n## Implementation\n\nCode goes here.".to_string(),
        managed_meta: None,
        chunks_packed: Some(chunks_packed),
        content_hash: None,
        metadata: None,
        open_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        goal: None,
        act: Default::default(),
        sources: Vec::new(),
    };

    let create_resp = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&ingest_payload)
        .send()
        .await
        .expect("ingest request failed");

    let status = create_resp.status().as_u16();
    let created: Value = create_resp.json().await.expect("expected JSON response");
    assert_eq!(status, 200, "ingest must return 200; body: {created}");

    let resource_id = created["id"]
        .as_str()
        .expect("id field missing from ingest response");

    // Read back via GET /api/resources/{id}/content
    let content_resp: Value = app
        .client
        .get(app.url(&format!("/api/resources/{resource_id}/content")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("content request failed")
        .json()
        .await
        .expect("expected JSON response");

    let output = content_resp["markdown"]
        .as_str()
        .expect("markdown field missing from content response");

    // Verify heading markers survived round-trip
    assert!(
        output.contains("## Decision"),
        "expected '## Decision' in output: {output}"
    );
    assert!(
        output.contains("### Rationale"),
        "expected '### Rationale' in output: {output}"
    );
    assert!(
        output.contains("## Implementation"),
        "expected '## Implementation' in output: {output}"
    );
    assert!(
        output.contains("Preamble text."),
        "expected preamble text in output: {output}"
    );

    // Verify no breadcrumb strings leaked through
    assert!(
        !output.contains("Decision > Rationale"),
        "breadcrumb 'Decision > Rationale' should not appear in output: {output}"
    );
}
