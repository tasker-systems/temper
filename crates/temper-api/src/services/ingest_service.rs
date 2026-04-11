//! Ingest service — accepts a fully-processed payload (content + chunks +
//! embeddings) and writes resource + chunks to the database in a single
//! transaction.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::context_service;
use temper_core::defaults::apply_doc_type_defaults;
#[cfg(feature = "ingest-pipeline")]
use temper_core::hash::compute_body_hash;
use temper_core::hash::{compute_managed_hash, compute_open_hash};
use temper_core::schema::ValidationIssue;
use temper_core::types::ids::{ContextId, EventId, ProfileId, ResourceAuditId, ResourceId};
use temper_core::types::ingest::chunks_to_jsonb;

use temper_core::types::ingest::{unpack_chunks, IngestPayload, PackedChunk};
use temper_core::types::resource::ResourceRow;

use super::resource_service;

/// Domain errors for the ingest pipeline.
///
/// Converts to [`ApiError`] for HTTP responses via the `From` impl.
#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    /// Schema validation failed for managed_meta fields.
    #[error("managed_meta validation failed for doc_type={doc_type}: {issues_count} issues", issues_count = .issues.len())]
    Validation {
        doc_type: String,
        issues: Vec<ValidationIssue>,
    },
    /// Attempted a structural move (context, doc_type) via the ingest path.
    #[error("structural move via field '{field}' is not supported: {message}")]
    StructuralMoveNotSupported { field: String, message: String },
    /// managed_meta has an invalid shape (not a JSON object, etc.).
    #[error("invalid managed_meta shape: {0}")]
    InvalidManagedMeta(String),
    /// Database error during ingest.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    /// Embedding/pipeline error (only available with ingest-pipeline feature).
    #[cfg(feature = "ingest-pipeline")]
    #[error("embed failed: {0}")]
    Embed(String),
    /// Chunk packing error.
    #[cfg(feature = "ingest-pipeline")]
    #[error("chunks pack failed: {0}")]
    Pack(String),
}

impl From<IngestError> for crate::error::ApiError {
    fn from(err: IngestError) -> Self {
        match err {
            IngestError::Validation { doc_type, issues } => {
                let detail: Vec<String> = issues
                    .iter()
                    .map(|i| format!("{}: {}", i.path, i.message))
                    .collect();
                crate::error::ApiError::BadRequest(format!(
                    "managed_meta validation failed for doc_type={doc_type}: {}",
                    detail.join("; ")
                ))
            }
            IngestError::StructuralMoveNotSupported { field, message } => {
                crate::error::ApiError::BadRequest(format!(
                    "structural move via field '{field}' is not supported: {message}"
                ))
            }
            IngestError::InvalidManagedMeta(msg) => {
                crate::error::ApiError::BadRequest(format!("invalid managed_meta shape: {msg}"))
            }
            IngestError::Database(e) => crate::error::ApiError::from(e),
            #[cfg(feature = "ingest-pipeline")]
            IngestError::Embed(msg) => {
                crate::error::ApiError::Internal(format!("embed failed: {msg}"))
            }
            #[cfg(feature = "ingest-pipeline")]
            IngestError::Pack(msg) => {
                crate::error::ApiError::Internal(format!("chunks pack failed: {msg}"))
            }
        }
    }
}

/// Remove tier-1 identity/audit fields from input `managed_meta`.
///
/// Agents may echo these back from a `get_resource` call; they should not cause
/// validation errors. Tier-2 fields (`temper-context`, `temper-type`, `slug`) are
/// NOT stripped here — they remain present so we can detect structural-move
/// attempts in the update path.
fn strip_system_managed_fields(mut meta: serde_json::Value) -> serde_json::Value {
    const TIER1_FIELDS: &[&str] = &[
        "temper-id",
        "temper-provisional-id",
        "temper-created",
        "temper-updated",
        "temper-owner",
        "temper-source",
        "temper-legacy-id",
    ];
    if let Some(obj) = meta.as_object_mut() {
        for field in TIER1_FIELDS {
            if obj.remove(*field).is_some() {
                tracing::warn!(
                    field = *field,
                    "stripped tier-1 system-managed field from input managed_meta"
                );
            }
        }
    }
    meta
}

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

