//! Ingest service — accepts a fully-processed payload (content + chunks +
//! embeddings) and writes resource + chunks to the database in a single
//! transaction.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::services::context_service;
use temper_core::defaults::apply_open_defaults;
#[cfg(feature = "ingest-pipeline")]
use temper_core::hash::compute_body_hash;
use temper_core::hash::{compute_managed_hash, compute_open_hash};
use temper_core::schema::ValidationIssue;
use temper_core::types::ids::{
    ContextId, EventId, ProfileId, ResourceAuditId, ResourceId, RevisionId,
};
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

/// Remove identity and tier-1 audit fields from input `managed_meta`.
///
/// Agents may echo these back from a `get_resource` call; they should not cause
/// validation errors.
///
/// Uses `IDENTITY_FIELDS` and a subset of `TIER1_SYSTEM_FIELDS` from
/// `temper_core::frontmatter::fields`. Intentionally does NOT strip
/// `temper-context` or `temper-type` — those remain so the update path can
/// detect structural-move attempts (see `update()` lines that check for
/// context/type changes).
fn strip_system_managed_fields(mut meta: serde_json::Value) -> serde_json::Value {
    use temper_core::frontmatter::fields::{IDENTITY_FIELDS, TIER1_SYSTEM_FIELDS};

    // temper-context and temper-type are kept for structural-move detection.
    const KEEP_FOR_MOVE_DETECTION: &[&str] = &["temper-context", "temper-type"];

    if let Some(obj) = meta.as_object_mut() {
        for field in IDENTITY_FIELDS
            .iter()
            .chain(TIER1_SYSTEM_FIELDS.iter())
            .filter(|f| !KEEP_FOR_MOVE_DETECTION.contains(f))
        {
            if obj.remove(*field).is_some() {
                tracing::warn!(
                    field = *field,
                    "stripped system field from input managed_meta"
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

/// Inputs for [`insert_event_and_audit`].
///
/// The `kb_events.payload` JSONB always carries a base hash rollup
/// (`body_hash` / `managed_hash` / `open_hash`). `payload_extra` is merged on
/// top for event-type-specific enrichment — e.g. `managed_meta_updated`
/// events carry the set of changed keys. Body-only events leave it `None`:
/// there is no key set to enumerate for a blob, so body changes stay
/// hash-only by design.
pub struct InsertEventAndAuditParams<'a> {
    pub profile_id: ProfileId,
    pub device_id: &'a str,
    pub context_id: ContextId,
    pub resource_id: ResourceId,
    pub event_type: &'a str,
    pub action: &'a str,
    pub body_hash: &'a str,
    pub managed_hash: &'a str,
    pub open_hash: &'a str,
    /// Event-type-specific payload merged into the base hash rollup. `None`
    /// for events with no enrichment.
    pub payload_extra: Option<serde_json::Value>,
}

/// Insert an event and audit trail row atomically via the SQL function.
pub async fn insert_event_and_audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    params: InsertEventAndAuditParams<'_>,
) -> ApiResult<(EventId, ResourceAuditId)> {
    let event_id = EventId::new();
    let payload_extra = params
        .payload_extra
        .unwrap_or_else(|| serde_json::json!({}));

    let row: (Uuid, Uuid) = sqlx::query_as(
        "SELECT event_id, audit_id FROM insert_event_and_audit($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(event_id)
    .bind(params.profile_id)
    .bind(params.device_id)
    .bind(params.context_id)
    .bind(params.resource_id)
    .bind(params.event_type)
    .bind(params.action)
    .bind(params.body_hash)
    .bind(params.managed_hash)
    .bind(params.open_hash)
    .bind(payload_extra)
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

/// Resolve doc_type name by UUID from kb_doc_types.
///
/// Used by handlers that receive a `kb_doc_type_id` UUID on the wire and need
/// the corresponding name to construct a typed operations command.
///
/// Returns `ApiError::BadRequest` when no doc_type with the given ID exists.
pub async fn resolve_doc_type_name_by_id(pool: &PgPool, id: Uuid) -> ApiResult<String> {
    let name = sqlx::query_scalar!("SELECT name FROM kb_doc_types WHERE id = $1", id)
        .fetch_optional(pool)
        .await?;

    name.ok_or_else(|| ApiError::BadRequest(format!("unknown doc_type id: '{id}'")))
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
/// Returns the newly-created `RevisionId`.
async fn persist_chunks(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    resource_id: ResourceId,
    audit_id: ResourceAuditId,
    body_hash: &str,
    chunks: &[PackedChunk],
) -> ApiResult<RevisionId> {
    let chunks_json = chunks_to_jsonb(chunks);

    let rev: Uuid = sqlx::query_scalar!(
        "SELECT persist_resource_chunks($1::uuid, $2::uuid, $3::text, $4::jsonb)",
        *resource_id,
        *audit_id,
        body_hash,
        chunks_json
    )
    .fetch_one(&mut **tx)
    .await?
    .expect("persist_resource_chunks returned NULL");

    Ok(RevisionId::from(rev))
}

/// Version-bump old chunks and batch-insert new ones via SQL function.
/// Gates search triggers, does bulk version-bump + INSERT, rebuilds once.
/// Returns the newly-created `RevisionId`.
pub(crate) async fn replace_chunks(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    resource_id: ResourceId,
    audit_id: ResourceAuditId,
    body_hash: &str,
    chunks: &[PackedChunk],
) -> ApiResult<RevisionId> {
    let chunks_json = chunks_to_jsonb(chunks);

    let rev: Uuid = sqlx::query_scalar!(
        "SELECT replace_resource_chunks($1::uuid, $2::uuid, $3::text, $4::jsonb)",
        *resource_id,
        *audit_id,
        body_hash,
        chunks_json
    )
    .fetch_one(&mut **tx)
    .await?
    .expect("replace_resource_chunks returned NULL");

    Ok(RevisionId::from(rev))
}

/// Everything needed to create a resource with its manifest (and optional chunks)
/// in one transaction.
#[derive(Debug)]
pub struct CreateResourceParams<'a> {
    /// Canonical resource id, generated by the caller before validation so
    /// the same UUID is used for the schema-validation document and the row.
    pub id: ResourceId,
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

    let resource_id = params.id;
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
    let (_event_id, audit_id) = insert_event_and_audit(
        &mut tx,
        InsertEventAndAuditParams {
            profile_id: params.profile_id,
            device_id: params.device_id,
            context_id: params.context_id,
            resource_id,
            event_type: "resource_created",
            action: "create",
            body_hash: params.content_hash,
            managed_hash: &managed_hash,
            open_hash: &open_hash,
            payload_extra: None,
        },
    )
    .await?;

    // Persist chunks (if any) inside the same transaction so the full
    // resource + manifest + event + chunks write is one atomic unit.
    if let Some(packed) = params.chunks_packed {
        let chunks = unpack_chunks(packed)
            .map_err(|e| ApiError::BadRequest(format!("invalid chunks_packed: {e}")))?;
        if !chunks.is_empty() {
            persist_chunks(&mut tx, resource_id, audit_id, params.content_hash, &chunks).await?;
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
    temper_core::operations::apply_defaults_value(&payload.doc_type_name, &mut managed);
    // Inject canonical identity keys before validation + hashing so the
    // server-stored managed_meta JSONB matches the local canonical form.
    // Idempotent — if the caller already injected (CLI / MCP send-side
    // wiring), this is a byte-identical no-op.
    let injected_slug = if payload.slug.is_empty() {
        None
    } else {
        Some(payload.slug.as_str())
    };
    temper_core::operations::ensure_managed_identity_keys(
        &mut managed,
        &payload.title,
        injected_slug,
    );
    // Generate the canonical resource id up front — before validation — so the
    // schema-validation document carries the real `temper-id` rather than a
    // placeholder. The same id is threaded into `create_resource_with_manifest`.
    let resource_id = ResourceId::new();
    let validate_params = ValidateParams {
        id: *resource_id,
        created: Utc::now(),
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
        // Only compute a body hash when there is actual content — empty strings
        // are not deduplicated because two resources with no body are not
        // semantically equivalent to a single resource with an empty body.
        if !payload.content.is_empty() {
            payload.content_hash = Some(compute_body_hash(&payload.content));
        }
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
    let mut open_meta = payload
        .open_meta
        .clone()
        .unwrap_or_else(|| empty_json.clone());
    // Apply open-tier doc-type defaults (e.g. `date` for session/research).
    // Phase 6's Migration A established `date` lives in open_meta; this
    // matches that canonical shape on new ingests.
    apply_open_defaults(&payload.doc_type_name, &mut open_meta);

    // 5. Create resource + manifest + event + chunks atomically
    let resource = create_resource_with_manifest(
        pool,
        &CreateResourceParams {
            id: resource_id,
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

    // 6. Extract and upsert edges from frontmatter relationship fields
    if let Some(ref open) = payload.open_meta {
        if let Err(e) = super::edge_service::extract_and_upsert_edges(
            pool,
            &profile_id,
            &context_id,
            &resource.id,
            payload.doc_type_name.as_str(),
            &managed_meta,
            open,
        )
        .await
        {
            // Edge extraction is non-fatal — log and continue
            tracing::warn!(
                resource_id = %resource.id,
                error = %e,
                "edge extraction failed during ingest"
            );
        }
    }

    // 7. Re-project any pending slug-target assertions whose target slug now
    // matches the newly-created resource. Runs on a fresh transaction so the
    // resource row is visible to the slug resolution query inside
    // `reproject_pending_for_resource`. Non-fatal: a re-projection failure is
    // logged and does not roll back the resource creation.
    if let Err(e) = async {
        let mut tx = pool.begin().await?;
        super::relationship_service::reproject_pending_for_resource(
            &mut tx,
            *resource.id,
            &payload.slug,
            *context_id,
        )
        .await?;
        tx.commit().await?;
        Ok::<_, ApiError>(())
    }
    .await
    {
        tracing::warn!(
            resource_id = %resource.id,
            slug = %payload.slug,
            error = %e,
            "pending slug re-projection failed during ingest"
        );
    }

    Ok(resource)
}

/// Parameters for updating a resource's manifest hashes.
#[derive(Debug)]
pub struct UpdateManifestParams<'a> {
    pub profile_id: ProfileId,
    pub device_id: &'a str,
    pub resource_id: ResourceId,
    pub doc_type_name: &'a str,
    pub content_hash: &'a str,
    pub managed_meta: &'a serde_json::Value,
    pub open_meta: &'a serde_json::Value,
}

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
    params: &UpdateManifestParams<'_>,
) -> ApiResult<ResourceAuditId> {
    let managed_hash = compute_managed_hash(params.doc_type_name, params.managed_meta);
    let open_hash = compute_open_hash(params.open_meta);

    let base = sqlx::query_as!(
        ResourceRowBase,
        r#"
        UPDATE kb_resources
        SET updated = now()
        WHERE id = $1
        RETURNING id, kb_context_id
        "#,
        *params.resource_id,
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
        *params.resource_id,
        params.content_hash,
        params.managed_meta,
        params.open_meta,
        managed_hash,
        open_hash,
    )
    .execute(&mut **tx)
    .await?;

    let (_event_id, audit_id) = insert_event_and_audit(
        tx,
        InsertEventAndAuditParams {
            profile_id: params.profile_id,
            device_id: params.device_id,
            context_id: base.kb_context_id,
            resource_id: params.resource_id,
            event_type: "body_updated",
            action: "update_body",
            body_hash: params.content_hash,
            managed_hash: &managed_hash,
            open_hash: &open_hash,
            payload_extra: None,
        },
    )
    .await?;

    Ok(audit_id)
}

/// Update an existing resource's content — re-chunk and re-embed.
/// Parameters for schema validation at the service-layer boundary.
pub(crate) struct ValidateParams<'a> {
    /// Canonical resource id — generated by the caller before validation.
    pub id: Uuid,
    /// Creation timestamp.
    pub created: DateTime<Utc>,
    pub doc_type: &'a str,
    pub managed_meta: Option<&'a serde_json::Value>,
    pub slug: &'a str,
    pub title: &'a str,
    pub context_name: &'a str,
}

/// Validate managed_meta against the doc_type schema.
///
/// Delegates document assembly to
/// [`temper_core::operations::assemble_frontmatter_document`] — the shared
/// helper that composes the managed tier with the tier-1/tier-2 identity keys
/// from typed inputs. The update path
/// (`resource_service::update`) uses the same helper, so identity injection is
/// defined in exactly one place.
pub(crate) fn validate_managed_meta(params: &ValidateParams<'_>) -> Result<(), IngestError> {
    use serde_json::json;

    let managed: serde_json::Value = params.managed_meta.cloned().unwrap_or_else(|| json!({}));
    if !managed.is_object() {
        return Err(IngestError::InvalidManagedMeta(
            "managed_meta must be a JSON object".to_owned(),
        ));
    }

    let identity = temper_core::operations::FrontmatterIdentity {
        id: params.id,
        created: params.created,
        context: params.context_name,
        doc_type: params.doc_type,
        title: params.title,
        slug: (!params.slug.is_empty()).then_some(params.slug),
    };
    let document = temper_core::operations::assemble_frontmatter_document(&managed, &identity);

    let yaml_value: serde_yaml::Value = serde_yaml::to_value(&document)
        .map_err(|e| IngestError::InvalidManagedMeta(format!("JSON→YAML conversion: {e}")))?;

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
            id: Uuid::now_v7(),
            created: Utc::now(),
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
    fn task_missing_temper_stage_is_defaulted_then_validates() {
        // `temper-stage` is schema-required for tasks but carries a doc-type
        // default ("backlog"). The shared assembly helper applies doc-type
        // defaults before validation, so an input that omits it is filled in
        // rather than rejected — matching the system contract that
        // schema-required defaults are populated at write time.
        let managed_meta = json!({"temper-mode": "build"});
        let params = ValidateParams {
            id: Uuid::now_v7(),
            created: Utc::now(),
            doc_type: "task",
            managed_meta: Some(&managed_meta),
            slug: "test-task",
            title: "Test Task",
            context_name: "ctx",
        };
        let result = validate_managed_meta(&params);
        assert!(
            result.is_ok(),
            "missing temper-stage should be defaulted, not rejected: {result:?}"
        );
    }

    #[test]
    fn validates_session_with_date_in_managed_meta() {
        let managed_meta = json!({"date": "2026-04-10"});
        let params = ValidateParams {
            id: Uuid::now_v7(),
            created: Utc::now(),
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
            id: Uuid::now_v7(),
            created: Utc::now(),
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
            id: Uuid::now_v7(),
            created: Utc::now(),
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
        // Task with an out-of-enum temper-stage — fails the schema enum
        // constraint. (A *missing* temper-stage is defaulted by the assembly
        // helper, so a genuine validation failure needs a bad value, not an
        // absent one.)
        let managed_meta = json!({"temper-stage": "not-a-real-stage"});
        let params = ValidateParams {
            id: Uuid::now_v7(),
            created: Utc::now(),
            doc_type: "task",
            managed_meta: Some(&managed_meta),
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
