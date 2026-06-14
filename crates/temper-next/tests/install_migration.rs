#![cfg(feature = "artifact-tests")]
//! Proves the install migration is additive: applied to a DB with `public` present, it creates the
//! temper_next namespace + tables and leaves public's table set unchanged.
mod common;
use sqlx::Row;

#[tokio::test]
async fn install_is_additive_and_creates_namespace() {
    let pool = temper_next::substrate::connect().await.unwrap();
    // public table count before (a clean migrated dev DB).
    let before: i64 =
        sqlx::query("SELECT count(*) FROM information_schema.tables WHERE table_schema='public'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get(0);
    // apply the generated install migration into a fresh temper_next.
    common::apply_install_migration(&pool).await;
    let after: i64 =
        sqlx::query("SELECT count(*) FROM information_schema.tables WHERE table_schema='public'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get(0);
    assert_eq!(before, after, "install migration must not touch public");
    let next_tables: i64 = sqlx::query(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema='temper_next'",
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .get(0);
    assert!(
        next_tables >= 20,
        "temper_next tables created, got {next_tables}"
    );
}