/// Everything needed to create a resource with its manifest (and optional chunks)
/// in one transaction.
#[derive(Debug)]
pub struct CreateResourceParams<'a> {
    pub profile_id: ProfileId,
    pub device_id: &'a str,
    pub context_id: ContextId,
    pub doc_type_id: Uuid,
    pub doc_type_name: &'a str,
    pub title: &'a str,
    pub slug: Option<&'a str>,
    pub origin_uri: &'a str,
    pub content_hash: &'a str,
    pub managed_meta: &'a serde_json::Value,
    pub open_meta: &'a serde_json::Value,
    /// Wire-format (`base64`-encoded MessagePack) packed chunks. `None` for
    /// callers that have no chunks to persist (e.g. the MCP create-resource
    /// path, which delegates content ingest to a follow-up async POST).
    pub chunks_packed: Option<&'a str>,
}

/// Create a resource with its manifest, event/audit trail, and chunk rows in
/// a single transaction.
///
/// This handles resource + manifest + event creation, plus optional chunk
/// persistence, making it reusable for both the full ingest path (CLI with
/// pre-computed chunks) and the MCP content creation path (no chunks).
///
/// # Atomicity
///
/// All writes performed by this function — the `kb_resources` insert, the
/// `kb_resource_manifests` insert, the `kb_events` + `kb_resource_audits` rows
/// produced by `insert_event_and_audit`, and the `kb_resource_chunks` rows
/// inserted via `persist_resource_chunks` — run inside a single
/// `sqlx::Transaction` opened at the top of the function and committed at the
/// end. A mid-call failure aborts the transaction and leaves no partial state.
/// Callers do not need (and cannot) extend the transaction across the call
/// boundary — every write that belongs with the resource is included.
pub async fn create_resource_with_manifest(
    pool: &PgPool,
    params: &CreateResourceParams<'_>,
) -> ApiResult<ResourceRow> {
    let managed_hash = compute_managed_hash(params.doc_type_name, params.managed_meta);
    let open_hash = compute_open_hash(params.open_meta);

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

    // Persist chunks (if any) inside the same transaction so the full
    // resource + manifest + event + chunks write is one atomic unit.
    if let Some(packed) = params.chunks_packed {
        let chunks = unpack_chunks(packed)
            .map_err(|e| ApiError::BadRequest(format!("invalid chunks_packed: {e}")))?;
        if !chunks.is_empty() {
            persist_chunks(&mut tx, resource_id, &chunks).await?;
        }
    }

    tx.commit().await?;

    // Re-fetch via the view to get full ResourceRow with joined fields
    resource_service::get_visible(pool, *params.profile_id, *resource_id).await
}

