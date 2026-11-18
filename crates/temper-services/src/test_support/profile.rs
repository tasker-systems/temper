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

/// A profile seeded the way the integration suites seed a door caller: a fresh UUIDv7 id, the
/// handle `<email local part>-<first 8 hex of the id>`, and both display name and email set to
/// `email`. Answers `(id, handle)`; the handle is what the caller's emitters are named after.
///
/// The 8-hex suffix is the incumbent shape, kept so a test that asserts on a handle reads the same.
/// Two v7 ids minted in one millisecond can share it; give such profiles different emails.
pub async fn insert_with_email(pool: &PgPool, email: &str) -> (Uuid, String) {
    let id = Uuid::now_v7();
    let local = email.split('@').next().unwrap_or("test-user");
    let handle = format!("{local}-{}", &id.simple().to_string()[..8]);
    sqlx::query!(
        "INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1, $2, $3, $4)",
        id,
        handle,
        email,
        email,
    )
    .execute(pool)
    .await
    .expect("insert profile");
    (id, handle)
}

/// The personal team every profile is born with (`kb_teams.personal_of`).
pub async fn personal_team(pool: &PgPool, profile: Uuid) -> Uuid {
    sqlx::query_scalar!("SELECT id FROM kb_teams WHERE personal_of = $1", profile)
        .fetch_one(pool)
        .await
        .expect("personal team")
}
