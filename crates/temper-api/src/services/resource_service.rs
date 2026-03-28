use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

/// Row type matching the `resources` table.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ResourceRow {
    pub id: Uuid,
    pub kb_context_id: Uuid,
    pub kb_doc_type_id: Uuid,
    pub uri: String,
    pub title: String,
    pub slug: Option<String>,
    pub content_hash: Option<String>,
    pub mimetype: Option<String>,
    pub originator_profile_id: Uuid,
    pub owner_profile_id: Uuid,
    pub is_active: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

/// Query parameters for listing visible resources.
#[derive(Debug, Deserialize)]
pub struct ResourceListParams {
    /// Filter by context ID.
    pub kb_context_id: Option<Uuid>,
    /// Maximum results to return (default 50, max 200).
    pub limit: Option<i64>,
    /// Offset for pagination.
    pub offset: Option<i64>,
}

/// Request body for creating a resource.
#[derive(Debug, Deserialize)]
pub struct ResourceCreateRequest {
    pub kb_context_id: Uuid,
    pub kb_doc_type_id: Uuid,
    pub uri: String,
    pub title: String,
    pub slug: Option<String>,
    pub mimetype: Option<String>,
}

/// Request body for updating a resource.
#[derive(Debug, Deserialize)]
pub struct ResourceUpdateRequest {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub mimetype: Option<String>,
}

/// List resources visible to the given profile.
///
/// Uses the `resources_visible_to(profile_id)` SQL function to scope results.
pub async fn list_visible(
    pool: &PgPool,
    profile_id: Uuid,
    params: ResourceListParams,
) -> ApiResult<Vec<ResourceRow>> {
    let limit = params.limit.unwrap_or(50).min(200);
    let offset = params.offset.unwrap_or(0).max(0);

    let rows = if let Some(ctx_id) = params.kb_context_id {
        sqlx::query_as::<_, ResourceRow>(
            r#"
            WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
            SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.uri, r.title,
                   r.slug, r.content_hash, r.mimetype,
                   r.originator_profile_id, r.owner_profile_id, r.is_active,
                   r.created, r.updated
              FROM resources r
              JOIN visible v ON v.resource_id = r.id
             WHERE r.is_active = true
               AND r.kb_context_id = $2
             ORDER BY r.updated DESC
             LIMIT $3 OFFSET $4
            "#,
        )
        .bind(profile_id)
        .bind(ctx_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, ResourceRow>(
            r#"
            WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
            SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.uri, r.title,
                   r.slug, r.content_hash, r.mimetype,
                   r.originator_profile_id, r.owner_profile_id, r.is_active,
                   r.created, r.updated
              FROM resources r
              JOIN visible v ON v.resource_id = r.id
             WHERE r.is_active = true
             ORDER BY r.updated DESC
             LIMIT $2 OFFSET $3
            "#,
        )
        .bind(profile_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?
    };

    Ok(rows)
}

/// Get a single resource by ID, scoped to profile visibility.
pub async fn get_visible(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
) -> ApiResult<ResourceRow> {
    let row = sqlx::query_as::<_, ResourceRow>(
        r#"
        WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
        SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.uri, r.title,
               r.slug, r.content_hash, r.mimetype,
               r.originator_profile_id, r.owner_profile_id, r.is_active,
               r.created, r.updated
          FROM resources r
          JOIN visible v ON v.resource_id = r.id
         WHERE r.id = $2
           AND r.is_active = true
        "#,
    )
    .bind(profile_id)
    .bind(resource_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;

    Ok(row)
}

/// Chunk used to reconstitute markdown content.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ContentChunk {
    pub chunk_index: i32,
    pub header_path: String,
    pub content: String,
}

/// Reconstitute resource content from `kb_current_chunks`, returning markdown.
pub async fn get_content(pool: &PgPool, profile_id: Uuid, resource_id: Uuid) -> ApiResult<String> {
    // Visibility check first.
    get_visible(pool, profile_id, resource_id).await?;

    let chunks = sqlx::query_as::<_, ContentChunk>(
        r#"
        SELECT chunk_index, header_path, content
          FROM kb_current_chunks
         WHERE resource_id = $1
         ORDER BY chunk_index
        "#,
    )
    .bind(resource_id)
    .fetch_all(pool)
    .await?;

    let markdown = chunks
        .into_iter()
        .map(|c| {
            if c.header_path.is_empty() {
                c.content
            } else {
                format!("{}\n\n{}", c.header_path, c.content)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    Ok(markdown)
}

/// Create a new resource. The caller is set as both originator and owner.
pub async fn create(
    pool: &PgPool,
    profile_id: Uuid,
    req: ResourceCreateRequest,
) -> ApiResult<ResourceRow> {
    let id = Uuid::now_v7();
    sqlx::query(
        r#"
        INSERT INTO resources
            (id, kb_context_id, kb_doc_type_id, uri, title, slug, mimetype,
             originator_profile_id, owner_profile_id, is_active, created, updated)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8, true, now(), now())
        "#,
    )
    .bind(id)
    .bind(req.kb_context_id)
    .bind(req.kb_doc_type_id)
    .bind(&req.uri)
    .bind(&req.title)
    .bind(&req.slug)
    .bind(&req.mimetype)
    .bind(profile_id)
    .execute(pool)
    .await?;

    get_visible(pool, profile_id, id).await
}

/// Update mutable fields on a resource. Requires `can_modify_resource()` to return true.
pub async fn update(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
    req: ResourceUpdateRequest,
) -> ApiResult<ResourceRow> {
    let can_modify: bool = sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(profile_id)
        .bind(resource_id)
        .fetch_one(pool)
        .await?;

    if !can_modify {
        return Err(ApiError::Forbidden);
    }

    let current = get_visible(pool, profile_id, resource_id).await?;

    let new_title = req.title.as_deref().unwrap_or(&current.title);
    let new_slug = req.slug.as_deref().or(current.slug.as_deref());
    let new_mimetype = req.mimetype.as_deref().or(current.mimetype.as_deref());

    sqlx::query(
        r#"
        UPDATE resources
           SET title    = $1,
               slug     = $2,
               mimetype = $3,
               updated  = now()
         WHERE id = $4
           AND is_active = true
        "#,
    )
    .bind(new_title)
    .bind(new_slug)
    .bind(new_mimetype)
    .bind(resource_id)
    .execute(pool)
    .await?;

    get_visible(pool, profile_id, resource_id).await
}

/// Soft-delete a resource. Requires `can_modify_resource()` to return true.
pub async fn delete(pool: &PgPool, profile_id: Uuid, resource_id: Uuid) -> ApiResult<()> {
    let can_modify: bool = sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(profile_id)
        .bind(resource_id)
        .fetch_one(pool)
        .await?;

    if !can_modify {
        return Err(ApiError::Forbidden);
    }

    sqlx::query(
        r#"
        UPDATE resources
           SET is_active = false,
               updated   = now()
         WHERE id = $1
           AND is_active = true
        "#,
    )
    .bind(resource_id)
    .execute(pool)
    .await?;

    Ok(())
}