/// Process a full ingest payload: resolve names, dedup, insert resource + chunks.
pub async fn ingest(
    pool: &PgPool,
    profile_id: ProfileId,
    device_id: &str,
    mut payload: IngestPayload,
) -> ApiResult<ResourceRow> {
    // 1. Resolve context
    let context = context_service::resolve_by_name(pool, profile_id, &payload.context_name).await?;
    let context_id = context.id;

    // 2. Resolve doc_type
    let doc_type_id = resolve_doc_type(pool, &payload.doc_type_name).await?;

    // 2.5. Strip tier-1 fields, apply doc-type defaults, and validate managed_meta
    let mut managed = payload
        .managed_meta
        .take()
        .map(strip_system_managed_fields)
        .unwrap_or_else(|| serde_json::json!({}));
    apply_doc_type_defaults(&payload.doc_type_name, &mut managed);
    let validate_params = ValidateParams {
        doc_type: &payload.doc_type_name,
        managed_meta: Some(&managed),
        slug: &payload.slug,
        title: &payload.title,
        context_name: &payload.context_name,
    };
    validate_managed_meta(&validate_params).map_err(ApiError::from)?;
    payload.managed_meta = Some(managed);

    // 2.6. If chunks_packed is absent, run the shared pipeline (ingest-pipeline feature)
    #[cfg(feature = "ingest-pipeline")]
    if payload.chunks_packed.is_none() {
        payload.content_hash = Some(compute_body_hash(&payload.content));
        let packed_chunks = temper_ingest::pipeline::prepare_markdown(&payload.content)
            .map_err(|e| IngestError::Embed(e.to_string()))
            .map_err(ApiError::from)?;
        payload.chunks_packed = Some(
            temper_core::types::ingest::pack_chunks(&packed_chunks)
                .map_err(|e| IngestError::Pack(e.to_string()))
                .map_err(ApiError::from)?,
        );
    }

    // 2.7. If ingest-pipeline feature is not enabled and chunks are missing, caller must provide them
    #[cfg(not(feature = "ingest-pipeline"))]
    if payload.chunks_packed.is_none() && !payload.content.is_empty() {
        return Err(ApiError::BadRequest(
            "chunks_packed required when server-side pipeline is not available".to_owned(),
        ));
    }

    // 3. Body-hash dedup (only if caller supplied a hash)
    if let Some(ref hash) = payload.content_hash {
        if let Some(existing) = find_by_body_hash(pool, profile_id, hash).await? {
            return Ok(existing);
        }
    }

    // 4. Compute meta
    let empty_json = serde_json::json!({});
    let managed_meta = payload
        .managed_meta
        .clone()
        .unwrap_or_else(|| empty_json.clone());
    let open_meta = payload
        .open_meta
        .clone()
        .unwrap_or_else(|| empty_json.clone());

    // 5. Create resource + manifest + event + chunks atomically
    let resource = create_resource_with_manifest(
        pool,
        &CreateResourceParams {
            profile_id,
            device_id,
            context_id,
            doc_type_id,
            doc_type_name: &payload.doc_type_name,
            title: &payload.title,
            slug: Some(payload.slug.as_str()),
            origin_uri: &payload.origin_uri,
            content_hash: payload.content_hash.as_deref().unwrap_or(""),
            managed_meta: &managed_meta,
            open_meta: &open_meta,
            chunks_packed: payload.chunks_packed.as_deref(),
        },
    )
    .await?;

    Ok(resource)
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
#[expect(
    clippy::too_many_arguments,
    reason = "manifest update requires all hash inputs plus resource/profile identifiers"
)]
pub async fn update_resource_manifest(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    profile_id: ProfileId,
    device_id: &str,
    resource_id: ResourceId,
    doc_type_name: &str,
    content_hash: &str,
    managed_meta: &serde_json::Value,
    open_meta: &serde_json::Value,
) -> ApiResult<()> {
    let managed_hash = compute_managed_hash(doc_type_name, managed_meta);
    let open_hash = compute_open_hash(open_meta);

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
#[cfg_attr(
    not(feature = "ingest-pipeline"),
    allow(
        unused_mut,
        reason = "mut needed when ingest-pipeline feature is enabled"
    )
)]
pub async fn update(
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: ResourceId,
    device_id: &str,
    mut payload: IngestPayload,
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

    // Strip tier-1 fields, apply doc-type defaults, and check for tier-2 structural moves
    let mut managed = payload
        .managed_meta
        .take()
        .map(strip_system_managed_fields)
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = managed.as_object() {
        for field in &["temper-context", "temper-type"] {
            if obj.contains_key(*field) {
                return Err(IngestError::StructuralMoveNotSupported {
                    field: field.to_string(),
                    message: format!("use dedicated move command to change {field}"),
                }
                .into());
            }
        }
    }
    apply_doc_type_defaults(&payload.doc_type_name, &mut managed);
    payload.managed_meta = Some(managed);

    // If chunks_packed is absent, run the shared pipeline (ingest-pipeline feature)
    #[cfg(feature = "ingest-pipeline")]
    if payload.chunks_packed.is_none() {
        payload.content_hash = Some(compute_body_hash(&payload.content));
        let packed_chunks = temper_ingest::pipeline::prepare_markdown(&payload.content)
            .map_err(|e| IngestError::Embed(e.to_string()))
            .map_err(ApiError::from)?;
        payload.chunks_packed = Some(
            temper_core::types::ingest::pack_chunks(&packed_chunks)
                .map_err(|e| IngestError::Pack(e.to_string()))
                .map_err(ApiError::from)?,
        );
    }

    // If ingest-pipeline feature is not enabled and chunks are missing, caller must provide them
    #[cfg(not(feature = "ingest-pipeline"))]
    if payload.chunks_packed.is_none() && !payload.content.is_empty() {
        return Err(ApiError::BadRequest(
            "chunks_packed required when server-side pipeline is not available".to_owned(),
        ));
    }

    let chunks = if let Some(ref packed) = payload.chunks_packed {
        unpack_chunks(packed)
            .map_err(|e| ApiError::BadRequest(format!("invalid chunks_packed: {e}")))?
    } else {
        vec![]
    };

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
        &payload.doc_type_name,
        payload.content_hash.as_deref().unwrap_or(""),
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

/// Parameters for schema validation at the service-layer boundary.
pub(crate) struct ValidateParams<'a> {
    pub doc_type: &'a str,
    pub managed_meta: Option<&'a serde_json::Value>,
    pub slug: &'a str,
    pub title: &'a str,
    pub context_name: &'a str,
}

