//! Ingest service — accepts a fully-processed payload (content + chunks +
//! embeddings) and writes resource + chunks to the database in a single
//! transaction.

use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::context_service;
use crate::services::search_service::format_embedding;

use temper_core::types::ingest::{unpack_chunks, IngestPayload, PackedChunk};
use temper_core::types::resource::ResourceRow;

/// Compute a `sha256:<hex>` hash of a JSON value (canonical form).
pub fn hash_json_value(value: &serde_json::Value) -> String {
    let serialized = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
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
) -> ApiResult<()> {
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
    Ok(())
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

/// Insert chunks with embeddings into kb_chunks + content into kb_chunk_content.
async fn insert_chunks(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    resource_id: Uuid,
    chunks: &[PackedChunk],
) -> ApiResult<()> {
    for chunk in chunks {
        let chunk_id = Uuid::now_v7();
        let embedding_str = format_embedding(&chunk.embedding);
        sqlx::query(
            r#"
            INSERT INTO kb_chunks (
                id, resource_id, chunk_index, version, header_path,
                content_hash, embedding, is_current
            )
            VALUES ($1, $2, $3, 1, $4, $5, $6::vector, true)
            "#,
        )
        .bind(chunk_id)
        .bind(resource_id)
        .bind(chunk.chunk_index as i32)
        .bind(&chunk.header_path)
        .bind(&chunk.content_hash)
        .bind(&embedding_str)
        .execute(&mut **tx)
        .await?;

        sqlx::query("INSERT INTO kb_chunk_content (chunk_id, content) VALUES ($1, $2)")
            .bind(chunk_id)
            .bind(&chunk.content)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Process a full ingest payload: resolve names, dedup, insert resource + chunks.
pub async fn ingest(
    pool: &PgPool,
    profile_id: Uuid,
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
    let managed_meta = payload.managed_meta.clone().unwrap_or_else(|| empty_json.clone());
    let open_meta = payload.open_meta.clone().unwrap_or_else(|| empty_json.clone());
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
    insert_chunks(&mut tx, resource_id, &chunks).await?;

    // 8. Insert event
    insert_event(
        &mut tx,
        profile_id,
        "api",
        Some(context.id),
        Some(resource_id),
        "resource.created",
        &serde_json::json!({"body_hash": &payload.content_hash}),
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
    let managed_meta = payload.managed_meta.clone().unwrap_or_else(|| empty_json.clone());
    let open_meta = payload.open_meta.clone().unwrap_or_else(|| empty_json.clone());
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

    // Version-bump old chunks
    sqlx::query(
        "UPDATE kb_chunks SET is_current = false WHERE resource_id = $1 AND is_current = true",
    )
    .bind(resource_id)
    .execute(&mut *tx)
    .await?;

    // Insert new chunks (version auto-computed)
    for chunk in &chunks {
        let chunk_id = Uuid::now_v7();
        let embedding_str = format_embedding(&chunk.embedding);
        sqlx::query(
            r#"
            INSERT INTO kb_chunks (
                id, resource_id, chunk_index, version, header_path,
                content_hash, embedding, is_current
            )
            VALUES ($1, $2, $3,
                    COALESCE((SELECT MAX(version) FROM kb_chunks
                              WHERE resource_id = $2 AND chunk_index = $3), 0) + 1,
                    $4, $5, $6::vector, true)
            "#,
        )
        .bind(chunk_id)
        .bind(resource_id)
        .bind(chunk.chunk_index as i32)
        .bind(&chunk.header_path)
        .bind(&chunk.content_hash)
        .bind(&embedding_str)
        .execute(&mut *tx)
        .await?;

        sqlx::query("INSERT INTO kb_chunk_content (chunk_id, content) VALUES ($1, $2)")
            .bind(chunk_id)
            .bind(&chunk.content)
            .execute(&mut *tx)
            .await?;
    }

    // Insert event
    insert_event(
        &mut tx,
        profile_id,
        "api",
        Some(resource.kb_context_id),
        Some(resource_id),
        "resource.modified",
        &serde_json::json!({"body_hash": &payload.content_hash}),
    )
    .await?;

    tx.commit().await?;

    Ok(resource)
}
