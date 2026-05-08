//! Resource tools — unified CRUD with name-based resolution and optional content.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_api::backend::DbBackend;
use temper_api::services::{
    context_service, doc_type_service, ingest_service, meta_service, resource_service,
};
use temper_core::error::TemperError;
use temper_core::operations::{Backend, BodyUpdate, CreateResource, Surface};
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::managed_meta::{ManagedMeta, MetaUpdatePayload};

use crate::service::TemperMcpService;

// ── Input structs ──────────────────────────────────────────────────

/// MCP input for create_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateResourceInput {
    /// Human-readable context name (must already exist).
    pub context_name: String,
    /// Human-readable doc type name (e.g. "task", "session", "research").
    pub doc_type_name: String,
    /// Resource title.
    pub title: String,
    /// Optional markdown content body. Processed through the ingest
    /// pipeline (chunk + embed) synchronously on create.
    pub content: Option<String>,
    /// Optional URL-friendly slug.
    pub slug: Option<String>,
    /// Optional origin URI. Defaults to `mcp://agent/{uuid}`.
    pub origin_uri: Option<String>,
    /// Optional owner (defaults to @me). Reserved for future team scoping.
    pub owner: Option<String>,
    /// Managed frontmatter (temper-* fields) as JSON.
    #[serde(default)]
    pub managed_meta: Option<serde_json::Value>,
    /// Open frontmatter (user-owned fields) as JSON.
    #[serde(default)]
    pub open_meta: Option<serde_json::Value>,
}

/// MCP input for get_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetResourceInput {
    /// UUID of the resource. Provide either id or slug (not both).
    pub id: Option<Uuid>,
    /// Slug of the resource. Requires context_name for disambiguation.
    pub slug: Option<String>,
    /// Context name. Required when looking up by slug.
    pub context_name: Option<String>,
    /// If true, includes the full reconstituted markdown content.
    pub include_content: Option<bool>,
}

/// MCP input for list_resources.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListResourcesInput {
    /// Filter by context name.
    pub context_name: Option<String>,
    /// Filter by doc type name (e.g. "task", "research").
    pub doc_type_name: Option<String>,
    /// Max results (default 50, max 200).
    pub limit: Option<i64>,
    /// Pagination offset.
    pub offset: Option<i64>,
}

/// MCP input for update_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateResourceInput {
    /// UUID of the resource to update.
    pub id: Uuid,
    /// New title.
    pub title: Option<String>,
    /// New slug.
    pub slug: Option<String>,
    /// New markdown content. Replaces existing content and triggers
    /// re-processing.
    pub content: Option<String>,
    /// Managed frontmatter (temper-* fields) as JSON.
    #[serde(default)]
    pub managed_meta: Option<serde_json::Value>,
    /// Open frontmatter (user-owned fields) as JSON.
    #[serde(default)]
    pub open_meta: Option<serde_json::Value>,
}

/// MCP input for update_resource_meta.
///
/// Use when the caller wants to change only a resource's frontmatter
/// (managed_meta / open_meta) without re-chunking or re-embedding the
/// body. This is the MCP peer of `PUT /api/resources/{id}/meta`.
///
/// `managed_meta` is typed: agents get a schema-validated shape for
/// the fields temper knows about, and the `extra` flatten bucket on
/// `ManagedMeta` accepts any additional keys (doc-type-schema fields
/// like `date`, plus forward-compat unknowns) without dropping them.
/// `open_meta` stays a free-form JSON value by design — the open tier
/// is intentionally untyped.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateResourceMetaInput {
    /// UUID of the resource to update.
    pub id: Uuid,
    /// New managed (temper-*) frontmatter. Typed fields cover every
    /// key temper governs; extras round-trip through `ManagedMeta::extra`.
    pub managed_meta: ManagedMeta,
    /// New open (user-defined) frontmatter as JSON.
    pub open_meta: serde_json::Value,
    /// SHA-256 hash of the managed_meta JSON. The caller computes this
    /// the same way the CLI sync path does (over the canonical form);
    /// the server writes it verbatim into
    /// `kb_resource_manifests.managed_hash`.
    pub managed_hash: String,
    /// SHA-256 hash of the open_meta JSON. Stored verbatim.
    pub open_hash: String,
}

/// MCP input for delete_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteResourceInput {
    /// UUID of the resource to delete.
    pub id: Uuid,
}

// ── Response types ─────────────────────────────────────────────────

/// Status of a create_resource operation.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateStatus {
    Created,
    Existing,
}

/// Typed response for create_resource.
#[derive(Debug, serde::Serialize)]
pub struct CreateResourceResponse {
    pub resource: EnrichedResource,
    pub status: CreateStatus,
}

