#![cfg(feature = "test-db")]
//! L0 kernel cognitive map: the public, root-team-joined system-default cogmap,
//! born deterministically by migration 20260625000001 via cogmap_genesis.

use sqlx::PgPool;
use uuid::Uuid;

const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);
const L0_TELOS: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000002);

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_cogmap_is_born_at_migration(pool: PgPool) {
    // The L0 cogmap exists with the reserved id, the canonical name, and its telos.
    let (name, telos): (String, Uuid) =
        sqlx::query_as("SELECT name, telos_resource_id FROM kb_cogmaps WHERE id = $1")
            .bind(L0_COGMAP)
            .fetch_one(&pool)
            .await
            .expect("L0 cogmap must exist after migrations");
    assert_eq!(name, "system-default");
    assert_eq!(telos, L0_TELOS);

    // Its telos resource exists and is stamped doc_type = cogmap_charter (genesis does this).
    let (title,): (String,) = sqlx::query_as("SELECT title FROM kb_resources WHERE id = $1")
        .bind(L0_TELOS)
        .fetch_one(&pool)
        .await
        .expect("L0 telos resource must exist");
    assert_eq!(title, "What Temper Is");

    let doc_type: serde_json::Value = sqlx::query_scalar(
        "SELECT property_value FROM kb_properties \
         WHERE owner_table = 'kb_resources' AND owner_id = $1 AND property_key = 'doc_type'",
    )
    .bind(L0_TELOS)
    .fetch_one(&pool)
    .await
    .expect("L0 telos must have a doc_type property");
    assert_eq!(doc_type, serde_json::json!("cogmap_charter"));
}
