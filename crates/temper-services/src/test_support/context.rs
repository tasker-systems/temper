//! Context builders: the one place a test makes a `kb_contexts` row.

use sqlx::PgPool;
use uuid::Uuid;

/// A context owned by `owner`'s profile (not a team), with slug `slug` and display name `name`.
/// Slugs are unique per owner, so a test that gives one profile two contexts gives them two slugs.
pub async fn insert_personal(pool: &PgPool, owner: Uuid, slug: &str, name: &str) -> Uuid {
    sqlx::query_scalar!(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, $2, $3) RETURNING id",
        owner,
        slug,
        name,
    )
    .fetch_one(pool)
    .await
    .expect("insert personal context")
}
