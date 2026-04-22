#![cfg(feature = "test-db")]

mod common;

use common::fixtures::{self, RESEARCH_DOC_TYPE_ID, TEMPER_CONTEXT_ID};
use serde_json::json;
use sqlx::PgPool;
use temper_api::services::ingest_service::{self, CreateResourceParams};
use temper_core::types::ids::{ContextId, ProfileId};
use temper_core::types::ingest::{pack_chunks, PackedChunk};
use uuid::Uuid;

fn make_packed_chunks() -> String {
    let chunks = vec![
        PackedChunk {
            chunk_index: 0,
            header_path: String::new(),
            heading_depth: 0,
            content: "hello".into(),
            content_hash: "h0".into(),
            embedding: vec![0.0; 768],
        },
        PackedChunk {
            chunk_index: 1,
            header_path: String::new(),
            heading_depth: 0,
            content: "world".into(),
            content_hash: "h1".into(),
            embedding: vec![0.0; 768],
        },
    ];
    pack_chunks(&chunks).expect("pack_chunks")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_resource_links_revision_to_create_audit(pool: PgPool) {
    fixtures::clean_and_seed(&pool).await;
    let profile_uuid = fixtures::create_test_profile(&pool, "alice@test.local").await;
    let context_uuid = Uuid::parse_str(TEMPER_CONTEXT_ID).unwrap();
    let doc_type_uuid = Uuid::parse_str(RESEARCH_DOC_TYPE_ID).unwrap();

    let packed = make_packed_chunks();
    let empty_meta = json!({});

    let resource = ingest_service::create_resource_with_manifest(
        &pool,
        &CreateResourceParams {
            profile_id: ProfileId::from(profile_uuid),
            device_id: "dev",
            context_id: ContextId::from(context_uuid),
            doc_type_id: doc_type_uuid,
            doc_type_name: "research",
            title: "T",
            slug: Some("revision-test"),
            origin_uri: "test://revision-test",
            content_hash: "sha256:deadbeef",
            managed_meta: &empty_meta,
            open_meta: &empty_meta,
            chunks_packed: Some(&packed),
        },
    )
    .await
    .expect("create_resource_with_manifest");

    let (rev_id, rev_audit, rev_body_hash, rev_chunk_count): (Uuid, Option<Uuid>, String, i32) =
        sqlx::query_as(
            "SELECT id, audit_id, body_hash, chunk_count FROM kb_resource_revisions \
             WHERE resource_id = $1",
        )
        .bind(*resource.id)
        .fetch_one(&pool)
        .await
        .unwrap();

    let create_audit_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_resource_audits \
         WHERE resource_id = $1 AND action = 'create'",
    )
    .bind(*resource.id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        rev_audit,
        Some(create_audit_id),
        "revision.audit_id = create audit.id"
    );
    assert_eq!(rev_body_hash, "sha256:deadbeef");
    assert_eq!(rev_chunk_count, 2);
    assert_ne!(rev_id, Uuid::nil());

    let chunk_revs: Vec<Uuid> =
        sqlx::query_scalar("SELECT first_revision_id FROM kb_chunks WHERE resource_id = $1")
            .bind(*resource.id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(chunk_revs.len(), 2);
    assert!(
        chunk_revs.iter().all(|r| *r == rev_id),
        "both chunks reference the newly-created revision"
    );
}
