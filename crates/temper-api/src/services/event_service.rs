use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;
use temper_core::types::ids::{ContextId, ProfileId};

/// The most recent event id produced against a context the profile owns (directly, or via a team it
/// belongs to). Returns `None` when the context has no events the profile may see. Post-collapse events
/// anchor via `producing_anchor` (no `kb_context_id`/`profile_id`/`resource_id` columns); the cursor is
/// the context's own event stream, gated by context ownership.
pub async fn latest_event_id_for_context(
    pool: &PgPool,
    profile_id: ProfileId,
    kb_context_id: ContextId,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"
        SELECT e.id AS "id!: Uuid"
          FROM kb_events e
         WHERE e.producing_anchor_table = 'kb_contexts'
           AND e.producing_anchor_id = $2
           AND EXISTS (                                   -- context-ownership gate
             SELECT 1 FROM kb_contexts c
              WHERE c.id = $2 AND (
                (c.owner_table='kb_profiles' AND c.owner_id = $1)
                OR (c.owner_table='kb_teams' AND c.owner_id IN
                     (SELECT team_id FROM kb_team_members WHERE profile_id = $1))))
         ORDER BY e.occurred_at DESC
         LIMIT 1
        "#,
        *profile_id,
        *kb_context_id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(id)
}
