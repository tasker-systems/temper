//! Ingest service — accepts a fully-processed payload (content + chunks +
//! embeddings) and writes resource + chunks to the database in a single
//! transaction.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::context_service;
use crate::services::search_service::format_embedding;

use temper_core::types::ingest::{unpack_chunks, IngestPayload, PackedChunk};
use temper_core::types::resource::ResourceRow;

/// Resolve doc_type name to UUID from kb_doc_types.
async fn resolve_doc_type(pool: &PgPool, name: &str) -> ApiResult<Uuid> {
    let row: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM kb_doc_types WHERE name = $1")
        .bind(name)
        .fetch_optional(pool)
        .await?;

    row.map(|(id,)| id)
        .ok_or_else(|| ApiError::BadRequest(format!("unknown doc_type: '{name}'")))
}

/// Check for content-hash dedup — returns existing resource if hash matches.
async fn find_by_content_hash(
    pool: &PgPool,
    profile_id: Uuid,
    content_hash: &str,
) -> ApiResult<Option<ResourceRow>> {
    let row = sqlx::query_as::<_, ResourceRow>(
        r#"
        WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
        SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.origin_uri, r.title,
               r.slug, r.content_hash, r.mimetype,
               r.originator_profile_id, r.owner_profile_id, r.is_active,
               r.created, r.updated
          FROM kb_resources r
          JOIN visible v ON v.resource_id = r.id
         WHERE r.content_hash = $2
           AND r.is_active = true
         LIMIT 1
        "#,
    )
    .bind(profile_id)
    .bind(content_hash)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Insert chunks with embeddings into kb_chunks.
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
                content, content_hash, embedding, is_current
            )
            VALUES ($1, $2, $3, 1, $4, $5, $6, $7::vector, true)
            "#,
        )
        .bind(chunk_id)
        .bind(resource_id)
        .bind(chunk.chunk_index as i32)
        .bind(&chunk.header_path)
        .bind(&chunk.content)
        .bind(&chunk.content_hash)
        .bind(&embedding_str)
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

    // 3. Content-hash dedup
    if let Some(existing) = find_by_content_hash(pool, profile_id, &payload.content_hash).await? {
        return Ok(existing);
    }

    // 4. Decode chunks
    let chunks = unpack_chunks(&payload.chunks_packed)
        .map_err(|e| ApiError::BadRequest(format!("invalid chunks_packed: {e}")))?;

    // 5. Insert resource + chunks in a transaction
    let mut tx = pool.begin().await?;

    let resource_id = Uuid::now_v7();
    let resource = sqlx::query_as::<_, ResourceRow>(
        r#"
        INSERT INTO kb_resources (
            id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
            content_hash, mimetype, resource_mode,
            originator_profile_id, owner_profile_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10)
        RETURNING id, kb_context_id, kb_doc_type_id, origin_uri, title,
                  slug, content_hash, mimetype,
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
    .bind(&payload.content_hash)
    .bind(&payload.mimetype)
    .bind(&payload.resource_mode)
    .bind(profile_id)
    .fetch_one(&mut *tx)
    .await?;

    // 6. Insert chunks with embeddings
    insert_chunks(&mut tx, resource_id, &chunks).await?;

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

    let mut tx = pool.begin().await?;

    // Update resource metadata
    let resource = sqlx::query_as::<_, ResourceRow>(
        r#"
        UPDATE kb_resources
        SET content_hash = $1, updated = now()
        WHERE id = $2
        RETURNING id, kb_context_id, kb_doc_type_id, origin_uri, title,
                  slug, content_hash, mimetype,
                  originator_profile_id, owner_profile_id, is_active,
                  created, updated
        "#,
    )
    .bind(&payload.content_hash)
    .bind(resource_id)
    .fetch_one(&mut *tx)
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
                content, content_hash, embedding, is_current
            )
            VALUES ($1, $2, $3,
                    COALESCE((SELECT MAX(version) FROM kb_chunks
                              WHERE resource_id = $2 AND chunk_index = $3), 0) + 1,
                    $4, $5, $6, $7::vector, true)
            "#,
        )
        .bind(chunk_id)
        .bind(resource_id)
        .bind(chunk.chunk_index as i32)
        .bind(&chunk.header_path)
        .bind(&chunk.content)
        .bind(&chunk.content_hash)
        .bind(&embedding_str)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(resource)
}
