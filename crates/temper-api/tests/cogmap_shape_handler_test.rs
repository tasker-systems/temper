#![cfg(feature = "test-db")]
//! `substrate_read::cogmap_shape_select` — the api-side service-direct wrapper over the
//! `readback::cogmap_shape` binding. Proves the readable path returns Ok against the root-joined L0
//! map, and that a non-readable map yields an empty (not errored) result.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_services::backend::substrate_read::cogmap_shape_select;

mod common;

const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_shape_is_readable_and_returns_ok(pool: PgPool) {
    // L0 is root-joined → readable by any approved profile. No regions materialized → Ok(empty).
    let profile = common::fixtures::create_test_profile(&pool, "reader@example.com").await;
    let rows = cogmap_shape_select(&pool, ProfileId::from(profile), L0_COGMAP, None)
        .await
        .expect("readable L0 shape read must be Ok");
    assert!(
        rows.is_empty(),
        "L0 has no materialized regions yet: {rows:?}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unknown_cogmap_is_empty_not_error(pool: PgPool) {
    // A random cogmap id the profile cannot read: the in-SQL gate yields zero rows, never an error.
    let profile = common::fixtures::create_test_profile(&pool, "nobody@example.com").await;
    let rows = cogmap_shape_select(&pool, ProfileId::from(profile), Uuid::now_v7(), None)
        .await
        .expect("non-readable map is empty, not an error");
    assert!(rows.is_empty());
}
