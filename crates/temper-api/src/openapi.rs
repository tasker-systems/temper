use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

use crate::error::{ErrorBody, ErrorDetail};
use temper_core::types::api::{
    EventRow, HealthResponse, ProfileUpdateRequest, SearchParams, SearchResultRow,
    UnifiedSearchResultRow,
};
use temper_core::types::context::ContextRowWithCounts;
use temper_core::types::resource::{
    ContentResponse, DeleteResponse, ResourceCreateRequest, ResourceFacets, ResourceListResponse,
    ResourceRow, ResourceUpdateRequest,
};

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::handlers::health::health_check,
        crate::handlers::resources::list,
        crate::handlers::resources::get,
        crate::handlers::resources::get_content,
        crate::handlers::resources::create,
        crate::handlers::resources::update,
        crate::handlers::resources::delete,
        crate::handlers::resources::by_uri,
        crate::handlers::profiles::get,
        crate::handlers::profiles::update,
        crate::handlers::profiles::list_auth_links,
        crate::handlers::events::list,
        crate::handlers::search::search,
        crate::handlers::meta::get_meta,
        crate::handlers::meta::update_meta,
        crate::handlers::edges::list,
    ),
    components(schemas(
        HealthResponse,
        ResourceRow,
        ResourceListResponse,
        ResourceFacets,
        ResourceCreateRequest,
        ResourceUpdateRequest,
        ContentResponse,
        DeleteResponse,
        ContextRowWithCounts,
        ProfileUpdateRequest,
        EventRow,
        SearchParams,
        SearchResultRow,
        UnifiedSearchResultRow,
        temper_core::types::managed_meta::MetaUpdatePayload,
        temper_core::types::managed_meta::ResourceMetaResponse,
        temper_core::types::managed_meta::ManagedMeta,
        ErrorBody,
        ErrorDetail,
        temper_core::types::graph::GraphEdgeRow,
        temper_core::types::Profile,
        temper_core::types::ProfileAuthLink,
    )),
    modifiers(&SecurityAddon),
    tags(
        (name = "Health", description = "Service health checks"),
        (name = "Resources", description = "Knowledge base resource management"),
        (name = "Profile", description = "Authenticated user profile"),
        (name = "Events", description = "Activity event log"),
        (name = "Search", description = "Semantic and keyword search"),
        (name = "Meta", description = "Resource frontmatter metadata management"),
    ),
    info(
        title = "Temper Cloud API",
        version = "0.1.0",
        description = "Knowledge base management API for temper cloud",
    )
)]
pub struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .build(),
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_spec_is_valid() {
        let spec = ApiDoc::openapi();
        let json = spec.to_pretty_json().expect("spec serializes to JSON");

        // Verify basic structure
        assert!(json.contains("\"title\": \"Temper Cloud API\""));
        assert!(json.contains("\"version\": \"0.1.0\""));

        // Verify all paths present
        assert!(json.contains("/api/health"));
        assert!(json.contains("/api/resources"));
        assert!(json.contains("/api/resources/{id}"));
        assert!(json.contains("/api/resources/{id}/content"));
        assert!(json.contains("/api/resources/by-uri"));
        assert!(json.contains("/api/profile"));
        assert!(json.contains("/api/profile/auth-links"));
        assert!(json.contains("/api/events"));
        assert!(json.contains("/api/search"));
        assert!(json.contains("/api/resources/{id}/meta"));
        assert!(json.contains("/api/resources/{id}/edges"));

        // Verify security scheme
        assert!(json.contains("bearer_auth"));
        assert!(json.contains("\"scheme\": \"bearer\""));
        assert!(json.contains("\"bearerFormat\": \"JWT\""));

        // Verify tags
        assert!(json.contains("\"name\": \"Resources\""));
        assert!(json.contains("\"name\": \"Profile\""));
        assert!(json.contains("\"name\": \"Events\""));
        assert!(json.contains("\"name\": \"Search\""));
        assert!(json.contains("\"name\": \"Health\""));
    }
}
