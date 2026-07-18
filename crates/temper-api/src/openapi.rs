use utoipa::openapi::path::{Parameter, ParameterBuilder, ParameterIn};
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::openapi::{ObjectBuilder, Required, Type};
use utoipa::{Modify, OpenApi};

use crate::handlers::resources::ListResourcesResponse;
use temper_core::types::api::{
    EventCursorResponse, HealthResponse, ProfileUpdateRequest, SearchDiagnostics, SearchParams,
    SearchReason, SearchResultRow, SearchScope, UnifiedSearchResultRow,
};
use temper_core::types::context::{
    ContextRowWithCounts, ReassignContextOutcome, ReassignContextRequest, ShareContextOutcome,
    ShareContextRequest, UnshareContextOutcome,
};
use temper_services::error::{ErrorBody, ErrorDetail};
use temper_workflow::types::managed_meta::ResourceMetaListResponse;
use temper_workflow::types::resource::{
    ContentResponse, DeleteResponse, ResourceCreateRequest, ResourceDetail, ResourceFacets,
    ResourceListResponse, ResourceRow, ResourceSortField, ResourceUpdateRequest, SortOrder,
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
        ResourceSortField,
        SortOrder,
        ResourceCreateRequest,
        ResourceUpdateRequest,
        ContentResponse,
        DeleteResponse,
        ContextRowWithCounts,
        ReassignContextRequest,
        ReassignContextOutcome,
        ShareContextRequest,
        ShareContextOutcome,
        UnshareContextOutcome,
        ProfileUpdateRequest,
        EventCursorResponse,
        SearchParams,
        SearchResultRow,
        UnifiedSearchResultRow,
        SearchDiagnostics,
        SearchScope,
        SearchReason,
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
        temper_core::types::access_gate::Entitlements,
        crate::handlers::profiles::ProfileWithEntitlements,
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
        temper_core::types::reassign::RemoveMemberOutcome,
        temper_core::types::reassign::ResidualOwnedReach,
        temper_core::types::reassign::ResidualContext,
        temper_core::types::access_gate::JoinRequest,
        temper_core::types::access_gate::JoinRequestStatus,
        temper_core::types::access_gate::PublicSystemSettings,
        crate::handlers::access::CreateRequestBody,
        temper_core::types::slack::SlackDisconnectRequest,
        temper_core::types::slack::SlackDisconnectResponse,
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
        (name = "Slack Link", description = "Slack account-link disconnect — self-serve and admin"),
    ),
    info(
        title = "Temper Cloud API",
        version = "0.1.0",
        description = "Knowledge base management API for temper cloud",
        // Declared explicitly rather than inherited from `CARGO_PKG_LICENSE`: no crate in this
        // workspace sets `license` in its Cargo.toml, so utoipa fabricates `{"name": ""}` — an
        // empty license name is invalid OpenAPI, and `openapi-generator validate` rejects the
        // document outright ("attribute info.license.identifier is missing"), which blocks every
        // generated client. `identifier` is the SPDX expression and is mutually exclusive with
        // `url`. Asserted by `openapi_spec_is_valid`.
        license(name = "MIT", identifier = "MIT"),
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

/// The `X-Temper-Surface` parameter, as documented on every path.
fn surface_header_parameter() -> Parameter {
    ParameterBuilder::new()
        .name(temper_workflow::operations::SURFACE_HEADER)
        .parameter_in(ParameterIn::Header)
        .required(Required::False)
        .description(Some(
            "The calling surface, for event-ledger attribution. Accepted values are `cli` \
             and `sdk`; an absent or unrecognized value attributes the write to `web`. This \
             is provenance, never authorization — an unrecognized value degrades, it never \
             rejects.",
        ))
        .schema(Some(
            ObjectBuilder::new()
                .schema_type(Type::String)
                .enum_values(Some(["cli", "sdk"]))
                .build(),
        ))
        .build()
}

/// Documents `X-Temper-Surface` once, on every path item.
///
/// **Not registered in `ApiDoc`'s `modifiers(...)`.** `ApiDoc::openapi()` runs its modifiers at
/// the moment `routes::openapi_spec()` seeds the `OpenApiRouter` — before the routers merge, when
/// `paths` is still empty. `SecurityAddon` survives that only because it edits `components`.
/// This addon edits `paths`, so `openapi_spec()` applies it *after* `split_for_parts()`.
///
/// Attaching at path-item level (rather than per-operation) matches the OpenAPI semantics of
/// "parameters common to all operations in this path item," which is exactly what a
/// client-identity header is. It also means a newly registered route cannot forget the header.
pub(crate) struct SurfaceHeaderAddon;

impl Modify for SurfaceHeaderAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let name = temper_workflow::operations::SURFACE_HEADER;
        for item in openapi.paths.paths.values_mut() {
            let params = item.parameters.get_or_insert_with(Vec::new);
            // Idempotent: `modify` must not double-insert if ever applied twice.
            if !params.iter().any(|p| p.name == name) {
                params.push(surface_header_parameter());
            }
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

        // An empty `info.license.name` is invalid OpenAPI and makes `openapi-generator validate`
        // reject the whole document, so no client can be generated. utoipa fabricates exactly that
        // when no crate declares `license` in Cargo.toml — hence the explicit `license(...)`.
        let license = &spec
            .info
            .license
            .as_ref()
            .expect("info.license is declared");
        assert!(
            !license.name.is_empty(),
            "info.license.name must not be empty"
        );
        assert_eq!(license.identifier.as_deref(), Some("MIT"));

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

    /// Every documented path carries the client-identity header, so a generated client can
    /// learn the header exists from the contract alone — which is the whole point of P0.
    ///
    /// Asserted against the *structure*, not the serialized string: a doc comment containing
    /// the header's name would make a `json.contains(..)` assertion pass vacuously.
    #[test]
    fn every_path_documents_the_surface_header() {
        use temper_workflow::operations::SURFACE_HEADER;

        let spec = crate::routes::openapi_spec();
        assert!(!spec.paths.paths.is_empty(), "spec has no paths to check");

        for (path, item) in spec.paths.paths.iter() {
            let params = item
                .parameters
                .as_ref()
                .unwrap_or_else(|| panic!("{path} has no path-level parameters"));
            assert!(
                params.iter().any(|p| p.name == SURFACE_HEADER),
                "{path} does not document {SURFACE_HEADER}",
            );
        }
    }

    /// The header is optional and never required: a browser omits it, and the server degrades.
    /// A `required: true` here would make every generated client demand it.
    #[test]
    fn the_surface_header_is_optional() {
        use temper_workflow::operations::SURFACE_HEADER;
        use utoipa::openapi::{path::ParameterIn, Required};

        let spec = crate::routes::openapi_spec();
        let (_, item) = spec.paths.paths.iter().next().expect("at least one path");
        let param = item
            .parameters
            .as_ref()
            .expect("path-level parameters")
            .iter()
            .find(|p| p.name == SURFACE_HEADER)
            .expect("surface header parameter");

        // `Required` and `ParameterIn` implement `PartialEq` but not `Debug`, so use `assert!`
        // with `==` rather than `assert_eq!` (which needs `Debug` to format a mismatch).
        assert!(param.required == Required::False);
        assert!(param.parameter_in == ParameterIn::Header);
    }

    /// Walk the serialized spec and collect every `#/components/schemas/<Name>` reference.
    fn collect_schema_refs(
        value: &serde_json::Value,
        out: &mut std::collections::BTreeSet<String>,
    ) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::String(reference)) = map.get("$ref") {
                    if let Some(name) = reference.strip_prefix("#/components/schemas/") {
                        out.insert(name.to_owned());
                    }
                }
                for nested in map.values() {
                    collect_schema_refs(nested, out);
                }
            }
            serde_json::Value::Array(items) => {
                for nested in items {
                    collect_schema_refs(nested, out);
                }
            }
            _ => {}
        }
    }

    /// A `$ref` to a component that does not exist makes the document invalid OpenAPI.
    /// `openapi-generator`'s 3.1 dereferencer throws on it and emits zero files, so this is
    /// the difference between a generatable contract and an unusable one.
    ///
    /// Enums reachable only through an `IntoParams` query struct are NOT auto-collected by
    /// `.routes()` — they must be named in `components(schemas(...))` by hand. `CorrelationId`
    /// is the control case: it is `$ref`'d from query params too, yet resolves, because it also
    /// hangs off `ActInput`, a request *body* schema.
    #[test]
    fn every_schema_ref_resolves() {
        use std::collections::BTreeSet;

        let spec = crate::routes::openapi_spec();
        let json = serde_json::to_value(&spec).expect("spec serializes to JSON");

        let defined: BTreeSet<String> = json["components"]["schemas"]
            .as_object()
            .expect("components.schemas is an object")
            .keys()
            .cloned()
            .collect();

        let mut referenced = BTreeSet::new();
        collect_schema_refs(&json, &mut referenced);

        // Guard against a vacuous pass: an empty spec references nothing and would trivially
        // report no dangling refs.
        assert!(
            !referenced.is_empty(),
            "spec references no component schemas"
        );

        let dangling: Vec<&String> = referenced.difference(&defined).collect();
        assert!(
            dangling.is_empty(),
            "spec $refs component schemas that are not defined: {dangling:?}",
        );
    }

    /// Every HTTP method that OpenAPI allows on a path item.
    const HTTP_METHODS: [&str; 8] = [
        "get", "put", "post", "delete", "options", "head", "patch", "trace",
    ];

    /// OpenAPI requires `operationId` to be present and unique across the whole document.
    /// utoipa defaults it to the handler's fn name, so two handlers both named `list` collide —
    /// and a generator partitions methods by tag, so a within-tag collision silently emits
    /// `list_0`. Uniqueness is what makes the generated client's method names stable.
    #[test]
    fn operation_ids_are_present_and_unique() {
        use std::collections::BTreeMap;

        let spec = crate::routes::openapi_spec();
        let json = serde_json::to_value(&spec).expect("spec serializes to JSON");
        let paths = json["paths"].as_object().expect("paths is an object");

        let mut owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut missing: Vec<String> = Vec::new();

        for (path, item) in paths {
            for method in HTTP_METHODS {
                let Some(operation) = item.get(method) else {
                    continue;
                };
                let location = format!("{} {path}", method.to_uppercase());
                match operation.get("operationId").and_then(|id| id.as_str()) {
                    Some(id) => owners.entry(id.to_owned()).or_default().push(location),
                    None => missing.push(location),
                }
            }
        }

        // Guard against a vacuous pass: an empty `paths` map has no duplicate ids either.
        assert!(!owners.is_empty(), "spec has no operations to check");

        assert!(
            missing.is_empty(),
            "operations without an operationId: {missing:?}"
        );

        let duplicates: BTreeMap<&String, &Vec<String>> =
            owners.iter().filter(|(_, ops)| ops.len() > 1).collect();
        assert!(
            duplicates.is_empty(),
            "operationId must be unique across the document; duplicates: {duplicates:#?}",
        );
    }
}
