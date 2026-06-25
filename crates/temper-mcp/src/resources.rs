//! MCP resources protocol — expose vault content as browsable resources.
//!
//! Implements `list_resources`, `read_resource`, and `list_resource_templates`
//! so MCP clients can browse and inject vault content into context without
//! explicit tool calls.

use rmcp::model::{
    AnnotateAble, ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams,
    RawResource, RawResourceTemplate, ReadResourceRequestParams, ReadResourceResult,
    ResourceContents,
};
use uuid::Uuid;

use temper_api::state::AppState;
use temper_core::types::Profile;

/// List all resources visible to the authenticated user as MCP resources.
///
/// Returns a flat list: one `Resource` per active knowledge base resource.
/// Each resource URI follows the pattern `temper://resources/{id}`.
pub async fn list_resources(
    state: &AppState,
    profile: &Profile,
    _request: Option<PaginatedRequestParams>,
) -> Result<ListResourcesResult, rmcp::ErrorData> {
    // Fetch all visible resources (no filters, reasonable limit for browsing).
    let params = temper_workflow::types::resource::ResourceListParams {
        limit: Some(200),
        ..Default::default()
    };

    let response =
        temper_api::backend::substrate_read::list_select(&state.pool, profile.id, params)
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to list resources: {e}"), None)
            })?;

    let resources = response
        .rows
        .into_iter()
        .map(|r| {
            RawResource::new(format!("temper://resources/{}", r.id), &r.title)
                .with_description(format!("Origin: {}", r.origin_uri))
                .with_mime_type("text/markdown")
                .no_annotation()
        })
        .collect();

    Ok(ListResourcesResult {
        resources,
        next_cursor: None,
        ..Default::default()
    })
}

/// Advertise URI templates that clients can use to construct resource URIs.
pub async fn list_resource_templates(
    _request: Option<PaginatedRequestParams>,
) -> Result<ListResourceTemplatesResult, rmcp::ErrorData> {
    let templates = vec![
        RawResourceTemplate::new("temper://resources/{id}", "Resource by ID")
            .with_description(
                "Retrieve a knowledge base resource by UUID. \
             Returns metadata and full markdown content.",
            )
            .with_mime_type("text/markdown")
            .no_annotation(),
        RawResourceTemplate::new("temper://resources/{id}/content", "Resource content")
            .with_description("Retrieve only the raw markdown content of a resource.")
            .with_mime_type("text/markdown")
            .no_annotation(),
        RawResourceTemplate::new("temper://contexts/{name}/resources", "Resources in context")
            .with_description("List all resources belonging to a named context (workspace).")
            .with_mime_type("application/json")
            .no_annotation(),
    ];

    Ok(ListResourceTemplatesResult {
        resource_templates: templates,
        next_cursor: None,
        ..Default::default()
    })
}

/// Read a single resource by URI.
///
/// Supported URI patterns:
/// - `temper://resources/{id}` — metadata + full markdown content
/// - `temper://resources/{id}/content` — raw markdown only
/// - `temper://contexts/{name}/resources` — JSON list of resources in context
pub async fn read_resource(
    state: &AppState,
    profile: &Profile,
    request: ReadResourceRequestParams,
) -> Result<ReadResourceResult, rmcp::ErrorData> {
    let uri = &request.uri;

    // temper://resources/{id}/content
    if let Some(id) = uri
        .strip_prefix("temper://resources/")
        .and_then(|rest| rest.strip_suffix("/content"))
        .and_then(|id| Uuid::try_parse(id).ok())
    {
        let content =
            temper_api::backend::substrate_read::get_content_select(&state.pool, profile.id, id)
                .await
                .map_err(|e| {
                    rmcp::ErrorData::internal_error(
                        format!("Failed to read resource content: {e}"),
                        None,
                    )
                })?;

        return Ok(ReadResourceResult::new(vec![ResourceContents::text(
            content.markdown,
            uri,
        )
        .with_mime_type("text/markdown")]));
    }

    // temper://resources/{id}
    if let Some(id) = uri
        .strip_prefix("temper://resources/")
        .and_then(|id| Uuid::try_parse(id).ok())
    {
        let row = temper_api::backend::substrate_read::show_select(&state.pool, profile.id, id)
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to read resource: {e}"), None)
            })?;

        let content =
            temper_api::backend::substrate_read::get_content_select(&state.pool, profile.id, id)
                .await
                .map_err(|e| {
                    rmcp::ErrorData::internal_error(
                        format!("Failed to read resource content: {e}"),
                        None,
                    )
                })?;

        // Return metadata as JSON + content as markdown.
        let meta_json = serde_json::to_string_pretty(&row).unwrap_or_default();
        return Ok(ReadResourceResult::new(vec![
            ResourceContents::text(meta_json, uri).with_mime_type("application/json"),
            ResourceContents::text(content.markdown, uri).with_mime_type("text/markdown"),
        ]));
    }

    // temper://contexts/{name}/resources
    if let Some(name) = uri
        .strip_prefix("temper://contexts/")
        .and_then(|rest| rest.strip_suffix("/resources"))
    {
        let context = temper_api::services::context_service::resolve_by_name(
            &state.pool,
            temper_core::types::ProfileId::from(profile.id),
            name,
        )
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(
                format!("Failed to resolve context '{name}': {e}"),
                None,
            )
        })?;

        let params = temper_workflow::types::resource::ResourceListParams {
            kb_context_id: Some(uuid::Uuid::from(context.id)),
            limit: Some(200),
            ..Default::default()
        };

        let response =
            temper_api::backend::substrate_read::list_select(&state.pool, profile.id, params)
                .await
                .map_err(|e| {
                    rmcp::ErrorData::internal_error(
                        format!("Failed to list resources in context: {e}"),
                        None,
                    )
                })?;

        let json = serde_json::to_string_pretty(&response.rows).unwrap_or_default();
        return Ok(ReadResourceResult::new(vec![ResourceContents::text(
            json, uri,
        )
        .with_mime_type("application/json")]));
    }

    Err(rmcp::ErrorData::invalid_params(
        format!("Unknown resource URI: {uri}"),
        None,
    ))
}
