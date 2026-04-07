//! Ingest tools — create and update resource content via MCP.
//!
//! These tools create resources synchronously (DB row + manifest + event)
//! then fire-and-forget a POST to `/api/content-ingest` for async
//! chunk/embed/store processing. The content-ingest endpoint may not exist
//! yet (Task 5), so POST failures are logged but do not fail the tool.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use temper_core::types::ids::{ProfileId, ResourceId};

use crate::service::TemperMcpService;

/// MCP input for `ingest_content` — creates a new resource with markdown content.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct IngestContentInput {
    /// Title of the resource.
    pub title: String,
    /// Markdown content body.
    pub content: String,
    /// Context (workspace) name. Auto-created if it does not exist.
    pub context_name: String,
    /// Document type name (must already exist). Use `list_doc_types` to see available types.
    pub doc_type_name: String,
    /// Optional URL-friendly slug for the resource.
    pub slug: Option<String>,
    /// Optional origin URI. Defaults to `mcp://agent/<uuid>` if not provided.
    pub origin_uri: Option<String>,
}

/// MCP input for `update_resource_content` — replaces content of an existing resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateResourceContentInput {
    /// UUID of the resource to update.
    pub resource_id: Uuid,
    /// New markdown content body.
    pub content: String,
}

/// Compute `sha256:<hex>` hash of raw content bytes.
fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

/// Extract the bearer token from HTTP request parts.
fn extract_bearer_token(parts: &http::request::Parts) -> Option<String> {
    let header = parts.headers.get(http::header::AUTHORIZATION)?;
    let value = header.to_str().ok()?;
    value.strip_prefix("Bearer ").map(|s| s.to_string())
}

/// Fire-and-forget POST to the content-ingest endpoint.
///
/// Logs a warning on failure but never propagates errors — the resource
/// has already been created in the database.
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
        let client = reqwest::Client::new();
        let mut req = client.post(&url).json(&serde_json::json!({
            "resource_id": resource_id,
            "content": content,
            "replace": replace,
            "context_id": context_id,
            "body_hash": body_hash,
        }));

        if let Some(token) = bearer_token {
            req = req.bearer_auth(token);
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::debug!(
                    resource_id = %resource_id,
                    "content-ingest POST accepted"
                );
            }
            Ok(resp) => {
                tracing::warn!(
                    resource_id = %resource_id,
                    status = %resp.status(),
                    "content-ingest POST returned non-success status"
                );
            }
            Err(e) => {
                tracing::warn!(
                    resource_id = %resource_id,
                    error = %e,
                    "content-ingest POST failed"
                );
            }
        }
    });
}

/// Create a new resource with markdown content.
///
/// Resolves context/doc_type by name, computes content hash, deduplicates
/// by body hash, creates the resource + manifest + event, then kicks off
/// async content processing.
pub async fn ingest_content(
    svc: &TemperMcpService,
    input: IngestContentInput,
    parts: &http::request::Parts,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    // 1. Resolve context by name — auto-create if not found
    let context = match temper_api::services::context_service::resolve_by_name(
        pool,
        profile_id,
        &input.context_name,
    )
    .await
    {
        Ok(ctx) => ctx,
        Err(temper_api::error::ApiError::NotFound) => {
            // Context doesn't exist — create it
            temper_api::services::context_service::create(pool, profile_id, &input.context_name)
                .await
                .map_err(|e| {
                    rmcp::ErrorData::internal_error(
                        format!("Failed to create context '{}': {e}", input.context_name),
                        None,
                    )
                })?
        }
        Err(e) => {
            return Err(rmcp::ErrorData::internal_error(
                format!("Failed to resolve context '{}': {e}", input.context_name),
                None,
            ));
        }
    };

    // 2. Resolve doc type by name
    let doc_type_id =
        temper_api::services::ingest_service::resolve_doc_type(pool, &input.doc_type_name)
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(
                    format!("Failed to resolve doc_type '{}': {e}", input.doc_type_name),
                    None,
                )
            })?;

    // 3. Compute body hash and check for duplicates
    let body_hash = content_hash(&input.content);

    if let Some(existing) =
        temper_api::services::ingest_service::find_by_body_hash(pool, profile_id, &body_hash)
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to check body hash: {e}"), None)
            })?
    {
        return Ok(CallToolResult::success(vec![rmcp::model::Content::text(
            to_text(&serde_json::json!({
                "resource_id": existing.id,
                "title": existing.title,
                "context_name": input.context_name,
                "status": "existing"
            })),
        )]));
    }

    // 4. Default origin_uri
    let origin_uri = input
        .origin_uri
        .unwrap_or_else(|| format!("mcp://agent/{}", Uuid::new_v4()));

    // 5. Create resource + manifest + event
    let resource = temper_api::services::ingest_service::create_resource_with_manifest(
        pool,
        profile_id,
        "mcp",
        context.id,
        doc_type_id,
        &input.title,
        input.slug.as_deref(),
        &origin_uri,
        &body_hash,
        &serde_json::json!({}),
        &serde_json::json!({}),
    )
    .await
    .map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Failed to create resource: {e}"), None)
    })?;

    // 6. Fire-and-forget POST to content-ingest
    let bearer_token = extract_bearer_token(parts);
    spawn_content_ingest_post(
        resource.id,
        input.content,
        false,
        bearer_token,
        context.id.to_string(),
        body_hash,
    );

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&serde_json::json!({
            "resource_id": resource.id,
            "title": resource.title,
            "context_name": input.context_name,
            "status": "created"
        })),
    )]))
}

