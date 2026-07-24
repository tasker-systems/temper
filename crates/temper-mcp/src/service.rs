//! MCP service — the central handler for all MCP tool calls.
//!
//! Each invocation creates a fresh `TemperMcpService`. The authenticated caller's
//! profile is resolved on **every** request by handing the shared auth seam the
//! `RawJwtClaims` + `BearerToken` the JWT middleware injected into the HTTP request
//! extensions. This surface constructs no principal of its own: it presents a verified
//! token and maps `AuthzError` to rmcp (see `map_authz_error`).
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

use temper_core::types::Profile;
use temper_services::auth::RawJwtClaims;
use temper_services::state::AppState;

use crate::middleware::BearerToken;
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

    /// Resolve the profile from HTTP request parts and cache it.
    ///
    /// In stateless mode each request creates a fresh service instance, so
    /// the profile must be resolved per-request from the JWT claims that
    /// the auth middleware injected into the HTTP extensions.
    pub async fn ensure_profile_from_parts(
        &self,
        parts: &http::request::Parts,
    ) -> Result<(), rmcp::ErrorData> {
        let (claims, token) = authed_request(parts)?;

        // Level 1: classify → human email ladder → resolve → deactivation gate, all in
        // the shared seam. This surface used to build the human `AuthClaims` itself,
        // with `email: ""` and no ladder — the drift that let an unnamable human
        // auto-provision a junk profile here while temper-api refused the same token.
        // It no longer constructs a principal at all; it hands over the verified token.
        let authed = temper_services::auth::authenticate_token(&self.api_state, claims, &token.0)
            .await
            .map_err(map_authz_error)?;

        tracing::debug!(profile_id = %authed.profile().id, sub = %claims.sub, "Profile resolved");

        // Level 2: system-access gate (shared seam).
        temper_services::auth::require_system_access(&self.api_state.pool, &authed)
            .await
            .map_err(map_authz_error)?;

        let mut guard = self.profile.lock().await;
        *guard = Some(authed.into_profile());
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
        description = "Trace a resource's derived_from lineage in both directions: `ancestors` (what it derives from) and `descendants` (what derives from it), each a transitive, access-gated walk. Every node carries the reaching edge and whether that edge is folded — a folded ancestor is shown, flagged, so you can see when what you rest on has been superseded. Optional `depth` bounds the walk (default 16)."
    )]
    async fn resource_lineage(
        &self,
        Parameters(input): Parameters<tools::resources::ResourceLineageInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::resources::resource_lineage(self, input).await
    }

    #[tool(
        description = "Get the itemized block-provenance for a resource — the sources each of its content blocks was distilled from, in (block, accretion) order. Access-scoped: an unreadable resource returns an empty list."
    )]
    async fn get_block_provenance(
        &self,
        Parameters(input): Parameters<tools::resources::GetBlockProvenanceInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::resources::get_block_provenance(self, input).await
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
        description = "Attach provenance sources to a resource's block WITHOUT a body revise (no re-chunk/re-embed) — the cheap, citation-grade backfill for a corpus imported without sources. Records block-provenance rows on the addressed block; body and embeddings are unchanged. A source URL may carry a span-locator fragment (e.g. '…/doc.md#L120-L180'), preserved verbatim and surfaced by get_block_provenance. Returns the resulting per-block provenance."
    )]
    async fn annotate_resource(
        &self,
        Parameters(input): Parameters<tools::resources::AnnotateResourceInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::resources::annotate_resource(self, input).await
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
        description = "Grant a capability on a resource to a profile or team (system-admin, a can_grant holder, OR the resource owner). Pass the resource by ref, exactly one principal (to_profile or to_team by UUID), and capability flags (read/write/grant; read is implied by write/grant)."
    )]
    async fn resource_grant(
        &self,
        Parameters(input): Parameters<tools::resources::ResourceGrantInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::resources::resource_grant(self, input).await
    }

    #[tool(
        description = "Revoke a capability grant on a resource (system-admin, a can_grant holder, or the resource owner). No-op safe. Pass the resource by ref and exactly one principal (from_profile or from_team by UUID)."
    )]
    async fn resource_revoke(
        &self,
        Parameters(input): Parameters<tools::resources::ResourceRevokeInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::resources::resource_revoke(self, input).await
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
        description = "Orient in a CONTEXT by its regions: read the context's materialized regions (salience, cohesion, label, member count), most salient first. This is the region-level view of everything in a context — the fastest way to see what a context is ABOUT before reading any resource in it. Pass the context by ref (@me/<slug>, +<team>/<slug>, or a UUID)."
    )]
    async fn context_shape(
        &self,
        Parameters(input): Parameters<temper_core::types::cognitive_maps::ContextShapeInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::context_shape(self, input).await
    }

    #[tool(
        description = "Read a context's per-region analytics metrics (centrality, content cohesion, internal tension, reference standing, telos alignment) under an optional lens. Pass the context by ref (@me/<slug>, +<team>/<slug>, or a UUID)."
    )]
    async fn context_region_metrics(
        &self,
        Parameters(input): Parameters<temper_core::types::cognitive_maps::ContextShapeInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::context_region_metrics(self, input).await
    }

    #[tool(
        description = "Re-form a context's regions when enough has changed since its last materialize (below threshold this is a safe no-op). Requires write access to the context. Pass the context by ref (@me/<slug>, +<team>/<slug>, or a UUID)."
    )]
    async fn context_materialize(
        &self,
        Parameters(input): Parameters<temper_core::types::materialize::ContextMaterializeInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::context_materialize(self, input).await
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
        description = "List the cognitive maps you can see, each with its id, name, held-by team scope, region/resource counts, and charter statement (what the map is for). The first move for orienting across maps — every row's id is addressable by the other cogmap tools. Optional name_contains narrows by name substring."
    )]
    async fn cogmap_list(
        &self,
        Parameters(input): Parameters<tools::cognitive_maps::CogmapListInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::cogmap_list(self, input).await
    }

    #[tool(
        description = "Begin a segmented (multi-block) ingest for a body too large to send in one call, landing its first segment. Prefer create_resource for anything that fits a single call — segmented ingest costs extra round-trips. content is segment 0's text and content_hash is its bare-hex sha256. Returns resource_id, the landed block set, and an opaque body_hash. Follow with ingest_append for each further segment, then ingest_finalize."
    )]
    async fn ingest_begin(
        &self,
        Parameters(input): Parameters<tools::ingest::IngestBeginInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::ingest::ingest_begin(self, input).await
    }

    #[tool(
        description = "Append one segment to an in-progress segmented ingest. Segments are zero-indexed and segment 0 landed at ingest_begin, so start at seq=1 and send them in order. content_hash is the bare-hex sha256 of content; a mismatch is rejected. Re-appending an already-landed segment is a safe no-op, which is what makes retry and resume safe. Returns the landed block set and the body_hash to echo at finalize. You do not chunk or embed anything — send raw markdown text."
    )]
    async fn ingest_append(
        &self,
        Parameters(input): Parameters<tools::ingest::IngestAppendInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::ingest::ingest_append(self, input).await
    }

    #[tool(
        description = "Declare a segmented ingest complete. expected_blocks is the total segment count including segment 0; expected_body_hash is the opaque body_hash from your most recent ingest_append or ingest_blocks response, echoed back verbatim. Fails if the landed set does not match, which means a segment is missing — call ingest_blocks to see which."
    )]
    async fn ingest_finalize(
        &self,
        Parameters(input): Parameters<tools::ingest::IngestFinalizeInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::ingest::ingest_finalize(self, input).await
    }

    #[tool(
        description = "Read back the segments that have landed for an in-progress segmented ingest. This is how you resume after an interruption: compare the returned seq set against your segments, re-send only the missing ones with ingest_append, then ingest_finalize. Also returns the current body_hash."
    )]
    async fn ingest_blocks(
        &self,
        Parameters(input): Parameters<tools::ingest::IngestBlocksInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::ingest::ingest_blocks(self, input).await
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

    #[tool(
        description = "Read the admin ledger: who granted what, to whom, and when. Exactly one axis — `subject` ('<kind>:<uuid>', e.g. kb_resources:0199...) asks what was done TO a thing; `actor` (a profile uuid) asks what a principal DID. Your own acts are always readable; reading another actor's is an audit and requires admin. The response carries `epoch`, the point admin history begins — an empty `entries` with an epoch means 'nothing since then', never 'nothing ever'."
    )]
    async fn admin_ledger(
        &self,
        Parameters(input): Parameters<temper_core::types::admin::AdminLedgerInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::admin_ledger::admin_ledger(self, input).await
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
        description = "Share a context into a team's read-reach (system-admin only). Every member of the team gains read access to the context's resources. Idempotent — safe to call when the share already exists. Pass the context and team by UUID (from list_contexts and your team listing)."
    )]
    async fn share_context(
        &self,
        Parameters(input): Parameters<tools::contexts::ShareContextInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::contexts::share_context(self, input).await
    }

    #[tool(
        description = "Unshare a context from a team (system-admin only), removing the team's read-reach into it. No-op safe when there is no share to remove. Pass the context and team by UUID."
    )]
    async fn unshare_context(
        &self,
        Parameters(input): Parameters<tools::contexts::ShareContextInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::contexts::unshare_context(self, input).await
    }

    #[tool(
        description = "Transfer a context's ownership to a team — the single path to shared authorship. Read-sharing (share_context) lets a team read a context; owning it lets the team's members write into it. Authorized for the context's administrator who also manages the target team (owner/maintainer), or an instance admin. Idempotent — safe when the context is already owned by the target team. Pass the context and target team by UUID."
    )]
    async fn transfer_context(
        &self,
        Parameters(input): Parameters<tools::contexts::TransferContextInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::contexts::transfer_context(self, input).await
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
        description = "Describe the recognized open_meta conventions. Returns the self-describing JSON schema for the open (caller-defined) frontmatter tier — recognized keys, their shapes, and which are FTS-indexed (and at what weight) vs shape-only — plus discouraged bare keys. The tier stays free-form (additionalProperties: true); this is guidance for attaching keywords/tags/descriptor/date so they rank and validate correctly."
    )]
    async fn describe_open_meta(
        &self,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::doc_types::describe_open_meta(self).await
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

    #[tool(description = "List the pending team invitations addressed to you (across all teams).")]
    async fn list_my_invitations(
        &self,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::invitations::list_my_invitations(self).await
    }

    #[tool(description = "Accept a team invitation by its token.")]
    async fn accept_invitation(
        &self,
        Parameters(input): Parameters<tools::invitations::AcceptInvitationInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::invitations::accept_invitation(self, input).await
    }

    #[tool(description = "Decline a team invitation by its token.")]
    async fn decline_invitation(
        &self,
        Parameters(input): Parameters<tools::invitations::DeclineInvitationInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::invitations::decline_invitation(self, input).await
    }
}

