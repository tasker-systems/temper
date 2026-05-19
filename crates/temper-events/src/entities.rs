use sqlx::PgPool;
use uuid::Uuid;

use crate::errors::LedgerError;
use crate::types::{Entity, Profile};

pub async fn create_entity(pool: &PgPool, name: &str) -> Result<(Entity, Profile), LedgerError> {
    let mut tx = pool.begin().await?;

    let profile_id = Uuid::now_v7();
    let profile_name = format!("default profile for {name}");
    let profile = sqlx::query_as!(
        Profile,
        r#"
        INSERT INTO event_substrate.profiles (id, name)
        VALUES ($1, $2)
        RETURNING id, name, created_at
        "#,
        profile_id,
        profile_name,
    )
    .fetch_one(&mut *tx)
    .await?;

    let entity_id = Uuid::now_v7();
    let entity = sqlx::query_as!(
        Entity,
        r#"
        INSERT INTO event_substrate.entities (id, profile_id, name)
        VALUES ($1, $2, $3)
        RETURNING id, profile_id, name, created_at
        "#,
        entity_id,
        profile.id,
        name,
    )
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok((entity, profile))
}

pub async fn move_entity(
    pool: &PgPool,
    entity_id: Uuid,
    target_profile_id: Uuid,
) -> Result<Entity, LedgerError> {
    let entity = sqlx::query_as!(
        Entity,
        r#"
        UPDATE event_substrate.entities
           SET profile_id = $2
         WHERE id = $1
        RETURNING id, profile_id, name, created_at
        "#,
        entity_id,
        target_profile_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(LedgerError::UnknownEntity(entity_id))?;

    Ok(entity)
}

pub async fn discard_profile(pool: &PgPool, profile_id: Uuid) -> Result<(), LedgerError> {
    let mut tx = pool.begin().await?;

    let referencing_count: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM event_substrate.entities WHERE profile_id = $1",
        profile_id,
    )
    .fetch_one(&mut *tx)
    .await?
    .unwrap_or(0);

    if referencing_count > 0 {
        return Err(LedgerError::ProfileNotEmpty(profile_id));
    }

    let result = sqlx::query!(
        "DELETE FROM event_substrate.profiles WHERE id = $1",
        profile_id,
    )
    .execute(&mut *tx)
    .await?;

    if result.rows_affected() == 0 {
        return Err(LedgerError::ProfileNotEmpty(profile_id));
    }

    tx.commit().await?;
    Ok(())
}
