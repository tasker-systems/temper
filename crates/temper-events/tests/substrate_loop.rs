#![cfg(feature = "test-db")]

use sqlx::PgPool;
use temper_events::{create_entity, MIGRATOR};

#[sqlx::test(migrator = "MIGRATOR")]
async fn create_entity_creates_default_profile(pool: PgPool) {
    let (entity, profile) = create_entity(&pool, "alice").await.expect("create_entity");

    assert_eq!(entity.name, "alice");
    assert_eq!(entity.profile_id, profile.id);
    assert_eq!(profile.name, "default profile for alice");
}
