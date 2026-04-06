//! Context CRUD service — queries scoped through `contexts_visible_to()`.
//!
//! Future scope (I5h): rename, delete (zero-resource guard), resource
//! relocation. See tasks/temper/2026-04-01-i5h-context-crud-lifecycle-
//! rename-delete-relocate.md.

use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};
use temper_core::types::ids::{ContextId, ProfileId};

pub use temper_core::types::context::{ContextCreateRequest, ContextRow};

/// List all contexts visible to the profile (owned + team-shared).
pub async fn list_visible(pool: &PgPool, profile_id: ProfileId) -> ApiResult<Vec<ContextRow>> {
    let rows = sqlx::query_as::<_, ContextRow>(
        r#"
        SELECT c.id, c.name, c.kb_owner_table, c.kb_owner_id, c.created, c.updated
          FROM contexts_visible_to($1) cv
          JOIN kb_contexts c ON c.id = cv.id
         ORDER BY c.name
        "#,
    )
    .bind(profile_id)
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
    sqlx::query_as::<_, ContextRow>(
        r#"
        SELECT c.id, c.name, c.kb_owner_table, c.kb_owner_id, c.created, c.updated
          FROM contexts_visible_to($1) cv
          JOIN kb_contexts c ON c.id = cv.id
         WHERE c.id = $2
        "#,
    )
    .bind(profile_id)
    .bind(context_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

/// Resolve a context by name within the profile's visible contexts.
pub async fn resolve_by_name(pool: &PgPool, profile_id: ProfileId, name: &str) -> ApiResult<ContextRow> {
    sqlx::query_as::<_, ContextRow>(
        r#"
        SELECT c.id, c.name, c.kb_owner_table, c.kb_owner_id, c.created, c.updated
          FROM contexts_visible_to($1) cv
          JOIN kb_contexts c ON c.id = cv.id
         WHERE c.name = $2
        "#,
    )
    .bind(profile_id)
    .bind(name)
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
    let row = sqlx::query_as::<_, ContextRow>(
        r#"
        INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id)
        VALUES ($1, $2, 'kb_profiles', $3)
        RETURNING id, name, kb_owner_table, kb_owner_id, created, updated
        "#,
    )
    .bind(id)
    .bind(name)
    .bind(profile_id)
    .fetch_one(pool)
    .await?;

    Ok(row)
}
