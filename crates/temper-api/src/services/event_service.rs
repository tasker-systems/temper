use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;

/// The most recent event id for a context, scoped to events the profile
/// may see. Returns `None` when the context has no visible events.
pub async fn latest_event_id_for_context(
    pool: &PgPool,
    profile_id: Uuid,
    kb_context_id: Uuid,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"
        WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
        SELECT e.id
          FROM kb_events e
         WHERE (e.profile_id = $1 OR e.resource_id IN (SELECT resource_id FROM visible))
           AND e.kb_context_id = $2
         ORDER BY e.created DESC
         LIMIT 1
        "#,
        profile_id,
        kb_context_id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(id)
}