/// The two things the JWT middleware injects for an authenticated request: the
/// decoded claims and the raw token the seam's `/userinfo` rung may need.
///
/// Their absence is not an authentication failure but a wiring bug — the middleware
/// injects both or rejects the request — so it maps to an internal error, as the
/// missing-claims case always has.
fn authed_request(
    parts: &http::request::Parts,
) -> Result<(&RawJwtClaims, &BearerToken), rmcp::ErrorData> {
    let claims = parts.extensions.get::<RawJwtClaims>().ok_or_else(|| {
        tracing::warn!("RawJwtClaims not found in HTTP request extensions");
        rmcp::ErrorData::internal_error("Not authenticated".to_string(), None)
    })?;
    let token = parts.extensions.get::<BearerToken>().ok_or_else(|| {
        tracing::warn!("BearerToken not found in HTTP request extensions");
        rmcp::ErrorData::internal_error("Not authenticated".to_string(), None)
    })?;
    Ok((claims, token))
}

/// Map the shared seam's refusal vocabulary onto rmcp transport errors.
/// The deactivation and access-required strings are terminal ("do not retry")
/// and byte-identical to the pre-seam inline messages.
fn map_authz_error(e: temper_services::auth::AuthzError) -> rmcp::ErrorData {
    use temper_services::auth::AuthzError;
    match e {
        // Terminal, like the machine-gate denial below: the token is structurally
        // incoherent (machine-shaped, but not coherently a machine), so retrying it
        // changes nothing. The seam has already logged the `sub` and the reason.
        AuthzError::Refused(_) => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            "This token is machine-shaped but does not declare a valid \
             client_credentials grant. This error is terminal and should not be retried."
                .to_string(),
            None,
        ),
        // Also terminal: a human token we cannot put a name to. Before the seam owned
        // the email ladder this surface skipped it entirely and auto-provisioned a
        // profile with `email: ''`; that junk-row path is closed on purpose. The token
        // carries no `email` claim and no earlier sign-in cached one, so re-sending it
        // resolves nothing — the fix is a token with an email claim, not a retry.
        AuthzError::EmailResolution(err) => {
            tracing::warn!(%err, "rejected: could not resolve an email for a human token");
            rmcp::ErrorData::new(
                rmcp::model::ErrorCode::INVALID_REQUEST,
                "Could not resolve an email address for this token. \
                 This error is terminal and should not be retried."
                    .to_string(),
                None,
            )
        }
        AuthzError::Deactivated { profile_id } => {
            tracing::warn!(%profile_id, "rejected: profile is deactivated");
            rmcp::ErrorData::new(
                rmcp::model::ErrorCode::INVALID_REQUEST,
                "This account has been deactivated. This error is terminal and should not be retried."
                    .to_string(),
                None,
            )
        }
        // The advertised remedy is the shared constant, not a literal: this surface
        // and temper-api's 403 must name the same command, and it must be one that
        // parses. Both drifted onto `temper team join` — which accepts a team
        // invitation and has no `--message`, so it does not request access at all.
        AuthzError::SystemAccessDenied { .. } => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!(
                "Access to this temper instance requires approval. \
                 Visit https://temperkb.io/request-access or run \
                 `{}` in the CLI to request access. \
                 This error is terminal and should not be retried.",
                temper_core::types::access_gate::REQUEST_ACCESS_COMMAND
            ),
            None,
        ),
        // An `Unauthorized` here is a terminal authentication denial, not a transient
        // failure — most often the machine-principal registration gate rejecting an
        // unregistered or revoked `client_id` (G3 Phase A). It must surface as a terminal
        // error the way `Deactivated` / `SystemAccessDenied` do, so a conformant client (or
        // a Sidekiq worker, per the temper-rb contract) does not retry a permanent denial.
        // The HTTP surface already returns a 401 for the same case; this keeps the two
        // surfaces consistent. Any other `ProfileResolution` error is a genuine internal
        // fault (a DB failure mid-resolution) and stays retryable.
        AuthzError::ProfileResolution(temper_services::error::ApiError::Unauthorized(msg)) => {
            tracing::warn!(%msg, "rejected: machine principal not admitted by the gate");
            rmcp::ErrorData::new(
                rmcp::model::ErrorCode::INVALID_REQUEST,
                format!("{msg} This error is terminal and should not be retried."),
                None,
            )
        }
        AuthzError::ProfileResolution(err) => {
            rmcp::ErrorData::internal_error(format!("Failed to resolve profile: {err}"), None)
        }
        AuthzError::AccessCheck(err) => {
            rmcp::ErrorData::internal_error(format!("Failed to check system access: {err}"), None)
        }
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

        // Resolve the session's principal through the shared seam, from the HTTP request
        // parts injected by the StreamableHttpService transport.
        //
        // This used to call `resolve_from_claims` directly, which skipped Level 1's
        // `is_active` gate: a deactivated account was refused on every *tool call* but
        // still opened a session. `authenticate_token` closes that — refusals here are
        // authentication decisions and propagate, rather than being warned past as the
        // old best-effort cache seed was.
        if let Some(parts) = context.extensions.get::<http::request::Parts>() {
            let (claims, token) = authed_request(parts)?;
            let authed =
                temper_services::auth::authenticate_token(&self.api_state, claims, &token.0)
                    .await
                    .map_err(map_authz_error)?;

            tracing::info!(
                profile_id = %authed.profile().id,
                sub = %claims.sub,
                "MCP session initialized"
            );
            let mut guard = self.profile.lock().await;
            *guard = Some(authed.into_profile());
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

#[cfg(test)]
mod tests {
    use super::TemperMcpService;

    /// A `#[tool]` written into the wrong impl block compiles fine and is simply never advertised.
    /// Assert the router actually carries the segmented-ingest four, rather than inferring it from
    /// "it compiled". Needs no database — `tool_router()` is a pure associated function.
    #[test]
    fn the_four_ingest_tools_are_advertised_by_the_router() {
        let router = TemperMcpService::tool_router();
        let names: Vec<String> = router
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();

        for expected in [
            "ingest_begin",
            "ingest_append",
            "ingest_finalize",
            "ingest_blocks",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "{expected} is not advertised; router has {names:?}"
            );
        }
    }
}
