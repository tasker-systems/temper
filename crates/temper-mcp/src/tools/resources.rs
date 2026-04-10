//! Resource tools — unified CRUD with name-based resolution and optional content.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use temper_api::services::{context_service, doc_type_service, ingest_service, resource_service};
use temper_core::types::ids::{ProfileId, ResourceId};

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
    /// Optional markdown content body. If provided, triggers async
    /// chunk/embed processing.
    pub content: Option<String>,
    /// Optional URL-friendly slug.
    pub slug: Option<String>,
    /// Optional origin URI. Defaults to `mcp://agent/{uuid}`.
    pub origin_uri: Option<String>,
    /// Optional owner (defaults to @me). Reserved for future team scoping.
    pub owner: Option<String>,
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
    /// async re-processing.
    pub content: Option<String>,
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

fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

fn extract_bearer_token(parts: &http::request::Parts) -> Option<String> {
    let header = parts.headers.get(http::header::AUTHORIZATION)?;
    let value = header.to_str().ok()?;
    value.strip_prefix("Bearer ").map(|s| s.to_string())
}

fn spawn_content_ingest_post(
    resource_id: ResourceId,
    content: String,
    replace: bool,
    bearer_token: Option<String>,
    context_id: String,
    body_hash: String,
) {
    tokio::spawn(async move {
        let base_url = match std::env::var("MCP_BASE_URL") {
            Ok(url) => url,
            Err(_) => {
                tracing::warn!("MCP_BASE_URL not set; skipping content-ingest POST");
                return;
            }
        };

        let url = format!("{base_url}/api/content-ingest");
        let payload = temper_core::types::ingest::ContentIngestRequest {
            resource_id: resource_id.to_string(),
            content,
            replace,
            context_id: Some(context_id),
            body_hash: Some(body_hash),
        };
        let client = reqwest::Client::new();
        let mut req = client.post(&url).json(&payload);

        if let Some(token) = bearer_token {
            req = req.bearer_auth(token);
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!(resource_id = %resource_id, "content-ingest POST accepted");
            }
            Ok(resp) => {
                tracing::warn!(resource_id = %resource_id, status = %resp.status(), "content-ingest POST returned non-success");
            }
            Err(e) => {
                tracing::warn!(resource_id = %resource_id, error = %e, "content-ingest POST failed");
            }
        }
    });
}

// ── Tool handlers ──────────────────────────────────────────────────

pub async fn create_resource(
    svc: &TemperMcpService,
    input: CreateResourceInput,
    parts: &http::request::Parts,
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

    // 1. Resolve context by name — error if not found
    let context = context_service::resolve_by_name(pool, profile_id, &input.context_name)
        .await
        .map_err(|e| match e {
            temper_api::error::ApiError::NotFound => rmcp::ErrorData::invalid_params(
                format!(
                    "Context '{}' not found. Use create_context to create it first.",
                    input.context_name
                ),
                None,
            ),
            other => {
                rmcp::ErrorData::internal_error(format!("Failed to resolve context: {other}"), None)
            }
        })?;

    // 2. Resolve doc type by name
    let doc_type_id = ingest_service::resolve_doc_type(pool, &input.doc_type_name)
        .await
        .map_err(|e| {
            rmcp::ErrorData::invalid_params(
                format!(
                    "Unknown doc_type '{}'. Use list_doc_types to see available types. Error: {e}",
                    input.doc_type_name
                ),
                None,
            )
        })?;

    // 3. Content handling — hash, dedup, ingest post
    let body_hash = input.content.as_ref().map(|c| content_hash(c));

    if let Some(ref hash) = body_hash {
        if let Some(existing) = ingest_service::find_by_body_hash(pool, profile_id, hash)
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to check body hash: {e}"), None)
            })?
        {
            let enriched = enrich_resource(pool, profile_id, &existing).await?;
            let response = CreateResourceResponse {
                resource: enriched,
                status: CreateStatus::Existing,
            };
            return Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                to_text(&response),
            )]));
        }
    }

    // 4. Default origin_uri
    let origin_uri = input
        .origin_uri
        .unwrap_or_else(|| format!("mcp://agent/{}", Uuid::new_v4()));

    // 5. Create resource + manifest + event
    let empty_json = serde_json::json!({});
    // SHA256 of empty string — used when no content is provided
    let hash_for_manifest = body_hash
        .as_deref()
        .unwrap_or("sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");

    let resource = ingest_service::create_resource_with_manifest(
        pool,
        &ingest_service::CreateResourceParams {
            profile_id,
            device_id: "mcp",
            context_id: context.id,
            doc_type_id,
            title: &input.title,
            slug: input.slug.as_deref(),
            origin_uri: &origin_uri,
            content_hash: hash_for_manifest,
            managed_meta: &empty_json,
            open_meta: &empty_json,
            // No chunks here — chunk persistence happens later via the
            // async content-ingest POST spawned below.
            chunks_packed: None,
        },
    )
    .await
    .map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Failed to create resource: {e}"), None)
    })?;

    // 6. Fire content-ingest POST if content provided
    if let (Some(content), Some(hash)) = (input.content, body_hash) {
        let bearer_token = extract_bearer_token(parts);
        spawn_content_ingest_post(
            resource.id,
            content,
            false,
            bearer_token,
            context.id.to_string(),
            hash,
        );
    }

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
        let markdown = resource_service::get_content(pool, profile.id, row.id.into())
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to get content: {e}"), None)
            })?;

        Ok(CallToolResult::success(vec![
            rmcp::model::Content::text(to_text(&enriched)),
            rmcp::model::Content::text(markdown),
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
    };

    let rows = resource_service::list_visible(pool, profile.id, params)
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to list resources: {e}"), None)
        })?;

    let enriched = enrich_resources(pool, profile_id, &rows).await?;
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&enriched),
    )]))
}

pub async fn update_resource(
    svc: &TemperMcpService,
    input: UpdateResourceInput,
    parts: &http::request::Parts,
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
        let update_req = temper_core::types::resource::ResourceUpdateRequest {
            title: input.title.clone(),
            slug: input.slug.clone(),
        };
        resource_service::update(pool, profile.id, input.id, update_req)
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to update resource: {e}"), None)
            })?;
    }

    // Update content if provided
    if let Some(content) = input.content {
        let body_hash = content_hash(&content);
        let empty_json = serde_json::json!({});

        let mut tx = pool.begin().await.map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to begin transaction: {e}"), None)
        })?;

        let updated_row = ingest_service::update_resource_manifest(
            &mut tx,
            profile_id,
            "mcp",
            resource_id,
            &body_hash,
            &empty_json,
            &empty_json,
        )
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to update manifest: {e}"), None)
        })?;

        tx.commit()
            .await
            .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to commit: {e}"), None))?;

        // Fire content-ingest POST using context_id from the manifest update response
        let bearer_token = extract_bearer_token(parts);
        spawn_content_ingest_post(
            resource_id,
            content,
            true,
            bearer_token,
            updated_row.kb_context_id.to_string(),
            body_hash,
        );
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