/// Validate managed_meta against the doc_type schema, merging in top-level
/// parameters so schema-required tier-2 fields (slug, temper-context, temper-type)
/// are satisfied without the agent having to pass them inside managed_meta.
pub(crate) fn validate_managed_meta(params: &ValidateParams<'_>) -> Result<(), IngestError> {
    use serde_json::json;

    // 1. Start with managed_meta (or empty object)
    let mut synthetic: serde_json::Value =
        params.managed_meta.cloned().unwrap_or_else(|| json!({}));

    // 2. Strip tier-1 fields (defensive)
    synthetic = strip_system_managed_fields(synthetic);

    // 3. Ensure it's an object
    if !synthetic.is_object() {
        return Err(IngestError::InvalidManagedMeta(
            "managed_meta must be a JSON object".to_owned(),
        ));
    }

    let obj = synthetic.as_object_mut().unwrap();

    // 4. Inject tier-2 fields and tier-1 placeholders for schema required checks
    obj.insert("slug".to_owned(), json!(params.slug));
    obj.insert("title".to_owned(), json!(params.title));
    obj.insert("temper-context".to_owned(), json!(params.context_name));
    obj.insert("temper-type".to_owned(), json!(params.doc_type));
    obj.insert(
        "temper-id".to_owned(),
        json!("00000000-0000-0000-0000-000000000000"),
    );
    obj.insert("temper-created".to_owned(), json!("2000-01-01T00:00:00Z"));

    // 5. Convert JSON → serde_yaml::Value for validate_frontmatter
    let yaml_value: serde_yaml::Value = serde_yaml::to_value(&synthetic)
        .map_err(|e| IngestError::InvalidManagedMeta(format!("JSON→YAML conversion: {e}")))?;

    // 6. Validate
    let issues = temper_core::schema::validate_frontmatter(params.doc_type, &yaml_value)
        .map_err(|e| IngestError::InvalidManagedMeta(format!("schema load: {e}")))?;

    if issues.is_empty() {
        Ok(())
    } else {
        Err(IngestError::Validation {
            doc_type: params.doc_type.to_owned(),
            issues,
        })
    }
}

#[cfg(test)]
mod tests_validate_managed_meta {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_task_with_complete_managed_meta() {
        let managed_meta =
            json!({"temper-stage": "backlog", "temper-mode": "build", "temper-effort": "medium"});
        let params = ValidateParams {
            doc_type: "task",
            managed_meta: Some(&managed_meta),
            slug: "test-task",
            title: "Test Task",
            context_name: "ctx",
        };
        let result = validate_managed_meta(&params);
        assert!(
            result.is_ok(),
            "task with complete meta should validate: {result:?}"
        );
    }

    #[test]
    fn rejects_task_missing_temper_stage() {
        let managed_meta = json!({"temper-mode": "build"});
        let params = ValidateParams {
            doc_type: "task",
            managed_meta: Some(&managed_meta),
            slug: "test-task",
            title: "Test Task",
            context_name: "ctx",
        };
        let result = validate_managed_meta(&params);
        match result {
            Err(IngestError::Validation { doc_type, issues }) => {
                assert_eq!(doc_type, "task");
                assert!(
                    issues
                        .iter()
                        .any(|i| i.path.contains("temper-stage")
                            || i.message.contains("temper-stage")),
                    "should mention temper-stage: {issues:?}"
                );
            }
            other => panic!("expected Validation error, got {other:?}"),
        }
    }

    #[test]
    fn validates_session_with_date_in_managed_meta() {
        let managed_meta = json!({"date": "2026-04-10"});
        let params = ValidateParams {
            doc_type: "session",
            managed_meta: Some(&managed_meta),
            slug: "test-session",
            title: "Test Session",
            context_name: "ctx",
        };
        let result = validate_managed_meta(&params);
        assert!(
            result.is_ok(),
            "session with date in managed_meta should validate: {result:?}"
        );
    }

