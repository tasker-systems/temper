//! Context CRUD service — queries scoped through `contexts_visible_to()`.
//!
//! Future scope (I5h): rename, delete (zero-resource guard), resource
//! relocation. See tasks/temper/2026-04-01-i5h-context-crud-lifecycle-
//! rename-delete-relocate.md.

use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};
use temper_core::types::ids::{ContextId, EventId, ProfileId};

pub use temper_core::types::context::{ContextCreateRequest, ContextRow, ContextRowWithCounts};

/// List all contexts visible to the profile (owned + team-shared), with resource counts.
pub async fn list_visible(
    pool: &PgPool,
    profile_id: ProfileId,
) -> ApiResult<Vec<ContextRowWithCounts>> {
    let rows = sqlx::query_as!(
        ContextRowWithCounts,
        r#"
        SELECT c.id, c.name, c.kb_owner_table, c.kb_owner_id, c.created, c.updated,
               COUNT(r.id) AS "resource_count!"
          FROM contexts_visible_to($1) cv
          JOIN kb_contexts c ON c.id = cv.id
          LEFT JOIN kb_resources r ON r.kb_context_id = c.id AND r.is_active = true
         GROUP BY c.id, c.name, c.kb_owner_table, c.kb_owner_id, c.created, c.updated
         ORDER BY c.name
        "#,
        *profile_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Get a single context by ID, scoped to profile visibility.
pub async fn get_visible(
    pool: &PgPool,
    profile_id: ProfileId,
    context_id: ContextId,
) -> ApiResult<ContextRow> {
    sqlx::query_as!(
        ContextRow,
        r#"
        SELECT c.id, c.name, c.kb_owner_table, c.kb_owner_id, c.created, c.updated
          FROM contexts_visible_to($1) cv
          JOIN kb_contexts c ON c.id = cv.id
         WHERE c.id = $2
        "#,
        *profile_id,
        *context_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

/// Resolve a context by name within the profile's visible contexts.
pub async fn resolve_by_name(
    pool: &PgPool,
    profile_id: ProfileId,
    name: &str,
) -> ApiResult<ContextRow> {
    sqlx::query_as!(
        ContextRow,
        r#"
        SELECT c.id, c.name, c.kb_owner_table, c.kb_owner_id, c.created, c.updated
          FROM contexts_visible_to($1) cv
          JOIN kb_contexts c ON c.id = cv.id
         WHERE c.name = $2
        "#,
        *profile_id,
        name
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

/// Create a new context owned by the profile.
///
/// Returns 409 Conflict if a context with the same name already exists
/// for this owner (enforced by `kb_contexts_owner_name_unique` constraint).
pub async fn create(pool: &PgPool, profile_id: ProfileId, name: &str) -> ApiResult<ContextRow> {
    let id = ContextId::new();
    let mut tx = pool.begin().await?;

    let row = sqlx::query_as!(
        ContextRow,
        r#"
        INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id)
        VALUES ($1, $2, 'kb_profiles', $3)
        RETURNING id, name, kb_owner_table, kb_owner_id, created, updated
        "#,
        *id,
        name,
        *profile_id
    )
    .fetch_one(&mut *tx)
    .await?;

    let event_id = EventId::new();
    sqlx::query(
        "INSERT INTO kb_events (id, profile_id, device_id, kb_context_id, event_type, payload, created)
         VALUES ($1, $2, $3, $4, $5, '{}', now())",
    )
    .bind(event_id)
    .bind(profile_id)
    .bind("api")
    .bind(id)
    .bind("context_created")
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(row)
}
