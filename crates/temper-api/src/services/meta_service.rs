//! Meta service — updates managed and open frontmatter on a resource
//! without requiring re-chunking.

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::ingest_service::insert_event_and_audit;
use crate::services::resource_service;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};

use temper_core::types::managed_meta::{ManagedMeta, MetaUpdatePayload, ResourceMetaResponse};

/// Fetch just the meta tier (managed_meta, open_meta, hashes) for a
/// resource without reconstructing the markdown body from `kb_chunks`.
///
/// Used by the CLI sync pull path when only meta has drifted.
/// Enforces visibility via `resource_service::get_visible`, which maps
/// both "missing" and "not visible to caller" to `ApiError::NotFound`.
pub async fn get_meta(
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: ResourceId,
) -> ApiResult<ResourceMetaResponse> {
    // Visibility / auth gate — returns NotFound for ghost or non-visible.
    resource_service::get_visible(pool, *profile_id, *resource_id).await?;

    let row = sqlx::query!(
        r#"SELECT managed_meta as "managed_meta: Value",
                  open_meta as "open_meta: Value",
                  managed_hash,
                  open_hash
             FROM kb_resource_manifests
            WHERE resource_id = $1"#,
        *resource_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;

    // Deserialize the stored JSONB into the typed `ManagedMeta`. The
    // `extra` flatten bucket catches any keys the typed fields don't
    // name (e.g. doc-type-schema fields like `date` for sessions), so
    // this is lossless — re-serializing produces the same canonical
    // JSON, and `managed_hash` remains stable across the round-trip.
    let managed_meta: ManagedMeta = serde_json::from_value(row.managed_meta).unwrap_or_default();

    Ok(ResourceMetaResponse {
        resource_id,
        managed_meta: Some(managed_meta),
        open_meta: Some(row.open_meta),
        managed_hash: row.managed_hash,
        open_hash: row.open_hash,
    })
}

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
        *profile_id,
        *resource_id
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
    //
    // The typed `ManagedMeta` is serialized back to a canonical JSONB
    // value here so the DB column stays a JSONB blob. The managed_hash
    // was computed by the caller over the canonical form, so the hash
    // stays stable across the typed round-trip.
    let managed_meta_json =
        serde_json::to_value(&payload.managed_meta).unwrap_or(serde_json::Value::Null);
    let rows = sqlx::query!(
        r#"
        UPDATE kb_resource_manifests
        SET managed_meta = $1, open_meta = $2, managed_hash = $3, open_hash = $4, updated = now()
        WHERE resource_id = $5
        "#,
        &managed_meta_json,
        &payload.open_meta as &serde_json::Value,
        &payload.managed_hash,
        &payload.open_hash,
        *resource_id,
    )
    .execute(&mut *tx)
    .await?;

    if rows.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    // 3. Cascade identity fields from managed_meta to kb_resources.
    // `payload.managed_meta` is already typed — no deserialize needed.
    let managed = &payload.managed_meta;

    // Update title and slug if present
    if let Some(ref title) = managed.title {
        sqlx::query!(
            "UPDATE kb_resources SET title = $1, updated = now() WHERE id = $2",
            title,
            *resource_id,
        )
        .execute(&mut *tx)
        .await?;
    }
    if let Some(ref slug) = managed.slug {
        sqlx::query!(
            "UPDATE kb_resources SET slug = $1, updated = now() WHERE id = $2",
            slug,
            *resource_id,
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
            *resource_id,
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
            *resource_id,
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

    // Reconcile edges from updated open_meta frontmatter.
    // The edge service reads declarations from open_meta and diffs against
    // existing frontmatter-provenance edges; manual edges are untouched.
    // Errors are logged, not propagated: the meta update itself succeeded.
    // `context_id` reflects the post-cascade state (any `temper-context`
    // change in managed_meta was applied earlier in the same tx), so we can
    // reuse the local directly instead of re-querying.
    let ctx_id = ContextId::from(context_id);
    if let Err(e) = super::edge_service::reconcile_edges(
        pool,
        &profile_id,
        &ctx_id,
        &resource_id,
        &payload.open_meta,
    )
    .await
    {
        tracing::warn!(
            resource_id = %resource_id,
            error = %e,
            "edge reconciliation failed during meta update"
        );
    }

    Ok(serde_json::json!({"updated": true, "resource_id": resource_id}))
}
