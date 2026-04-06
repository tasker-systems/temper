//! Ingest service — accepts a fully-processed payload (content + chunks +
//! embeddings) and writes resource + chunks to the database in a single
//! transaction.

use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::context_service;
use temper_core::types::ingest::chunks_to_jsonb;

use temper_core::types::ingest::{unpack_chunks, IngestPayload, PackedChunk};
use temper_core::types::resource::ResourceRow;

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

/// Insert an event into kb_events.
pub async fn insert_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    profile_id: Uuid,
    device_id: &str,
    context_id: Option<Uuid>,
    resource_id: Option<Uuid>,
    event_type: &str,
    payload: &serde_json::Value,
) -> ApiResult<Uuid> {
    let event_id = Uuid::now_v7();
    sqlx::query(
        r#"
        INSERT INTO kb_events (id, profile_id, device_id, kb_context_id, resource_id, event_type, payload, created)
        VALUES ($1, $2, $3, $4, $5, $6, $7, now())
        "#,
    )
    .bind(event_id)
    .bind(profile_id)
    .bind(device_id)
    .bind(context_id)
    .bind(resource_id)
    .bind(event_type)
    .bind(payload)
    .execute(&mut **tx)
    .await?;
    Ok(event_id)
}

/// Insert an audit trail row into kb_resource_audits.
#[expect(
    clippy::too_many_arguments,
    reason = "audit row requires all hash fields plus identifiers"
)]
pub async fn insert_audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    resource_id: Uuid,
    event_id: Uuid,
    profile_id: Uuid,
    device_id: &str,
    body_hash: &str,
    managed_hash: &str,
    open_hash: &str,
    action: &str,
) -> ApiResult<Uuid> {
    let audit_id: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO kb_resource_audits
            (resource_id, event_id, profile_id, device_id, body_hash, managed_hash, open_hash, action)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING id
        "#,
    )
    .bind(resource_id)
    .bind(event_id)
    .bind(profile_id)
    .bind(device_id)
    .bind(body_hash)
    .bind(managed_hash)
    .bind(open_hash)
    .bind(action)
    .fetch_one(&mut **tx)
    .await?;
    Ok(audit_id.0)
}

/// Resolve doc_type name to UUID from kb_doc_types.
async fn resolve_doc_type(pool: &PgPool, name: &str) -> ApiResult<Uuid> {
    let row: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM kb_doc_types WHERE name = $1")
        .bind(name)
        .fetch_optional(pool)
        .await?;

    row.map(|(id,)| id)
        .ok_or_else(|| ApiError::BadRequest(format!("unknown doc_type: '{name}'")))
}

