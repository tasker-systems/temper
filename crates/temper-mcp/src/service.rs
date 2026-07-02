//! MCP service — the central handler for all MCP tool calls.
//!
//! Each invocation creates a fresh `TemperMcpService`. The authenticated
//! caller's profile is resolved from `McpClaims` (injected by the JWT
//! middleware into the HTTP request extensions) on **every** request.
//!
//! In stateless mode (Vercel serverless), `initialize()` may run on a
//! different invocation than the subsequent tool call, so we cannot rely
//! on profile caching across requests. Instead, each tool handler
//! extracts the HTTP `Parts` from rmcp's `Extension` and resolves the
//! profile from the JWT claims before executing.

use rmcp::{
    handler::server::{common::Extension, router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams,
        ReadResourceRequestParams, ReadResourceResult, ServerCapabilities, ServerInfo,
    },
    tool, tool_handler, tool_router,
};
use std::sync::Arc;
use tokio::sync::Mutex;

use temper_core::types::ids::ProfileId;
use temper_core::types::{AuthClaims, Profile};
use temper_services::services::profile_service;
use temper_services::state::AppState;

use crate::middleware::McpClaims;
use crate::tools;

/// Central MCP service. One instance per client session.
#[derive(Clone)]
pub struct TemperMcpService {
    pub api_state: AppState,
    tool_router: ToolRouter<Self>,
    /// Cached profile resolved from the Auth0 `sub` claim.
    profile: Arc<Mutex<Option<Profile>>>,
}

// Manual impl: `ToolRouter` does not implement Debug, so `tool_router` is omitted.
impl std::fmt::Debug for TemperMcpService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TemperMcpService")
            .field("api_state", &self.api_state)
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

#[tool_router]
impl TemperMcpService {
    pub fn new(api_state: AppState) -> Self {
        Self {
            api_state,
            tool_router: Self::tool_router(),
            profile: Arc::new(Mutex::new(None)),
        }
    }

    /// Resolve the caller's profile from JWT claims.
    async fn resolve_profile(&self, claims: &McpClaims) -> Result<Profile, rmcp::ErrorData> {
        let auth_claims = AuthClaims {
            provider: self.api_state.config.auth_provider_name.clone(),
            external_user_id: claims.sub.clone(),
            // MCP tokens may not include email; the profile service will
            // look it up from cached auth links.
            email: String::new(),
            email_verified: None,
            exp: claims.exp,
            iat: 0,
        };

        profile_service::resolve_from_claims(&self.api_state.pool, &auth_claims)
            .await
            .map_err(|e| {
                rmcp::ErrorData::internal_error(format!("Failed to resolve profile: {e}"), None)
            })
    }

