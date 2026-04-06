//! Meta service — updates managed and open frontmatter on a resource
//! without requiring re-chunking.

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::ingest_service::insert_event_and_audit;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};

use temper_core::types::managed_meta::{ManagedMeta, MetaUpdatePayload};

/// Update resource manifests with new managed/open meta, and cascade
/// identity fields (title, slug, temper-type, temper-context) to kb_resources.
pub async fn update_meta(
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: ResourceId,
    device_id: &str,
    payload: MetaUpdatePayload,
) -> ApiResult<Value> {
    // 1. Check can_modify_resource
    let can_modify = sqlx::query_scalar!(
        "SELECT can_modify_resource($1, $2)",
        profile_id.0,
        resource_id.0
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);

    if !can_modify {
        return Err(ApiError::Forbidden);
    }

    let mut tx = pool.begin().await?;

    // 2. Update kb_resource_manifests (plain UPDATE — must already exist;
    //    we don't want to insert a row with an empty body_hash).
    let rows = sqlx::query!(
        r#"
        UPDATE kb_resource_manifests
        SET managed_meta = $1, open_meta = $2, managed_hash = $3, open_hash = $4, updated = now()
        WHERE resource_id = $5
        "#,
        &payload.managed_meta as &serde_json::Value,
        &payload.open_meta as &serde_json::Value,
        &payload.managed_hash,
        &payload.open_hash,
        resource_id.0,
    )
    .execute(&mut *tx)
    .await?;

    if rows.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    // 3. Cascade identity fields from managed_meta to kb_resources
    let managed: ManagedMeta =
        serde_json::from_value(payload.managed_meta.clone()).unwrap_or_default();

    // Update title and slug if present
    if let Some(ref title) = managed.title {
        sqlx::query!(
            "UPDATE kb_resources SET title = $1, updated = now() WHERE id = $2",
            title,
            resource_id.0,
        )
        .execute(&mut *tx)
        .await?;
    }
    if let Some(ref slug) = managed.slug {
        sqlx::query!(
            "UPDATE kb_resources SET slug = $1, updated = now() WHERE id = $2",
            slug,
            resource_id.0,
        )
        .execute(&mut *tx)
        .await?;
    }

    // Cascade temper-type to kb_doc_type_id
    if let Some(ref doc_type) = managed.doc_type {
        let dt_id = sqlx::query_scalar!("SELECT id FROM kb_doc_types WHERE name = $1", doc_type,)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| ApiError::BadRequest(format!("unknown doc_type: '{doc_type}'")))?;
        let dt_rows = sqlx::query!(
            "UPDATE kb_resources SET kb_doc_type_id = $1, updated = now() WHERE id = $2",
            dt_id,
            resource_id.0,
        )
        .execute(&mut *tx)
        .await?;
        if dt_rows.rows_affected() == 0 {
            return Err(ApiError::NotFound);
        }
    }

    // Cascade temper-context to kb_context_id
    if let Some(ref context_name) = managed.context {
        let ctx_id =
            sqlx::query_scalar!("SELECT id FROM kb_contexts WHERE name = $1", context_name,)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| {
                    ApiError::BadRequest(format!("unknown context: '{context_name}'"))
                })?;
        sqlx::query!(
            "UPDATE kb_resources SET kb_context_id = $1, updated = now() WHERE id = $2",
            ctx_id,
            resource_id.0,
        )
        .execute(&mut *tx)
        .await?;
    }

    // 4. Insert kb_event + audit atomically
    // Fetch current body_hash and context_id for the event + audit records.
    let (body_hash, context_id): (String, Uuid) = sqlx::query_as(
        r#"SELECT m.body_hash, r.kb_context_id
           FROM kb_resource_manifests m
           JOIN kb_resources r ON r.id = m.resource_id
           WHERE m.resource_id = $1"#,
    )
    .bind(resource_id)
    .fetch_one(&mut *tx)
    .await?;

    insert_event_and_audit(
        &mut tx,
        profile_id,
        device_id,
        ContextId::from(context_id),
        resource_id,
        "managed_meta_updated",
        "update_meta",
        &body_hash,
        &payload.managed_hash,
        &payload.open_hash,
    )
    .await?;

    tx.commit().await?;

    Ok(serde_json::json!({"updated": true, "resource_id": resource_id}))
}
