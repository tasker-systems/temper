//! Resource tools — list, get, and create resources in the knowledge base.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use crate::service::TemperMcpService;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListResourcesInput {
    /// Filter by context ID.
    #[schemars(description = "Optional context UUID to filter resources by")]
    pub context_id: Option<Uuid>,
    /// Maximum results to return (default 50, max 200).
    #[schemars(description = "Maximum number of resources to return (default 50, max 200)")]
    pub limit: Option<i64>,
}

pub async fn list_resources(
    svc: &TemperMcpService,
    input: ListResourcesInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let limit = input.limit.unwrap_or(50).min(200);

    let params = temper_core::types::resource::ResourceListParams {
        kb_context_id: input.context_id,
        limit: Some(limit),
        offset: Some(0),
    };

    let rows =
        temper_api::services::resource_service::list_visible(pool, profile.id, params).await;

    match rows {
        Ok(rows) => {
            let items: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.id,
                        "title": r.title,
                        "context_id": r.kb_context_id,
                        "origin_uri": r.origin_uri,
                        "mimetype": r.mimetype,
                        "created": r.created,
                        "updated": r.updated,
                    })
                })
                .collect();

            let text = serde_json::to_string_pretty(&items)
                .unwrap_or_else(|_| "[]".to_string());
            Ok(CallToolResult::success(vec![
                rmcp::model::Content::text(text),
            ]))
        }
        Err(e) => Err(rmcp::ErrorData::internal_error(
            format!("Failed to list resources: {e}"),
            None,
        )),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetResourceInput {
    /// The resource UUID.
    #[schemars(description = "UUID of the resource to retrieve")]
    pub id: Uuid,
    /// Whether to include the full markdown content.
    #[schemars(description = "If true, includes the full markdown content of the resource")]
    pub include_content: Option<bool>,
}

pub async fn get_resource(
    svc: &TemperMcpService,
    input: GetResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;

    let row =
        temper_api::services::resource_service::get_visible(pool, profile.id, input.id).await;

    match row {
        Ok(r) => {
            let mut result = serde_json::json!({
                "id": r.id,
                "title": r.title,
                "context_id": r.kb_context_id,
                "origin_uri": r.origin_uri,
                "slug": r.slug,
                "mimetype": r.mimetype,
                "created": r.created,
                "updated": r.updated,
            });

            if input.include_content.unwrap_or(false) {
                match temper_api::services::resource_service::get_content(
                    pool, profile.id, input.id,
                )
                .await
                {
                    Ok(markdown) => {
                        result["content"] = serde_json::Value::String(markdown);
                    }
                    Err(e) => {
                        result["content_error"] =
                            serde_json::Value::String(format!("Failed to load content: {e}"));
                    }
                }
            }

            let text = serde_json::to_string_pretty(&result)
                .unwrap_or_else(|_| "{}".to_string());
            Ok(CallToolResult::success(vec![
                rmcp::model::Content::text(text),
            ]))
        }
        Err(e) => Err(rmcp::ErrorData::internal_error(
            format!("Failed to get resource: {e}"),
            None,
        )),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateResourceInput {
    /// Title of the new resource.
    #[schemars(description = "Title for the new resource")]
    pub title: String,
    /// Context ID to create the resource in.
    #[schemars(description = "UUID of the context to create the resource in")]
    pub context_id: Uuid,
    /// Document type ID.
    #[schemars(description = "UUID of the document type")]
    pub doc_type_id: Uuid,
    /// Origin URI for the resource.
    #[schemars(description = "Origin URI identifying the source of this resource")]
    pub origin_uri: String,
    /// Optional slug.
    #[schemars(description = "Optional URL-friendly slug")]
    pub slug: Option<String>,
    /// Optional MIME type.
    #[schemars(description = "Optional MIME type (e.g. text/markdown)")]
    pub mimetype: Option<String>,
}

pub async fn create_resource(
    svc: &TemperMcpService,
    input: CreateResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;

    let req = temper_core::types::resource::ResourceCreateRequest {
        kb_context_id: input.context_id,
        kb_doc_type_id: input.doc_type_id,
        origin_uri: input.origin_uri,
        title: input.title,
        slug: input.slug,
        mimetype: input.mimetype,
    };

    match temper_api::services::resource_service::create(pool, profile.id, req).await {
        Ok(r) => {
            let result = serde_json::json!({
                "id": r.id,
                "title": r.title,
                "context_id": r.kb_context_id,
                "created": r.created,
            });
            let text = serde_json::to_string_pretty(&result)
                .unwrap_or_else(|_| "{}".to_string());
            Ok(CallToolResult::success(vec![
                rmcp::model::Content::text(text),
            ]))
        }
        Err(e) => Err(rmcp::ErrorData::internal_error(
            format!("Failed to create resource: {e}"),
            None,
        )),
    }
}