    /// Resolve the profile from HTTP request parts and cache it.
    ///
    /// In stateless mode each request creates a fresh service instance, so
    /// the profile must be resolved per-request from the JWT claims that
    /// the auth middleware injected into the HTTP extensions.
    pub async fn ensure_profile_from_parts(
        &self,
        parts: &http::request::Parts,
    ) -> Result<(), rmcp::ErrorData> {
        let claims = parts.extensions.get::<McpClaims>().ok_or_else(|| {
            tracing::warn!("McpClaims not found in HTTP request extensions");
            rmcp::ErrorData::internal_error("Not authenticated".to_string(), None)
        })?;

        let profile = self.resolve_profile(claims).await?;
        tracing::debug!(
            profile_id = %profile.id,
            sub = %claims.sub,
            "Profile resolved from request"
        );

        // Account deactivation is the authn lever (parity with temper-api's auth middleware):
        // a soft-deleted profile cannot use MCP tools even with an otherwise-valid token. Checked
        // before system-access so a deactivated account is refused outright.
        if !profile.is_active {
            tracing::warn!(profile_id = %profile.id, "rejected: profile is deactivated");
            return Err(rmcp::ErrorData::new(
                rmcp::model::ErrorCode::INVALID_REQUEST,
                "This account has been deactivated. This error is terminal and should not be retried."
                    .to_string(),
                None,
            ));
        }

        // Check system access before allowing any tool use.
        let has_access = temper_services::services::access_service::has_system_access(
            &self.api_state.pool,
            ProfileId::from(profile.id),
        )
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to check system access: {e}"), None)
        })?;

        if !has_access {
            return Err(rmcp::ErrorData::new(
                rmcp::model::ErrorCode::INVALID_REQUEST,
                "Access to this temper instance requires approval. \
                 Visit https://temperkb.io/request-access or run \
                 `temper team join` in the CLI to request access. \
                 This error is terminal and should not be retried."
                    .to_string(),
                None,
            ));
        }

        let mut guard = self.profile.lock().await;
        *guard = Some(profile);
        Ok(())
    }

    /// Get the authenticated caller's profile, or return a protocol error.
    pub async fn require_profile(&self) -> Result<Profile, rmcp::ErrorData> {
        let guard = self.profile.lock().await;
        guard
            .clone()
            .ok_or_else(|| rmcp::ErrorData::internal_error("Not authenticated".to_string(), None))
    }

    // ── Tools ──────────────────────────────────────────────────────────

    #[tool(
        description = "Create a new resource in the knowledge base. Optionally include markdown content for indexing and search. Context must already exist — use create_context first if needed. Use list_doc_types to see available types."
    )]
    async fn create_resource(
        &self,
        Parameters(input): Parameters<tools::resources::CreateResourceInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::resources::create_resource(self, input).await
    }

    #[tool(
        description = "Get a resource by its ref (UUID or the decorated `slug-<uuid>` form). Set include_content to true to get the full markdown body."
    )]
    async fn get_resource(
        &self,
        Parameters(input): Parameters<tools::resources::GetResourceInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::resources::get_resource(self, input).await
    }

    #[tool(
        description = "List resources in the knowledge base. Filter by context and/or document type. Returns most recent first."
    )]
    async fn list_resources(
        &self,
        Parameters(input): Parameters<tools::resources::ListResourcesInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::resources::list_resources(self, input).await
    }

    #[tool(
        description = "Update a resource's title, slug, or content. Only provided fields are changed. New content triggers re-indexing."
    )]
    async fn update_resource(
        &self,
        Parameters(input): Parameters<tools::resources::UpdateResourceInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::resources::update_resource(self, input).await
    }

    #[tool(
        description = "Update a resource's frontmatter (managed_meta and open_meta) without re-chunking or re-embedding. Use for metadata-only edits like stage, tags, or relationship declarations. For content changes, use update_resource."
    )]
    async fn update_resource_meta(
        &self,
        Parameters(input): Parameters<tools::resources::UpdateResourceMetaInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::resources::update_resource_meta(self, input).await
    }

    #[tool(
        description = "Soft-delete a resource by ID. The resource is deactivated, not permanently removed."
    )]
    async fn delete_resource(
        &self,
        Parameters(input): Parameters<tools::resources::DeleteResourceInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::resources::delete_resource(self, input).await
    }

    #[tool(
        description = "Assert a directed relationship from a source resource to a target slug. Specify the source by owner/context/doctype/slug. Returns a edge_handle that identifies this relationship for future retype, reweight, or fold calls."
    )]
    async fn assert_relationship(
        &self,
        Parameters(input): Parameters<tools::relationships::AssertRelationshipInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::relationships::assert_relationship(self, input).await
    }

    #[tool(
        description = "Change the edge_kind and polarity of an existing relationship. Use the edge_handle returned by assert_relationship to identify the relationship."
    )]
    async fn retype_relationship(
        &self,
        Parameters(input): Parameters<tools::relationships::RetypeRelationshipInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::relationships::retype_relationship(self, input).await
    }

    #[tool(
        description = "Update the weight of an existing relationship. Use the edge_handle returned by assert_relationship to identify the relationship."
    )]
    async fn reweight_relationship(
        &self,
        Parameters(input): Parameters<tools::relationships::ReweightRelationshipInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::relationships::reweight_relationship(self, input).await
    }

    #[tool(
        description = "Retract (fold) an existing relationship, marking it inactive. Optionally provide a human-readable reason. Use the edge_handle returned by assert_relationship to identify the relationship."
    )]
    async fn fold_relationship(
        &self,
        Parameters(input): Parameters<tools::relationships::FoldRelationshipInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::relationships::fold_relationship(self, input).await
    }

    #[tool(description = "Set a facet (typed property) on a resource — the steward's facet act")]
    async fn facet_set(
        &self,
        Parameters(input): Parameters<tools::facets::FacetSetInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::facets::facet_set(self, input).await
    }

    #[tool(
        description = "Read a team-self-cognition cogmap's ingest delta: how many new resources + events have landed in the team's contexts since the steward's watermark, and whether that clears the threshold (i.e. the steward should run)."
    )]
    async fn steward_ingest_delta(
        &self,
        Parameters(input): Parameters<temper_core::types::steward::StewardDeltaInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::steward::steward_ingest_delta(self, input).await
    }

    #[tool(
        description = "Advance a team-self-cognition cogmap's ingest watermark to a given event id — the cursor a completed steward run records so the next delta counts only newer material. Requires cogmap-write."
    )]
    async fn steward_advance_watermark(
        &self,
        Parameters(input): Parameters<temper_core::types::steward::StewardAdvanceWatermarkInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::steward::steward_advance_watermark(self, input).await
    }

    #[tool(
        description = "Search resources using text queries, embedding vectors, or both. Send a plain text 'query' for full-text search — no embedding required."
    )]
    async fn search(
        &self,
        Parameters(input): Parameters<temper_core::types::api::SearchParams>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::search::search(self, input).await
    }

    #[tool(
        description = "Create (genesis) a new cognitive map: a cogmap plus its telos charter resource. System-admin only. The map is born with an EMPTY charter — author the charter and deliver it afterwards with `temper cogmap reconcile` (which embeds client-side). Idempotent at a supplied cogmap_id (re-creating is a no-op)."
    )]
    async fn cogmap_create(
        &self,
        Parameters(input): Parameters<tools::cognitive_maps::CogmapCreateInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::cogmap_create(self, input).await
    }

    #[tool(
        description = "Read a cognitive map's materialize delta: how many formation events (created resources, asserted/folded edges, facets, block edits) have landed on the map since it was last materialized, and whether that clears the threshold (i.e. the map should re-materialize)."
    )]
    async fn cogmap_materialize_delta(
        &self,
        Parameters(input): Parameters<temper_core::types::materialize::MaterializeDeltaInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::cogmap_materialize_delta(self, input).await
    }

    #[tool(
        description = "Re-materialize a cognitive map's regions when its formation delta since the last materialize clears the threshold; a safe no-op below threshold (materialized: false). This is the substrate's deterministic region-formation cadence — not an authored act. Requires cogmap-write."
    )]
    async fn cogmap_materialize(
        &self,
        Parameters(input): Parameters<temper_core::types::materialize::MaterializeTriggerInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::cogmap_materialize(self, input).await
    }

    #[tool(
        description = "Bind a cognitive map to a team (system-admin only). Widens the map's producer-intersection reach to include the team's shared resources. Idempotent — re-binding is a no-op (bound: false). Pass the map by ref and the team by UUID."
    )]
    async fn cogmap_bind(
        &self,
        Parameters(input): Parameters<tools::cognitive_maps::CogmapBindInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::cogmap_bind(self, input).await
    }

    #[tool(
        description = "Unbind a cognitive map from a team (system-admin only). No-op safe — unbinding a non-existent binding returns unbound: false. Pass the map by ref and the team by UUID."
    )]
    async fn cogmap_unbind(
        &self,
        Parameters(input): Parameters<tools::cognitive_maps::CogmapBindInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::cogmap_unbind(self, input).await
    }

    #[tool(
        description = "Grant a capability on a cognitive map (system-admin OR a holder of can_grant on the map). Post-Q-A, authoring a map requires an explicit write grant, not team membership. Pass the map by ref, exactly one principal (to_profile or to_team by UUID), and capability flags (read/write/grant; read is implied by write/grant)."
    )]
    async fn cogmap_grant(
        &self,
        Parameters(input): Parameters<tools::cognitive_maps::CogmapGrantInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::cogmap_grant(self, input).await
    }

    #[tool(
        description = "Revoke a capability grant on a cognitive map (system-admin OR a holder of can_grant on the map). No-op safe. Pass the map by ref and exactly one principal (from_profile or from_team by UUID)."
    )]
    async fn cogmap_revoke(
        &self,
        Parameters(input): Parameters<tools::cognitive_maps::CogmapRevokeInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::cogmap_revoke(self, input).await
    }

    #[tool(
        description = "Read a cognitive map's surface tier: its materialized regions (salience, cohesion, label, member count) under an optional lens. Pass the map by ref."
    )]
    async fn cogmap_shape(
        &self,
        Parameters(input): Parameters<temper_core::types::cognitive_maps::CogmapShapeInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::cogmap_shape(self, input).await
    }

    #[tool(
        description = "Read a cognitive map's per-region analytics metrics (centrality, content cohesion, internal tension, reference standing, telos alignment) under an optional lens. Pass the map by ref."
    )]
    async fn cogmap_region_metrics(
        &self,
        Parameters(input): Parameters<temper_core::types::cognitive_maps::CogmapRegionMetricsInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::cogmap_region_metrics(self, input).await
    }

    #[tool(
        description = "Read a cognitive map's map-level analytics: its telos charter resource id, staleness, and the regulation concept set. Pass the map by ref."
    )]
    async fn cogmap_analytics(
        &self,
        Parameters(input): Parameters<temper_core::types::cognitive_maps::CogmapAnalyticsInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::cogmap_analytics(self, input).await
    }

    #[tool(
        description = "Read a cognitive map's telos/charter blocks (statement / questions / framing) — the steward orients on this before acting. Pass the map by ref."
    )]
    async fn cogmap_read_charter(
        &self,
        Parameters(input): Parameters<tools::cognitive_maps::CogmapReadCharterInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::cogmap_read_charter(self, input).await
    }

    #[tool(
        description = "Open an agent-invocation envelope — an append-only accountability record for one agent run against a cognitive map. Returns the server-minted invocation_id; feed it into invocation_close when the run terminates. Pass the originating map by ref."
    )]
    async fn invocation_open(
        &self,
        Parameters(input): Parameters<temper_core::types::invocation::InvocationOpenInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::invocations::invocation_open(self, input).await
    }

    #[tool(
        description = "Close an open agent-invocation envelope with a terminal disposition (completed/failed/abandoned) and an optional opaque outcome. Identify the envelope by the invocation_id returned by invocation_open."
    )]
    async fn invocation_close(
        &self,
        Parameters(input): Parameters<temper_core::types::invocation::InvocationCloseInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::invocations::invocation_close(self, input).await
    }

    #[tool(
        description = "Show one agent-invocation envelope plus its acts (the stamped events that occurred under it), by raw UUID. Returns null if the invocation is absent or not readable."
    )]
    async fn invocation_show(
        &self,
        Parameters(input): Parameters<temper_core::types::invocation::InvocationShowInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::invocations::invocation_show(self, input).await
    }

    #[tool(
        description = "List agent-invocation envelopes, optionally narrowed by originating cognitive map (by ref) and/or lifecycle status (open/completed/failed/abandoned)."
    )]
    async fn invocation_list(
        &self,
        Parameters(input): Parameters<temper_core::types::invocation::InvocationListInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::invocations::invocation_list(self, input).await
    }

    #[tool(description = "List all contexts (workspaces) available to the authenticated user.")]
    async fn list_contexts(
        &self,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::contexts::list_contexts(self).await
    }

    #[tool(description = "Get details of a specific context by ID.")]
    async fn get_context(
        &self,
        Parameters(input): Parameters<tools::contexts::GetContextInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::contexts::get_context(self, input).await
    }

    #[tool(description = "Create a new context (workspace) in the knowledge base.")]
    async fn create_context(
        &self,
        Parameters(input): Parameters<temper_core::types::context::ContextCreateRequest>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::contexts::create_context(self, input).await
    }

    #[tool(
        description = "List all available document types with schema summaries. Returns id, name, has_schema, and required_fields for each type. Use describe_doc_type for full schema details."
    )]
    async fn list_doc_types(
        &self,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::doc_types::list_doc_types(self).await
    }

    #[tool(
        description = "Describe a specific document type in detail. Returns the full JSON schema, required fields, enum field values, and an example managed_meta object showing the tier-3 fields agents should supply."
    )]
    async fn describe_doc_type(
        &self,
        Parameters(input): Parameters<tools::doc_types::DescribeDocTypeInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::doc_types::describe_doc_type(self, input).await
    }

    #[tool(
        description = "Get the authenticated user's profile, including display name, email, and preferences."
    )]
    async fn get_profile(
        &self,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::profiles::get_profile(self).await
    }
}

