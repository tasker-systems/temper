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
use temper_core::types::managed_meta::ManagedMeta;

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
///
/// `managed_meta` and `open_meta` are populated by single-resource read
/// paths (`get_resource`) via [`enrich_resource_with_meta`]. List paths
/// leave both `None` to avoid one DB round-trip per row; the
/// `skip_serializing_if` keeps the wire shape backward-compatible for
/// callers that don't request meta.
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<ManagedMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<serde_json::Value>,
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
        managed_meta: None,
        open_meta: None,
    })
}

/// Enrich a single resource row with its `managed_meta` and `open_meta`
/// blocks from `kb_resource_manifests`, via [`meta_service::get_meta`].
///
/// Used by the `get_resource` MCP tool to make frontmatter readable —
/// without this the tool returned only core fields plus an optional
/// body, and anything written via `update_resource_meta` was
/// write-blind. Visibility is enforced inside `meta_service::get_meta`
/// (delegates to `resource_service::get_visible`).
///
/// List paths intentionally do not call this — adding a per-row meta
/// fetch would N+1 the list query. List rows leave both fields `None`
/// and serialize-skip them.
pub async fn enrich_resource_with_meta(
    pool: &sqlx::PgPool,
    profile_id: ProfileId,
    row: &temper_core::types::resource::ResourceRow,
) -> Result<EnrichedResource, rmcp::ErrorData> {
    let mut enriched = enrich_resource(pool, profile_id, row).await?;
    let meta = meta_service::get_meta(pool, profile_id, row.id)
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to get meta: {e}"), None))?;
    enriched.managed_meta = meta.managed_meta;
    enriched.open_meta = meta.open_meta;
    Ok(enriched)
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

    if input.include_content.unwrap_or(false) {
        // get_content already returns managed_meta + open_meta alongside the
        // body, so reuse those values rather than firing a second meta query.
        let content = resource_service::get_content(pool, profile.id, row.id.into())
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to get content: {e}"), None)
            })?;

        let mut enriched = enrich_resource(pool, profile_id, &row).await?;
        enriched.managed_meta = content.managed_meta;
        enriched.open_meta = content.open_meta;

        Ok(CallToolResult::success(vec![
            rmcp::model::Content::text(to_text(&enriched)),
            rmcp::model::Content::text(content.markdown),
        ]))
    } else {
        let enriched = enrich_resource_with_meta(pool, profile_id, &row).await?;
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

    // Send-side canonical-key injection (Phase 5 symmetric defense). When the
    // caller is also touching title or slug, fetch existing.title for the
    // canonical-key fill so the wire payload's temper-title / temper-slug
    // match what the receive-side will write. Pure meta-only updates skip
    // the fetch — resource_service::update's receive-side ensure call fills
    // canonical keys from the stored title/slug for us.
    let mut managed_meta_value = input.managed_meta.unwrap_or_else(|| serde_json::json!({}));
    if input.title.is_some() || input.slug.is_some() || input.content.is_some() {
        let existing = resource_service::get_visible(pool, profile.id, input.id)
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None)
            })?;
        let title = input.title.clone().unwrap_or(existing.title);
        let slug_opt = input.slug.clone().or(existing.slug);
        temper_core::operations::ensure_managed_identity_keys(
            &mut managed_meta_value,
            &title,
            slug_opt.as_deref(),
        );
    }

    // Build the typed cmd. Mirror title/slug onto ManagedMeta so the
    // translator's manifest-merge path picks them up from cmd.managed_meta
    // alongside any caller-supplied managed_meta keys.
    let mut managed_meta: ManagedMeta = serde_json::from_value(managed_meta_value)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("invalid managed_meta: {e}"), None))?;
    if input.title.is_some() {
        managed_meta.title = input.title.clone();
    }
    if input.slug.is_some() {
        managed_meta.slug = input.slug.clone();
    }

    let cmd = temper_core::operations::UpdateResource {
        resource: temper_core::operations::ResourceRef::Uuid { id: resource_id },
        body: input.content.map(BodyUpdate::new),
        managed_meta: Some(managed_meta),
        open_meta: input.open_meta,
        move_to: None,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id, "mcp".to_string(), Surface::Mcp);
    backend.update_resource(cmd).await.map_err(|e| match e {
        TemperError::Forbidden => rmcp::ErrorData::invalid_params(
            "Resource not found or not modifiable".to_string(),
            None,
        ),
        TemperError::NotFound(msg) => {
            rmcp::ErrorData::invalid_params(format!("Resource not found: {msg}"), None)
        }
        TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        other => {
            rmcp::ErrorData::internal_error(format!("Failed to update resource: {other}"), None)
        }
    })?;

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

    // Dispatch through the unified DbBackend write path. The translator's
    // meta-only branch runs resource_service::update with body=None, which
    // merges managed_meta / open_meta into the manifest, cascades identity
    // fields (doc_type / context), recomputes managed_hash / open_hash
    // server-side (Phase 5: caller-supplied hashes are no longer trusted),
    // emits the update_meta audit, and reconciles edges.
    let cmd = temper_core::operations::UpdateResource {
        resource: temper_core::operations::ResourceRef::Uuid { id: resource_id },
        body: None,
        managed_meta: Some(input.managed_meta),
        open_meta: Some(input.open_meta),
        move_to: None,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id, "mcp".to_string(), Surface::Mcp);
    backend.update_resource(cmd).await.map_err(|e| match e {
        TemperError::Forbidden => rmcp::ErrorData::invalid_params(
            "Resource not found or not modifiable".to_string(),
            None,
        ),
        TemperError::NotFound(msg) => {
            rmcp::ErrorData::invalid_params(format!("Resource not found: {msg}"), None)
        }
        TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
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
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    let cmd = temper_core::operations::DeleteResource {
        resource: temper_core::operations::ResourceRef::Uuid {
            id: ResourceId::from(input.id),
        },
        // CLI-side concern; DbBackend ignores per spec (the local-file
        // confirmation prompt lives in CliLocalVault, not here).
        force: false,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id, "mcp".to_string(), Surface::Mcp);
    backend.delete_resource(cmd).await.map_err(|e| match e {
        TemperError::Forbidden => rmcp::ErrorData::invalid_params(
            "Resource not found or not modifiable".to_string(),
            None,
        ),
        TemperError::NotFound(msg) => {
            rmcp::ErrorData::invalid_params(format!("Resource not found: {msg}"), None)
        }
        other => {
            rmcp::ErrorData::internal_error(format!("Failed to delete resource: {other}"), None)
        }
    })?;

    let response = DeleteResourceResponse {
        deleted: true,
        id: input.id,
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&response),
    )]))
}
