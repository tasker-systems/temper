//! Team builders: the one place a test makes a `kb_teams` row or a native membership.

use sqlx::PgPool;
use uuid::Uuid;

/// A team whose slug and name are both `slug`, with no members.
pub async fn insert(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar!(
        "INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id",
        slug,
    )
    .fetch_one(pool)
    .await
    .expect("insert team")
}

/// Make `profile` a native member of `team` with `role` (a `team_role` label), or change the role
/// it already holds. Written directly, not through the membership writer, so it fires no event and
/// takes no team lock: a test that needs the routed writer uses [`super::hold_member_removal`]'s
/// family instead.
pub async fn set_member(pool: &PgPool, team: Uuid, profile: Uuid, role: &str) {
    sqlx::query!(
        "INSERT INTO kb_team_members (team_id, profile_id, role, source) \
         VALUES ($1, $2, $3::text::team_role, 'native'::team_member_source) \
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
        team,
        profile,
        role,
    )
    .execute(pool)
    .await
    .expect("set team member");
}
