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

        content_hash: "synctest00000000000000000000000000000000000000000000000000000000"
            .to_string(),
        slug: "sync-test-doc".to_string(),

        content: "# Sync Test\n\nContent for sync testing.".to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
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

        content_hash: content_hash.clone(),
        slug: "sync-match-doc".to_string(),

        content: "# Match\n\nSame on both sides.".to_string(),
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
                    managed_hash: String::new(),
                    remote_managed_hash: String::new(),
                    open_hash: String::new(),
                    remote_open_hash: String::new(),
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

        content_hash: "old0000000000000000000000000000000000000000000000000000000000000"
            .to_string(),
        slug: "sync-complete-doc".to_string(),

        content: "# Complete\n\nFor sync complete testing.".to_string(),
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

/// GET /api/sync/manifest — empty vault returns empty manifest.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_manifest_empty(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let resp = app
        .client
        .sync()
        .manifest()
        .await
        .expect("sync manifest failed");

    // New profile with no resources — manifest should be empty.
    // (Seed resource belongs to system profile, not this test user.)
    assert!(
        resp.items.is_empty(),
        "expected empty manifest for new profile, got {} items",
        resp.items.len()
    );
}

/// GET /api/sync/manifest — returns ingested resources with correct metadata.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_manifest_returns_resources(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("mfst-res")
        .await
        .expect("context create failed");

    let content_hash =
        "mfstres000000000000000000000000000000000000000000000000000000000".to_string();

    let payload = IngestPayload {
        title: "Manifest Res Doc".to_string(),
        origin_uri: "test://e2e/mfst-res".to_string(),
        context_name: "mfst-res".to_string(),
        doc_type_name: "research".to_string(),

        content_hash: content_hash.clone(),
        slug: "mfst-res-doc".to_string(),

        content: "# Manifest Test\n\nContent for manifest testing.".to_string(),
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
        .expect("ingest failed");

    let resp = app
        .client
        .sync()
        .manifest()
        .await
        .expect("sync manifest failed");

    assert!(
        !resp.items.is_empty(),
        "expected at least one item in manifest"
    );

    let item = resp
        .items
        .iter()
        .find(|i| i.resource_id == resource.id)
        .expect("expected ingested resource in manifest");

    assert_eq!(item.context, "mfst-res");
    assert_eq!(item.doc_type, "research");
    assert_eq!(item.slug, "mfst-res-doc");
    assert_eq!(item.content_hash, content_hash);
    assert!(
        item.uri.contains(&resource.id.to_string()),
        "URI should contain resource ID"
    );
}

/// GET /api/sync/manifest — resource without audit rows returns null last_audit_id.
///
/// Regression test: the sqlx query_as! macro inferred last_audit_id as non-null
/// from local dev data. In production, resources can exist without audit rows
/// (e.g. migrated data), causing a runtime decode error on column 7.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_manifest_handles_null_last_audit_id(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("mfst-null-audit")
        .await
        .expect("context create failed");

    let payload = IngestPayload {
        title: "No Audit Doc".to_string(),
        origin_uri: "test://e2e/mfst-null-audit".to_string(),
        context_name: "mfst-null-audit".to_string(),
        doc_type_name: "research".to_string(),
        content_hash: "nullaudit0000000000000000000000000000000000000000000000000000000"
            .to_string(),
        slug: "no-audit-doc".to_string(),
        content: "# No Audit\n\nResource with audit rows removed.".to_string(),
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
        .expect("ingest failed");

    // Delete audit rows to simulate a resource without audit trail
    sqlx::query("DELETE FROM kb_resource_audits WHERE resource_id = $1")
        .bind(resource.id)
        .execute(&pool)
        .await
        .expect("delete audit rows");

    // Manifest should still work — last_audit_id will be NULL
    let resp = app
        .client
        .sync()
        .manifest()
        .await
        .expect("sync manifest should handle null last_audit_id");

    let item = resp
        .items
        .iter()
        .find(|i| i.resource_id == resource.id)
        .expect("resource should appear in manifest");

    assert!(
        item.last_audit_id.is_none(),
        "last_audit_id should be None after removing audit rows"
    );
}

/// GET /api/sync/manifest — inactive resources are excluded.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn sync_manifest_excludes_inactive(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    app.client
        .contexts()
        .create("manifest-inactive")
        .await
        .expect("context create failed");

    let payload = IngestPayload {
        title: "Will Be Deleted".to_string(),
        origin_uri: "test://e2e/sync-manifest-inactive".to_string(),
        context_name: "manifest-inactive".to_string(),
        doc_type_name: "research".to_string(),

        content_hash: "inactive00000000000000000000000000000000000000000000000000000000"
            .to_string(),
        slug: "will-be-deleted".to_string(),

        content: "# Will Be Deleted".to_string(),
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
        .expect("ingest failed");

    // Delete the resource (soft delete — sets is_active=false)
    app.client
        .resources()
        .delete(resource.id.into())
        .await
        .expect("delete failed");

    let resp = app
        .client
        .sync()
        .manifest()
        .await
        .expect("sync manifest failed");

    assert!(
        !resp.items.iter().any(|i| i.resource_id == resource.id),
        "deleted resource should not appear in manifest"
    );
}
