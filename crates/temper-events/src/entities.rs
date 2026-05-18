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