/// Typed response for delete_resource.
#[derive(Debug, serde::Serialize)]
pub struct DeleteResourceResponse {
    pub deleted: bool,
    pub id: Uuid,
}

/// Typed response for update_resource_meta.
#[derive(Debug, serde::Serialize)]
pub struct UpdateResourceMetaResponse {
    pub updated: bool,
    pub id: Uuid,
}

// ── Response enrichment ────────────────────────────────────────────

/// Enriched resource response with human-readable names.
#[derive(Debug, serde::Serialize)]
pub struct EnrichedResource {
    pub id: Uuid,
    pub title: String,
    pub slug: Option<String>,
    pub context_name: String,
    pub doc_type_name: String,
    pub owner: String,
    pub origin_uri: String,
    pub is_active: bool,
    pub created: chrono::DateTime<chrono::Utc>,
    pub updated: chrono::DateTime<chrono::Utc>,
}

async fn enrich_resource(
    pool: &sqlx::PgPool,
    profile_id: ProfileId,
    row: &temper_core::types::resource::ResourceRow,
) -> Result<EnrichedResource, rmcp::ErrorData> {
    let context = context_service::get_visible(pool, profile_id, row.kb_context_id)
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to resolve context: {e}"), None)
        })?;

    let doc_type_name = doc_type_service::get_name_by_id(pool, row.kb_doc_type_id.into())
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to resolve doc_type: {e}"), None)
        })?;

    Ok(EnrichedResource {
        id: row.id.into(),
        title: row.title.clone(),
        slug: row.slug.clone(),
        context_name: context.name,
        doc_type_name,
        owner: "@me".to_string(),
        origin_uri: row.origin_uri.clone(),
        is_active: row.is_active,
        created: row.created,
        updated: row.updated,
    })
}

async fn enrich_resources(
    pool: &sqlx::PgPool,
    profile_id: ProfileId,
    rows: &[temper_core::types::resource::ResourceRow],
) -> Result<Vec<EnrichedResource>, rmcp::ErrorData> {
    let mut enriched = Vec::with_capacity(rows.len());
    for row in rows {
        enriched.push(enrich_resource(pool, profile_id, row).await?);
    }
    Ok(enriched)
}

// ── Helpers ────────────────────────────────────────────────────────

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

// ── Tool handlers ──────────────────────────────────────────────────

pub async fn create_resource(
    svc: &TemperMcpService,
    input: CreateResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    // Validate owner format if provided (stub for R11)
    if let Some(ref owner) = input.owner {
        if !owner.starts_with('@') && !owner.starts_with('+') {
            return Err(rmcp::ErrorData::invalid_params(
                "owner must start with @ (profile) or + (team)".to_string(),
                None,
            ));
        }
    }

    // Build slug from title if not provided
    let slug = input.slug.unwrap_or_else(|| {
        input
            .title
            .to_lowercase()
            .replace(|c: char| !c.is_alphanumeric() && c != '-', "-")
            .trim_matches('-')
            .to_owned()
    });

    let origin_uri = input
        .origin_uri
        .unwrap_or_else(|| format!("mcp://agent/{}", Uuid::new_v4()));

    let content = input.content.unwrap_or_default();

    // Inject canonical temper-title / temper-slug into managed_meta JSONB so
    // the local canonical form matches what the server will hash. Symmetric
    // with the CLI send-side wiring in build_ingest_payload (Phase 5 Task 3).
    let mut managed_meta_value = input.managed_meta.unwrap_or_else(|| serde_json::json!({}));
    temper_core::operations::ensure_managed_identity_keys(
        &mut managed_meta_value,
        &input.title,
        Some(&slug),
    );

    // Dispatch through DbBackend so MCP shares the unified write path with
    // HTTP. The send-side ensure_managed_identity_keys above ran on the
    // JSONB form; deserialize back to the typed ManagedMeta the cmd carries
    // (extras bucket preserves unknown keys; serde renames re-emit canonical
    // temper-* keys on round-trip).
    let managed_meta: ManagedMeta = serde_json::from_value(managed_meta_value)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("invalid managed_meta: {e}"), None))?;

    let body = if content.is_empty() {
        None
    } else {
        Some(BodyUpdate::new(content))
    };

    let cmd = CreateResource {
        slug,
        doctype: input.doc_type_name,
        context: input.context_name,
        title: input.title,
        body,
        managed_meta,
        open_meta: input.open_meta,
        origin_uri: Some(origin_uri),
        chunks_packed: None,
        content_hash: None,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id, "mcp".to_string(), Surface::Mcp);
    let out = backend.create_resource(cmd).await.map_err(|e| match e {
        TemperError::NotFound(_) => rmcp::ErrorData::invalid_params(
            "Context or doc_type not found. Use create_context / list_doc_types to verify."
                .to_string(),
            None,
        ),
        TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        other => {
            rmcp::ErrorData::internal_error(format!("Failed to create resource: {other}"), None)
        }
    })?;
    let resource = out.value;

    let enriched = enrich_resource(pool, profile_id, &resource).await?;
    let response = CreateResourceResponse {
        resource: enriched,
        status: CreateStatus::Created,
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&response),
    )]))
}

