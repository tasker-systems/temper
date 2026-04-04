//! MCP service — the central handler for all MCP tool calls.
//!
//! Each invocation creates a fresh `TemperMcpService`. The authenticated
//! caller's profile is resolved from `McpClaims` injected by the JWT
//! middleware and cached for the lifetime of the service.

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
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

#[tool_router]
impl TemperMcpService {
    pub fn new(api_state: AppState) -> Self {
        Self {
            api_state,
            tool_router: Self::tool_router(),
            profile: Arc::new(Mutex::new(None)),
        }
    }

    /// Resolve and cache the caller's profile from the database.
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

    /// Get the authenticated caller's profile, or return a protocol error.
    pub async fn require_profile(&self) -> Result<Profile, rmcp::ErrorData> {
        let guard = self.profile.lock().await;
        guard
            .clone()
            .ok_or_else(|| rmcp::ErrorData::internal_error("Not authenticated".to_string(), None))
    }

    // ── Tools ──────────────────────────────────────────────────────────

    #[tool(description = "List resources in the knowledge base. Optionally filter by context.")]
    async fn list_resources(
        &self,
        Parameters(input): Parameters<temper_core::types::resource::ResourceListParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        tools::resources::list_resources(self, input).await
    }

    #[tool(description = "Get a specific resource by ID, optionally including full content.")]
    async fn get_resource(
        &self,
        Parameters(input): Parameters<tools::resources::GetResourceInput>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        tools::resources::get_resource(self, input).await
    }

    #[tool(description = "Create a new resource in the knowledge base.")]
    async fn create_resource(
        &self,
        Parameters(input): Parameters<temper_core::types::resource::ResourceCreateRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        tools::resources::create_resource(self, input).await
    }

    #[tool(description = "Semantic search across resources using an embedding vector.")]
    async fn search(
        &self,
        Parameters(input): Parameters<temper_core::types::api::SearchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        tools::search::search(self, input).await
    }

    #[tool(description = "List all contexts (workspaces) available to the authenticated user.")]
    async fn list_contexts(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        tools::contexts::list_contexts(self).await
    }

    #[tool(description = "Get details of a specific context by ID.")]
    async fn get_context(
        &self,
        Parameters(input): Parameters<tools::contexts::GetContextInput>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        tools::contexts::get_context(self, input).await
    }

    #[tool(description = "Create a new context (workspace) in the knowledge base.")]
    async fn create_context(
        &self,
        Parameters(input): Parameters<temper_core::types::context::ContextCreateRequest>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        tools::contexts::create_context(self, input).await
    }

    #[tool(
        description = "Update a resource's title, slug, or mimetype. Only the fields provided will be changed."
    )]
    async fn update_resource(
        &self,
        Parameters(input): Parameters<tools::resources::UpdateResourceInput>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        tools::resources::update_resource(self, input).await
    }

    #[tool(
        description = "Soft-delete a resource by ID. The resource is deactivated, not permanently removed."
    )]
    async fn delete_resource(
        &self,
        Parameters(input): Parameters<tools::resources::DeleteResourceInput>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        tools::resources::delete_resource(self, input).await
    }

    #[tool(
        description = "Get the full markdown content of a resource. Returns the reconstituted document."
    )]
    async fn get_resource_content(
        &self,
        Parameters(input): Parameters<tools::resources::GetResourceContentInput>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        tools::resources::get_resource_content(self, input).await
    }

    #[tool(
        description = "List events in the knowledge base. Useful for auditing and debugging. Optionally filter by resource ID or event type."
    )]
    async fn list_events(
        &self,
        Parameters(input): Parameters<temper_core::types::api::EventListParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        tools::events::list_events(self, input).await
    }

    #[tool(
        description = "Get the authenticated user's profile, including display name, email, and preferences."
    )]
    async fn get_profile(&self) -> Result<CallToolResult, rmcp::ErrorData> {
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
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListResourcesResult, rmcp::ErrorData> {
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
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ReadResourceResult, rmcp::ErrorData> {
        let profile = self.require_profile().await?;
        crate::resources::read_resource(&self.api_state, &profile, request).await
    }
}
