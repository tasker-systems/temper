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

use temper_api::services::profile_service;
use temper_api::state::AppState;
use temper_core::types::ids::ProfileId;
use temper_core::types::{AuthClaims, Profile};

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

        // Check system access before allowing any tool use.
        let has_access = temper_api::services::access_service::has_system_access(
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
        description = "Get a resource by ID or slug. When using slug, provide context_name to disambiguate. Set include_content to true to get the full markdown."
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
