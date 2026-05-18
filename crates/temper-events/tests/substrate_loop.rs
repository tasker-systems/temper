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

#[sqlx::test(migrator = "MIGRATOR")]
async fn move_entity_to_other_profile(pool: PgPool) {
    use temper_events::move_entity;

    let (entity, source_profile) = create_entity(&pool, "alice").await.unwrap();
    let (_, target_profile) = create_entity(&pool, "bob").await.unwrap();

    let moved = move_entity(&pool, entity.id, target_profile.id)
        .await
        .unwrap();
    assert_eq!(moved.profile_id, target_profile.id);

    // Source profile still exists, just unreferenced.
    let source_still_present: bool = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM event_substrate.profiles WHERE id = $1)",
        source_profile.id,
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or(false);
    assert!(
        source_still_present,
        "move_entity must not auto-discard source profile"
    );
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn discard_empty_profile_succeeds(pool: PgPool) {
    use temper_events::{discard_profile, move_entity};

    let (entity, source_profile) = create_entity(&pool, "alice").await.unwrap();
    let (_, target_profile) = create_entity(&pool, "bob").await.unwrap();
    move_entity(&pool, entity.id, target_profile.id)
        .await
        .unwrap();

    discard_profile(&pool, source_profile.id).await.unwrap();

    let still_present: bool = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM event_substrate.profiles WHERE id = $1)",
        source_profile.id,
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap_or(false);
    assert!(!still_present);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn discard_profile_with_entities_errors(pool: PgPool) {
    use temper_events::{discard_profile, LedgerError};

    let (_, profile) = create_entity(&pool, "alice").await.unwrap();
    let err = discard_profile(&pool, profile.id).await.unwrap_err();
    assert!(matches!(err, LedgerError::ProfileNotEmpty(id) if id == profile.id));
}