    #[test]
    fn synthetic_merge_injects_slug_from_params() {
        let managed_meta = json!({"temper-stage": "backlog"});
        let params = ValidateParams {
            doc_type: "task",
            managed_meta: Some(&managed_meta),
            slug: "slug-from-params",
            title: "T",
            context_name: "ctx",
        };
        let result = validate_managed_meta(&params);
        assert!(
            result.is_ok(),
            "slug from params should satisfy schema: {result:?}"
        );
    }

    #[test]
    fn rejects_invalid_enum_value() {
        let managed_meta = json!({"temper-stage": "bogus-stage"});
        let params = ValidateParams {
            doc_type: "task",
            managed_meta: Some(&managed_meta),
            slug: "t",
            title: "T",
            context_name: "ctx",
        };
        let result = validate_managed_meta(&params);
        match result {
            Err(IngestError::Validation { issues, .. }) => {
                assert!(!issues.is_empty(), "should have validation issues");
            }
            other => panic!("expected Validation error, got {other:?}"),
        }
    }

    #[test]
    fn validation_error_includes_field_details() {
        // Task with no managed_meta — should fail validation because temper-stage is required
        let params = ValidateParams {
            doc_type: "task",
            managed_meta: None,
            slug: "test",
            title: "test",
            context_name: "test",
        };

        let err = validate_managed_meta(&params).unwrap_err();
        // Convert to ApiError to test the user-facing message
        let api_err = crate::error::ApiError::from(err);
        let msg = format!("{api_err}");
        // The error message should include field-level detail, not just a count
        assert!(
            !msg.contains(" issues"),
            "error should not just show a count: {msg}"
        );
        // Should include at least one field path
        assert!(
            msg.contains(':'),
            "error should include field: message detail: {msg}"
        );
    }
}

#[cfg(test)]
mod tests_ingest_error {
    use super::*;

    #[test]
    fn validation_error_carries_issues() {
        let err = IngestError::Validation {
            doc_type: "task".to_owned(),
            issues: vec![ValidationIssue {
                path: "temper-stage".to_owned(),
                message: "temper-stage is required".to_owned(),
                auto_fixable: false,
            }],
        };
        match err {
            IngestError::Validation { doc_type, issues } => {
                assert_eq!(doc_type, "task");
                assert_eq!(issues.len(), 1);
                assert_eq!(issues[0].path, "temper-stage");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn structural_move_not_supported_carries_field() {
        let err = IngestError::StructuralMoveNotSupported {
            field: "temper-context".to_owned(),
            message: "use `temper resource update --context-to` to move".to_owned(),
        };
        match err {
            IngestError::StructuralMoveNotSupported { field, .. } => {
                assert_eq!(field, "temper-context");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn validation_error_converts_to_bad_request() {
        let err = IngestError::Validation {
            doc_type: "task".to_owned(),
            issues: vec![],
        };
        let api_err: crate::error::ApiError = err.into();
        match api_err {
            crate::error::ApiError::BadRequest(msg) => {
                assert!(msg.contains("task"));
            }
            _ => panic!("expected BadRequest"),
        }
    }
}

#[cfg(test)]
mod tests_strip_system_managed_fields {
    use super::*;
    use serde_json::json;

    #[test]
    fn strips_tier1_fields() {
        let input = json!({
            "temper-id": "abc",
            "temper-created": "2026-04-09",
            "temper-owner": "@me",
            "temper-stage": "backlog"
        });
        let stripped = strip_system_managed_fields(input);
        let obj = stripped.as_object().unwrap();
        assert!(!obj.contains_key("temper-id"));
        assert!(!obj.contains_key("temper-created"));
        assert!(!obj.contains_key("temper-owner"));
        assert!(obj.contains_key("temper-stage"), "tier-3 fields preserved");
    }

    #[test]
    fn strips_all_system_managed_fields() {
        let input = json!({
            "temper-id": "a",
            "temper-provisional-id": "b",
            "temper-created": "c",
            "temper-updated": "d",
            "temper-owner": "e",
            "temper-source": "f",
            "temper-legacy-id": "g",
            "temper-stage": "backlog"
        });
        let stripped = strip_system_managed_fields(input);
        let obj = stripped.as_object().unwrap();
        assert_eq!(obj.len(), 1);
        assert!(obj.contains_key("temper-stage"));
    }

    #[test]
    fn handles_non_object_value() {
        let input = serde_json::Value::Null;
        let stripped = strip_system_managed_fields(input);
        assert!(stripped.is_null());
    }
}
