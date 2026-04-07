#![cfg(feature = "test-db")]

mod common;

use temper_api::services::doc_type_service;

/// list_all returns system doc types.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_all_doc_types(pool: sqlx::PgPool) {
    let rows = doc_type_service::list_all(&pool)
        .await
        .expect("list_all failed");
    assert!(!rows.is_empty());
    assert!(rows.iter().any(|r| r.name == "research"));
    assert!(rows.iter().any(|r| r.name == "session"));
}
