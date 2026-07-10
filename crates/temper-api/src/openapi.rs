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

// NOTE: no `paths(...)` list here. The set of documented paths is derived from the
// axum router in `routes::openapi_spec()`, which seeds itself with this `ApiDoc`
// (for info/tags/security/component-schemas) and then collects paths from every
// `.routes(routes!(…))` registration. The router is the single source of truth;
// this struct supplies only the ambient document metadata.
#[derive(OpenApi)]
#[openapi(
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
        temper_core::types::graph_context::ContextPanorama,
        temper_core::types::graph_context::ResidualGroups,
        temper_core::types::graph_context::ResidualBucket,
        temper_core::types::graph_context::GroupKeyMeta,
        temper_core::types::graph_territory::Territory,
        temper_core::types::graph_territory::TerritoryKind,
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
        temper_core::types::access_gate::JoinRequest,
        temper_core::types::access_gate::JoinRequestStatus,
        temper_core::types::access_gate::PublicSystemSettings,
        crate::handlers::access::CreateRequestBody,
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
        (name = "Access", description = "System access gate — self-service join requests and public settings"),
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
    #[test]
    fn openapi_spec_is_valid() {
        // Drive the router-derived spec, not `ApiDoc::openapi()` directly: `ApiDoc`
        // no longer carries a `paths(...)` list, so the paths only exist once the
        // router registrations are collected by `openapi_spec()`.
        let spec = crate::routes::openapi_spec();
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
        assert!(json.contains("/api/graph/contexts/panorama"));
        assert!(json.contains("/api/graph/contexts/composition"));
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

        // Verify previously-orphaned paths are now present (they gained documentation
        // only by being derived from the router — they were never in `paths(...)`).
        assert!(json.contains("/api/teams"));
        assert!(json.contains("/api/teams/{id}/members"));
        assert!(json.contains("/api/ingest"));
        assert!(json.contains("/api/ingest/{id}"));
        assert!(json.contains("/api/contexts"));
        assert!(json.contains("/api/contexts/{id}"));
        assert!(json.contains("/api/resources/{id}/blocks"));
        assert!(json.contains("/api/resources/{id}/finalize"));
        assert!(json.contains("/api/resources/{id}/provenance"));
        assert!(json.contains("/api/graph/home"));
        assert!(json.contains("/api/graph/regions/composition"));
        assert!(json.contains("/api/graph/cogmaps/{id}/panorama"));
        assert!(json.contains("/api/cogmaps/{id}/graph/slice"));
        assert!(json.contains("/api/graph/elements/{kind}/{id}/trail"));

        // Verify the operator / internal surfaces are ABSENT from the contract.
        // These are mounted with plain `.route()` (admin) or on sub-routers that
        // `openapi_spec()` deliberately does not merge (internal, embed drain).
        // Check the actual path keys, not a raw-JSON substring: `/api/embed/dispatch`
        // appears verbatim inside a component schema's doc-comment description, so a
        // `json.contains` check would spuriously match it.
        for absent in [
            "/api/access/admin/promote",
            "/api/access/admin/requests",
            "/api/access/admin/requests/{id}",
            "/api/access/admin/settings",
            "/internal/saml/reconcile",
            "/api/embed/dispatch",
        ] {
            assert!(
                !spec.paths.paths.contains_key(absent),
                "operator/internal path {absent} must not be in the contract",
            );
        }

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
