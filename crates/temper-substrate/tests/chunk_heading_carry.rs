#![cfg(feature = "artifact-tests")]
//! A resource_create whose sidecar carries header_path + heading_depth persists them onto kb_chunks,
//! so a downstream read can reconstruct headed markdown identically to production (§8 carry-as-is).
mod common;
use sqlx::Row;

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn chunk_carries_header_path_and_heading_depth(pool: sqlx::PgPool) {
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    // fire a resource_create with a single block, one chunk carrying heading metadata in the sidecar.
    let resource = common::fire_resource_with_headed_chunk(&pool, "Intro > Goals", 2_i16).await;
    let row = sqlx::query("SELECT header_path, heading_depth FROM kb_chunks WHERE resource_id=$1")
        .bind(resource)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("header_path"), "Intro > Goals");
    assert_eq!(row.get::<i16, _>("heading_depth"), 2);
}
