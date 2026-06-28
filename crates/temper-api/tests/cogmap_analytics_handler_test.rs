#![cfg(feature = "test-db")]
//! `substrate_read::{cogmap_region_metrics_select, cogmap_analytics_select}` — the api-side
//! service-direct wrappers. Proves the readable path returns Ok against the root-joined L0 map (region
//! metrics empty until materialized; analytics Some with the L0 telos), and a non-readable map yields
//! empty / None.

use sqlx::PgPool;
use uuid::Uuid;

use temper_api::backend::substrate_read::{cogmap_analytics_select, cogmap_region_metrics_select};
use temper_core::types::ids::ProfileId;

mod common;

const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);
const L0_TELOS: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000002);

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_region_metrics_readable_empty(pool: PgPool) {
    let profile = common::fixtures::create_test_profile(&pool, "reader1@example.com").await;
    let rows = cogmap_region_metrics_select(&pool, ProfileId::from(profile), L0_COGMAP, None)
        .await
        .expect("readable L0 region-metrics must be Ok");
    assert!(rows.is_empty(), "L0 has no materialized regions yet: {rows:?}");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_analytics_readable_some_with_telos(pool: PgPool) {
    let profile = common::fixtures::create_test_profile(&pool, "reader2@example.com").await;
    // Promote to 'approved' so the sync_system_membership trigger adds this profile to
    // temper-system → cogmap_readable_by_profile(profile, L0) returns true.
    sqlx::query("UPDATE kb_profiles SET system_access = 'approved' WHERE id = $1")
        .bind(profile)
        .execute(&pool)
        .await
        .expect("promote to approved");
    let got = cogmap_analytics_select(&pool, ProfileId::from(profile), L0_COGMAP)
        .await
        .expect("readable L0 analytics must be Ok")
        .expect("L0 is root-joined → Some for an approved profile");
    assert_eq!(got.telos_resource_id, L0_TELOS, "L0 telos charter resource");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unknown_cogmap_metrics_empty_analytics_none(pool: PgPool) {
    let profile = common::fixtures::create_test_profile(&pool, "nobody@example.com").await;
    let unknown = Uuid::now_v7();
    let metrics = cogmap_region_metrics_select(&pool, ProfileId::from(profile), unknown, None)
        .await
        .expect("non-readable map metrics is empty, not an error");
    assert!(metrics.is_empty());
    let analytics = cogmap_analytics_select(&pool, ProfileId::from(profile), unknown)
        .await
        .expect("non-readable map analytics is None, not an error");
    assert!(analytics.is_none());
}
