use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;

/// Row type matching the `kb_events` table.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct EventRow {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub client_id: String,
    pub kb_context_id: Option<Uuid>,
    pub resource_id: Option<Uuid>,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created: DateTime<Utc>,
}

/// Query parameters for listing events.
#[derive(Debug, Deserialize)]
pub struct EventListParams {
    /// Filter by resource ID.
    pub resource_id: Option<Uuid>,
    /// Filter by event type.
    pub event_type: Option<String>,
    /// Maximum results to return (default 50, max 200).
    pub limit: Option<i64>,
    /// Offset for pagination.
    pub offset: Option<i64>,
}

/// List events visible to the given profile.
///
/// Events are visible when:
/// - The actor (`profile_id`) is the authenticated profile, OR
/// - The event is associated with a resource visible to the profile.
pub async fn list_visible(
    pool: &PgPool,
    profile_id: Uuid,
    params: EventListParams,
) -> ApiResult<Vec<EventRow>> {
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0).max(0);

    let rows = match (params.resource_id, params.event_type.as_deref()) {
        (Some(rid), Some(etype)) => {
            sqlx::query_as::<_, EventRow>(
                r#"
                WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
                SELECT e.id, e.profile_id, e.client_id, e.kb_context_id,
                       e.resource_id, e.event_type, e.payload, e.created
                  FROM kb_events e
                 WHERE (e.profile_id = $1 OR e.resource_id IN (SELECT resource_id FROM visible))
                   AND e.resource_id = $2
                   AND e.event_type  = $3
                 ORDER BY e.created DESC
                 LIMIT $4 OFFSET $5
                "#,
            )
            .bind(profile_id)
            .bind(rid)
            .bind(etype)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (Some(rid), None) => {
            sqlx::query_as::<_, EventRow>(
                r#"
                WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
                SELECT e.id, e.profile_id, e.client_id, e.kb_context_id,
                       e.resource_id, e.event_type, e.payload, e.created
                  FROM kb_events e
                 WHERE (e.profile_id = $1 OR e.resource_id IN (SELECT resource_id FROM visible))
                   AND e.resource_id = $2
                 ORDER BY e.created DESC
                 LIMIT $3 OFFSET $4
                "#,
            )
            .bind(profile_id)
            .bind(rid)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (None, Some(etype)) => {
            sqlx::query_as::<_, EventRow>(
                r#"
                WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
                SELECT e.id, e.profile_id, e.client_id, e.kb_context_id,
                       e.resource_id, e.event_type, e.payload, e.created
                  FROM kb_events e
                 WHERE (e.profile_id = $1 OR e.resource_id IN (SELECT resource_id FROM visible))
                   AND e.event_type = $2
                 ORDER BY e.created DESC
                 LIMIT $3 OFFSET $4
                "#,
            )
            .bind(profile_id)
            .bind(etype)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
        (None, None) => {
            sqlx::query_as::<_, EventRow>(
                r#"
                WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
                SELECT e.id, e.profile_id, e.client_id, e.kb_context_id,
                       e.resource_id, e.event_type, e.payload, e.created
                  FROM kb_events e
                 WHERE (e.profile_id = $1 OR e.resource_id IN (SELECT resource_id FROM visible))
                 ORDER BY e.created DESC
                 LIMIT $2 OFFSET $3
                "#,
            )
            .bind(profile_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?
        }
    };

    Ok(rows)
}
