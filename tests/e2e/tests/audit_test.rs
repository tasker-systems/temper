#![cfg(feature = "test-db")]

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload};

/// Helper: create a context and ingest a resource, return (resource_id, context_name).
async fn ingest_test_resource(app: &common::E2eTestApp, suffix: &str) -> (uuid::Uuid, String) {
    let context_name = format!("e2e-audit-{suffix}");
    app.client
        .contexts()
        .create(&context_name)
        .await
        .expect("context create failed");

    let payload = IngestPayload {
        title: format!("Audit Test Doc {suffix}"),
        origin_uri: format!("test://e2e/audit-{suffix}"),
        context_name: context_name.clone(),
        doc_type_name: "research".to_string(),
        content_hash: Some(format!(
            "audit{suffix}000000000000000000000000000000000000000000000000000000000"
        )),
        slug: format!("audit-test-{suffix}"),
        content: format!("# Audit Test {suffix}\n\nContent for audit testing."),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest create failed");

    (resource.id.into(), context_name)
}

/// Ingest creates a resource_created event and a corresponding audit row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn audit_row_created_on_ingest(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let (resource_id, _ctx) = ingest_test_resource(&app, "create").await;

    // Verify audit row exists via direct DB query
    let audit_rows: Vec<(uuid::Uuid, String, String)> = sqlx::query_as(
        "SELECT id, action, body_hash FROM kb_resource_audits WHERE resource_id = $1 ORDER BY created",
    )
    .bind(resource_id)
    .fetch_all(&pool)
    .await
    .expect("query audit rows");

    assert_eq!(
        audit_rows.len(),
        1,
        "expected exactly one audit row after ingest"
    );
    assert_eq!(audit_rows[0].1, "create");
    assert!(!audit_rows[0].2.is_empty(), "body_hash should not be empty");
}

/// Updating a resource's body creates an update_body audit row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn audit_row_created_on_update(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let (resource_id, ctx) = ingest_test_resource(&app, "update").await;

    // Update the resource
    let update_payload = IngestPayload {
        title: "Audit Test Doc update - Updated".to_string(),
        origin_uri: "test://e2e/audit-update".to_string(),
        context_name: ctx,
        doc_type_name: "research".to_string(),
        content_hash: Some(
            "auditupd0000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        slug: "audit-test-update".to_string(),
        content: "# Updated\n\nNew content.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"date": "2026-04-10"})),
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[]).expect("encode empty chunks")),
    };

    app.client
        .ingest()
        .update(resource_id, &update_payload)
        .await
        .expect("ingest update failed");

    // Verify two audit rows: create + update_body
    let audit_rows: Vec<(String,)> = sqlx::query_as(
        "SELECT action FROM kb_resource_audits WHERE resource_id = $1 ORDER BY created",
    )
    .bind(resource_id)
    .fetch_all(&pool)
    .await
    .expect("query audit rows");

    assert_eq!(audit_rows.len(), 2);
    assert_eq!(audit_rows[0].0, "create");
    assert_eq!(audit_rows[1].0, "update_body");
}

/// Updating managed meta creates an update_meta audit row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn audit_row_created_on_meta_update(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let (resource_id, _ctx) = ingest_test_resource(&app, "meta").await;

    // Update meta via the API using raw reqwest (the typed client may not have this method)
    let meta_payload = serde_json::json!({
        "resource_id": resource_id.to_string(),
        "managed_meta": {"title": "Updated Title"},
        "open_meta": {},
        "managed_hash": "sha256:newmanaged",
        "open_hash": "sha256:newopen",
    });

    let url = app.url(&format!("/api/resources/{resource_id}/meta"));
    let resp = app
        .reqwest_client
        .put(&url)
        .bearer_auth(&app.token)
        .json(&meta_payload)
        .send()
        .await
        .expect("meta update request failed");

    assert!(
        resp.status().is_success(),
        "meta update failed: {}",
        resp.status()
    );

    // Verify audit rows: create + update_meta
    let audit_rows: Vec<(String,)> = sqlx::query_as(
        "SELECT action FROM kb_resource_audits WHERE resource_id = $1 ORDER BY created",
    )
    .bind(resource_id)
    .fetch_all(&pool)
    .await
    .expect("query audit rows");

    assert_eq!(audit_rows.len(), 2);
    assert_eq!(audit_rows[0].0, "create");
    assert_eq!(audit_rows[1].0, "update_meta");
}

/// Deleting a resource creates a delete audit row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn audit_row_created_on_delete(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let (resource_id, _ctx) = ingest_test_resource(&app, "delete").await;

    // Delete the resource
    app.client
        .resources()
        .delete(resource_id)
        .await
        .expect("delete failed");

    // Verify audit rows: create + delete
    let audit_rows: Vec<(String,)> = sqlx::query_as(
        "SELECT action FROM kb_resource_audits WHERE resource_id = $1 ORDER BY created",
    )
    .bind(resource_id)
    .fetch_all(&pool)
    .await
    .expect("query audit rows");

    assert_eq!(audit_rows.len(), 2);
    assert_eq!(audit_rows[0].0, "create");
    assert_eq!(audit_rows[1].0, "delete");
}

/// Audit rows link to valid events (foreign key integrity).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn audit_row_references_valid_event(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let (resource_id, _ctx) = ingest_test_resource(&app, "fk").await;

    // Verify the audit row's event_id exists in kb_events
    let valid_count: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) FROM kb_resource_audits a
        JOIN kb_events e ON e.id = a.event_id
        WHERE a.resource_id = $1
        "#,
    )
    .bind(resource_id)
    .fetch_one(&pool)
    .await
    .expect("join query");

    assert_eq!(valid_count.0, 1, "audit row should reference a valid event");
}

/// Fetch manifest includes last_audit_id for resources with audits.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn manifest_includes_last_audit_id(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let (resource_id, _ctx) = ingest_test_resource(&app, "manifest").await;

    // Fetch manifest via API
    let url = app.url("/api/sync/manifest");
    let resp = app
        .reqwest_client
        .get(&url)
        .bearer_auth(&app.token)
        .send()
        .await
        .expect("manifest request failed");

    assert!(resp.status().is_success());

    let body: serde_json::Value = resp.json().await.expect("parse manifest response");
    let items = body["items"].as_array().expect("items is array");

    let item = items
        .iter()
        .find(|i| i["resource_id"].as_str() == Some(&resource_id.to_string()))
        .expect("resource not found in manifest");

    assert!(
        item["last_audit_id"].is_string(),
        "last_audit_id should be present after ingest"
    );
}
