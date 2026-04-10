//! Ingest service — accepts a fully-processed payload (content + chunks +
//! embeddings) and writes resource + chunks to the database in a single
//! transaction.

use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::context_service;
use temper_core::types::ids::{ContextId, EventId, ProfileId, ResourceAuditId, ResourceId};
use temper_core::types::ingest::chunks_to_jsonb;

use temper_core::types::ingest::{unpack_chunks, IngestPayload, PackedChunk};
use temper_core::types::resource::ResourceRow;

use super::resource_service;

/// Lightweight row type for ingest-internal INSERT/UPDATE RETURNING queries.
///
/// `ResourceRow` now includes joined display fields from the browse view
/// that aren't available during in-transaction INSERT/UPDATE RETURNING.
/// This struct captures only the base columns needed within the transaction,
/// and the public-facing functions re-fetch the full `ResourceRow` via the view.
#[derive(Debug, sqlx::FromRow)]
struct ResourceRowBase {
    #[expect(dead_code, reason = "required by FromRow derive for RETURNING query")]
    id: ResourceId,
    kb_context_id: ContextId,
}

/// Compute a `sha256:<hex>` hash of a JSON value (canonical form).
///
/// Keys are sorted recursively to ensure deterministic output regardless
/// of the insertion order of `serde_json::Map`.
pub fn hash_json_value(value: &serde_json::Value) -> String {
    let canonical = canonicalize_json(value);
    let serialized = serde_json::to_string(&canonical).unwrap_or_else(|_| "{}".to_string());
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

fn canonicalize_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let sorted: std::collections::BTreeMap<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), canonicalize_json(v)))
                .collect();
            serde_json::to_value(sorted).unwrap_or(serde_json::Value::Object(map.clone()))
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(canonicalize_json).collect())
        }
        other => other.clone(),
    }
}

/// Insert an event and audit trail row atomically via the SQL function.
#[expect(
    clippy::too_many_arguments,
    reason = "event+audit require all hash fields plus identifiers"
)]
pub async fn insert_event_and_audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    profile_id: ProfileId,
    device_id: &str,
    context_id: ContextId,
    resource_id: ResourceId,
    event_type: &str,
    action: &str,
    body_hash: &str,
    managed_hash: &str,
    open_hash: &str,
) -> ApiResult<(EventId, ResourceAuditId)> {
    let event_id = EventId::new();

    let row: (Uuid, Uuid) = sqlx::query_as(
        "SELECT event_id, audit_id FROM insert_event_and_audit($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(event_id)
    .bind(profile_id)
    .bind(device_id)
    .bind(context_id)
    .bind(resource_id)
    .bind(event_type)
    .bind(action)
    .bind(body_hash)
    .bind(managed_hash)
    .bind(open_hash)
    .fetch_one(&mut **tx)
    .await?;

    Ok((EventId::from(row.0), ResourceAuditId::from(row.1)))
}

/// Resolve doc_type name to UUID from kb_doc_types.
pub async fn resolve_doc_type(pool: &PgPool, name: &str) -> ApiResult<Uuid> {
    let id = sqlx::query_scalar!("SELECT id FROM kb_doc_types WHERE name = $1", name)
        .fetch_optional(pool)
        .await?;

    id.ok_or_else(|| ApiError::BadRequest(format!("unknown doc_type: '{name}'")))
}

/// Check for body-hash dedup — returns existing resource ID if hash matches.
pub async fn find_by_body_hash(
    pool: &PgPool,
    profile_id: ProfileId,
    body_hash: &str,
) -> ApiResult<Option<ResourceRow>> {
    // Find the resource ID via a lightweight query, then fetch the full row via the view.
    let maybe_id = sqlx::query_scalar!(
        r#"
        WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
        SELECT r.id
          FROM kb_resources r
          JOIN visible v ON v.resource_id = r.id
          JOIN kb_resource_manifests m ON m.resource_id = r.id
         WHERE m.body_hash = $2
           AND r.is_active = true
         LIMIT 1
        "#,
        *profile_id,
        body_hash,
    )
    .fetch_optional(pool)
    .await?;

    match maybe_id {
        Some(id) => {
            let row = resource_service::get_visible(pool, *profile_id, id).await?;
            Ok(Some(row))
        }
        None => Ok(None),
    }
}

/// Batch-insert chunks for a new resource via SQL function.
/// Gates search triggers, does bulk INSERT, rebuilds search index once.
async fn persist_chunks(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    resource_id: ResourceId,
    chunks: &[PackedChunk],
) -> ApiResult<i32> {
    let chunks_json = chunks_to_jsonb(chunks);

    let count = sqlx::query_scalar!(
        "SELECT persist_resource_chunks($1, $2)",
        *resource_id,
        chunks_json
    )
    .fetch_one(&mut **tx)
    .await?
    .expect("persist_resource_chunks returned NULL");

    Ok(count)
}

/// Version-bump old chunks and batch-insert new ones via SQL function.
/// Gates search triggers, does bulk version-bump + INSERT, rebuilds once.
async fn replace_chunks(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    resource_id: ResourceId,
    chunks: &[PackedChunk],
) -> ApiResult<i32> {
    let chunks_json = chunks_to_jsonb(chunks);

    let count = sqlx::query_scalar!(
        "SELECT replace_resource_chunks($1, $2)",
        *resource_id,
        chunks_json
    )
    .fetch_one(&mut **tx)
    .await?
    .expect("replace_resource_chunks returned NULL");

    Ok(count)
}

/// Everything needed to create a resource with its manifest in one transaction.
#[derive(Debug)]
pub struct CreateResourceParams<'a> {
    pub profile_id: ProfileId,
    pub device_id: &'a str,
    pub context_id: ContextId,
    pub doc_type_id: Uuid,
    pub title: &'a str,
    pub slug: Option<&'a str>,
    pub origin_uri: &'a str,
    pub content_hash: &'a str,
    pub managed_meta: &'a serde_json::Value,
    pub open_meta: &'a serde_json::Value,
}

