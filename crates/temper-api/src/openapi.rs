use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

use crate::error::{ErrorBody, ErrorDetail};
use crate::handlers::resources::ListResourcesResponse;
use temper_core::types::api::{
    EventCursorResponse, HealthResponse, ProfileUpdateRequest, SearchParams, SearchResultRow,
    UnifiedSearchResultRow,
};
use temper_core::types::context::ContextRowWithCounts;
use temper_workflow::types::managed_meta::ResourceMetaListResponse;
use temper_workflow::types::resource::{
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
        crate::handlers::profiles::get,
        crate::handlers::profiles::update,
        crate::handlers::profiles::list_auth_links,
        crate::handlers::events::cursor,
        crate::handlers::search::search,
        crate::handlers::meta::get_meta,
        crate::handlers::meta::update_meta,
        crate::handlers::edges::list,
        crate::handlers::edges::assert,
        crate::handlers::edges::retype,
        crate::handlers::edges::reweight,
        crate::handlers::edges::fold,
        crate::handlers::graph::get_subgraph,
        crate::handlers::cognitive_maps::reconcile,
        crate::handlers::cognitive_maps::shape,
        crate::handlers::invocations::open,
        crate::handlers::invocations::close,
        crate::handlers::invocations::show,
        crate::handlers::invocations::list,
    ),
    components(schemas(
        HealthResponse,
        ResourceRow,
        ResourceListResponse,
        ResourceMetaListResponse,
        ListResourcesResponse,
        ResourceFacets,
        ResourceCreateRequest,
        ResourceUpdateRequest,
        ContentResponse,
        DeleteResponse,
        ContextRowWithCounts,
        ProfileUpdateRequest,
        EventCursorResponse,
        SearchParams,
        SearchResultRow,
        UnifiedSearchResultRow,
        temper_workflow::types::managed_meta::MetaUpdatePayload,
        temper_workflow::types::managed_meta::ResourceMetaResponse,
        temper_workflow::types::managed_meta::ManagedMeta,
        ErrorBody,
        ErrorDetail,
        temper_workflow::types::graph::GraphEdgeRow,
        temper_workflow::types::graph::GraphNode,
        temper_workflow::types::graph::GraphEdge,
        temper_workflow::types::graph::SubgraphResponse,
        temper_core::types::Profile,
        temper_core::types::ProfileAuthLink,
        temper_core::types::relationship_requests::AssertRelationshipRequest,
        temper_core::types::relationship_requests::RetypeRelationshipRequest,
        temper_core::types::relationship_requests::ReweightRelationshipRequest,
        temper_core::types::relationship_requests::FoldRelationshipRequest,
        temper_core::types::relationship_requests::RelationshipAck,
        temper_core::types::reconcile::ReconcileCogmapRequest,
        temper_core::types::reconcile::ReconcileEntry,
        temper_core::types::reconcile::ReconcileEdge,
        temper_core::types::reconcile::ReconcileTombstone,
        temper_core::types::reconcile::ReconcileEdgeTombstone,
        temper_core::types::reconcile::ReconcileOutcome,
        temper_core::types::cognitive_maps::CogmapRegionRow,
        temper_core::types::invocation_requests::OpenInvocationRequest,
        temper_core::types::invocation_requests::CloseInvocationRequest,
        temper_core::types::invocation_requests::InvocationAck,
        temper_core::types::invocation_requests::InvocationCloseAck,
        temper_core::types::invocation::InvocationView,
        temper_core::types::invocation::InvocationSummary,
        temper_core::types::invocation::InvocationActRow,
        temper_core::types::invocation::Disposition,
    )),
    modifiers(&SecurityAddon),
    tags(
        (name = "Health", description = "Service health checks"),
        (name = "Resources", description = "Knowledge base resource management"),
        (name = "Profile", description = "Authenticated user profile"),
        (name = "Events", description = "Activity event log"),
        (name = "Search", description = "Semantic and keyword search"),
        (name = "Meta", description = "Resource frontmatter metadata management"),
        (name = "Graph", description = "Knowledge graph traversal"),
        (name = "Relationships", description = "Knowledge-graph relationship writes (assert/retype/reweight/fold)"),
        (name = "Cognitive Maps", description = "Cognitive-map content reconcile (admin-gated)"),
        (name = "Invocations", description = "Agent-invocation envelope (accountability)"),
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
        assert!(json.contains("/api/profile"));
        assert!(json.contains("/api/profile/auth-links"));
        assert!(json.contains("/api/events"));
        assert!(json.contains("/api/search"));
        assert!(json.contains("/api/resources/{id}/meta"));
        assert!(json.contains("/api/resources/{id}/edges"));
        assert!(json.contains("/api/relationships"));
        assert!(json.contains("/api/graph/subgraph"));
        assert!(json.contains("/api/cognitive-maps/{id}/shape"));
        assert!(json.contains("/api/invocations"));
        assert!(json.contains("/api/invocations/{id}"));
        assert!(json.contains("/api/invocations/{id}/close"));

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
        assert!(json.contains("\"name\": \"Relationships\""));
    }
}
