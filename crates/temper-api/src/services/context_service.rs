//! Context CRUD service over the substrate.
//!
//! Visibility is derived inline (owner OR `kb_team_contexts` share) — there is
//! no `contexts_visible_to()` SQL function. `kb_owner_table`/`kb_owner_id`/
//! `updated` are synthesized to the `ContextRow` shape from the substrate's
//! `owner_table`/`owner_id`/`created` columns. Resource counts come from
//! `kb_resource_homes`. Context creation is a plain INSERT (no event emission —
//! product decision 5: contexts are infrastructure).

use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};
use temper_core::types::ids::{ContextId, ProfileId};
use temper_workflow::operations::sluggify;

pub use temper_core::types::context::{ContextCreateRequest, ContextRow, ContextRowWithCounts};

/// List all contexts visible to the profile (owned + team-shared), with resource counts.
pub async fn list_visible(
    pool: &PgPool,
    profile_id: ProfileId,
) -> ApiResult<Vec<ContextRowWithCounts>> {
    let rows = sqlx::query_as!(
        ContextRowWithCounts,
        r#"
        SELECT c.id, c.name,
               c.owner_table AS "kb_owner_table!",
               c.owner_id AS "kb_owner_id!",
               c.created,
               c.created AS "updated!",
               COUNT(rh.resource_id) AS "resource_count!"
          FROM kb_contexts c
          LEFT JOIN kb_resource_homes rh
                 ON rh.anchor_table = 'kb_contexts' AND rh.anchor_id = c.id
         WHERE (c.owner_table = 'kb_profiles' AND c.owner_id = $1)
            OR EXISTS (
                 SELECT 1 FROM kb_team_contexts tc
                   JOIN kb_team_members tm ON tm.team_id = tc.team_id
                  WHERE tc.context_id = c.id AND tm.profile_id = $1)
         GROUP BY c.id, c.name, c.owner_table, c.owner_id, c.created
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
        SELECT c.id, c.name,
               c.owner_table AS "kb_owner_table!",
               c.owner_id AS "kb_owner_id!",
               c.created,
               c.created AS "updated!"
          FROM kb_contexts c
         WHERE c.id = $2
           AND ((c.owner_table = 'kb_profiles' AND c.owner_id = $1)
                OR EXISTS (
                     SELECT 1 FROM kb_team_contexts tc
                       JOIN kb_team_members tm ON tm.team_id = tc.team_id
                      WHERE tc.context_id = c.id AND tm.profile_id = $1))
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
        SELECT c.id, c.name,
               c.owner_table AS "kb_owner_table!",
               c.owner_id AS "kb_owner_id!",
               c.created,
               c.created AS "updated!"
          FROM kb_contexts c
         WHERE c.name = $2
           AND ((c.owner_table = 'kb_profiles' AND c.owner_id = $1)
                OR EXISTS (
                     SELECT 1 FROM kb_team_contexts tc
                       JOIN kb_team_members tm ON tm.team_id = tc.team_id
                      WHERE tc.context_id = c.id AND tm.profile_id = $1))
        "#,
        *profile_id,
        name
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

/// Resolve a context name by ID without a visibility gate.
///
/// Used by handlers that receive a `kb_context_id` UUID on the wire and need
/// the corresponding name to construct a typed operations command. The
/// visibility check is enforced downstream by the create path (via
/// `resolve_by_name`, which is visibility-gated).
///
/// Returns `ApiError::BadRequest` when no context with the given ID exists.
pub async fn resolve_name_by_id(pool: &PgPool, context_id: uuid::Uuid) -> ApiResult<String> {
    let name = sqlx::query_scalar!("SELECT name FROM kb_contexts WHERE id = $1", context_id)
        .fetch_optional(pool)
        .await?;

    name.ok_or_else(|| ApiError::BadRequest(format!("unknown context id: '{context_id}'")))
}

/// Pick a slug for a new context, unique within `(owner_table, owner_id, slug)`.
///
/// Bases the slug on the name; on collision (two distinct names can sluggify to
/// the same value, and the substrate's unique constraint is on the slug, not the
/// name) appends a numeric suffix. The `(owner_table, owner_id, slug)` unique
/// constraint is the backstop against the check-then-insert race.
async fn next_unique_context_slug(
    pool: &PgPool,
    owner_id: ProfileId,
    name: &str,
) -> ApiResult<String> {
    let base = {
        let s = sluggify(name);
        if s.is_empty() {
            "context".to_owned()
        } else {
            s
        }
    };
    let mut candidate = base.clone();
    let mut n = 2;
    loop {
        let taken = sqlx::query_scalar!(
            r#"
            SELECT EXISTS (
                SELECT 1 FROM kb_contexts
                 WHERE owner_table = 'kb_profiles' AND owner_id = $1 AND slug = $2
            ) AS "taken!"
            "#,
            *owner_id,
            candidate
        )
        .fetch_one(pool)
        .await?;

        if !taken {
            return Ok(candidate);
        }
        candidate = format!("{base}-{n}");
        n += 1;
    }
}

/// Create a new context owned by the profile: a plain INSERT with a generated
/// slug and NO event emission (product decision 5 — contexts are infrastructure).
///
/// The substrate enforces uniqueness on the generated slug
/// (`(owner_table, owner_id, slug)`), not the name — `next_unique_context_slug`
/// auto-suffixes on collision, so two contexts sharing a name coexist under
/// distinct slugs rather than 409ing.
pub async fn create(pool: &PgPool, profile_id: ProfileId, name: &str) -> ApiResult<ContextRow> {
    let id = ContextId::new();
    let slug = next_unique_context_slug(pool, profile_id, name).await?;

    let row = sqlx::query_as!(
        ContextRow,
        r#"
        INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name)
        VALUES ($1, 'kb_profiles', $2, $3, $4)
        RETURNING id, name,
                  owner_table AS "kb_owner_table!",
                  owner_id AS "kb_owner_id!",
                  created,
                  created AS "updated!"
        "#,
        *id,
        *profile_id,
        slug,
        name
    )
    .fetch_one(pool)
    .await?;

    Ok(row)
}
