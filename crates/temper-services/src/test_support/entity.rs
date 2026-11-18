//! Entity builders: the one place a test makes a `kb_entities` row, the emitter an act is recorded
//! under. A door names its emitter `<handle>@<surface>` (`resolve_emitter`), so a test that drives a
//! door through a profile names the entity that way.

use sqlx::PgPool;
use uuid::Uuid;

/// An emitter entity named `name` for `profile`, with empty metadata.
pub async fn insert(pool: &PgPool, profile: Uuid, name: &str) -> Uuid {
    sqlx::query_scalar!(
        "INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2) RETURNING id",
        profile,
        name,
    )
    .fetch_one(pool)
    .await
    .expect("insert entity")
}