/// Update an existing resource's content.
///
/// Verifies ownership, updates the manifest hash, fires an event, and
/// kicks off async content re-processing with `replace: true`.
pub async fn update_resource_content(
    svc: &TemperMcpService,
    input: UpdateResourceContentInput,
    parts: &http::request::Parts,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);
    let resource_id = ResourceId::from(input.resource_id);

    // 1. Verify the caller can modify this resource
    let can_modify = sqlx::query_scalar!(
        "SELECT true FROM can_modify_resource($1, $2)",
        *profile_id,
        *resource_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Failed to check permissions: {e}"), None)
    })?;

    if can_modify.is_none() {
        return Err(rmcp::ErrorData::internal_error(
            "Resource not found or not modifiable".to_string(),
            None,
        ));
    }

    // 2. Compute new body hash
    let body_hash = content_hash(&input.content);

    // 3. Get the resource row (need kb_context_id for event)
    let resource =
        temper_api::services::resource_service::get_visible(pool, *profile_id, *resource_id)
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None)
            })?;

    // 4. Transaction: upsert manifest, update timestamp, insert event+audit
    let empty_json = serde_json::json!({});
    let managed_hash = temper_api::services::ingest_service::hash_json_value(&empty_json);
    let open_hash = temper_api::services::ingest_service::hash_json_value(&empty_json);

    let mut tx = pool.begin().await.map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Failed to begin transaction: {e}"), None)
    })?;

    // Update resource timestamp
    sqlx::query!(
        "UPDATE kb_resources SET updated = now() WHERE id = $1",
        *resource_id
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Failed to update resource: {e}"), None)
    })?;

    // Upsert manifest row
    sqlx::query!(
        r#"
        INSERT INTO kb_resource_manifests (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
        VALUES ($1, $2, $3, $4, $5, $6, now())
        ON CONFLICT (resource_id)
        DO UPDATE SET body_hash = $2, managed_meta = $3, open_meta = $4,
                      managed_hash = $5, open_hash = $6, updated = now()
        "#,
        *resource_id,
        body_hash,
        empty_json,
        empty_json,
        managed_hash,
        open_hash,
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Failed to upsert manifest: {e}"), None)
    })?;

    // Insert event + audit
    temper_api::services::ingest_service::insert_event_and_audit(
        &mut tx,
        profile_id,
        "mcp",
        resource.kb_context_id,
        resource_id,
        "body_updated",
        "update_body",
        &body_hash,
        &managed_hash,
        &open_hash,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to insert event: {e}"), None))?;

    tx.commit().await.map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Failed to commit transaction: {e}"), None)
    })?;

    // 5. Fire-and-forget POST to content-ingest with replace: true
    let bearer_token = extract_bearer_token(parts);
    spawn_content_ingest_post(
        resource_id,
        input.content,
        true,
        bearer_token,
        resource.kb_context_id.to_string(),
        body_hash,
    );

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&serde_json::json!({
            "resource_id": resource_id,
            "status": "processing"
        })),
    )]))
}
