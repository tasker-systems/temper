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

/// The surfaces a door caller acts through, in the order the suites seed them.
pub const DOOR_SURFACES: [&str; 3] = ["web", "cli", "mcp"];

/// One emitter per surface, named `<handle>@<surface>`, in `surfaces` order. Answers their ids in
/// the same order.
pub async fn insert_surfaces(
    pool: &PgPool,
    profile: Uuid,
    handle: &str,
    surfaces: &[&str],
) -> Vec<Uuid> {
    let mut ids = Vec::with_capacity(surfaces.len());
    for surface in surfaces {
        ids.push(insert(pool, profile, &format!("{handle}@{surface}")).await);
    }
    ids
}