/// Check for body-hash dedup — returns existing resource if hash matches.
async fn find_by_body_hash(
    pool: &PgPool,
    profile_id: Uuid,
    body_hash: &str,
) -> ApiResult<Option<ResourceRow>> {
    let row = sqlx::query_as::<_, ResourceRow>(
        r#"
        WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
        SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.origin_uri, r.title,
               r.slug,
               r.originator_profile_id, r.owner_profile_id, r.is_active,
               r.created, r.updated
          FROM kb_resources r
          JOIN visible v ON v.resource_id = r.id
          JOIN kb_resource_manifests m ON m.resource_id = r.id
         WHERE m.body_hash = $2
           AND r.is_active = true
         LIMIT 1
        "#,
    )
    .bind(profile_id)
    .bind(body_hash)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Batch-insert chunks for a new resource via SQL function.
/// Gates search triggers, does bulk INSERT, rebuilds search index once.
async fn persist_chunks(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    resource_id: Uuid,
    chunks: &[PackedChunk],
) -> ApiResult<i32> {
    let chunks_json = chunks_to_jsonb(chunks);

    let (count,): (i32,) = sqlx::query_as("SELECT persist_resource_chunks($1, $2)")
        .bind(resource_id)
        .bind(&chunks_json)
        .fetch_one(&mut **tx)
        .await?;

    Ok(count)
}

/// Version-bump old chunks and batch-insert new ones via SQL function.
/// Gates search triggers, does bulk version-bump + INSERT, rebuilds once.
async fn replace_chunks(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    resource_id: Uuid,
    chunks: &[PackedChunk],
) -> ApiResult<i32> {
    let chunks_json = chunks_to_jsonb(chunks);

    let (count,): (i32,) = sqlx::query_as("SELECT replace_resource_chunks($1, $2)")
        .bind(resource_id)
        .bind(&chunks_json)
        .fetch_one(&mut **tx)
        .await?;

    Ok(count)
}

/// Process a full ingest payload: resolve names, dedup, insert resource + chunks.
pub async fn ingest(
    pool: &PgPool,
    profile_id: Uuid,
    device_id: &str,
    payload: IngestPayload,
) -> ApiResult<ResourceRow> {
    // 1. Resolve context
    let context = context_service::resolve_by_name(pool, profile_id, &payload.context_name).await?;

    // 2. Resolve doc_type
    let doc_type_id = resolve_doc_type(pool, &payload.doc_type_name).await?;

    // 3. Body-hash dedup
    if let Some(existing) = find_by_body_hash(pool, profile_id, &payload.content_hash).await? {
        return Ok(existing);
    }

    // 4. Decode chunks
    let chunks = unpack_chunks(&payload.chunks_packed)
        .map_err(|e| ApiError::BadRequest(format!("invalid chunks_packed: {e}")))?;

    // 5. Compute meta hashes
    let empty_json = serde_json::json!({});
    let managed_meta = payload
        .managed_meta
        .clone()
        .unwrap_or_else(|| empty_json.clone());
    let open_meta = payload
        .open_meta
        .clone()
        .unwrap_or_else(|| empty_json.clone());
    let managed_hash = hash_json_value(&managed_meta);
    let open_hash = hash_json_value(&open_meta);

    // 6. Insert resource + manifest + chunks in a transaction
    let mut tx = pool.begin().await?;

    let resource_id = Uuid::now_v7();
    let resource = sqlx::query_as::<_, ResourceRow>(
        r#"
        INSERT INTO kb_resources (
            id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
            originator_profile_id, owner_profile_id,
            created, updated
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $7, now(), now())
        RETURNING id, kb_context_id, kb_doc_type_id, origin_uri, title,
                  slug,
                  originator_profile_id, owner_profile_id, is_active,
                  created, updated
        "#,
    )
    .bind(resource_id)
    .bind(context.id)
    .bind(doc_type_id)
    .bind(&payload.origin_uri)
    .bind(&payload.title)
    .bind(&payload.slug)
    .bind(profile_id)
    .fetch_one(&mut *tx)
    .await?;

    // Insert manifest row
    sqlx::query(
        r#"
        INSERT INTO kb_resource_manifests (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
        VALUES ($1, $2, $3, $4, $5, $6, now())
        "#,
    )
    .bind(resource_id)
    .bind(&payload.content_hash)
    .bind(&managed_meta)
    .bind(&open_meta)
    .bind(&managed_hash)
    .bind(&open_hash)
    .execute(&mut *tx)
    .await?;

    // 7. Insert chunks with embeddings
    persist_chunks(&mut tx, resource_id, &chunks).await?;

    // 8. Insert event
    let event_id = insert_event(
        &mut tx,
        profile_id,
        device_id,
        Some(context.id),
        Some(resource_id),
        "resource_created",
        &serde_json::json!({
            "body_hash": &payload.content_hash,
            "managed_hash": &managed_hash,
            "open_hash": &open_hash,
        }),
    )
    .await?;

    insert_audit(
        &mut tx,
        resource_id,
        event_id,
        profile_id,
        device_id,
        &payload.content_hash,
        &managed_hash,
        &open_hash,
        "create",
    )
    .await?;

    tx.commit().await?;

    Ok(resource)
}

/// Update an existing resource's content — re-chunk and re-embed.
pub async fn update(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
    device_id: &str,
    payload: IngestPayload,
) -> ApiResult<ResourceRow> {
    // Verify the profile can modify this resource
    let can_modify: Option<(bool,)> =
        sqlx::query_as("SELECT true FROM can_modify_resource($1, $2)")
            .bind(profile_id)
            .bind(resource_id)
            .fetch_optional(pool)
            .await?;

    if can_modify.is_none() {
        return Err(ApiError::NotFound);
    }

    let chunks = unpack_chunks(&payload.chunks_packed)
        .map_err(|e| ApiError::BadRequest(format!("invalid chunks_packed: {e}")))?;

    // Compute meta hashes
    let empty_json = serde_json::json!({});
    let managed_meta = payload
        .managed_meta
        .clone()
        .unwrap_or_else(|| empty_json.clone());
    let open_meta = payload
        .open_meta
        .clone()
        .unwrap_or_else(|| empty_json.clone());
    let managed_hash = hash_json_value(&managed_meta);
    let open_hash = hash_json_value(&open_meta);

    let mut tx = pool.begin().await?;

    // Update resource timestamp
    let resource = sqlx::query_as::<_, ResourceRow>(
        r#"
        UPDATE kb_resources
        SET updated = now()
        WHERE id = $1
        RETURNING id, kb_context_id, kb_doc_type_id, origin_uri, title,
                  slug,
                  originator_profile_id, owner_profile_id, is_active,
                  created, updated
        "#,
    )
    .bind(resource_id)
    .fetch_one(&mut *tx)
    .await?;

    // Upsert manifest row
    sqlx::query(
        r#"
        INSERT INTO kb_resource_manifests (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
        VALUES ($1, $2, $3, $4, $5, $6, now())
        ON CONFLICT (resource_id)
        DO UPDATE SET body_hash = $2, managed_meta = $3, open_meta = $4,
                      managed_hash = $5, open_hash = $6, updated = now()
        "#,
    )
    .bind(resource_id)
    .bind(&payload.content_hash)
    .bind(&managed_meta)
    .bind(&open_meta)
    .bind(&managed_hash)
    .bind(&open_hash)
    .execute(&mut *tx)
    .await?;

    // Replace chunks — version-bump + batch insert + search rebuild in one call
    replace_chunks(&mut tx, resource_id, &chunks).await?;

    // Insert event
    let event_id = insert_event(
        &mut tx,
        profile_id,
        device_id,
        Some(resource.kb_context_id),
        Some(resource_id),
        "body_updated",
        &serde_json::json!({
            "body_hash": &payload.content_hash,
            "managed_hash": &managed_hash,
            "open_hash": &open_hash,
        }),
    )
    .await?;

    insert_audit(
        &mut tx,
        resource_id,
        event_id,
        profile_id,
        device_id,
        &payload.content_hash,
        &managed_hash,
        &open_hash,
        "update_body",
    )
    .await?;

    tx.commit().await?;

    Ok(resource)
}