pub async fn get_resource(
    svc: &TemperMcpService,
    input: GetResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    // Validate input: exactly one of id or slug
    let row = match (input.id, input.slug.as_deref()) {
        (Some(id), None) => resource_service::get_visible(pool, profile.id, id)
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None)
            })?,
        (None, Some(slug)) => {
            let context_name = input.context_name.as_deref().ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    "context_name is required when looking up by slug".to_string(),
                    None,
                )
            })?;
            let context = context_service::resolve_by_name(pool, profile_id, context_name)
                .await
                .map_err(|e| {
                    rmcp::ErrorData::invalid_params(
                        format!("Context '{context_name}' not found: {e}"),
                        None,
                    )
                })?;
            resource_service::get_by_slug(pool, profile.id, slug, context.id.into())
                .await
                .map_err(|e| {
                    rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None)
                })?
        }
        (Some(_), Some(_)) => {
            return Err(rmcp::ErrorData::invalid_params(
                "Provide either id or slug, not both".to_string(),
                None,
            ));
        }
        (None, None) => {
            return Err(rmcp::ErrorData::invalid_params(
                "Provide either id or slug".to_string(),
                None,
            ));
        }
    };

    let enriched = enrich_resource(pool, profile_id, &row).await?;

    if input.include_content.unwrap_or(false) {
        let content = resource_service::get_content(pool, profile.id, row.id.into())
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to get content: {e}"), None)
            })?;

        Ok(CallToolResult::success(vec![
            rmcp::model::Content::text(to_text(&enriched)),
            rmcp::model::Content::text(content.markdown),
        ]))
    } else {
        Ok(CallToolResult::success(vec![rmcp::model::Content::text(
            to_text(&enriched),
        )]))
    }
}

pub async fn list_resources(
    svc: &TemperMcpService,
    input: ListResourcesInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    // Resolve names to IDs
    let context_id = if let Some(ref name) = input.context_name {
        Some(
            context_service::resolve_by_name(pool, profile_id, name)
                .await
                .map_err(|e| {
                    rmcp::ErrorData::invalid_params(
                        format!("Context '{name}' not found: {e}"),
                        None,
                    )
                })?
                .id
                .into(),
        )
    } else {
        None
    };

    let doc_type_id = if let Some(ref name) = input.doc_type_name {
        Some(
            ingest_service::resolve_doc_type(pool, name)
                .await
                .map_err(|e| {
                    rmcp::ErrorData::invalid_params(format!("Unknown doc_type '{name}': {e}"), None)
                })?,
        )
    } else {
        None
    };

    let params = resource_service::ResourceListParams {
        kb_context_id: context_id,
        kb_doc_type_id: doc_type_id,
        limit: input.limit,
        offset: input.offset,
        ..Default::default()
    };

    let response = resource_service::list_visible(pool, profile.id, params)
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to list resources: {e}"), None)
        })?;

    let enriched = enrich_resources(pool, profile_id, &response.rows).await?;
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&enriched),
    )]))
}

