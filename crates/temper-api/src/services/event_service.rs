use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;

pub use temper_core::types::api::{EventListParams, EventRow};

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
            sqlx::query_as!(
                EventRow,
                r#"
                WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
                SELECT e.id, e.profile_id, e.device_id,
                       e.kb_context_id as "kb_context_id: Uuid",
                       e.resource_id as "resource_id: Uuid",
                       et.name AS "event_type!", e.payload as "payload: serde_json::Value", e.created
                  FROM kb_events e
                  JOIN kb_event_types et ON et.id = e.event_type_id
                 WHERE (e.profile_id = $1 OR e.resource_id IN (SELECT resource_id FROM visible))
                   AND e.resource_id = $2
                   AND et.name        = $3
                 ORDER BY e.created DESC
                 LIMIT $4 OFFSET $5
                "#,
                profile_id,
                rid,
                etype,
                limit,
                offset,
            )
            .fetch_all(pool)
            .await?
        }
        (Some(rid), None) => {
            sqlx::query_as!(
                EventRow,
                r#"
                WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
                SELECT e.id, e.profile_id, e.device_id,
                       e.kb_context_id as "kb_context_id: Uuid",
                       e.resource_id as "resource_id: Uuid",
                       et.name AS "event_type!", e.payload as "payload: serde_json::Value", e.created
                  FROM kb_events e
                  JOIN kb_event_types et ON et.id = e.event_type_id
                 WHERE (e.profile_id = $1 OR e.resource_id IN (SELECT resource_id FROM visible))
                   AND e.resource_id = $2
                 ORDER BY e.created DESC
                 LIMIT $3 OFFSET $4
                "#,
                profile_id,
                rid,
                limit,
                offset,
            )
            .fetch_all(pool)
            .await?
        }
        (None, Some(etype)) => {
            sqlx::query_as!(
                EventRow,
                r#"
                WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
                SELECT e.id, e.profile_id, e.device_id,
                       e.kb_context_id as "kb_context_id: Uuid",
                       e.resource_id as "resource_id: Uuid",
                       et.name AS "event_type!", e.payload as "payload: serde_json::Value", e.created
                  FROM kb_events e
                  JOIN kb_event_types et ON et.id = e.event_type_id
                 WHERE (e.profile_id = $1 OR e.resource_id IN (SELECT resource_id FROM visible))
                   AND et.name = $2
                 ORDER BY e.created DESC
                 LIMIT $3 OFFSET $4
                "#,
                profile_id,
                etype,
                limit,
                offset,
            )
            .fetch_all(pool)
            .await?
        }
        (None, None) => {
            sqlx::query_as!(
                EventRow,
                r#"
                WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
                SELECT e.id, e.profile_id, e.device_id,
                       e.kb_context_id as "kb_context_id: Uuid",
                       e.resource_id as "resource_id: Uuid",
                       et.name AS "event_type!", e.payload as "payload: serde_json::Value", e.created
                  FROM kb_events e
                  JOIN kb_event_types et ON et.id = e.event_type_id
                 WHERE (e.profile_id = $1 OR e.resource_id IN (SELECT resource_id FROM visible))
                 ORDER BY e.created DESC
                 LIMIT $2 OFFSET $3
                "#,
                profile_id,
                limit,
                offset,
            )
            .fetch_all(pool)
            .await?
        }
    };

    Ok(rows)
}

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