#[tool_handler]
impl rmcp::ServerHandler for TemperMcpService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(
            rmcp::model::Implementation::new("temper-mcp", env!("CARGO_PKG_VERSION"))
                .with_title("Temper Knowledge Base"),
        )
        .with_instructions(
            "Access and manage your Temper knowledge base. \
                 Search notes, list resources, create new content, and explore contexts.",
        )
    }

    async fn initialize(
        &self,
        request: rmcp::model::InitializeRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::InitializeResult, rmcp::ErrorData> {
        // Let the default handler set up peer info.
        if context.peer.peer_info().is_none() {
            context.peer.set_peer_info(request);
        }

        // Extract McpClaims from the HTTP request parts injected by the
        // StreamableHttpService transport.
        if let Some(parts) = context.extensions.get::<http::request::Parts>() {
            if let Some(claims) = parts.extensions.get::<McpClaims>() {
                match self.resolve_profile(claims).await {
                    Ok(profile) => {
                        tracing::info!(
                            profile_id = %profile.id,
                            sub = %claims.sub,
                            "MCP session initialized"
                        );
                        let mut guard = self.profile.lock().await;
                        *guard = Some(profile);
                    }
                    Err(e) => {
                        tracing::warn!(sub = %claims.sub, "Failed to resolve profile: {e:?}");
                    }
                }
            } else {
                tracing::warn!("McpClaims not found in request extensions after middleware");
            }
        }

        Ok(self.get_info())
    }

    // ── Resources protocol ────────────────────────────────────────────

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListResourcesResult, rmcp::ErrorData> {
        if let Some(parts) = context.extensions.get::<http::request::Parts>() {
            self.ensure_profile_from_parts(parts).await?;
        }
        let profile = self.require_profile().await?;
        crate::resources::list_resources(&self.api_state, &profile, request).await
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListResourceTemplatesResult, rmcp::ErrorData> {
        crate::resources::list_resource_templates(request).await
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ReadResourceResult, rmcp::ErrorData> {
        if let Some(parts) = context.extensions.get::<http::request::Parts>() {
            self.ensure_profile_from_parts(parts).await?;
        }
        let profile = self.require_profile().await?;
        crate::resources::read_resource(&self.api_state, &profile, request).await
    }
}
