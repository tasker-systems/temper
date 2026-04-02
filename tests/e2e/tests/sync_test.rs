#![cfg(feature = "test-db")]

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload};
use temper_core::types::sync::{
    MergedResource, SyncCompleteRequest, SyncContextEntries, SyncManifestEntry, SyncStatusRequest,
};

/// POST /api/sync/status — empty manifest returns empty diff.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_status_empty_manifest(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let resp = app
        .client
        .sync()
        .status(&SyncStatusRequest { contexts: vec![] })
        .await
        .expect("sync status failed");

    assert!(resp.to_push.is_empty());
    assert!(resp.to_pull.is_empty());
    assert!(resp.conflicts.is_empty());
    assert!(resp.removed.is_empty());
}

/// POST /api/sync/status — server-only resource appears as to_pull.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_status_detects_server_resource(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    // Create a context and ingest a resource so the server has something.
    app.client
        .contexts()
        .create("sync-test")
        .await
        .expect("context create failed");

    let payload = IngestPayload {
        title: "Sync Test Doc".to_string(),
        origin_uri: "test://e2e/sync-status".to_string(),
        context_name: "sync-test".to_string(),
        doc_type_name: "research".to_string(),
        resource_mode: "imported".to_string(),
        content_hash: "synctest00000000000000000000000000000000000000000000000000000000"
            .to_string(),
        slug: "sync-test-doc".to_string(),
        mimetype: "text/markdown".to_string(),
        content: "# Sync Test\n\nContent for sync testing.".to_string(),
        metadata: None,
        chunks_packed: pack_chunks(&[]).expect("encode empty chunks"),
    };

    app.client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");

    // Send an empty manifest for the context — server should tell us to pull.
    let resp = app
        .client
        .sync()
        .status(&SyncStatusRequest {
            contexts: vec![SyncContextEntries {
                name: "sync-test".to_string(),
                entries: vec![],
            }],
        })
        .await
        .expect("sync status failed");

    assert!(
        !resp.to_pull.is_empty(),
        "expected server-only resource in to_pull, got: {resp:?}"
    );
    // URIs are kb://context/doc_type/uuid format.
    assert!(
        resp.to_pull.iter().any(|p| p.uri.contains("sync-test")),
        "expected sync-test context in to_pull URIs, got: {:?}",
        resp.to_pull
    );
}

/// POST /api/sync/status — matching hash means nothing to sync.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_status_matching_hash_no_diff(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("sync-match")
        .await
        .expect("context create failed");

    let content_hash =
        "matchtest0000000000000000000000000000000000000000000000000000000".to_string();

    let payload = IngestPayload {
        title: "Matching Hash Doc".to_string(),
        origin_uri: "test://e2e/sync-match".to_string(),
        context_name: "sync-match".to_string(),
        doc_type_name: "research".to_string(),
        resource_mode: "imported".to_string(),
        content_hash: content_hash.clone(),
        slug: "sync-match-doc".to_string(),
        mimetype: "text/markdown".to_string(),
        content: "# Match\n\nSame on both sides.".to_string(),
        metadata: None,
        chunks_packed: pack_chunks(&[]).expect("encode empty chunks"),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");

    // Build the kb:// URI that the sync SQL function expects.
    let kb_uri = format!("kb://sync-match/research/{}", resource.id);

    // Client manifest matches server — no diff expected.
    let resp = app
        .client
        .sync()
        .status(&SyncStatusRequest {
            contexts: vec![SyncContextEntries {
                name: "sync-match".to_string(),
                entries: vec![SyncManifestEntry {
                    uri: kb_uri.clone(),
                    local_hash: content_hash.clone(),
                    remote_hash: content_hash,
                }],
            }],
        })
        .await
        .expect("sync status failed");

    // With matching hashes, nothing should be pushed or pulled for this resource.
    let our_uri = &kb_uri;
    assert!(
        !resp.to_push.iter().any(|p| &p.uri == our_uri),
        "matching hash should not appear in to_push"
    );
    assert!(
        !resp.to_pull.iter().any(|p| &p.uri == our_uri),
        "matching hash should not appear in to_pull"
    );
}

/// POST /api/sync/complete — finalize with empty merged_resources.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_complete_empty_round(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let resp = app
        .client
        .sync()
        .complete(&SyncCompleteRequest {
            device_id: "e2e-test-device".to_string(),
            merged_resources: vec![],
        })
        .await
        .expect("sync complete failed");

    assert_eq!(resp.updated_count, 0);
    // last_sync_at should be recent (within last 10 seconds).
    let age = chrono::Utc::now() - resp.last_sync_at;
    assert!(
        age.num_seconds() < 10,
        "last_sync_at should be recent, was {age:?} ago"
    );
}

/// POST /api/sync/complete — update content hash for a merged resource.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_complete_updates_content_hash(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("sync-complete")
        .await
        .expect("context create failed");

    let payload = IngestPayload {
        title: "Complete Test Doc".to_string(),
        origin_uri: "test://e2e/sync-complete".to_string(),
        context_name: "sync-complete".to_string(),
        doc_type_name: "research".to_string(),
        resource_mode: "imported".to_string(),
        content_hash: "old0000000000000000000000000000000000000000000000000000000000000"
            .to_string(),
        slug: "sync-complete-doc".to_string(),
        mimetype: "text/markdown".to_string(),
        content: "# Complete\n\nFor sync complete testing.".to_string(),
        metadata: None,
        chunks_packed: pack_chunks(&[]).expect("encode empty chunks"),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");

    let new_hash = "new0000000000000000000000000000000000000000000000000000000000000".to_string();

    let resp = app
        .client
        .sync()
        .complete(&SyncCompleteRequest {
            device_id: "e2e-test-device".to_string(),
            merged_resources: vec![MergedResource {
                resource_id: resource.id,
                content_hash: new_hash,
            }],
        })
        .await
        .expect("sync complete failed");

    assert_eq!(resp.updated_count, 1);
}
