use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};

use crate::handlers::resources::ListResourcesResponse;
use temper_core::types::api::{
    EventCursorResponse, HealthResponse, ProfileUpdateRequest, SearchParams, SearchResultRow,
    UnifiedSearchResultRow,
};
use temper_core::types::context::{
    ContextRowWithCounts, ShareContextOutcome, ShareContextRequest, UnshareContextOutcome,
};
use temper_services::error::{ErrorBody, ErrorDetail};
use temper_workflow::types::managed_meta::ResourceMetaListResponse;
use temper_workflow::types::resource::{
    ContentResponse, DeleteResponse, ResourceCreateRequest, ResourceDetail, ResourceFacets,
    ResourceListResponse, ResourceRow, ResourceUpdateRequest,
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
        crate::handlers::resources::grant,
        crate::handlers::resources::revoke,
        crate::handlers::contexts::share_team,
        crate::handlers::contexts::unshare_team,
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
        crate::handlers::facets::set_facet,
        crate::handlers::graph::get_subgraph,
        crate::handlers::cognitive_maps::genesis,
        crate::handlers::cognitive_maps::reconcile,
        crate::handlers::cognitive_maps::shape,
        crate::handlers::cognitive_maps::materialize_delta,
        crate::handlers::cognitive_maps::materialize,
        crate::handlers::cognitive_maps::region_metrics,
        crate::handlers::cognitive_maps::analytics,
        crate::handlers::cognitive_maps::bind_team,
        crate::handlers::cognitive_maps::unbind_team,
        crate::handlers::cognitive_maps::grant,
        crate::handlers::cognitive_maps::revoke,
        crate::handlers::invocations::open,
        crate::handlers::invocations::close,
        crate::handlers::invocations::show,
        crate::handlers::invocations::list,
        crate::handlers::steward::delta,
        crate::handlers::steward::advance,
        crate::handlers::steward::sweep,
        crate::handlers::steward::candidates,
        crate::handlers::steward::dispatch,
        crate::handlers::embed::dispatch,
        crate::handlers::invitations::create,
        crate::handlers::invitations::list,
        crate::handlers::invitations::list_mine,
        crate::handlers::invitations::accept,
        crate::handlers::invitations::decline,
        crate::handlers::reassign::reassign_resource,
        crate::handlers::reassign::reassign_team,
    ),
    components(schemas(
        HealthResponse,
        ResourceRow,
        ResourceDetail,
        ResourceListResponse,
        ResourceMetaListResponse,
        ListResourcesResponse,
        ResourceFacets,
        ResourceCreateRequest,
        ResourceUpdateRequest,
        ContentResponse,
        DeleteResponse,
        ContextRowWithCounts,
        ShareContextRequest,
        ShareContextOutcome,
        UnshareContextOutcome,
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
        temper_core::types::facet_requests::FacetSetRequest,
        temper_core::types::facet_requests::FacetAck,
        temper_core::types::reconcile::ReconcileCogmapRequest,
        temper_core::types::reconcile::ReconcileEntry,
        temper_core::types::reconcile::ReconcileEdge,
        temper_core::types::reconcile::ReconcileTombstone,
        temper_core::types::reconcile::ReconcileEdgeTombstone,
        temper_core::types::reconcile::ReconcileOutcome,
        temper_core::types::reconcile::ReconcileTelos,
        temper_core::types::reconcile::ReconcileTelosBlock,
        temper_core::types::reconcile::CreateCogmapRequest,
        temper_core::types::reconcile::CreateCogmapOutcome,
        temper_core::types::cognitive_maps::CogmapRegionRow,
        temper_core::types::materialize::MaterializeDelta,
        temper_core::types::materialize::MaterializeRequest,
        temper_core::types::materialize::MaterializeAck,
        temper_core::types::cognitive_maps::CogmapRegionMetricsRow,
        temper_core::types::cognitive_maps::CogmapAnalyticsRow,
        temper_core::types::cognitive_maps::CogmapStaleness,
        temper_core::types::cognitive_maps::CogmapRegulationRow,
        temper_core::types::cognitive_maps::BindTeamRequest,
        temper_core::types::cognitive_maps::BindTeamOutcome,
        temper_core::types::cognitive_maps::UnbindTeamOutcome,
        temper_core::types::cognitive_maps::CogmapGrantBody,
        temper_core::types::cognitive_maps::CogmapRevokeBody,
        temper_core::types::resource_grant::ResourceGrantBody,
        temper_core::types::resource_grant::ResourceRevokeBody,
        temper_core::types::cognitive_maps::GrantOutcome,
        temper_core::types::cognitive_maps::RevokeOutcome,
        temper_core::types::invocation_requests::OpenInvocationRequest,
        temper_core::types::invocation_requests::CloseInvocationRequest,
        temper_core::types::invocation_requests::InvocationAck,
        temper_core::types::invocation_requests::InvocationCloseAck,
        temper_core::types::invocation::InvocationView,
        temper_core::types::invocation::InvocationSummary,
        temper_core::types::invocation::InvocationActRow,
        temper_core::types::invocation::Disposition,
        temper_core::types::steward::IngestDelta,
        temper_core::types::steward::AdvanceWatermarkRequest,
        temper_core::types::steward::AdvanceWatermarkAck,
        temper_core::types::steward::DriftSweepRow,
        temper_core::types::steward::DispatchTickRequest,
        temper_core::types::steward::DispatchTickResponse,
        temper_core::types::workflow_job::ClaimedJob,
        temper_core::types::workflow_job::EmbedDispatchSummary,
        temper_core::types::invitation::TeamInvitation,
        temper_core::types::invitation::InvitationStatus,
        temper_core::types::invitation::InviteeInvitation,
        temper_core::types::invitation::CreateInvitationRequest,
        temper_core::types::invitation::AcceptInvitationResponse,
        temper_core::types::reassign::ReassignResourceRequest,
        temper_core::types::reassign::ReassignAck,
        temper_core::types::reassign::BulkReassignRequest,
        temper_core::types::reassign::BulkReassignAck,
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
        (name = "Facets", description = "Typed facet property writes (facet_set)"),
        (name = "Cognitive Maps", description = "Cognitive-map content reconcile (admin-gated)"),
        (name = "Invocations", description = "Agent-invocation envelope (accountability)"),
        (name = "Invitations", description = "Team invitations (invite/list/accept/decline)"),
        (name = "Reassign", description = "Resource ownership reassignment (single + bulk team-scoped)"),
        (name = "Steward", description = "Team-self-cognition steward ingest trigger (delta + watermark)"),
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
        assert!(json.contains("/api/facets"));
        assert!(json.contains("/api/graph/subgraph"));
        assert!(json.contains("/api/cognitive-maps/{id}/shape"));
        assert!(json.contains("/api/cognitive-maps/{id}/region-metrics"));
        assert!(json.contains("/api/cognitive-maps/{id}/analytics"));
        assert!(json.contains("/api/invocations"));
        assert!(json.contains("/api/invocations/{id}"));
        assert!(json.contains("/api/invocations/{id}/close"));
        assert!(json.contains("/api/teams/{id}/invite"));
        assert!(json.contains("/api/teams/{id}/invitations"));
        assert!(json.contains("/api/invitations/mine"));
        assert!(json.contains("/api/invitations/{token}/accept"));
        assert!(json.contains("/api/invitations/{token}/decline"));
        assert!(json.contains("/api/resources/{id}/reassign"));
        assert!(json.contains("/api/teams/{id}/reassign"));

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
        assert!(json.contains("\"name\": \"Facets\""));
    }
}