pub async fn update_resource(
    svc: &TemperMcpService,
    input: UpdateResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);
    let resource_id = ResourceId::from(input.id);

    // Auth check via service layer
    resource_service::check_can_modify(pool, profile.id, input.id)
        .await
        .map_err(|e| match e {
            temper_api::error::ApiError::Forbidden => rmcp::ErrorData::invalid_params(
                "Resource not found or not modifiable".to_string(),
                None,
            ),
            other => rmcp::ErrorData::internal_error(
                format!("Failed to check permissions: {other}"),
                None,
            ),
        })?;

    // Update title/slug if provided
    if input.title.is_some() || input.slug.is_some() {
        // Mirror the identity fields into the typed managed_meta partial so
        // the server's merge path rewrites temper-title / temper-slug in the
        // JSONB. Setting ManagedMeta.title / .slug (Option<String>) is the
        // typed-direct equivalent of running ensure_managed_identity_keys —
        // serde renames produce the canonical `temper-title` / `temper-slug`
        // keys in the resulting JSONB. Symmetric with the CLI send-side
        // wiring (Phase 5 Task 3).
        let managed_meta_partial = ManagedMeta {
            title: input.title.clone(),
            slug: input.slug.clone(),
            ..Default::default()
        };
        let update_req = temper_core::types::resource::ResourceUpdateRequest {
            title: input.title.clone(),
            slug: input.slug.clone(),
            managed_meta: Some(managed_meta_partial),
            ..Default::default()
        };
        resource_service::update(pool, profile.id, input.id, "mcp", update_req)
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to update resource: {e}"), None)
            })?;
    }

    // Update content if provided — route through the ingest service update
    // which handles managed_meta validation, chunking, and embedding.
    if let Some(content) = input.content {
        // Fetch the existing resource to get context/doc_type names for the payload
        let existing = resource_service::get_visible(pool, profile.id, input.id)
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None)
            })?;

        let payload_title = input.title.clone().unwrap_or(existing.title);
        // Slug is nullable on kb_resources; preserve column-NULL semantics in
        // the JSONB by passing Option<&str> through to the helper.
        let payload_slug_opt = existing.slug.as_deref();

        // Inject canonical temper-title / temper-slug into managed_meta JSONB
        // so the local canonical form matches what the server will hash.
        // Symmetric with the CLI send-side wiring (Phase 5 Task 3).
        let mut managed_meta_value = input.managed_meta.unwrap_or_else(|| serde_json::json!({}));
        temper_core::operations::ensure_managed_identity_keys(
            &mut managed_meta_value,
            &payload_title,
            payload_slug_opt,
        );

        let payload = temper_core::types::IngestPayload {
            title: payload_title,
            origin_uri: existing.origin_uri,
            context_name: existing.context_name,
            doc_type_name: existing.doc_type_name,
            content_hash: None,
            // IngestPayload::slug is required (non-Option String); fall back to
            // empty string when the row has no slug. The canonical JSONB has
            // no temper-slug key per the helper above, and the kb_resources
            // column stays NULL via the existing row state.
            slug: payload_slug_opt.unwrap_or("").to_owned(),
            content,
            metadata: None,
            managed_meta: Some(managed_meta_value),
            open_meta: input.open_meta,
            chunks_packed: None,
        };

        ingest_service::update(pool, profile_id, resource_id, "mcp", payload)
            .await
            .map_err(|e| match e {
                temper_api::error::ApiError::BadRequest(msg) => {
                    rmcp::ErrorData::invalid_params(msg, None)
                }
                other => rmcp::ErrorData::internal_error(
                    format!("Failed to update resource: {other}"),
                    None,
                ),
            })?;
    }

    // Return enriched current state
    let row = resource_service::get_visible(pool, profile.id, input.id)
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None)
        })?;

    let enriched = enrich_resource(pool, profile_id, &row).await?;
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&enriched),
    )]))
}

pub async fn update_resource_meta(
    svc: &TemperMcpService,
    input: UpdateResourceMetaInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);
    let resource_id = ResourceId::from(input.id);

    // Delegate to the shared service used by the REST PUT handler.
    // `meta_service::update_meta` runs its own `can_modify_resource`
    // check, updates the manifest, cascades identity fields, writes
    // the event + audit, and reconciles edges. The MCP tool does not
    // duplicate any of that.
    let payload = MetaUpdatePayload {
        resource_id,
        managed_meta: input.managed_meta,
        open_meta: input.open_meta,
        managed_hash: input.managed_hash,
        open_hash: input.open_hash,
    };

    meta_service::update_meta(pool, profile_id, resource_id, "mcp", payload)
        .await
        .map_err(|e| match e {
            temper_api::error::ApiError::Forbidden => rmcp::ErrorData::invalid_params(
                "Resource not found or not modifiable".to_string(),
                None,
            ),
            temper_api::error::ApiError::NotFound => {
                rmcp::ErrorData::invalid_params("Resource not found".to_string(), None)
            }
            temper_api::error::ApiError::BadRequest(msg) => {
                rmcp::ErrorData::invalid_params(msg, None)
            }
            other => rmcp::ErrorData::internal_error(
                format!("Failed to update resource meta: {other}"),
                None,
            ),
        })?;

    let response = UpdateResourceMetaResponse {
        updated: true,
        id: input.id,
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&response),
    )]))
}

pub async fn delete_resource(
    svc: &TemperMcpService,
    input: DeleteResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    temper_api::services::resource_service::delete(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        ResourceId::from(input.id),
        "mcp",
    )
    .await
    .map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Failed to delete resource: {e}"), None)
    })?;

    let response = DeleteResourceResponse {
        deleted: true,
        id: input.id,
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&response),
    )]))
}
