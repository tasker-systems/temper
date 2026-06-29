#![cfg(feature = "test-db")]

mod common;

use temper_workflow::types::resource::{
    ResourceCreateRequest, ResourceListParams, ResourceUpdateRequest,
};

/// Create a resource via the client, then get it by ID and verify fields match.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_create_and_get(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let context = app
        .client
        .contexts()
        .create("e2e-resource-create-get", None)
        .await
        .expect("context create failed");

    let request = ResourceCreateRequest {
        kb_context_id: context.id.into(),
        doc_type: "research".to_string(),
        origin_uri: "test://e2e/resource-create-get".to_string(),
        title: "E2E Create & Get Test".to_string(),
        slug: Some("e2e-create-get-test".to_string()),
        act: Default::default(),
    };

    let created = app
        .client
        .resources()
        .create(&request)
        .await
        .expect("resource create failed");

    assert_eq!(created.title, "E2E Create & Get Test");
    assert_eq!(created.origin_uri, "test://e2e/resource-create-get");
    assert_eq!(created.kb_context_id, context.id);
    assert_eq!(created.doc_type_name, "research");
    assert!(created.is_active);

    let fetched = app
        .client
        .resources()
        .get(created.id.into())
        .await
        .expect("resource get failed");

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.title, created.title);
    assert_eq!(fetched.origin_uri, created.origin_uri);
}

/// Create a resource, update its title, get again, verify the new title.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_update(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let context = app
        .client
        .contexts()
        .create("e2e-resource-update", None)
        .await
        .expect("context create failed");

    let created = app
        .client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id.into(),
            doc_type: "research".to_string(),
            origin_uri: "test://e2e/resource-update".to_string(),
            title: "Original Title".to_string(),
            slug: None,
            act: Default::default(),
        })
        .await
        .expect("resource create failed");

    assert_eq!(created.title, "Original Title");

    let updated = app
        .client
        .resources()
        .update(
            created.id.into(),
            &ResourceUpdateRequest {
                title: Some("Updated Title".to_string()),
                slug: None,
                ..Default::default()
            },
        )
        .await
        .expect("resource update failed");

    assert_eq!(updated.title, "Updated Title");
    assert_eq!(updated.id, created.id);

    let fetched = app
        .client
        .resources()
        .get(created.id.into())
        .await
        .expect("resource get after update failed");

    assert_eq!(fetched.title, "Updated Title");
}

/// Create a resource, delete it, verify it no longer appears in list.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_delete(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let context = app
        .client
        .contexts()
        .create("e2e-resource-delete", None)
        .await
        .expect("context create failed");

    let created = app
        .client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id.into(),
            doc_type: "research".to_string(),
            origin_uri: "test://e2e/resource-delete".to_string(),
            title: "Resource To Delete".to_string(),
            slug: None,
            act: Default::default(),
        })
        .await
        .expect("resource create failed");

    let delete_resp = app
        .client
        .resources()
        .delete(created.id.into(), &Default::default())
        .await
        .expect("resource delete failed");

    assert!(
        delete_resp.deleted,
        "delete response should report deleted=true"
    );

    let resources = app
        .client
        .resources()
        .list(&ResourceListParams {
            context_ref: Some(context.id.to_string()),
            limit: Some(50),
            ..Default::default()
        })
        .await
        .expect("resource list after delete failed");

    assert!(
        !resources.rows.iter().any(|r| r.id == created.id),
        "deleted resource should not appear in list"
    );
}

/// Timestamps are real and stable across reads (not read-time `now()`), and an
/// update advances `updated` without moving `created`. Pre-shim-exit the backend
/// stamped `Utc::now()` per read, so two reads of the same resource returned
/// different `created` — this test pins the native, event-sourced behavior.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_timestamps_are_real_and_stable(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");
    let context = app
        .client
        .contexts()
        .create("e2e-resource-timestamps", None)
        .await
        .expect("context create failed");

    let created = app
        .client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id.into(),
            doc_type: "research".to_string(),
            origin_uri: "test://e2e/resource-timestamps".to_string(),
            title: "Timestamp Test".to_string(),
            slug: None,
            act: Default::default(),
        })
        .await
        .expect("resource create failed");

    let first = app
        .client
        .resources()
        .get(created.id.into())
        .await
        .expect("first get failed");
    let second = app
        .client
        .resources()
        .get(created.id.into())
        .await
        .expect("second get failed");

    assert_eq!(
        first.created, second.created,
        "created must be stable across reads, not read-time now()"
    );
    assert_eq!(
        first.updated, second.updated,
        "updated must be stable across reads"
    );

    app.client
        .resources()
        .update(
            created.id.into(),
            &ResourceUpdateRequest {
                title: Some("Timestamp Test v2".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("update failed");

    let after = app
        .client
        .resources()
        .get(created.id.into())
        .await
        .expect("get after update failed");
    assert_eq!(
        after.created, first.created,
        "created must not change on update"
    );
    assert!(
        after.updated >= first.updated,
        "updated must advance (or hold) after an update"
    );
}

/// The native ResourceRow drops the four shim fields (kb_doc_type_id, slug,
/// managed_hash, open_hash) and keeps name-only doc type. Asserts on the
/// serialized wire shape so it fails (red) while the fields still exist.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_row_native_shape_drops_shim_fields(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");
    let context = app
        .client
        .contexts()
        .create("e2e-native-shape", None)
        .await
        .expect("context create failed");

    let created = app
        .client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id.into(),
            doc_type: "research".to_string(),
            origin_uri: "test://e2e/native-shape".to_string(),
            title: "Native Shape".to_string(),
            slug: None,
            act: Default::default(),
        })
        .await
        .expect("resource create failed");

    let fetched = app
        .client
        .resources()
        .get(created.id.into())
        .await
        .expect("get failed");

    let json = serde_json::to_value(&fetched).expect("serialize ResourceRow");
    let obj = json
        .as_object()
        .expect("ResourceRow serializes to an object");
    for k in ["kb_doc_type_id", "slug", "managed_hash", "open_hash"] {
        assert!(
            !obj.contains_key(k),
            "native ResourceRow must drop `{k}`, got: {json}"
        );
    }
    assert_eq!(
        obj.get("doc_type_name").and_then(|v| v.as_str()),
        Some("research"),
        "native ResourceRow keeps name-only doc type"
    );
}

/// Create 3 resources, list with limit=2, verify exactly 2 returned.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_list_pagination(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    let context = app
        .client
        .contexts()
        .create("e2e-resource-pagination", None)
        .await
        .expect("context create failed");

    for i in 1..=3 {
        app.client
            .resources()
            .create(&ResourceCreateRequest {
                kb_context_id: context.id.into(),
                doc_type: "research".to_string(),
                origin_uri: format!("test://e2e/resource-page/{i}"),
                title: format!("Pagination Resource {i}"),
                slug: None,
                act: Default::default(),
            })
            .await
            .unwrap_or_else(|e| panic!("resource create {i} failed: {e}"));
    }

    let page = app
        .client
        .resources()
        .list(&ResourceListParams {
            context_ref: Some(context.id.to_string()),
            limit: Some(2),
            ..Default::default()
        })
        .await
        .expect("resource list with limit failed");

    assert_eq!(
        page.rows.len(),
        2,
        "expected 2 resources with limit=2, got {}",
        page.rows.len()
    );
}
