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

use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::Profile;
use temper_services::state::AppState;

/// Page size for the resource-browsing list calls. MCP resource listing is a
/// flat browse surface (no client-driven pagination), so we cap each fetch at a
/// reasonable ceiling rather than streaming the whole vault.
const MCP_RESOURCE_BROWSE_LIMIT: i64 = 200;

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
        limit: Some(MCP_RESOURCE_BROWSE_LIMIT),
        ..Default::default()
    };

    let response = temper_services::backend::substrate_read::list_select(
        &state.pool,
        ProfileId::from(profile.id),
        params,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to list resources: {e}"), None))?;

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
        RawResourceTemplate::new("temper://contexts/{ref}/resources", "Resources in context")
            .with_description(
                "List all resources belonging to a context. \
                 The ref must be a UUID or decorated form `@owner/slug` (e.g. `@me/my-context`).",
            )
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
/// - `temper://contexts/{ref}/resources` — JSON list of resources in context (ref = UUID or `@owner/slug`)
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
        let content = temper_services::backend::substrate_read::get_content_select(
            &state.pool,
            ProfileId::from(profile.id),
            ResourceId::from(id),
        )
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to read resource content: {e}"), None)
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
        let row = temper_services::backend::substrate_read::show_select(
            &state.pool,
            ProfileId::from(profile.id),
            ResourceId::from(id),
        )
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to read resource: {e}"), None)
        })?;

        let content = temper_services::backend::substrate_read::get_content_select(
            &state.pool,
            ProfileId::from(profile.id),
            ResourceId::from(id),
        )
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to read resource content: {e}"), None)
        })?;

        // Return metadata as JSON + content as markdown.
        let meta_json = serde_json::to_string_pretty(&row).map_err(|e| {
            rmcp::ErrorData::internal_error(
                format!("Failed to serialize resource metadata: {e}"),
                None,
            )
        })?;
        return Ok(ReadResourceResult::new(vec![
            ResourceContents::text(meta_json, uri).with_mime_type("application/json"),
            ResourceContents::text(content.markdown, uri).with_mime_type("text/markdown"),
        ]));
    }

    // temper://contexts/{ref}/resources  (ref = UUID or @owner/slug)
    if let Some(context_ref_str) = uri
        .strip_prefix("temper://contexts/")
        .and_then(|rest| rest.strip_suffix("/resources"))
    {
        let r = temper_core::context_ref::parse_context_ref(context_ref_str).map_err(|e| {
            rmcp::ErrorData::invalid_params(
                format!("Invalid context ref {context_ref_str:?}: {e}"),
                None,
            )
        })?;
        let context_id = temper_services::services::context_service::resolve_context_ref(
            &state.pool,
            temper_core::types::ids::ProfileId::from(profile.id),
            &r,
        )
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(
                format!("Failed to resolve context ref {context_ref_str:?}: {e}"),
                None,
            )
        })?;

        let params = temper_workflow::types::resource::ResourceListParams {
            context_ref: Some(uuid::Uuid::from(context_id).to_string()),
            limit: Some(MCP_RESOURCE_BROWSE_LIMIT),
            ..Default::default()
        };

        let response = temper_services::backend::substrate_read::list_select(
            &state.pool,
            ProfileId::from(profile.id),
            params,
        )
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(
                format!("Failed to list resources in context: {e}"),
                None,
            )
        })?;

        let json = serde_json::to_string_pretty(&response.rows).map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to serialize resource list: {e}"), None)
        })?;
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