/// Create a resource with its manifest and event/audit trail in a single transaction.
///
/// This handles resource + manifest + event creation WITHOUT chunk insertion,
/// making it reusable for both the full ingest path (CLI with pre-computed chunks)
/// and the MCP content creation path (no chunks).
pub async fn create_resource_with_manifest(
    pool: &PgPool,
    params: &CreateResourceParams<'_>,
) -> ApiResult<ResourceRow> {
    let managed_hash = hash_json_value(params.managed_meta);
    let open_hash = hash_json_value(params.open_meta);

    let mut tx = pool.begin().await?;

    let resource_id = ResourceId::new();
    sqlx::query!(
        r#"
        INSERT INTO kb_resources (
            id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
            originator_profile_id, owner_profile_id,
            created, updated
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now(), now())
        "#,
        *resource_id,
        *params.context_id,
        params.doc_type_id,
        params.origin_uri,
        params.title,
        params.slug,
        *params.profile_id,
        *params.profile_id,
    )
    .execute(&mut *tx)
    .await?;

    // Insert manifest row
    sqlx::query!(
        r#"
        INSERT INTO kb_resource_manifests (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
        VALUES ($1, $2, $3, $4, $5, $6, now())
        "#,
        *resource_id,
        params.content_hash,
        params.managed_meta,
        params.open_meta,
        managed_hash,
        open_hash,
    )
    .execute(&mut *tx)
    .await?;

    // Insert event + audit atomically
    insert_event_and_audit(
        &mut tx,
        params.profile_id,
        params.device_id,
        params.context_id,
        resource_id,
        "resource_created",
        "create",
        params.content_hash,
        &managed_hash,
        &open_hash,
    )
    .await?;

    tx.commit().await?;

    // Re-fetch via the view to get full ResourceRow with joined fields
    resource_service::get_visible(pool, *params.profile_id, *resource_id).await
}

/// Process a full ingest payload: resolve names, dedup, insert resource + chunks.
pub async fn ingest(
    pool: &PgPool,
    profile_id: ProfileId,
    device_id: &str,
    payload: IngestPayload,
) -> ApiResult<ResourceRow> {
    // 1. Resolve context
    let context = context_service::resolve_by_name(pool, profile_id, &payload.context_name).await?;
    let context_id = context.id;

    // 2. Resolve doc_type
    let doc_type_id = resolve_doc_type(pool, &payload.doc_type_name).await?;

    // 3. Body-hash dedup
    if let Some(existing) = find_by_body_hash(pool, profile_id, &payload.content_hash).await? {
        return Ok(existing);
    }

    // 4. Decode chunks
    let chunks = unpack_chunks(&payload.chunks_packed)
        .map_err(|e| ApiError::BadRequest(format!("invalid chunks_packed: {e}")))?;

    // 5. Compute meta
    let empty_json = serde_json::json!({});
    let managed_meta = payload
        .managed_meta
        .clone()
        .unwrap_or_else(|| empty_json.clone());
    let open_meta = payload
        .open_meta
        .clone()
        .unwrap_or_else(|| empty_json.clone());

    // 6. Create resource + manifest + event
    let resource = create_resource_with_manifest(
        pool,
        &CreateResourceParams {
            profile_id,
            device_id,
            context_id,
            doc_type_id,
            title: &payload.title,
            slug: Some(payload.slug.as_str()),
            origin_uri: &payload.origin_uri,
            content_hash: &payload.content_hash,
            managed_meta: &managed_meta,
            open_meta: &open_meta,
        },
    )
    .await?;

    // 7. Insert chunks in a separate transaction (if any)
    if !chunks.is_empty() {
        let mut tx = pool.begin().await?;
        persist_chunks(&mut tx, resource.id, &chunks).await?;
        tx.commit().await?;
    }

    Ok(resource)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_empty_object() {
        let hash = hash_json_value(&serde_json::json!({}));
        assert_eq!(
            hash,
            "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
    }

    #[test]
    fn hash_key_order_independent() {
        let a = hash_json_value(&serde_json::json!({"b": 2, "a": 1}));
        let b = hash_json_value(&serde_json::json!({"a": 1, "b": 2}));
        assert_eq!(a, b);
    }

    #[test]
    fn hash_json_shared_fixture() {
        let fixture = serde_json::json!({
            "temper-type": "task",
            "temper-stage": "in-progress",
            "temper-seq": 42,
            "title": "Test task"
        });
        let hash = hash_json_value(&fixture);
        // This exact value must match the TypeScript canonicalJsonHash test
        assert_eq!(
            hash,
            "sha256:d39e1380d3b0ce969fe93f1df8b2da5d1caabf90b33e2e30f01d661f2c3c4895"
        );
    }
}

/// Update a resource's manifest (body hash, metadata hashes) and fire an event.
///
/// Updates the resource timestamp, upserts the manifest row, and inserts
/// a `body_updated` event + audit trail atomically. Does NOT handle chunks —
/// callers add chunk operations to the same transaction or separately.
/// Update a resource's manifest (body hash, metadata hashes) and fire an event.
///
/// Updates the resource timestamp, upserts the manifest row, and inserts
/// a `body_updated` event + audit trail atomically. The context_id for the
/// event is derived from the resource row itself (via UPDATE RETURNING).
///
/// Does NOT handle chunks — callers add chunk operations to the same
/// transaction or trigger async processing separately.
pub async fn update_resource_manifest(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    profile_id: ProfileId,
    device_id: &str,
    resource_id: ResourceId,
    content_hash: &str,
    managed_meta: &serde_json::Value,
    open_meta: &serde_json::Value,
) -> ApiResult<()> {
    let managed_hash = hash_json_value(managed_meta);
    let open_hash = hash_json_value(open_meta);

    let base = sqlx::query_as!(
        ResourceRowBase,
        r#"
        UPDATE kb_resources
        SET updated = now()
        WHERE id = $1
        RETURNING id, kb_context_id
        "#,
        *resource_id,
    )
    .fetch_one(&mut **tx)
    .await?;

    sqlx::query!(
        r#"
        INSERT INTO kb_resource_manifests (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
        VALUES ($1, $2, $3, $4, $5, $6, now())
        ON CONFLICT (resource_id)
        DO UPDATE SET body_hash = $2, managed_meta = $3, open_meta = $4,
                      managed_hash = $5, open_hash = $6, updated = now()
        "#,
        *resource_id,
        content_hash,
        managed_meta,
        open_meta,
        managed_hash,
        open_hash,
    )
    .execute(&mut **tx)
    .await?;

    insert_event_and_audit(
        tx,
        profile_id,
        device_id,
        base.kb_context_id,
        resource_id,
        "body_updated",
        "update_body",
        content_hash,
        &managed_hash,
        &open_hash,
    )
    .await?;

    Ok(())
}

/// Update an existing resource's content — re-chunk and re-embed.
pub async fn update(
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: ResourceId,
    device_id: &str,
    payload: IngestPayload,
) -> ApiResult<ResourceRow> {
    // Verify the profile can modify this resource
    let can_modify = sqlx::query_scalar!(
        "SELECT true FROM can_modify_resource($1, $2)",
        *profile_id,
        *resource_id,
    )
    .fetch_optional(pool)
    .await?;

    if can_modify.is_none() {
        return Err(ApiError::NotFound);
    }

    let chunks = unpack_chunks(&payload.chunks_packed)
        .map_err(|e| ApiError::BadRequest(format!("invalid chunks_packed: {e}")))?;

    // Compute meta
    let empty_json = serde_json::json!({});
    let managed_meta = payload
        .managed_meta
        .clone()
        .unwrap_or_else(|| empty_json.clone());
    let open_meta = payload
        .open_meta
        .clone()
        .unwrap_or_else(|| empty_json.clone());

    let mut tx = pool.begin().await?;

    // Update manifest + fire event (context_id derived from resource row)
    update_resource_manifest(
        &mut tx,
        profile_id,
        device_id,
        resource_id,
        &payload.content_hash,
        &managed_meta,
        &open_meta,
    )
    .await?;

    // Replace chunks — version-bump + batch insert + search rebuild in one call
    replace_chunks(&mut tx, resource_id, &chunks).await?;

    tx.commit().await?;

    // Re-fetch via the view to get full ResourceRow with joined fields
    resource_service::get_visible(pool, *profile_id, *resource_id).await
}
