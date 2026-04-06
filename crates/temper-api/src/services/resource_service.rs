use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::ingest_service::insert_event_and_audit;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};

pub use temper_core::types::resource::{
    ContentChunk, ResourceCreateRequest, ResourceListParams, ResourceRow, ResourceUpdateRequest,
};

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
        sqlx::query_as!(
            ResourceRow,
            r#"
            WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
            SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.origin_uri, r.title,
                   r.slug,
                   r.originator_profile_id, r.owner_profile_id, r.is_active,
                   r.created, r.updated
              FROM kb_resources r
              JOIN visible v ON v.resource_id = r.id
             WHERE r.is_active = true
               AND r.kb_context_id = $2
             ORDER BY r.updated DESC
             LIMIT $3 OFFSET $4
            "#,
            profile_id,
            ctx_id,
            limit,
            offset,
        )
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as!(
            ResourceRow,
            r#"
            WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
            SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.origin_uri, r.title,
                   r.slug,
                   r.originator_profile_id, r.owner_profile_id, r.is_active,
                   r.created, r.updated
              FROM kb_resources r
              JOIN visible v ON v.resource_id = r.id
             WHERE r.is_active = true
             ORDER BY r.updated DESC
             LIMIT $2 OFFSET $3
            "#,
            profile_id,
            limit,
            offset,
        )
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
    let row = sqlx::query_as!(
        ResourceRow,
        r#"
        WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
        SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.origin_uri, r.title,
               r.slug,
               r.originator_profile_id, r.owner_profile_id, r.is_active,
               r.created, r.updated
          FROM kb_resources r
          JOIN visible v ON v.resource_id = r.id
         WHERE r.id = $2
           AND r.is_active = true
        "#,
        profile_id,
        resource_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;

    Ok(row)
}

/// Reconstitute resource content from `kb_current_chunks`, returning markdown.
pub async fn get_content(pool: &PgPool, profile_id: Uuid, resource_id: Uuid) -> ApiResult<String> {
    // Visibility check first.
    get_visible(pool, profile_id, resource_id).await?;

    let chunks = sqlx::query_as!(
        ContentChunk,
        r#"
        SELECT chunk_index as "chunk_index!: i32", header_path as "header_path!: String", content as "content!: String"
          FROM kb_current_chunks
         WHERE resource_id = $1
         ORDER BY chunk_index
        "#,
        resource_id,
    )
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
    sqlx::query!(
        r#"
        INSERT INTO kb_resources
            (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
             originator_profile_id, owner_profile_id, is_active, created, updated)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $7, true, now(), now())
        "#,
        id,
        req.kb_context_id,
        req.kb_doc_type_id,
        req.origin_uri,
        req.title,
        req.slug,
        profile_id,
    )
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
    let can_modify = sqlx::query_scalar!(
        "SELECT can_modify_resource($1, $2)",
        profile_id,
        resource_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);

    if !can_modify {
        return Err(ApiError::Forbidden);
    }

    let current = get_visible(pool, profile_id, resource_id).await?;

    let new_title = req.title.as_deref().unwrap_or(&current.title);
    let new_slug = req.slug.as_deref().or(current.slug.as_deref());

    sqlx::query!(
        r#"
        UPDATE kb_resources
           SET title    = $1,
               slug     = $2,
               updated  = now()
         WHERE id = $3
           AND is_active = true
        "#,
        new_title,
        new_slug,
        resource_id,
    )
    .execute(pool)
    .await?;

    get_visible(pool, profile_id, resource_id).await
}

/// Soft-delete a resource. Requires `can_modify_resource()` to return true.
pub async fn delete(
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: ResourceId,
    device_id: &str,
) -> ApiResult<()> {
    let can_modify = sqlx::query_scalar!(
        "SELECT can_modify_resource($1, $2)",
        *profile_id,
        *resource_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);

    if !can_modify {
        return Err(ApiError::Forbidden);
    }

    let mut tx = pool.begin().await?;

    // Fetch current hashes for the audit snapshot before soft-delete
    let hashes = sqlx::query!(
        "SELECT body_hash, managed_hash, open_hash FROM kb_resource_manifests WHERE resource_id = $1",
        *resource_id,
    )
    .fetch_optional(&mut *tx)
    .await?;

    let (body_hash, managed_hash, open_hash) = hashes
        .map(|h| (h.body_hash, h.managed_hash, h.open_hash))
        .unwrap_or_default();

    // Fetch context_id for the event
    let context_id = sqlx::query_scalar!(
        "SELECT kb_context_id FROM kb_resources WHERE id = $1",
        *resource_id,
    )
    .fetch_one(&mut *tx)
    .await?;

    // Soft-delete the resource
    sqlx::query!(
        r#"
        UPDATE kb_resources
           SET is_active = false,
               updated   = now()
         WHERE id = $1
           AND is_active = true
        "#,
        *resource_id,
    )
    .execute(&mut *tx)
    .await?;

    // Record event and audit atomically
    insert_event_and_audit(
        &mut tx,
        profile_id,
        device_id,
        ContextId::from(context_id),
        resource_id,
        "resource_deleted",
        "delete",
        &body_hash,
        &managed_hash,
        &open_hash,
    )
    .await?;

    tx.commit().await?;

    Ok(())
}
