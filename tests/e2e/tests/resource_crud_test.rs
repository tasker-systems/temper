#![cfg(feature = "test-db")]

mod common;

use uuid::Uuid;

use temper_core::types::resource::{
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
        .create("e2e-resource-create-get")
        .await
        .expect("context create failed");

    let doc_type_id = Uuid::parse_str(common::RESEARCH_DOC_TYPE_ID).expect("parse doc type UUID");

    let request = ResourceCreateRequest {
        kb_context_id: context.id,
        kb_doc_type_id: doc_type_id,
        origin_uri: "test://e2e/resource-create-get".to_string(),
        title: "E2E Create & Get Test".to_string(),
        slug: Some("e2e-create-get-test".to_string()),
        mimetype: Some("text/markdown".to_string()),
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
    assert_eq!(created.kb_doc_type_id, doc_type_id);
    assert!(created.is_active);

    let fetched = app
        .client
        .resources()
        .get(created.id)
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
        .create("e2e-resource-update")
        .await
        .expect("context create failed");

    let doc_type_id = Uuid::parse_str(common::RESEARCH_DOC_TYPE_ID).expect("parse doc type UUID");

    let created = app
        .client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id,
            kb_doc_type_id: doc_type_id,
            origin_uri: "test://e2e/resource-update".to_string(),
            title: "Original Title".to_string(),
            slug: None,
            mimetype: None,
        })
        .await
        .expect("resource create failed");

    assert_eq!(created.title, "Original Title");

    let updated = app
        .client
        .resources()
        .update(
            created.id,
            &ResourceUpdateRequest {
                title: Some("Updated Title".to_string()),
                slug: None,
                mimetype: None,
            },
        )
        .await
        .expect("resource update failed");

    assert_eq!(updated.title, "Updated Title");
    assert_eq!(updated.id, created.id);

    let fetched = app
        .client
        .resources()
        .get(created.id)
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
        .create("e2e-resource-delete")
        .await
        .expect("context create failed");

    let doc_type_id = Uuid::parse_str(common::RESEARCH_DOC_TYPE_ID).expect("parse doc type UUID");

    let created = app
        .client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id,
            kb_doc_type_id: doc_type_id,
            origin_uri: "test://e2e/resource-delete".to_string(),
            title: "Resource To Delete".to_string(),
            slug: None,
            mimetype: None,
        })
        .await
        .expect("resource create failed");

    let delete_resp = app
        .client
        .resources()
        .delete(created.id)
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
            kb_context_id: Some(context.id),
            limit: Some(50),
            offset: None,
        })
        .await
        .expect("resource list after delete failed");

    assert!(
        !resources.iter().any(|r| r.id == created.id),
        "deleted resource should not appear in list"
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
        .create("e2e-resource-pagination")
        .await
        .expect("context create failed");

    let doc_type_id = Uuid::parse_str(common::RESEARCH_DOC_TYPE_ID).expect("parse doc type UUID");

    for i in 1..=3 {
        app.client
            .resources()
            .create(&ResourceCreateRequest {
                kb_context_id: context.id,
                kb_doc_type_id: doc_type_id,
                origin_uri: format!("test://e2e/resource-page/{i}"),
                title: format!("Pagination Resource {i}"),
                slug: None,
                mimetype: None,
            })
            .await
            .unwrap_or_else(|e| panic!("resource create {i} failed: {e}"));
    }

    let page = app
        .client
        .resources()
        .list(&ResourceListParams {
            kb_context_id: Some(context.id),
            limit: Some(2),
            offset: None,
        })
        .await
        .expect("resource list with limit failed");

    assert_eq!(
        page.len(),
        2,
        "expected 2 resources with limit=2, got {}",
        page.len()
    );
}
