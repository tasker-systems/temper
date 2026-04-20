#![cfg(feature = "test-db")]

use sqlx::PgPool;

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn schema_has_kb_resource_revisions_table(pool: PgPool) {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
         WHERE table_name = 'kb_resource_revisions')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(exists, "kb_resource_revisions table must exist");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn kb_chunks_has_revision_columns(pool: PgPool) {
    let cols: Vec<String> = sqlx::query_scalar(
        "SELECT column_name FROM information_schema.columns \
         WHERE table_name = 'kb_chunks' \
           AND column_name IN ('first_revision_id', 'superseded_revision_id') \
         ORDER BY column_name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        cols,
        vec![
            "first_revision_id".to_string(),
            "superseded_revision_id".to_string()
        ]
    );
}
