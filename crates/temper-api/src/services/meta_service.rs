//! Meta service — updates managed and open frontmatter on a resource
//! without requiring re-chunking.

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::ingest_service::insert_event;

use temper_core::types::managed_meta::{ManagedMeta, MetaUpdatePayload};

/// Update resource manifests with new managed/open meta, and cascade
/// identity fields (title, slug, temper-type, temper-context) to kb_resources.
pub async fn update_meta(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
    payload: MetaUpdatePayload,
) -> ApiResult<Value> {
    // 1. Check can_modify_resource
    let can_modify: bool = sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(profile_id)
        .bind(resource_id)
        .fetch_one(pool)
        .await?;

    if !can_modify {
        return Err(ApiError::Forbidden);
    }

    let mut tx = pool.begin().await?;

    // 2. Update kb_resource_manifests (plain UPDATE — must already exist;
    //    we don't want to insert a row with an empty body_hash).
    let rows = sqlx::query(
        r#"
        UPDATE kb_resource_manifests
        SET managed_meta = $1, open_meta = $2, managed_hash = $3, open_hash = $4, updated = now()
        WHERE resource_id = $5
        "#,
    )
    .bind(&payload.managed_meta)
    .bind(&payload.open_meta)
    .bind(&payload.managed_hash)
    .bind(&payload.open_hash)
    .bind(resource_id)
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
        sqlx::query("UPDATE kb_resources SET title = $1, updated = now() WHERE id = $2")
            .bind(title)
            .bind(resource_id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(ref slug) = managed.slug {
        sqlx::query("UPDATE kb_resources SET slug = $1, updated = now() WHERE id = $2")
            .bind(slug)
            .bind(resource_id)
            .execute(&mut *tx)
            .await?;
    }

    // Cascade temper-type to kb_doc_type_id
    if let Some(ref doc_type) = managed.doc_type {
        let doc_type_id: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM kb_doc_types WHERE name = $1")
                .bind(doc_type)
                .fetch_optional(&mut *tx)
                .await?;
        let (dt_id,) = doc_type_id
            .ok_or_else(|| ApiError::BadRequest(format!("unknown doc_type: '{doc_type}'")))?;
        let dt_rows = sqlx::query(
            "UPDATE kb_resources SET kb_doc_type_id = $1, updated = now() WHERE id = $2",
        )
        .bind(dt_id)
        .bind(resource_id)
        .execute(&mut *tx)
        .await?;
        if dt_rows.rows_affected() == 0 {
            return Err(ApiError::NotFound);
        }
    }

    // Cascade temper-context to kb_context_id
    if let Some(ref context_name) = managed.context {
        let context_id: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM kb_contexts WHERE name = $1")
                .bind(context_name)
                .fetch_optional(&mut *tx)
                .await?;
        let (ctx_id,) = context_id
            .ok_or_else(|| ApiError::BadRequest(format!("unknown context: '{context_name}'")))?;
        sqlx::query("UPDATE kb_resources SET kb_context_id = $1, updated = now() WHERE id = $2")
            .bind(ctx_id)
            .bind(resource_id)
            .execute(&mut *tx)
            .await?;
    }

    // 4. Insert kb_event
    let _event_id = insert_event(
        &mut tx,
        profile_id,
        "api",
        None,
        Some(resource_id),
        "managed_meta_updated",
        &serde_json::json!({
            "managed_hash": &payload.managed_hash,
            "open_hash": &payload.open_hash,
        }),
    )
    .await?;

    tx.commit().await?;

    Ok(serde_json::json!({"updated": true, "resource_id": resource_id}))
}
