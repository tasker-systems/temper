//! Meta service — read-only frontmatter accessors. Write paths flow
//! through `DbBackend::update_resource` (see `resource_service::update`'s
//! meta-only branch).

use serde_json::Value;
use sqlx::PgPool;

use crate::error::{ApiError, ApiResult};
use crate::services::resource_service;
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::managed_meta::{ManagedMeta, ResourceMetaResponse};

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
