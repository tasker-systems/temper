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
    handler::server::{common::Extension, wrapper::Parameters},
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
///
/// The `ToolRouter` is **not** stored as a field. Under rmcp ≥ 1.4 the `#[tool_handler]` macro
/// defaults to `Self::tool_router()` — rebuilding the router per `call_tool` / `list_tools`
/// call — so a stored field would be dead weight: built in `new` and never read. Temper runs the
/// streamable-HTTP transport in **stateless mode** (`with_stateful_mode(false)` in
/// `router::build_router`), which calls the service factory — and thus `new` — once per HTTP
/// request, so each service instance serves exactly one call. Building the router in `new` and
/// reading it in `call_tool` is the same number of builds as building it in `call_tool` alone;
/// the field bought nothing. Removing it also keeps the code aligned with the doc below: each
/// invocation creates a fresh service, and the router is just as fresh.
///
/// `ToolRouter<Self>` is imported only because the `#[tool_router]` macro references it in its
/// generated associated function; no value of that type lives on this struct.
#[derive(Clone)]
pub struct TemperMcpService {
    pub api_state: AppState,
    /// Cached profile resolved from the Auth0 `sub` claim.
    profile: Arc<Mutex<Option<Profile>>>,
}

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

        // Fill the `mcp_request` root span's deferred `profile_id` (declared Empty in
        // `build_router`). Recorded here rather than in `require_mcp_auth` because that middleware
        // only validates the JWT — this is the first point at which a *profile* exists. Same
        // deferred-field pattern as temper-api's auth middleware.
        tracing::Span::current().record("profile_id", tracing::field::display(authed.profile().id));

        // `profile_id` is the identifier to carry here — the raw OAuth `sub` is deliberately NOT
        // emitted. At this point the profile has resolved, so `sub` adds nothing an operator can act
        // on that `profile_id` does not, while a `google-oauth2|…` value joins our exported traces to
        // the same person in unrelated systems (Auth0's social-connection `user_id` embeds the Google
        // account id). A `profile_id` is inert outside temper. Decided 2026-08-01; see the task's §5.
        tracing::debug!(profile_id = %authed.profile().id, "Profile resolved");

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

    // ── Tools (consolidated: 64 → 26) ─────────────────────────────────

    // ── Resources (unchanged) ──────────────────────────────────────────

    #[tool(
        description = "Create a new resource in the knowledge base. Optionally include markdown content for indexing and search. Context must already exist — use context_manage (action: create) first if needed. Use describe_schema (view: doc_types) to see available types."
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
        description = "List resources in the knowledge base. Filter by context and/or document type. Returns most recent first. The response is a page: `rows` plus `total` (all matching rows), `returned`, `truncated`, `limit` and `offset`. When `truncated` is true there are matching rows you have not seen — do not conclude a resource is absent, or a set complete, from a truncated page; raise `limit`, page with `offset`, or narrow the filters. Each row carries a decorated `ref` (`slug-<uuid>`) you can pass straight back to any tool that takes one."
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

    // ── Search & Query (unchanged) ─────────────────────────────────────

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
        description = "Run a composed query — a DAG of act invocations (find-exact, find-about-anywhere, find-about-within, follow-from, find-resources-with) and set combinations (union, intersect, difference), with per-stage filters and property predicates. The `plan` object IS the composition contract; its schema describes every stage, act, filter, and combinator. A refused plan returns every refusal at once — each names its stage and reason — so the plan can be repaired in one round trip. Set `trace: false` to omit the per-stage trace and receive only the returned arms."
    )]
    async fn run_query(
        &self,
        Parameters(input): Parameters<tools::query::QueryInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::query::run_query(self, input).await
    }

    // ── Trail (unchanged) ──────────────────────────────────────────────

    #[tool(
        description = "Read the event trail (append-only history) of a graph element — a resource (node) or a relationship (edge). A time-ordered list of the events that produced and mutated it: created, updated, relationship asserted/folded, facets set, etc. Each event carries its actor, time, and replay-sufficient payload. An unreadable or nonexistent element returns an empty trail, never an error. Pass `kind` (node | edge) and `element` (a resource ref for a node, an edge UUID for an edge; the decorated `slug-<uuid>` form is accepted and the slug half ignored)."
    )]
    async fn element_trail(
        &self,
        Parameters(input): Parameters<tools::trail::ElementTrailInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::trail::element_trail(self, input).await
    }

    // ── Relationship (consolidated 4→1 write) ──────────────────────────

    #[tool(
        description = "Manage relationships (graph edges) with an `action` discriminator. Actions: `assert` (create a directed relationship from source to target — requires source, target, edge_kind, polarity, label, weight), `retype` (change edge_kind and polarity — requires edge_handle, edge_kind, polarity), `reweight` (change weight — requires edge_handle, weight), `fold` (retract/mark inactive — requires edge_handle, optional reason). The edge_handle comes from the assert response. Per-act authorship fields (confidence, reasoning, invocation_id, etc.) are accepted on all actions."
    )]
    async fn relationship(
        &self,
        Parameters(input): Parameters<tools::relationships::RelationshipInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::relationships::relationship(self, input).await
    }

    // ── Citation audit (unchanged write) ───────────────────────────────

    #[tool(
        description = "Record an auditor's signed defensibility verdict on one (block, source) citation of a finding. The value spans [-1.0, 1.0]: assess how much this source supports the specific connection the citation claims — never whether the underlying claim is true, and never what the source itself says. A strongly negative value expresses that the source does not carry the connection made here, without asserting what the source does say; a positive value reinforces the citation. Only Resource-kind sources are auditable. Append-only: a later verdict never erases an earlier one."
    )]
    async fn record_citation_audit(
        &self,
        Parameters(input): Parameters<tools::citation_audits::RecordCitationAuditInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::citation_audits::record_citation_audit(self, input).await
    }

    // ── Facets (consolidated: 2→1 read, 2→1 write) ─────────────────────

    #[tool(
        description = "Read the live facets of a resource or a relationship (edge) — one entry per assert, each with its weight and author. Set `target` to `resource` (requires `resource` ref) or `edge` (requires `edge_handle`). Use this to confirm a facet_set landed: get_resource collapses facets into a single newest-wins value in open_meta and drops the weight."
    )]
    async fn facets_read(
        &self,
        Parameters(input): Parameters<tools::facets::FacetsReadInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::facets::facets_read(self, input).await
    }

    #[tool(
        description = "Set a facet (typed property) on a resource or a relationship (edge). Set `target` to `resource` (requires `resource` ref) or `edge` (requires `edge_handle`). The facet's typed value payload goes in `values`; optional `weight` (0.0-1.0, defaults to 1.0). Per-act authorship fields accepted."
    )]
    async fn facet_set(
        &self,
        Parameters(input): Parameters<tools::facets::FacetSetUnifiedInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::facets::facet_set_unified(self, input).await
    }

    // ── Cogmap reads (consolidated 6→1) + list + create + materialize ─

    #[tool(
        description = "Read a cognitive map with a `view` discriminator. Views: `show` (orient on one map — identity, charter, foundational resources), `shape` (materialized regions, most salient first), `metrics` (per-region analytics: centrality, cohesion, tension, reference standing, telos alignment), `analytics` (map-level: telos charter, staleness, regulation concepts), `charter` (telos/charter blocks — statement, questions, framing), `materialize_delta` (how many formation events since last materialize, whether threshold is cleared). Pass the map by ref (`cogmap`); `lens` narrows `shape`/`metrics`; `threshold` gates `materialize_delta`."
    )]
    async fn cogmap_read(
        &self,
        Parameters(input): Parameters<tools::cognitive_maps::CogmapReadInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::cognitive_maps::cogmap_read(self, input).await
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
        description = "Create (genesis) a new cognitive map: a cogmap plus its telos charter resource. Open to any authenticated profile; the creator is granted read+write+grant on the new map, and a caller-supplied cogmap_id is honored only for a system-admin. The map is born with an EMPTY charter — author the charter and deliver it afterwards with `temper cogmap reconcile` (which embeds client-side). Idempotent at a supplied cogmap_id (re-creating is a no-op)."
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

    // ── Context (consolidated 4→1 read, 5→1 write) ────────────────────

    #[tool(
        description = "Read a context with a `view` discriminator. Views: `list` (all contexts available to you), `get` (one context by UUID — requires `id`), `shape` (materialized regions, most salient first — the fastest orientation move; requires `context` ref), `metrics` (per-region analytics: centrality, cohesion, tension, reference standing, telos alignment; requires `context` ref). The `context` field takes a context ref (`@me/<slug>`, `+<team>/<slug>`, or UUID); `lens` narrows `shape`/`metrics`."
    )]
    async fn context_read(
        &self,
        Parameters(input): Parameters<tools::contexts::ContextReadInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::contexts::context_read(self, input).await
    }

    #[tool(
        description = "Manage contexts with an `action` discriminator. Actions: `create` (new context — requires `name`, optional `owner`), `rename` (change display name, re-addresses the context — requires `context` UUID, `name`), `share` (share into a team's read-reach — requires `context` UUID, `team` UUID), `unshare` (remove a team's read-reach — requires `context` UUID, `team` UUID), `transfer` (transfer ownership to a team — requires `context` UUID, `team` UUID). Share/unshare/transfer require system-admin, OR that you administer the context AND manage the target team (owner/maintainer). Rename re-addresses: the old slug stops resolving."
    )]
    async fn context_manage(
        &self,
        Parameters(input): Parameters<tools::contexts::ContextManageInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::contexts::context_manage(self, input).await
    }

    // ── Schema (consolidated 3→1 read) ─────────────────────────────────

    #[tool(
        description = "Describe schema with a `view` discriminator. Views: `doc_types` (list all available document types with schema summaries), `doc_type` (describe one type in detail — full JSON schema, required fields, enum values, example managed_meta; requires `name`), `open_meta` (the recognized open_meta conventions — recognized keys, their shapes, which are FTS-indexed)."
    )]
    async fn describe_schema(
        &self,
        Parameters(input): Parameters<tools::doc_types::DescribeSchemaInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::doc_types::describe_schema(self, input).await
    }

    // ── Invocation (consolidated 2→1 read, 2→1 write) ─────────────────

    #[tool(
        description = "Read agent-invocation envelopes with a `view` discriminator. Views: `show` (one envelope plus its acts by UUID — requires `invocation` ref), `list` (list envelopes, optionally narrowed by `cogmap` ref and/or `status`: open/completed/failed/abandoned)."
    )]
    async fn invocation_read(
        &self,
        Parameters(input): Parameters<tools::invocations::InvocationReadInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::invocations::invocation_read(self, input).await
    }

    #[tool(
        description = "Manage agent-invocation envelopes with an `action` discriminator. Actions: `open` (start an accountability envelope for one agent run against a cognitive map — requires `trigger_kind`, `originating_cogmap` ref; optional `parent_cogmap`; returns the server-minted invocation_id), `close` (terminate an open envelope — requires `invocation` ref, `disposition`: completed/failed/abandoned; optional `outcome`)."
    )]
    async fn invocation_manage(
        &self,
        Parameters(input): Parameters<tools::invocations::InvocationManageInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::invocations::invocation_manage(self, input).await
    }

    // ── Segmented ingest (consolidated 4→1 write) ──────────────────────

    #[tool(
        description = "Segmented (multi-block) ingest for bodies too large to send in one call, with an `action` discriminator. Actions: `begin` (land segment 0 and create the resource — requires flattened `create` fields, `content_hash`; optional `block_budget`, `total_blocks_hint`, `source_hash`; returns resource_id, landed block set, opaque body_hash), `append` (land segment N — requires `resource`, `seq` (starts at 1), `content`, `content_hash`; optional `sources`; idempotent re-append is a safe no-op), `finalize` (declare complete — requires `resource`, `expected_blocks`, `expected_body_hash` echoed verbatim), `blocks` (read landed segments for resume — requires `resource`). Prefer create_resource for anything that fits a single call."
    )]
    async fn segmented_ingest(
        &self,
        Parameters(input): Parameters<tools::ingest::SegmentedIngestInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::ingest::segmented_ingest(self, input).await
    }

    // ── Steward (unchanged, scoped descriptions) ───────────────────────

    #[tool(
        description = "This tool is for the team-self-cognition steward agent. If you are not running a steward cycle, you do not need this tool. Read a team-self-cognition cogmap's ingest delta: how many new resources + events have landed in the team's contexts since the steward's watermark, and whether that clears the threshold (i.e. the steward should run)."
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
        description = "This tool is for the team-self-cognition steward agent. If you are not running a steward cycle, you do not need this tool. Advance a team-self-cognition cogmap's ingest watermark to a given event id — the cursor a completed steward run records so the next delta counts only newer material. Requires cogmap-write."
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
        description = "List data artifacts owned by a resource. Each artifact is structured data committed by an agent session, retrieved whole. Filter by kind (the bare family name) or intent (current/member/pinned). Set include_folded to include superseded artifacts. Visibility-gated: you only see artifacts whose owning resource you can read."
    )]
    async fn list_data_artifacts(
        &self,
        Parameters(input): Parameters<tools::data_artifacts::ListArtifactsInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::data_artifacts::list_artifacts(self, input).await
    }

    #[tool(
        description = "Get a single data artifact by its ID. Returns the full artifact with content payload. Visibility-gated: returns 'not found' if the owning resource is not visible to you."
    )]
    async fn get_data_artifact(
        &self,
        Parameters(input): Parameters<tools::data_artifacts::GetArtifactInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::data_artifacts::get_artifact(self, input).await
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

            // Carry `profile_id` only — never the raw OAuth `sub`. This event was the specific one
            // that surfaced the decision: it exported `profile_id` *and* `sub: google-oauth2|…`, and
            // the `sub` was read by an LLM into a chat transcript during triage. The profile has
            // resolved here, so `sub` is redundant for attribution and its only remaining effect is
            // cross-boundary linkability. Decided 2026-08-01; see the task's §5.
            tracing::info!(
                profile_id = %authed.profile().id,
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
    /// Assert the router actually carries the consolidated segmented-ingest tool, rather than
    /// inferring it from "it compiled". Needs no database — `tool_router()` is a pure
    /// associated function.
    #[test]
    fn the_segmented_ingest_tool_is_advertised_by_the_router() {
        let router = TemperMcpService::tool_router();
        let names: Vec<String> = router
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();

        assert!(
            names.iter().any(|n| n == "segmented_ingest"),
            "segmented_ingest is not advertised; router has {names:?}"
        );
    }

    /// Same failure mode as above, for the context manage tool: `rename_context` is now one
    /// action under the consolidated `context_manage` tool. If the consolidation drops the
    /// `rename` action, the one context act an agent cannot perform is silently missing.
    #[test]
    fn context_manage_is_advertised_by_the_router() {
        let names: Vec<String> = TemperMcpService::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();

        assert!(
            names.iter().any(|n| n == "context_manage"),
            "context_manage is not advertised; router has {names:?}"
        );
    }

    /// The MCP `search` tool takes the same type the API door does — which is what lets one test
    /// speak for two doors.
    ///
    /// `door_coverage`'s term axis is checked for the API and MCP doors by a single assertion in
    /// temper-core (`act_door_coverage_reachability.rs`), against `SearchParams`' wire slots. That
    /// check is only legitimate while both doors really take `SearchParams`: `temper-api`'s handler
    /// takes `Json<SearchParams>` and this one takes `Parameters<SearchParams>`. If MCP's tool ever
    /// took a wrapper, a subset, or a hand-rolled twin, that test would keep passing while silently
    /// describing a door it no longer reads — the same shape as a declaration checked against its
    /// own literal.
    ///
    /// Asserted against the ROUTER's advertised schema rather than against the handler signature,
    /// because the schema is what a caller standing at this door actually sees. The sibling table in
    /// `tests/steward_skill_recipe_test.rs` pairs `"search"` with `schema_for!(SearchParams)` too,
    /// but that table is hand-written and validates the skill doc — it could drift from the router
    /// without anything noticing, which is the drift this closes.
    ///
    /// **rmcp 1.8 strips the top-level `title` and `description`** from the advertised input
    /// schema (`schema_for_input` → `validate_and_strip` in `rmcp/handler/server/common.rs`),
    /// deliberately, because the wrapper type name ("SearchParams") and its doc comment are noise
    /// to the LLM. That is a presentation concern of the SDK, not a change in *which type* this
    /// door declares — so the expected schema is stripped the same way before comparing. A real
    /// drift (a wrapper, a subset, a hand-rolled twin) still fails this test: the `properties` map
    /// and `type` are unaffected by the strip.
    #[test]
    fn the_search_tool_advertises_exactly_the_shared_search_params_schema() {
        let advertised = TemperMcpService::tool_router()
            .list_all()
            .into_iter()
            .find(|t| t.name == "search")
            .expect("the router advertises a `search` tool")
            .input_schema;

        let mut shared =
            serde_json::to_value(schemars::schema_for!(temper_core::types::api::SearchParams))
                .expect("SearchParams schema serializes");
        // Mirror rmcp 1.8's `validate_and_strip`: drop the top-level wrapper-type metadata so
        // the comparison is against the schema the door actually advertises, not the raw
        // schemars output. rmcp strips these because the type name and doc are noise to the LLM.
        if let Some(obj) = shared.as_object_mut() {
            obj.remove("title");
            obj.remove("description");
        }

        assert_eq!(
            serde_json::to_value(&*advertised).expect("advertised schema serializes"),
            shared,
            "the MCP search tool no longer advertises `SearchParams`. temper-core's \
             `the_shared_params_doors_declare_exactly_the_terms_that_type_carries` checks the API \
             and MCP doors together on the premise that they share this type — fix that test's \
             reach before changing this one."
        );
    }

    /// **Every tool the shipped MCP skill tells an agent to call must actually exist.**
    ///
    /// This closes the half of the skill-drift gate that gate cannot reach. That gate re-emits the
    /// generated files and diffs them, so it proves the tree matches its source — it says nothing
    /// about whether the source is *true*, and it does not look at `knowledge-base.md` at all,
    /// which is hand-written. The skill shipped for months naming a `list_events` tool this server
    /// has never exposed; an agent following it burns a turn on a call that cannot succeed.
    ///
    /// It lives here rather than in temper-cli because the router is the authority and temper-cli
    /// does not depend on temper-mcp. `tool_router()` is a pure associated function — no database.
    ///
    /// **What it covers:** every name in a `Tool:` / `Tools:` position — i.e. every worked
    /// invocation example. That is where a wrong name actually costs an agent a failed call, and it
    /// is exactly where `list_events` was.
    ///
    /// **Declared remainder:** a tool named only in running prose or a bullet (`- `list_resources`
    /// — paginated list`) is not checked. Distinguishing those from the many backticked *field*
    /// names (`context_ref`, `open_meta`, `expected_blocks`, …) would need a hand-maintained
    /// denylist, and a hand-maintained list of what-not-to-check is the same rot one level down.
    #[test]
    fn every_tool_the_shipped_skill_names_exists_in_the_router() {
        use std::collections::BTreeSet;

        let skill_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../agent-skills/temper-knowledge-base");

        // Walk one level plus `references/`; the bundle is deliberately shallow.
        let mut docs: Vec<(String, String)> = Vec::new();
        let mut dirs = vec![skill_dir.clone()];
        while let Some(dir) = dirs.pop() {
            let entries = std::fs::read_dir(&dir)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "md") {
                    let body = std::fs::read_to_string(&path).expect("read skill doc");
                    docs.push((path.display().to_string(), body));
                }
            }
        }

        assert!(
            !docs.is_empty(),
            "no markdown found under {} — this test would pass having checked nothing",
            skill_dir.display()
        );

        let advertised: BTreeSet<String> = TemperMcpService::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();

        // Names in a `Tool:` / `Tools:` position. A line may carry several
        // (`Tools: `ingest_begin` → `ingest_append``), so take every backticked or bare
        // snake_case token up to the end of the segment.
        let mut checked = 0usize;
        let mut missing: Vec<String> = Vec::new();
        for (path, body) in &docs {
            for line in body.lines() {
                let Some(idx) = line.find("Tool:").or_else(|| line.find("Tools:")) else {
                    continue;
                };
                let tail = &line[idx..];
                // Cut at the first boundary that ends the tool-name region. `Input:` matters most:
                // the worked examples put the payload on the SAME line, and without this every
                // field name (`doc_type_name`, `context_ref`, …) reads as a tool. `|` ends a
                // table cell so a row's prose column is never scanned.
                let end = ["Input:", "|"]
                    .iter()
                    .filter_map(|b| tail.find(b))
                    .min()
                    .unwrap_or(tail.len());
                let segment = &tail[..end];
                let after_anchor = segment.split_once(':').map_or("", |(_, rest)| rest);

                // Backticked names first — a line may carry several
                // (``Tools: `ingest_begin` → `ingest_append` ``). Falling back to the first bare
                // token covers the unbackticked `Tool: list_resources` form. Taking every bare
                // token instead would scoop up prose.
                let ticked: Vec<&str> = segment
                    .split('`')
                    .skip(1)
                    .step_by(2)
                    .map(str::trim)
                    .collect();
                let candidates: Vec<&str> = if ticked.is_empty() {
                    after_anchor.split_whitespace().take(1).collect()
                } else {
                    ticked
                };

                for token in candidates.into_iter().filter(|t| {
                    t.len() > 2 && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                }) {
                    checked += 1;
                    if !advertised.contains(token) {
                        missing.push(format!("{path}: `{token}`"));
                    }
                }
            }
        }

        // A rewording that removes the `Tool:` anchor would silently reduce this test to nothing.
        // Refuse instead — a check that cannot fail reads as coverage.
        assert!(
            checked >= 10,
            "only {checked} tool references found across {} skill docs; the `Tool:` anchor this \
             test extracts on has probably been reworded, leaving it checking nothing",
            docs.len()
        );

        assert!(
            missing.is_empty(),
            "the shipped MCP skill names {} tool(s) this server does not advertise:\n  {}\n\
             Advertised tools: {:?}",
            missing.len(),
            missing.join("\n  "),
            advertised
        );
    }

    /// **Every `#[tool]` method must call `ensure_profile_from_parts` before dispatching.**
    ///
    /// The MCP surface authenticates per-request from the JWT claims injected into the HTTP
    /// extensions. A `#[tool]` method that skips `ensure_profile_from_parts` compiles fine and
    /// is advertised by the router — it just runs unauthenticated, silently. The consolidation
    /// that restructured every tool body could have dropped the call in any one of them; this
    /// gate catches that.
    ///
    /// It parses this file's own source rather than reflecting on the router, because the
    /// router's `Tool` entries carry only the description + schema + a function pointer — there
    /// is no way to inspect the function body at runtime to see whether it calls
    /// `ensure_profile_from_parts`. Source parsing is the cheapest faithful check.
    ///
    /// **What it covers:** every `#[tool]` method body in this file. The split is on
    /// `#[tool(`, and each segment runs from the attribute to the next `#[tool(` or end of
    /// file — so a method that calls `ensure_profile_from_parts` anywhere in its body passes.
    /// A method that calls it conditionally (inside an `if`) would also pass; the invariant is
    /// "the call is present", not "the call is unconditional", and every existing call IS
    /// unconditional (the first line of every method body).
    #[test]
    fn every_tool_method_calls_ensure_profile_from_parts() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/service.rs"),
        )
        .expect("read service.rs");

        // Parse only the tool-router impl block — the #[tool] methods live between
        // `#[tool_router]` and `#[tool_handler]` (or `#[cfg(test)]`, whichever comes first).
        // The test module's doc comments mention `#[tool(` in backticks, which would split
        // as false segments; scoping to the impl block avoids that.
        let impl_start = source
            .find("#[tool_router]")
            .expect("#[tool_router] attribute not found");
        let impl_end = source[impl_start..]
            .find("#[tool_handler]")
            .or_else(|| source[impl_start..].find("#[cfg(test)]"))
            .expect("end of tool-router impl not found");
        let impl_block = &source[impl_start..impl_start + impl_end];

        let tool_segments: Vec<&str> = impl_block.split("#[tool(").skip(1).collect();

        assert!(
            !tool_segments.is_empty(),
            "no #[tool] attributes found in the tool-router impl — the split found nothing, \
             so this test checks nothing"
        );

        let mut missing: Vec<String> = Vec::new();
        for segment in &tool_segments {
            if !segment.contains("ensure_profile_from_parts") {
                let fn_name = segment
                    .split("async fn ")
                    .nth(1)
                    .and_then(|s| s.split('(').next())
                    .unwrap_or("<unknown>")
                    .trim();
                missing.push(fn_name.to_string());
            }
        }

        assert!(
            missing.is_empty(),
            "these #[tool] methods do not call ensure_profile_from_parts — every tool must \
             authenticate before dispatching:\n  {}\n\
             The call is the first line of every existing tool body; a new tool that omits it \
             runs unauthenticated.",
            missing.join("\n  ")
        );
    }
}
