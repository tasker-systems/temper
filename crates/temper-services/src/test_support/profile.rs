//! Profile builders: the one place a test makes a `kb_profiles` row.
//!
//! A profile made here is **born `Denied`** (see the parent module): it has no standing row, so
//! anything that admits a caller refuses it until the test calls [`super::approve`]. That is the
//! shape every authority resolver test wants, since resolution reads reach, not standing.

use sqlx::PgPool;
use uuid::Uuid;

/// A bare profile whose handle and display name are both `handle`. Handles are unique, so a test
/// that makes two profiles gives them two handles. The insert's trigger also makes the profile's
/// personal team; [`personal_team`] finds it.
pub async fn insert(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar!(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        handle,
    )
    .fetch_one(pool)
    .await
    .expect("insert profile")
}

/// [`insert`] with a caller-chosen id, for a test whose handle is derived from the id (a UUIDv7 is
/// unique where a counter or a truncated id is not).
pub async fn insert_with_id(pool: &PgPool, id: Uuid, handle: &str) {
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)",
        id,
        handle,
    )
    .execute(pool)
    .await
    .expect("insert profile");
}

/// The personal team every profile is born with (`kb_teams.personal_of`).
pub async fn personal_team(pool: &PgPool, profile: Uuid) -> Uuid {
    sqlx::query_scalar!("SELECT id FROM kb_teams WHERE personal_of = $1", profile)
        .fetch_one(pool)
        .await
        .expect("personal team")
}
