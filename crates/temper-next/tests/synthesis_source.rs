#![cfg(feature = "artifact-tests")]
//! synthesis::source reads active production-shape rows from public.* (the synthesis source).
//!
//! Unlike the scenario write-path tests (which OWN the shared `temper_next` namespace and reset it),
//! this test runs on its own ephemeral DB via `#[sqlx::test(migrator = ...)]` — the full migration
//! chain (including `20260613000001_install_temper_next.sql`) is applied, giving an empty migrated
//! `public` plus an empty `temper_next`. The prod-shape fixture seeds `public.*` only. Because each
//! test owns its own DB, these tests parallelize safely and are NOT in the `temper-next-write` group.

mod common;

#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn source_reads_active_resources_only(pool: sqlx::PgPool) {
    common::seed_prod_shape_fixture(&pool).await;
    let rows = temper_next::synthesis::source::active_resources(&pool)
        .await
        .unwrap();
    assert_eq!(
        rows.len(),
        3,
        "soft-deleted resource excluded (§0 active-only)"
    );
}
