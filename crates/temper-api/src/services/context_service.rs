//! Context CRUD service — queries scoped through `contexts_visible_to()`.
//!
//! Future scope (I5h): rename, delete (zero-resource guard), resource
//! relocation. See tasks/temper/2026-04-01-i5h-context-crud-lifecycle-
//! rename-delete-relocate.md.

use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};
#[cfg(feature = "next-backend")]
use temper_core::operations::sluggify;
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

/// Resolve a context name by ID without a visibility gate.
///
/// Used by handlers that receive a `kb_context_id` UUID on the wire and need
/// the corresponding name to construct a typed operations command. The
/// visibility check is enforced downstream by `ingest_service::ingest` (via
/// `resolve_by_name`, which is visibility-gated through `contexts_visible_to`).
///
/// Returns `ApiError::BadRequest` when no context with the given ID exists.
pub async fn resolve_name_by_id(pool: &PgPool, context_id: uuid::Uuid) -> ApiResult<String> {
    let name = sqlx::query_scalar!("SELECT name FROM kb_contexts WHERE id = $1", context_id)
        .fetch_optional(pool)
        .await?;

    name.ok_or_else(|| ApiError::BadRequest(format!("unknown context id: '{context_id}'")))
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
        "INSERT INTO kb_events (id, profile_id, device_id, kb_context_id, event_type_id, payload, created)
         VALUES ($1, $2, $3, $4, resolve_event_type($5), '{}', now())",
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

// ─────────────────────────────────────────────────────────────────────────────
// Substrate (`temper_next.*`) dark-launch variants.
//
// These mirror the legacy fns above against the collapsed substrate schema. They
// are unreached in production until the flip (which de-qualifies them and swaps
// the call sites). Substrate deltas vs. legacy:
//   - There is no `contexts_visible_to()` SQL function — visibility is derived
//     inline (owner OR `kb_team_contexts` share).
//   - `kb_owner_table`/`kb_owner_id` are renamed to `owner_table`/`owner_id`.
//   - There is no `updated` column — it is synthesized from `created`.
//   - Resource counts come from `kb_resource_homes`, not `kb_resources.kb_context_id`.
//   - `kb_contexts.slug` is NOT NULL and generated from the name.
//   - Context creation is a plain INSERT with NO event emission (product
//     decision 5 — contexts are infrastructure).
// ─────────────────────────────────────────────────────────────────────────────

/// List all contexts visible to the profile (owned + team-shared) over the
/// substrate, with resource counts. Substrate variant of [`list_visible`].
#[cfg(feature = "next-backend")]
pub async fn list_visible_next(
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
          FROM temper_next.kb_contexts c
          LEFT JOIN temper_next.kb_resource_homes rh
                 ON rh.anchor_table = 'kb_contexts' AND rh.anchor_id = c.id
         WHERE (c.owner_table = 'kb_profiles' AND c.owner_id = $1)
            OR EXISTS (
                 SELECT 1 FROM temper_next.kb_team_contexts tc
                   JOIN temper_next.kb_team_members tm ON tm.team_id = tc.team_id
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

/// Get a single substrate context by ID, scoped to profile visibility.
/// Substrate variant of [`get_visible`].
#[cfg(feature = "next-backend")]
pub async fn get_visible_next(
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
          FROM temper_next.kb_contexts c
         WHERE c.id = $2
           AND ((c.owner_table = 'kb_profiles' AND c.owner_id = $1)
                OR EXISTS (
                     SELECT 1 FROM temper_next.kb_team_contexts tc
                       JOIN temper_next.kb_team_members tm ON tm.team_id = tc.team_id
                      WHERE tc.context_id = c.id AND tm.profile_id = $1))
        "#,
        *profile_id,
        *context_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

/// Resolve a substrate context by name within the profile's visible contexts.
/// Substrate variant of [`resolve_by_name`].
#[cfg(feature = "next-backend")]
pub async fn resolve_by_name_next(
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
          FROM temper_next.kb_contexts c
         WHERE c.name = $2
           AND ((c.owner_table = 'kb_profiles' AND c.owner_id = $1)
                OR EXISTS (
                     SELECT 1 FROM temper_next.kb_team_contexts tc
                       JOIN temper_next.kb_team_members tm ON tm.team_id = tc.team_id
                      WHERE tc.context_id = c.id AND tm.profile_id = $1))
        "#,
        *profile_id,
        name
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)
}

/// Resolve a substrate context name by ID without a visibility gate.
/// Substrate variant of [`resolve_name_by_id`].
#[cfg(feature = "next-backend")]
pub async fn resolve_name_by_id_next(pool: &PgPool, context_id: uuid::Uuid) -> ApiResult<String> {
    let name = sqlx::query_scalar!(
        "SELECT name FROM temper_next.kb_contexts WHERE id = $1",
        context_id
    )
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
#[cfg(feature = "next-backend")]
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
                SELECT 1 FROM temper_next.kb_contexts
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

/// Create a new substrate context owned by the profile. Substrate variant of
/// [`create`]: a plain INSERT with a generated slug and NO event emission
/// (product decision 5 — contexts are infrastructure).
///
/// Unlike legacy [`create`], the substrate enforces uniqueness on the generated
/// slug (`(owner_table, owner_id, slug)`), not the name — `next_unique_context_slug`
/// auto-suffixes on collision, so two contexts sharing a name coexist under
/// distinct slugs rather than 409ing.
#[cfg(feature = "next-backend")]
pub async fn create_next(
    pool: &PgPool,
    profile_id: ProfileId,
    name: &str,
) -> ApiResult<ContextRow> {
    let id = ContextId::new();
    let slug = next_unique_context_slug(pool, profile_id, name).await?;

    let row = sqlx::query_as!(
        ContextRow,
        r#"
        INSERT INTO temper_next.kb_contexts (id, owner_table, owner_id, slug, name)
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
