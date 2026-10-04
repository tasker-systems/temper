//! MCP service — the central handler for all MCP tool calls.
//!
//! Each invocation creates a fresh `TemperMcpService`. The service runs no auth seam
//! of its own: the host's JWT edge (`temper-mcp-server`'s `require_mcp_auth`) validates the token before anything
//! forwards, and every tool relays the caller's bearer across the network door, where
//! Level 1 (resolve, deactivation) and Level 2 (system access) run at the API.
//! Post-edge refusals come back as preserved 401/403 bodies and are mapped
//! arm-for-arm (`AcrossAuth`, `map_post_edge_refusal`).
//!
//! The service carries NO auth state: the identity a request acts under exists only
//! as a per-request value. Every tool handler extracts the HTTP `Parts` from rmcp's
//! `Extension` and hands them to its tools module, which builds a per-request relay
//! client from the bearer in them. The last in-process gate (`ensure_profile_from_parts`,
//! kept for the context-ref resolution) left at teardown, when `context_anchor` began
//! relaying to `GET /api/contexts/resolve`; the source gates (`source_gates.rs`)
//! keep it from coming back.

use rmcp::{
    handler::server::{common::Extension, wrapper::Parameters},
    model::{
        CallToolResult, ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult,
        ListToolsResult, PaginatedRequestParams, ProtocolVersion, ReadResourceRequestParams,
        ReadResourceResponse, ServerCapabilities, ServerConfig,
    },
    tool, tool_handler, tool_router,
};
use std::borrow::Cow;
use std::sync::Arc;

use temper_client::auth::MemoryTokenStore;
use temper_client::error::ClientError;
use temper_client::TemperClient;

use crate::host::BlobDoor;
use crate::seam::{not_connected, IdentitySeam, OutgoingIdentity, RelayConfig};
use crate::tools;

/// Total attempts for a non-idempotent (unkeyed) tool act. The ruled "1 retry on
/// non-idempotent tool acts" (design §2.1/§11.6): a cold-start 500 on the API function is
/// the hop's common transient, and the CALLER's own redrive has the same double-apply
/// property a retry would, so the single retry adds no new replay surface while sparing a
/// round trip. Safe/idempotent-keyed requests keep temper-client's stock budget.
const RELAY_NON_IDEMPOTENT_ATTEMPTS: u32 = 2;

/// Headers the relay client sets itself — the identity it re-issues (`authorization`, the
/// surface, the device id), the trace context, and the transport's own. A host's extra headers
/// may not restate them.
const RESERVED_HEADERS: &[&str] = &[
    "authorization",
    "x-temper-surface",
    "x-temper-device-id",
    "traceparent",
    "tracestate",
    "host",
    "content-length",
    "content-type",
];

/// The identity `call_tool` resolved for one call, carried to `relay_client` in the request's
/// parts. Crate-private: a host or caller cannot plant one — only the dispatch gate does.
#[derive(Clone)]
struct ResolvedIdentity(OutgoingIdentity);

/// How this service relays: through a host's seam, or not at all.
#[derive(Clone)]
enum Door {
    /// The relay is configured: the API's base URL, the ONE connection pool (built once, in
    /// [`TemperMcpService::new`], and refcount-cloned into every per-request client), and the
    /// host's seam.
    Relay {
        api_base_url: Arc<str>,
        connections: reqwest::Client,
        seam: Arc<dyn IdentitySeam>,
    },
    /// The host cannot relay at all — it says why, in its own words, and every relayed act
    /// answers that sentence (a dark tool door beats a silently mis-attributed one).
    Unavailable(Arc<str>),
}

/// Central MCP service.
///
/// **Build it once per process and clone it per request.** Construction builds the relay's
/// connection pool; a clone shares it. Temper's deployed host runs the streamable-HTTP transport
/// in stateless mode, whose factory is called once per HTTP request — so the factory clones a
/// service built at router assembly, and no tool call pays a fresh TLS handshake.
///
/// The `ToolRouter` is **not** stored as a field. Under rmcp ≥ 1.4 the `#[tool_handler]` macro
/// defaults to `Self::tool_router()` — rebuilding the router per call — so a stored field would
/// be dead weight. `ToolRouter<Self>` is imported only because the `#[tool_router]` macro
/// references it in its generated associated function; no value of that type lives here.
///
/// The service carries NO auth state: who a request acts as is answered per request by the
/// host's [`IdentitySeam`], and exists only as that request's client.
#[derive(Clone)]
pub struct TemperMcpService {
    /// The deployment's blob posture, a plain value the host read at boot — the only
    /// per-process fact a tool consults besides the relay.
    pub blob_door: BlobDoor,
    door: Door,
}

impl std::fmt::Debug for TemperMcpService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let door = match &self.door {
            Door::Relay { .. } => "relay",
            Door::Unavailable(_) => "unavailable",
        };
        f.debug_struct("TemperMcpService")
            .field("blob_door", &self.blob_door)
            .field("door", &door)
            .finish_non_exhaustive()
    }
}

#[tool_router]
impl TemperMcpService {
    /// A service that relays every tool act to `relay.api_base_url`, as whoever `seam` says
    /// each request acts as. Builds the relay's connection pool — call once, clone per request.
    pub fn new(blob_door: BlobDoor, relay: RelayConfig, seam: Arc<dyn IdentitySeam>) -> Self {
        Self {
            blob_door,
            door: Door::Relay {
                connections: relay.connection_pool(),
                api_base_url: relay.api_base_url.into(),
                seam,
            },
        }
    }

    /// A service whose host cannot relay: every relayed act answers `refusal`, the host's own
    /// sentence (the crate never words a host's misconfiguration). The tool list is unchanged.
    pub fn unavailable(blob_door: BlobDoor, refusal: impl Into<String>) -> Self {
        Self {
            blob_door,
            door: Door::Unavailable(refusal.into().into()),
        }
    }

    /// The identity this request acts as, from the host's seam — or the refusal a request
    /// without one answers. `Ok(None)` only on an unavailable door, whose refusal the relay
    /// itself speaks.
    fn outgoing_identity(
        &self,
        parts: &http::request::Parts,
    ) -> Result<Option<OutgoingIdentity>, rmcp::ErrorData> {
        match &self.door {
            Door::Relay { seam, .. } => seam
                .outgoing_identity(parts)
                .map(Some)
                .ok_or_else(not_connected),
            Door::Unavailable(_) => Ok(None),
        }
    }

    /// Build the per-request relay client — the ONE place any host's client is built.
    ///
    /// The identity's bearer is re-issued by THIS process (never inbound header bytes copied),
    /// the host's extra headers ride as constructor-level default headers so they cross retries
    /// in one place, the shared pool sits underneath, and the ruled transport figures apply
    /// (§2.1/§11.6). `device_id` is pinned absent — a relay is not a device.
    ///
    /// An unavailable door answers its host's sentence; a seam that yields no identity answers
    /// the crate's not-connected refusal.
    pub fn relay_client(
        &self,
        parts: &http::request::Parts,
    ) -> Result<TemperClient, rmcp::ErrorData> {
        let (api_base_url, connections, seam) = match &self.door {
            Door::Relay {
                api_base_url,
                connections,
                seam,
            } => (api_base_url, connections, seam),
            Door::Unavailable(refusal) => {
                return Err(rmcp::ErrorData::internal_error(refusal.to_string(), None))
            }
        };
        // The identity `call_tool` resolved for this call, when it did — so the client is built
        // from the identity the dispatch gate checked and the correlation fill used, never from a
        // second consultation that might answer differently. Otherwise (the resources protocol, a
        // direct call) the seam is consulted here.
        let identity = match parts.extensions.get::<ResolvedIdentity>() {
            Some(ResolvedIdentity(identity)) => identity.clone(),
            None => seam.outgoing_identity(parts).ok_or_else(not_connected)?,
        };
        let (bearer, surface, extra_headers) = identity.into_parts();
        if let Some(name) = extra_headers
            .keys()
            .find(|name| RESERVED_HEADERS.contains(&name.as_str()))
        {
            // Fail closed: a default header that restates identity would ride BESIDE the one
            // the client sets (reqwest appends), and which copy a reader honours is not ours to
            // leave to chance.
            return Err(rmcp::ErrorData::internal_error(
                format!(
                    "This host's identity carries the reserved header `{name}`, which the relay \
                     sets itself; refusing to forward."
                ),
                None,
            ));
        }

        TemperClient::with_token(
            api_base_url,
            None,
            surface,
            bearer,
            Arc::new(MemoryTokenStore::empty()),
        )
        .map_err(|e| rmcp::ErrorData::internal_error(format!("relay client: {e}"), None))
        .map(|client| {
            client
                .with_default_headers(extra_headers)
                .with_non_idempotent_attempts(RELAY_NON_IDEMPOTENT_ATTEMPTS)
                .with_connection_pool(connections.clone())
        })
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
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::resources::create_resource(self, &parts, input).await
    }

    #[tool(
        description = "Get a resource by its ref (UUID or the decorated `slug-<uuid>` form). Set include_content to true to get the full markdown body."
    )]
    async fn get_resource(
        &self,
        Parameters(input): Parameters<tools::resources::GetResourceInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::resources::get_resource(self, &parts, input).await
    }

    #[tool(
        description = "Trace a resource's derived_from lineage in both directions: `ancestors` (what it derives from) and `descendants` (what derives from it), each a transitive, access-gated walk. Every node carries the reaching edge and whether that edge is folded — a folded ancestor is shown, flagged, so you can see when what you rest on has been superseded. Optional `depth` bounds the walk (default 16)."
    )]
    async fn resource_lineage(
        &self,
        Parameters(input): Parameters<tools::resources::ResourceLineageInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::resources::resource_lineage(self, &parts, input).await
    }

    #[tool(
        description = "Get the itemized block-provenance for a resource — the sources each of its content blocks was distilled from, in (block, accretion) order. Access-scoped: an unreadable resource returns an empty list."
    )]
    async fn get_block_provenance(
        &self,
        Parameters(input): Parameters<tools::resources::GetBlockProvenanceInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::resources::get_block_provenance(self, &parts, input).await
    }

    #[tool(
        description = "Read one content block by address — the three-state resolution. `state` is always named: `live` returns the block's identity, its chunks' identities (structure and hashes, never prose), and its provenance rows; `folded` means a re-partition folded it away and returns its persisted attribution history plus `disposition` — where the content went (`located` names the absorber/carried successor blocks, only those you can read, each with its `home_resource_id`), `content_gone`, or `unrecorded` (the ledger does not carry the mapping). A non-existent address answers `state: \"absent\"`; only a block whose HOME RESOURCE you cannot see is a not-found error. Address a folded successor by passing its `home_resource_id` as `resource` and its `block_id` as `block_id`."
    )]
    async fn get_block(
        &self,
        Parameters(input): Parameters<tools::resources::GetBlockInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::resources::get_block(self, &parts, input).await
    }

    #[tool(
        description = "List resources in the knowledge base. Filter by context and/or document type. Returns most recent first. The response is a page: `rows` plus `total` (all matching rows), `returned`, `truncated`, `limit` and `offset`. When `truncated` is true there are matching rows you have not seen — do not conclude a resource is absent, or a set complete, from a truncated page; raise `limit`, page with `offset`, or narrow the filters. Each row carries a decorated `ref` (`slug-<uuid>`) you can pass straight back to any tool that takes one."
    )]
    async fn list_resources(
        &self,
        Parameters(input): Parameters<tools::resources::ListResourcesInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::resources::list_resources(self, &parts, input).await
    }

    #[tool(
        description = "Update a resource's title, slug, or content. Only provided fields are changed. New content triggers re-indexing."
    )]
    async fn update_resource(
        &self,
        Parameters(input): Parameters<tools::resources::UpdateResourceInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::resources::update_resource(self, &parts, input).await
    }

    #[tool(
        description = "Attach provenance sources to a resource's block WITHOUT a body revise (no re-chunk/re-embed) — the cheap, citation-grade backfill for a corpus imported without sources. Records block-provenance rows on the addressed block; body and embeddings are unchanged. A source URL may carry a span-locator fragment (e.g. '…/doc.md#L120-L180'), preserved verbatim and surfaced by get_block_provenance. Returns the resulting per-block provenance."
    )]
    async fn annotate_resource(
        &self,
        Parameters(input): Parameters<tools::resources::AnnotateResourceInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::resources::annotate_resource(self, &parts, input).await
    }

    #[tool(
        description = "Update a resource's frontmatter (managed_meta and open_meta) without re-chunking or re-embedding. Use for metadata-only edits like stage, tags, or relationship declarations. For content changes, use update_resource."
    )]
    async fn update_resource_meta(
        &self,
        Parameters(input): Parameters<tools::resources::UpdateResourceMetaInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::resources::update_resource_meta(self, &parts, input).await
    }

    #[tool(
        description = "Soft-delete a resource by ID. The resource is deactivated, not permanently removed."
    )]
    async fn delete_resource(
        &self,
        Parameters(input): Parameters<tools::resources::DeleteResourceInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::resources::delete_resource(self, &parts, input).await
    }

    #[tool(
        description = "Run one bounded, resumable re-blocking step: re-block resources under the current chunking policy — each candidate's stored body is recomposed and the stored chunking replaced. Survey first: call with dry_run true to classify every candidate without changing anything, then run with dry_run false to act, then survey again to verify. Exactly one scope per call: scope `resource` plus a resource ref re-blocks that one resource; scope `context` plus a ref to a context you can read walks every candidate homed in that context; scope `all` covers the whole deployment and requires system-administrator standing. limit bounds how many candidates one call considers (a conservative default applies when omitted); after_id resumes a walk from the previous receipt's cursor. The response is the receipt: one outcome row per candidate (reblocked, no-op, would_change on a survey, denied, in_progress, byteless, or drift each with a typed reason, or error), per-class counts, a batch correlation id, and the continuation cursor. A candidate already conforming to the current policy is a no-op — it fires nothing, so re-running a completed step changes nothing."
    )]
    async fn resource_reblock(
        &self,
        Parameters(input): Parameters<tools::reblock::ResourceReblockInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::reblock::resource_reblock(self, &parts, input).await
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
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::search::search(self, &parts, input).await
    }

    #[tool(
        description = "Run a composed query — a DAG of act invocations (find-exact, find-about-anywhere, find-about-within, follow-from, find-resources-with) and set combinations (union, intersect, difference), with per-stage filters and property predicates. The `plan` object IS the composition contract; its schema describes every stage, act, filter, and combinator. A refused plan returns every refusal at once — each names its stage and reason — so the plan can be repaired in one round trip. Set `trace: false` to omit the per-stage trace and receive only the returned arms."
    )]
    async fn run_query(
        &self,
        Parameters(input): Parameters<tools::query::QueryInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::query::run_query(self, &parts, input).await
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
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::trail::element_trail(self, &parts, input).await
    }

    // ── Blob (read 2→1, manage 2→1) ────────────────────────────────────

    #[tool(
        description = "Read blob surfaces with an `action` discriminator. Actions: `read` (one blob's bytes back whole, base64, with its stored media type, byte count, and content hash — refused over the read ceiling, which names the streaming API/CLI surfaces), `list` (the blobs you can read, optionally scoped to one home anchor via `home_table`/`home_id` — the response is your readable set, never a discovery oracle). An invisible-or-absent blob renders the same not-found an unknown id gets."
    )]
    async fn blob_read(
        &self,
        Parameters(input): Parameters<tools::blobs::BlobReadInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::blobs::blob_read(self, &parts, input).await
    }

    #[tool(
        description = "Manage blobs with an `action` discriminator. Actions: `commit` (commit base64 `content` bytes as a blob homed in `home_table`/`home_id` under `content_type` — get-or-create PER-HOME on the content hash: a re-commit of bytes this home already holds returns the same id, always a row you can read; the same bytes in another principal's scope never surface here; the server allowlist-checks the media type and refuses over its single-request threshold, naming the CLI's segmented path), `relate` (assert one relation between `blob_id` and a `peer_table`/`peer_id` anchor — `direction` picks which end the blob occupies, default `blob_as_source`; retraction rides the incumbent fold endpoint, not this tool). Relations are only assertable to anchors you can already see. Per-act authorship fields accepted on `relate`."
    )]
    async fn blob_manage(
        &self,
        Parameters(input): Parameters<tools::blobs::BlobManageInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::blobs::blob_manage(self, &parts, input).await
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
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::relationships::relationship(self, &parts, input).await
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
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::citation_audits::record_citation_audit(self, &parts, input).await
    }

    // ── Facets (consolidated: 2→1 read, 2→1 write) ─────────────────────

    #[tool(
        description = "Read the live facets of a resource or a relationship (edge) — one entry per assert, each with its weight and author. Set `target` to `resource` (requires `resource` ref) or `edge` (requires `edge_handle`). Use this to confirm a facet_set landed: get_resource collapses facets into a single newest-wins value in open_meta and drops the weight. Rows keyed `anchored-at` additionally state how their address resolved (`live`, `folded` with the block read's gated disposition, or `absent`) and — for a live address on a `derived_from` edge, anchored source-side — whether the anchored block's own attribution `corroborated`, is `divergent`, or is `unattributed`."
    )]
    async fn facets_read(
        &self,
        Parameters(input): Parameters<tools::facets::FacetsReadInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::facets::facets_read(self, &parts, input).await
    }

    #[tool(
        description = "Set a facet (typed property) on a resource or a relationship (edge). Set `target` to `resource` (requires `resource` ref) or `edge` (requires `edge_handle`). The facet's typed value payload goes in `values`; optional `weight` (0.0-1.0, defaults to 1.0). Per-act authorship fields accepted. On `target=edge`, optional `property_key` asserts `values` as ONE row under that key instead of the clustering facet verb — for `anchored-at` (the span qualification) the value is exactly `{\"endpoint\": \"source\"|\"target\", \"address\": \"<resource-uuid>#<block-uuid>\"}`: the address must be one canonical `<resource>#<block>` pair naming the named endpoint's own resource side; the row lands per (endpoint, block), a repeated assert of a live address acks the existing row, and the address is never probed for existence at write time. Other keys are refused."
    )]
    async fn facet_set(
        &self,
        Parameters(input): Parameters<tools::facets::FacetSetUnifiedInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::facets::facet_set_unified(self, &parts, input).await
    }

    #[tool(
        description = "Retract one facet row of a relationship (edge) by id — the correction verb when an assertion named the wrong span. Set `target` to `edge` (requires `edge_handle` and the `property_id` that facets_read returned). For an `anchored-at` row the retract frees its address: re-assert the same address with facet_set to mint a fresh row. A property_id naming another edge, an unknown one, and an already-retracted one answer identically — not found. `target=resource` is refused: a resource's facet rows have no stable ids, so overwrite them with facet_set instead."
    )]
    async fn facet_retract(
        &self,
        Parameters(input): Parameters<tools::facets::FacetRetractInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::facets::facet_retract(self, &parts, input).await
    }

    // ── Cogmap reads (consolidated 6→1) + list + create + materialize ─

    #[tool(
        description = "Read a cognitive map with a `view` discriminator. Views: `show` (orient on one map — identity, charter, foundational resources), `shape` (an OBJECT, not an array: `regions` most salient first, plus `population`, `emptiness` and `materialized_at`), `metrics` (per-region analytics: centrality, cohesion, tension, reference standing, telos alignment), `analytics` (map-level: telos charter, staleness, regulation concepts), `charter` (telos/charter blocks — statement, questions, framing), `materialize_delta` (how many formation events since last materialize, whether threshold is cleared). Pass the map by ref (`cogmap`); `lens` narrows `shape`/`metrics`; `threshold` gates `materialize_delta`. On `shape`, an empty `regions` is never mute — read `emptiness`: `never_clustered` (run cogmap_materialize), `nothing_visible` (it materialized but nothing came back — it may have formed no regions, or none may hold a member you can read; the two are one answer on purpose, so do NOT conclude you lack access), `lens_narrowed` (your `lens` excluded all `population` regions), or `unreadable_or_absent`. `population` is your all-lens region count and is >= `regions.len()`, equal when nothing was narrowed away."
    )]
    async fn cogmap_read(
        &self,
        Parameters(input): Parameters<tools::cognitive_maps::CogmapReadInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::cognitive_maps::cogmap_read(self, &parts, input).await
    }

    #[tool(
        description = "List the cognitive maps you can see, each with its id, name, held-by team scope, region/resource counts, and charter statement (what the map is for). The first move for orienting across maps — every row's id is addressable by the other cogmap tools. Optional name_contains narrows by name substring."
    )]
    async fn cogmap_list(
        &self,
        Parameters(input): Parameters<tools::cognitive_maps::CogmapListInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::cognitive_maps::cogmap_list(self, &parts, input).await
    }

    #[tool(
        description = "Create (genesis) a new cognitive map: a cogmap plus its telos charter resource. Open to any authenticated profile; the creator is granted read+write+grant on the new map, and a caller-supplied cogmap_id is honored only for a system-admin. The map is born with an EMPTY charter — author the charter and deliver it afterwards with `temper cogmap reconcile` (which embeds client-side). Idempotent at a supplied cogmap_id (re-creating is a no-op)."
    )]
    async fn cogmap_create(
        &self,
        Parameters(input): Parameters<tools::cognitive_maps::CogmapCreateInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::cognitive_maps::cogmap_create(self, &parts, input).await
    }

    #[tool(
        description = "Re-materialize a cognitive map's regions when its formation delta since the last materialize clears the threshold; a safe no-op below threshold (materialized: false). This is the substrate's deterministic region-formation cadence — not an authored act. Requires cogmap-write."
    )]
    async fn cogmap_materialize(
        &self,
        Parameters(input): Parameters<temper_core::types::materialize::MaterializeTriggerInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::cognitive_maps::cogmap_materialize(self, &parts, input).await
    }

    // ── Context (consolidated 5→1 read, 5→1 write) ────────────────────

    #[tool(
        description = "Read a context with a `view` discriminator. Views: `list` (all contexts available to you), `get` (one context by UUID — requires `id`), `shape` (the fastest orientation move; requires `context` ref — returns an OBJECT, not an array: `regions` most salient first, plus `population`, `emptiness` and `materialized_at`), `metrics` (per-region analytics: centrality, cohesion, tension, reference standing, telos alignment; requires `context` ref), `analytics` (context-level staleness: `materialized_at`, `latest_touch`, `is_stale` — three fields, NOT the five of `cogmap_read` view `analytics`, because a context has no charter resource and no regulation set; requires `context` ref). The `context` field takes a context ref (`@me/<slug>`, `+<team>/<slug>`, or UUID); `lens` narrows `shape`/`metrics`. On `shape`, an empty `regions` is never mute — read `emptiness`: `never_clustered` (it has never been materialized — nothing here is broken, the regions simply do not exist yet; run context_materialize), `nothing_visible` (it materialized but nothing came back — it may have formed no regions, or none may hold a resource you can read; the two are one answer on purpose, so do NOT conclude you lack access), `lens_narrowed` (your `lens` excluded all `population` regions), or `unreadable_or_absent`. `population` is your all-lens region count and is >= `regions.len()`, equal when nothing was narrowed away."
    )]
    async fn context_read(
        &self,
        Parameters(input): Parameters<tools::contexts::ContextReadInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::contexts::context_read(self, &parts, input).await
    }

    #[tool(
        description = "Manage contexts with an `action` discriminator. Actions: `create` (new context — requires `name`, optional `owner`), `rename` (change display name, re-addresses the context — requires `context` UUID, `name`), `share` (share into a team's read-reach — requires `context` UUID, `team` UUID), `unshare` (remove a team's read-reach — requires `context` UUID, `team` UUID), `transfer` (transfer ownership to a team — requires `context` UUID, `team` UUID). Share/unshare/transfer require system-admin, OR that you administer the context AND manage the target team (owner/maintainer). Rename re-addresses: the old slug stops resolving."
    )]
    async fn context_manage(
        &self,
        Parameters(input): Parameters<tools::contexts::ContextManageInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::contexts::context_manage(self, &parts, input).await
    }

    #[tool(
        description = "Re-materialize a CONTEXT's regions when its formation delta since the last materialize clears the threshold; a safe no-op below threshold (materialized: false). The context-addressed peer of cogmap_materialize — the substrate's deterministic region-formation cadence, not an authored act. Pass the context by ref (`@me/<slug>`, `+<team>/<slug>`, or UUID). Requires context-write: DIRECT membership with an authoring role."
    )]
    async fn context_materialize(
        &self,
        Parameters(input): Parameters<temper_core::types::materialize::ContextMaterializeInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::cognitive_maps::context_materialize(self, &parts, input).await
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
        // NOT the network door — there is no binding behind it: every view is pure
        // compute over the compile-time-embedded schemas, so nothing forwards and
        // NO gate runs beyond the MCP edge's JWT validation. The parts are taken
        // shape-only so the source gate's one dispatch shape holds for the cluster.
        // A recorded decision (review round 2026-09-27): the exposure is static
        // product vocabulary — the same schemas the public docs publish — never
        // tenant data; the direct binding's Level 2 check ran for nothing here.
        tools::doc_types::describe_schema(self, &parts, input).await
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
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::invocations::invocation_read(self, &parts, input).await
    }

    #[tool(
        description = "Manage agent-invocation envelopes with an `action` discriminator. Actions: `open` (start an accountability envelope for one agent run against a cognitive map — requires `trigger_kind`, `originating_cogmap` ref; optional `parent_cogmap`; returns the server-minted invocation_id), `close` (terminate an open envelope — requires `invocation` ref, `disposition`: completed/failed/abandoned; optional `outcome`)."
    )]
    async fn invocation_manage(
        &self,
        Parameters(input): Parameters<tools::invocations::InvocationManageInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::invocations::invocation_manage(self, &parts, input).await
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
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::ingest::segmented_ingest(self, &parts, input).await
    }

    // ── Steward (scoped descriptions; crosses the network door — beat 5) ──

    #[tool(
        description = "This tool is for the team-self-cognition steward agent. If you are not running a steward cycle, you do not need this tool. Read a team-self-cognition cogmap's ingest delta: how many new resources + events have landed in the team's contexts since the steward's watermark, and whether that clears the threshold (i.e. the steward should run)."
    )]
    async fn steward_ingest_delta(
        &self,
        Parameters(input): Parameters<temper_core::types::steward::StewardDeltaInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::steward::steward_ingest_delta(self, &parts, input).await
    }

    #[tool(
        description = "This tool is for the team-self-cognition steward agent. If you are not running a steward cycle, you do not need this tool. Advance a team-self-cognition cogmap's ingest watermark to a given event id — the cursor a completed steward run records so the next delta counts only newer material. Requires cogmap-write."
    )]
    async fn steward_advance_watermark(
        &self,
        Parameters(input): Parameters<temper_core::types::steward::StewardAdvanceWatermarkInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::steward::steward_advance_watermark(self, &parts, input).await
    }

    #[tool(
        description = "List data artifacts owned by a resource. Each artifact is structured data committed by an agent session, retrieved whole. Filter by kind (the bare family name) or intent (current/member/pinned). Set include_folded to include superseded artifacts. Visibility-gated: you only see artifacts whose owning resource you can read."
    )]
    async fn list_data_artifacts(
        &self,
        Parameters(input): Parameters<tools::data_artifacts::ListArtifactsInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::data_artifacts::list_artifacts(self, &parts, input).await
    }

    #[tool(
        description = "Get a single data artifact by its ID. Returns the full artifact with content payload. Visibility-gated: returns 'not found' if the owning resource is not visible to you."
    )]
    async fn get_data_artifact(
        &self,
        Parameters(input): Parameters<tools::data_artifacts::GetArtifactInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::data_artifacts::get_artifact(self, &parts, input).await
    }

    #[tool(
        description = "Commit one data artifact to a resource. The content payload is JSON, hashed and stored verbatim. Auth-gated: requires write standing on the owning resource. Use intent 'current' for the active value, 'member' for an ordered peer, or 'pinned' for a frozen reference. Use supersedes to name artifacts this one replaces (they become folded)."
    )]
    async fn commit_data_artifact(
        &self,
        Parameters(input): Parameters<tools::data_artifacts::CommitArtifactInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::data_artifacts::commit_artifact(self, &parts, input).await
    }

    #[tool(
        description = "List data-artifact shapes declared for a context or cogmap home. Each shape is a JSON Schema (draft 2020-12) governing one artifact family in that home. Visibility-gated: you only see shapes whose home anchor you can read. An unreadable home yields an empty set, never an error."
    )]
    async fn list_data_artifact_shapes(
        &self,
        Parameters(input): Parameters<tools::data_artifact_shapes::ListShapesInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::data_artifact_shapes::list_shapes(self, &parts, input).await
    }

    #[tool(
        description = "Get a single data-artifact shape by its ID. Returns the full shape with its JSON Schema, enforcement mode, and lineage version. Visibility-gated: returns 'not found' if the owning home anchor is not readable to you. Includes folded (superseded) shapes for audit history."
    )]
    async fn get_data_artifact_shape(
        &self,
        Parameters(input): Parameters<tools::data_artifact_shapes::GetShapeInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::data_artifact_shapes::get_shape(self, &parts, input).await
    }

    #[tool(
        description = "Declare a data-artifact shape for a family within a context or cogmap home. The schema is a JSON Schema (draft 2020-12) that governs the family; enforcement 'advisory' (default) records non-conformance, 'enforcing' refuses the commit. Authority-gated: requires authoring authority over the home (context_authorable_by_profile or cogmap_authorable_by_profile). A principal who cannot author the home is refused."
    )]
    async fn declare_data_artifact_shape(
        &self,
        Parameters(input): Parameters<tools::data_artifact_shapes::DeclareShapeInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // The network door: Level 1 + 2 execute at the API on the caller's bearer;
        // post-edge refusals are mapped arm-for-arm from the preserved bodies.
        tools::data_artifact_shapes::declare_shape(self, &parts, input).await
    }
}

/// The terminal sentences the relay's `map_post_edge_auth` speaks (from the API's
/// preserved 401 body text). Until teardown a second mapping — the direct binding's
/// `map_authz_error`, over the typed `AuthzError` computed in-process — consumed them
/// too, and hand-copied literals had drifted between the two once (`EmailResolution`
/// was missed — the catch-all spoke the raw API body); the sentences stay named
/// constants, after the repo's `REQUEST_ACCESS_COMMAND` precedent.
pub const TERMINAL_MACHINE_GATE_SENTENCE: &str =
    "This token is machine-shaped but does not declare a valid \
     client_credentials grant. This error is terminal and should not be retried.";
pub const TERMINAL_DEACTIVATION_SENTENCE: &str =
    "This account has been deactivated. This error is terminal and should not be retried.";
pub const TERMINAL_EMAIL_RESOLUTION_SENTENCE: &str =
    "Could not resolve an email address for this token. \
     This error is terminal and should not be retried.";

/// Client results cross the post-edge refusal mapping before any call-site mapping:
/// a deactivated / machine-gate / expired-in-flight / email-resolution / system-
/// access refusal speaks its own sentence (the §3 arm-for-arm discipline), everything
/// else falls through unchanged to the call site's own voice. The ONE idiom every
/// relayed call site uses — both the `tools` module and the protocol module.
pub(crate) trait AcrossAuth<T> {
    fn across_auth(
        self,
        own: impl FnOnce(ClientError) -> rmcp::ErrorData,
    ) -> Result<T, rmcp::ErrorData>;
}

impl<T> AcrossAuth<T> for Result<T, ClientError> {
    fn across_auth(
        self,
        own: impl FnOnce(ClientError) -> rmcp::ErrorData,
    ) -> Result<T, rmcp::ErrorData> {
        self.map_err(|e| map_post_edge_refusal(&e).unwrap_or_else(|| own(e)))
    }
}

/// Map a post-edge refusal — Level 1's 401 arms plus Level 2's system-access 403 —
/// onto the rmcp sentence that refusal's arm speaks (design §3; the arm-for-arm
/// mapping ruling 7 completed, extended to Level 2 when the review round found
/// `SystemAccessRequired` falling through to an internal fault).
///
/// `None` for every other error — only post-edge refusals speak here; the tool's
/// own mapping owns the rest.
/// The 400/409 bodies carry the API's RENDERED Display (`Bad request: …` /
/// `Conflict: …`) — exactly as the 401 bodies carry `Unauthorized: …` — so the
/// label is the status echoed back, not part of the sentence. The caller-actionable
/// arms speak the server's OWN sentence, so the label comes off first (the same
/// care `map_post_edge_auth` applies to the 401 label); a body without either label
/// passes through whole.
pub(crate) fn api_error_cause(message: &str) -> &str {
    message
        .strip_prefix("Bad request: ")
        .or_else(|| message.strip_prefix("Conflict: "))
        .unwrap_or(message)
}

pub fn map_post_edge_refusal(refusal: &ClientError) -> Option<rmcp::ErrorData> {
    map_system_access_refusal(refusal).or_else(|| map_post_edge_auth(refusal))
}

/// Level 2's refusal through the door: the gated stack's `require_system_access`
/// 403 arrives typed as `ClientError::SystemAccessRequired`, carrying the API's
/// `SystemAccessDetails` reconstructed defensively (`CliAccessDetails`). The tool
/// renders the direct binding's sentence and details — the FIRST refusal a caller on
/// an invite-only deployment hits, so it must name the identity, the reason, and the
/// remediation, not reduce to "system access required" as an internal fault.
fn map_system_access_refusal(refusal: &ClientError) -> Option<rmcp::ErrorData> {
    let ClientError::SystemAccessRequired(details) = refusal else {
        return None;
    };
    let who = details.email.as_deref().unwrap_or("your account");
    // `CliAccessDetails.refusal` is `Option` only because the error chain
    // reconstructs defensively; every current server populates it. The fallback
    // sentence states the standing fact without inventing a kind.
    let reason = details
        .refusal
        .as_ref()
        .map(|r| r.reason())
        .unwrap_or_else(|| "your standing does not include system access".to_string());
    let payload = serde_json::json!({
        "email": details.email,
        "display_name": details.display_name,
        "refusal": details.refusal,
        // The remediation rides the constants even when the wire left them unset:
        // one address and one command, shared with the API's 403 and the direct
        // binding, so the surfaces cannot disagree.
        "request_url": details.request_url.clone()
            .unwrap_or_else(|| temper_core::types::access_gate::REQUEST_ACCESS_URL.to_string()),
        "cli_command": details.cli_command.clone()
            .unwrap_or_else(|| temper_core::types::access_gate::REQUEST_ACCESS_COMMAND.to_string()),
    });
    Some(rmcp::ErrorData::new(
        rmcp::model::ErrorCode::INVALID_REQUEST,
        format!(
            "Access to this temper instance requires approval for {who} — {reason}. \
             Visit {} or run `{}` in the CLI to request access. \
             This error is terminal and should not be retried.",
            temper_core::types::access_gate::REQUEST_ACCESS_URL,
            temper_core::types::access_gate::REQUEST_ACCESS_COMMAND
        ),
        Some(payload),
    ))
}

/// Map a post-edge authentication refusal onto the rmcp sentence that refusal's arm
/// speaks (design §3 — the arm-for-arm mapping ruling 7 completed).
///
/// Through the network door, Level 1 and Level 2 run at the API, and their refusals
/// arrive as 401/403 responses whose bodies temper-client preserves
/// ([`ClientError::UnauthorizedDetails`]). The distinguishing information lives in the
/// body, so THIS is where the arms split — one sentence per refusal kind, each carried
/// over word-for-word from the direct binding's mapping (`map_authz_error`):
///
/// - **Deactivated** ("account is deactivated") → the terminal deactivation sentence.
/// - **Machine-credential refusal** (the API's `machine credential refused: {why}`,
///   ruling 7) → the terminal machine-gate sentence.
/// - **Machine-principal gate** (the registration gate's own 401 message riding
///   `ProfileResolution`'s wire voice) → the same terminal framing the direct binding
///   gave it.
/// - **Expired-in-flight** ("Invalid or expired token" — a face the direct binding
///   could not produce: expiry used to die at the edge) → the NEW re-authenticate
///   sentence, consistent with the `WWW-Authenticate` remediation the edge emits
///   (design §10's named parity arm).
/// - **Email resolution** (the API passes `EmailResolution` straight through as the
///   401 body) → the direct binding's terminal email sentence, via the shared
///   constant.
///
/// `None` for every other error — only authentication refusals speak here; the tool's
/// own mapping owns the rest.
pub fn map_post_edge_auth(refusal: &ClientError) -> Option<rmcp::ErrorData> {
    let ClientError::UnauthorizedDetails { message } = refusal else {
        return None;
    };
    // The preserved body is the API's RENDERED 401 voice: `ApiError::Unauthorized`'s
    // Display is "Unauthorized: {cause}" (temper-services/src/error.rs:29), and the
    // body carries that Display verbatim. The arms below split on the CAUSE — the
    // body's distinguishing text — so the prefix comes off first; matching the raw
    // body never hit any arm and every refusal fell through to the catch-all, a
    // deactivation speaking the machine-gate's framing (witnessed by
    // `resources_wire_arms_test` against the real listener).
    let cause = message.strip_prefix("Unauthorized: ").unwrap_or(message);
    let terminal =
        |msg: String| rmcp::ErrorData::new(rmcp::model::ErrorCode::INVALID_REQUEST, msg, None);
    if cause.starts_with("machine credential refused:") {
        // Terminal, like the direct binding's `AuthzError::Refused` arm: the token is
        // structurally incoherent, so retrying changes nothing.
        Some(terminal(TERMINAL_MACHINE_GATE_SENTENCE.to_string()))
    } else if cause == "account is deactivated" {
        Some(terminal(TERMINAL_DEACTIVATION_SENTENCE.to_string()))
    } else if cause == "Token missing email claim and userinfo lookup failed" {
        // The email-ladder 401 (the API passes `EmailResolution` straight through as
        // the body, auth.rs:136). A human token with no resolvable email — terminal:
        // re-sending changes nothing (the fix is a token with an email claim). Speaks
        // the direct binding's `AuthzError::EmailResolution` sentence via the shared
        // constant.
        Some(terminal(TERMINAL_EMAIL_RESOLUTION_SENTENCE.to_string()))
    } else if cause == "Invalid or expired token" {
        // The expired-in-flight face. The bearer verified at the edge but no longer
        // decodes at the API — its lifetime ended inside the hop. The remedy is
        // re-authentication, the same one the edge's 401 advertises via
        // `WWW-Authenticate`; this sentence names it because the hop's 401 body cannot
        // carry that header's meaning through a JSON-RPC answer.
        Some(terminal(
            "This session's token has expired. Re-authenticate (the MCP client's OAuth \
             flow will refresh it) and retry the call."
                .to_string(),
        ))
    } else if cause == "Missing Authorization header"
        || cause == "Authorization header must use Bearer scheme"
        || cause == "Invalid Authorization header encoding"
    {
        // The bearer-scheme faces: the edge verified A token, so these mean the relay's
        // re-issued credential was mangled in transit — an operator-visible fault
        // phrased as the re-authentication it reduces to.
        Some(terminal(
            "This call's credentials did not survive the hop. Re-authenticate and retry; \
             if it recurs, contact the operator."
                .to_string(),
        ))
    } else if cause == "Authentication service unavailable" {
        // The API's JWKS-retrieval-failure 401 (auth middleware's `error!` arm): a
        // transient infrastructure fault whose honest remedy is exactly a retry — the
        // one 401 cause that must NOT take the terminal framing. It cannot be produced
        // on demand at the listener (it needs the key store to fail, not the caller),
        // so this arm is witnessed at the mapping's unit grain rather than on the wire.
        Some(rmcp::ErrorData::internal_error(
            "The authentication service is temporarily unavailable. Retry the call; if \
             it recurs, contact the operator."
                .to_string(),
            None,
        ))
    } else {
        // The catch-all: the machine-principal REGISTRATION GATE's own 401 voice
        // (unregistered or revoked client_id — G3 Phase A's gate) arrives here, and
        // the direct binding framed it terminal, so the framing carries. Every other
        // named cause above got its arm BECAUSE it stopped belonging here — a cause
        // landing in this arm is either the gate's voice or a face nobody has named
        // yet; the latter is the drift this comment exists to prevent reading as
        // intended.
        Some(terminal(format!(
            "{cause} This error is terminal and should not be retried."
        )))
    }
}

/// The blob tools, spelled once — the `list_tools` advertisement filter and the router
/// tests both read this list.
pub(crate) const BLOB_TOOL_NAMES: [&str; 2] = ["blob_read", "blob_manage"];

/// `list_tools`' advertisement posture: when the blob door is closed (no credential
/// resolves, or `BLOB_ENABLED=false` closed it deliberately), the blob pair is NOT
/// advertised — an agent discovering the surface only to learn it refuses is noise.
/// Existence is untouched: `tools/call` on a closed door still answers the typed
/// refusal (`blob_door_open` answers the `BlobDoor::Closed` sentence), so a stale cached tool list
/// degrades to the same voice it always had, never to silence.
fn advertise_blob_tools(tools: Vec<rmcp::model::Tool>, blob_ready: bool) -> Vec<rmcp::model::Tool> {
    if blob_ready {
        tools
    } else {
        tools
            .into_iter()
            .filter(|t| !BLOB_TOOL_NAMES.contains(&t.name.as_ref()))
            .collect()
    }
}

/// The `tools/list` answer: the advertised tool set, stamped with the deployment
/// surface's cache policy (see `cache_policy`) — MCP 2026-07-28 clients reject a list
/// result without `ttlMs` and `cacheScope`.
fn list_tools_result(blob_ready: bool) -> ListToolsResult {
    ListToolsResult::with_all_items(advertise_blob_tools(
        TemperMcpService::tool_router().list_all(),
        blob_ready,
    ))
    .with_ttl_ms(crate::cache_policy::DEPLOYMENT_SURFACE_TTL_MS)
    .with_cache_scope(crate::cache_policy::DEPLOYMENT_SURFACE_SCOPE)
}

/// The request's HTTP parts, or empty parts when the transport carried none.
fn request_parts(extensions: &rmcp::model::Extensions) -> http::request::Parts {
    extensions
        .get::<http::request::Parts>()
        .cloned()
        .unwrap_or_else(|| http::Request::new(()).into_parts().0)
}

/// The host's `correlation_id`, applied to a call only when the tool declares the argument and
/// the agent sent none (absent or `null`). An agent-supplied value is never overridden, and a
/// tool without the argument is never given one.
fn fill_correlation_id(request: &mut rmcp::model::CallToolRequestParams, id: uuid::Uuid) {
    let declares = TemperMcpService::tool_router()
        .get(request.name.as_ref())
        .and_then(|tool| tool.input_schema.get("properties").cloned())
        .is_some_and(|props| props.get("correlation_id").is_some());
    if !declares {
        return;
    }
    let arguments = request.arguments.get_or_insert_with(Default::default);
    if arguments
        .get("correlation_id")
        .is_none_or(serde_json::Value::is_null)
    {
        arguments.insert(
            "correlation_id".to_string(),
            serde_json::Value::String(id.to_string()),
        );
    }
}

#[tool_handler]
impl rmcp::ServerHandler for TemperMcpService {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
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

    /// Manual override — `#[tool_handler]` generates `call_tool` only when the impl lacks one.
    /// Every tool call crosses the host's seam here, before dispatch:
    ///
    /// - a transport that carries no HTTP parts gets empty ones, so every tool (each takes
    ///   `Extension<Parts>`) and every seam sees the same shape on every transport;
    /// - a seam that yields no identity answers the not-connected refusal for EVERY tool,
    ///   relayed or not — a host that supplies nothing gets refusals, never answers (an
    ///   unavailable door has no seam: its relayed tools answer the host's sentence, and a
    ///   pure-compute tool still answers);
    /// - the identity is resolved ONCE here and carried to `relay_client` in the parts, so a
    ///   seam that would answer differently on a second consultation cannot split one call;
    /// - the identity's `correlation_id`, if any, fills the call's `correlation_id` argument
    ///   when the tool declares one and the agent sent none (`fill_correlation_id`).
    async fn call_tool(
        &self,
        mut request: rmcp::model::CallToolRequestParams,
        mut context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, rmcp::ErrorData> {
        let mut parts = request_parts(&context.extensions);
        // Any identity riding in from elsewhere is discarded: only this gate resolves one.
        parts.extensions.remove::<ResolvedIdentity>();
        if let Some(identity) = self.outgoing_identity(&parts)? {
            if let Some(correlation_id) = identity.correlation_id() {
                fill_correlation_id(&mut request, correlation_id);
            }
            parts.extensions.insert(ResolvedIdentity(identity));
        }
        // The tools read the parts from the context: replace them with the resolved ones (or
        // supply empty ones when the transport carried none).
        context.extensions.insert(parts);
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        Self::tool_router().call(tcc).await
    }

    /// Manual override — `#[tool_handler]` generates `list_tools` only when the impl
    /// lacks one, so defining it here is how the closed-door filter (see
    /// `advertise_blob_tools`) reaches the wire. The router itself is the static floor:
    /// both blob doors are always REGISTERED (witnessed by
    /// `both_blob_doors_are_advertised_by_the_router`); advertisement is the runtime,
    /// per-instance decision, and in stateless mode every request builds a fresh
    /// service, so this reads THIS request's instance state.
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Ok(list_tools_result(self.blob_door.is_open()))
    }

    /// No prompts are offered and the capability is not advertised, but a client that
    /// probes anyway must meet a list result MCP 2026-07-28 accepts: rmcp's default leaves
    /// `ttlMs`/`cacheScope` unset. The empty list is the same for every caller.
    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListPromptsResult, rmcp::ErrorData> {
        Ok(ListPromptsResult::with_all_items(Vec::new())
            .with_ttl_ms(crate::cache_policy::DEPLOYMENT_SURFACE_TTL_MS)
            .with_cache_scope(crate::cache_policy::DEPLOYMENT_SURFACE_SCOPE))
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

        // **No principal resolution here** — the network door's named delta (design §3,
        // §10). `initialize` used to run `authenticate_token` (Level 1, DB-backed) at
        // this function; the door moves Level 1 + 2 to the API, and re-running them at
        // the edge would be the duplicate resolution the door removes. The edge's JWT
        // verification is the only gate `initialize` passes; a deactivated caller now
        // learns at FIRST TOOL CALL, when the API refuses and the tool layer maps the
        // sentence — accepted on the record (§10, named deltas).

        Ok(self.get_info())
    }

    /// The protocol versions this server can serve: the 2025-11-25 initialize
    /// handshake plus the 2026-07-28 spec, whose lifecycle lives in per-request
    /// metadata rather than the handshake. Deliberately EXPLICIT, not the SDK
    /// default: rmcp 3.4.1's blanket default advertises every version it knows
    /// (back to 2024-11-05), a surface this server has never served, and a
    /// 2026-07-28-only list would strand every legacy client — negotiation
    /// echoes a legacy client's requested version only when the server supports
    /// it, and rejects when it supports no initialize-handshake version at all.
    /// With this list a legacy client is echoed the version it asked for and a
    /// modern client resolves through per-request negotiation; `get_info()`
    /// stays at the SDK default (`V_2025_11_25`), which is both today's
    /// initialize observable and the fallback answer for modern requests.
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&[ProtocolVersion::V_2025_11_25, ProtocolVersion::V_2026_07_28])
    }

    // ── Resources protocol ────────────────────────────────────────────

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListResourcesResult, rmcp::ErrorData> {
        // The network door: the browse read crosses the wire on the request's own identity;
        // visibility is decided at the API (design §3).
        let client = self.relay_client(&request_parts(&context.extensions))?;
        crate::resources::list_resources(&client, request).await
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
    ) -> Result<ReadResourceResponse, rmcp::ErrorData> {
        let client = self.relay_client(&request_parts(&context.extensions))?;
        crate::resources::read_resource(&client, request)
            .await
            .map(ReadResourceResponse::from)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        advertise_blob_tools, list_tools_result, map_post_edge_auth, TemperMcpService,
        BLOB_TOOL_NAMES,
    };
    use temper_client::error::ClientError;

    fn call(name: &'static str, args: serde_json::Value) -> rmcp::model::CallToolRequestParams {
        let mut p = rmcp::model::CallToolRequestParams::new(name);
        p.arguments = args.as_object().cloned();
        p
    }

    /// The host's `correlation_id` fills a declared, absent argument — and only that.
    /// FAILS IF: the fill overrides an agent's value, skips a `null`, or gives the argument to a
    /// tool that does not declare it.
    #[test]
    fn the_host_correlation_id_fills_only_a_declared_argument_the_agent_left_empty() {
        let host = uuid::Uuid::now_v7();
        let agent = "01a10000-0000-7000-8000-000000000001";

        let mut absent = call("create_resource", serde_json::json!({"title": "t"}));
        super::fill_correlation_id(&mut absent, host);
        assert_eq!(
            absent.arguments.unwrap()["correlation_id"],
            serde_json::json!(host.to_string())
        );

        let mut null = call("relationship", serde_json::json!({"correlation_id": null}));
        super::fill_correlation_id(&mut null, host);
        assert_eq!(
            null.arguments.unwrap()["correlation_id"],
            serde_json::json!(host.to_string())
        );

        let mut sent = call(
            "create_resource",
            serde_json::json!({"correlation_id": agent}),
        );
        super::fill_correlation_id(&mut sent, host);
        assert_eq!(
            sent.arguments.unwrap()["correlation_id"],
            serde_json::json!(agent),
            "the agent's value is never overridden"
        );

        let mut read = call("search", serde_json::json!({"query": "q"}));
        super::fill_correlation_id(&mut read, host);
        assert!(
            !read.arguments.unwrap().contains_key("correlation_id"),
            "a tool that does not declare the argument is never given it"
        );
    }

    struct FixedSeam(Option<crate::seam::OutgoingIdentity>);
    impl crate::seam::IdentitySeam for FixedSeam {
        fn outgoing_identity(
            &self,
            _parts: &http::request::Parts,
        ) -> Option<crate::seam::OutgoingIdentity> {
            self.0.clone()
        }
    }

    fn relaying(seam: FixedSeam) -> TemperMcpService {
        TemperMcpService::new(
            crate::BlobDoor::Closed {
                refusal: "unused".to_string(),
            },
            crate::RelayConfig::new("http://127.0.0.1:9", std::time::Duration::from_secs(1)),
            std::sync::Arc::new(seam),
        )
    }

    fn empty_parts() -> http::request::Parts {
        http::Request::new(()).into_parts().0
    }

    /// FAILS IF: a host's extra headers may restate a header the relay sets itself — reqwest
    /// would send both copies, and which one a reader honours would decide the identity.
    #[test]
    fn a_host_header_that_restates_identity_refuses_the_call() {
        use temper_workflow::operations::Surface;
        for reserved in [
            "authorization",
            "x-temper-surface",
            "x-temper-device-id",
            "traceparent",
        ] {
            let mut headers = http::HeaderMap::new();
            headers.insert(
                http::HeaderName::from_static(reserved),
                http::HeaderValue::from_static("smuggled"),
            );
            let svc = relaying(FixedSeam(Some(
                crate::seam::OutgoingIdentity::new("bearer", Surface::CliCloud)
                    .with_extra_headers(headers),
            )));
            let err = svc.relay_client(&empty_parts()).unwrap_err();
            assert!(
                err.message.contains(reserved),
                "{reserved}: {}",
                err.message
            );
        }
        let mut opaque = http::HeaderMap::new();
        opaque.insert("x-opaque", http::HeaderValue::from_static("fine"));
        relaying(FixedSeam(Some(
            crate::seam::OutgoingIdentity::new("bearer", Surface::CliCloud)
                .with_extra_headers(opaque),
        )))
        .relay_client(&empty_parts())
        .expect("an opaque header is the host's business");
    }

    /// The identity the dispatch gate resolved is the one the client is built from: a seam that
    /// would now answer nothing cannot split the call. FAILS IF: `relay_client` re-consults the
    /// seam when the call already carries its resolved identity.
    #[test]
    fn the_resolved_identity_outranks_a_second_consultation() {
        use temper_workflow::operations::Surface;
        let svc = relaying(FixedSeam(None));
        let mut parts = empty_parts();
        assert!(
            svc.relay_client(&parts).is_err(),
            "the seam alone answers nothing"
        );
        parts
            .extensions
            .insert(super::ResolvedIdentity(crate::seam::OutgoingIdentity::new(
                "bearer",
                Surface::CliCloud,
            )));
        svc.relay_client(&parts)
            .expect("the call's resolved identity builds the client");
    }

    /// Every tool that carries the agent's act fields carries `correlation_id` where the fill
    /// can see it (top-level `properties`). FAILS IF: a tool's input schema stops flattening the
    /// act fields (a tagged enum, an `allOf`), which would silently exempt it from a host's thread.
    #[test]
    fn every_tool_with_act_fields_declares_correlation_id_where_the_fill_reads() {
        let mut authored = 0;
        for tool in TemperMcpService::tool_router().list_all() {
            let props = tool.input_schema.get("properties");
            let has = |k: &str| props.is_some_and(|p| p.get(k).is_some());
            let mentions_reasoning = serde_json::to_string(&*tool.input_schema)
                .unwrap()
                .contains("\"reasoning\"");
            if mentions_reasoning {
                authored += 1;
                assert!(
                    has("reasoning") && has("correlation_id"),
                    "`{}` carries act fields the correlation fill cannot see",
                    tool.name
                );
            }
        }
        assert!(authored >= 10, "found only {authored} authored tools");
    }

    /// The JWKS-outage 401 ("Authentication service unavailable") is TRANSIENT — the
    /// one post-edge cause whose remedy is a retry [added — 2026-09-24, found in review]. The catch-all would
    /// frame it terminal; this arm must map it to an internal, retryable voice. The
    /// face needs the key store to FAIL, not the caller, so it cannot be produced on
    /// demand at the listener — the mapping's other arms carry the wire witnesses.
    #[test]
    fn the_jwks_outage_401_maps_retryable_never_terminal() {
        let err = map_post_edge_auth(&ClientError::UnauthorizedDetails {
            message: "Unauthorized: Authentication service unavailable".to_string(),
        })
        .expect("the arm maps");

        assert_eq!(
            err.code.0, -32603,
            "an infrastructure fault is an internal error, never a caller refusal: {err}"
        );
        assert!(
            !err.message.contains("terminal"),
            "the one retryable 401 must not be framed terminal: {err}"
        );
        assert!(
            err.message.contains("Retry the call"),
            "the sentence names its remedy: {err}"
        );
    }

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

    /// Same failure mode as `context_manage`, asserted as a PAIR because the pairing is the
    /// invariant: the read door and the manage door must both exist, or one anchor of the
    /// blob surface (bytes vs relations) is unreachable from MCP while the other is.
    /// This pins the ROUTER — the static floor; the runtime advertisement filter lives in
    /// `list_tools` (`advertise_blob_tools`) and is witnessed just below.
    #[test]
    fn both_blob_doors_are_advertised_by_the_router() {
        let names: Vec<String> = TemperMcpService::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();

        for peer in BLOB_TOOL_NAMES {
            assert!(
                names.iter().any(|n| n == peer),
                "{peer} is not advertised, so half the blob surface is unreachable from MCP; \
                 router has {names:?}"
            );
        }
    }

    /// The RUNTIME half of the pair above: a closed door (unconfigured, or
    /// `BLOB_ENABLED=false` closing it deliberately) stops ADVERTISING the blob pair and
    /// hides nothing else. The assertion is SET-based, not count-based (C-S2,
    /// 2026-09-04 review): a future blob tool registered but omitted from
    /// `BLOB_TOOL_NAMES` would keep the count equal while leaving the tool advertised —
    /// under-hiding goes red here. The one-line call site inside `list_tools` is
    /// witnessed over the wire by `a_closed_door_hides_the_blob_pair_from_the_wire`
    /// (tests/e2e/tests/mcp_blob_transport_e2e.rs), which rides the harness this diff
    /// ships.
    #[test]
    fn a_closed_door_stops_advertising_the_blob_pair_and_nothing_else() {
        let all: Vec<String> = TemperMcpService::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();

        let names = |tools: Vec<rmcp::model::Tool>| -> Vec<String> {
            let mut names: Vec<String> = tools.into_iter().map(|t| t.name.to_string()).collect();
            names.sort();
            names
        };
        let open = names(advertise_blob_tools(
            TemperMcpService::tool_router().list_all(),
            true,
        ));
        let closed = names(advertise_blob_tools(
            TemperMcpService::tool_router().list_all(),
            false,
        ));
        let mut expected_closed = all.clone();
        for peer in BLOB_TOOL_NAMES {
            expected_closed.retain(|n| n != peer);
        }
        expected_closed.sort();

        assert_eq!(open, all, "an open door advertises the full router");
        assert_eq!(
            closed, expected_closed,
            "a closed door advertises exactly the router minus the blob pair — no more, \
             no less; got {closed:?}"
        );
    }

    /// **The two anchor kinds materialize through PEER tools, and both must be advertised.**
    ///
    /// This is the assertion whose absence let a defect sit: `context_materialize` was fully
    /// implemented in `tools::cognitive_maps` — profile gate, anchor resolution, the same
    /// `MaterializeOnThreshold` command, even `origin: Surface::Mcp` — and was never registered.
    /// It compiled, clippy was happy (it is `pub`, so not dead code), and `cargo make check`
    /// passed, because nothing anywhere asserted it should be reachable.
    ///
    /// The consequence was a hole only visible by comparing doors: CLI and HTTP could materialize
    /// either anchor kind, MCP could materialize only a cogmap. So an agent reading
    /// `context_read(view: shape)` and getting `never_clustered` — a cause that names an action —
    /// had no action available at its own door, while the same agent on a cogmap did.
    ///
    /// Asserted as a PAIR rather than as one more single-tool test, because the singular test is
    /// what was missing: nobody omits a tool they are thinking about. The pairing is the invariant.
    #[test]
    fn both_anchor_kinds_can_be_materialized_through_the_router() {
        let names: Vec<String> = TemperMcpService::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();

        for peer in ["cogmap_materialize", "context_materialize"] {
            assert!(
                names.iter().any(|n| n == peer),
                "{peer} is not advertised, so one anchor kind cannot be materialized from MCP \
                 while the other can; router has {names:?}"
            );
        }
    }

    /// The facet tools answer as a set — set, read, retract. The retract door is the
    /// correction verb: without it an agent can assert an anchored-at row it later reads as
    /// divergent but can never correct, so its absence would strand the disagreement state
    /// with no action at the agent's own door (the same shape the anchor-materialize test
    /// above exists for).
    #[test]
    fn facet_retract_is_advertised_beside_the_other_facet_doors() {
        let names: Vec<String> = TemperMcpService::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();

        for peer in ["facet_set", "facets_read", "facet_retract"] {
            assert!(
                names.iter().any(|n| n == peer),
                "{peer} is not advertised, so the facet doors are incomplete; router has \
                 {names:?}"
            );
        }
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
    /// **The ceilings `/api/query` refuses on are in the schema this door hands a client.**
    ///
    /// `[added — 2026-08-28, found in review]` The shape pass may raise a bound only if the number
    /// is published, because a client must never refuse a plan a newer server would run. Until
    /// 2026-08-28 the MCP door published none of them — `max_items`/`max_length` are `utoipa`
    /// attributes gated on `web-api`, schemars reads its own namespace, and nothing bridged the
    /// two — so an agent read *"at most [`MAX_ID_SET_IDS`] of them"*, a Rust symbol with no
    /// resolvable value.
    ///
    /// **temper-core has a test for that, and it is one door short.** It asserts against
    /// `schemars::schema_for!`, which proves the DERIVE emits the constraint, not that rmcp hands
    /// it over — it would stay green if the SDK changed how it builds input schemas from a
    /// `Parameters<_>` wrapper. This asserts against `tool_router().list_all()`, the object a
    /// connected client actually receives, which is the pattern the search test beside it already
    /// uses.
    #[test]
    fn the_query_tool_advertises_the_ceilings_the_server_enforces() {
        use temper_core::types::query::composition::{MAX_INTENTION_QUERY_BYTES, MAX_STAGES};
        use temper_core::types::query::filter::MAX_FILTER_VALUES;
        use temper_core::types::query::id_set::MAX_ID_SET_IDS;

        let advertised = TemperMcpService::tool_router()
            .list_all()
            .into_iter()
            .find(|t| t.name == "run_query")
            .expect("the router advertises a `run_query` tool")
            .input_schema;
        let schema = serde_json::to_value(&*advertised).expect("input schema serializes");

        // Declarations are self-contained (no `$ref`, no `$defs` on the wire), so the ceilings
        // now live INLINE under the advertised object — reaching them where a client reads them
        // is the whole point of this test. The pointer walks one `stages` item's `anyOf` arm
        // (StageNode is untagged) and StageInput's `oneOf` caller arm; a wire-form change moves
        // these, and the panic below names the pointer that went missing.
        let stage = "/properties/plan/properties/stages/items/anyOf/0";
        for (pointer, expected) in [
            ("/properties/plan/properties/stages/maxItems", MAX_STAGES),
            (
                &format!("{stage}/properties/intention/properties/query/maxLength"),
                MAX_INTENTION_QUERY_BYTES,
            ),
            (
                &format!("{stage}/properties/inputs/items/oneOf/0/properties/ids/properties/ids/maxItems"),
                MAX_ID_SET_IDS,
            ),
            (
                &format!("{stage}/properties/resource_filter/properties/doc_type/maxItems"),
                MAX_FILTER_VALUES,
            ),
            (
                &format!("{stage}/properties/resource_filter/properties/tags/maxItems"),
                MAX_FILTER_VALUES,
            ),
            (
                &format!("{stage}/properties/edge_filter/properties/labels/maxItems"),
                MAX_FILTER_VALUES,
            ),
        ] {
            let published = schema
                .pointer(pointer)
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_else(|| {
                    panic!("the advertised run_query schema publishes nothing at `{pointer}`")
                });
            assert_eq!(
                published as usize, expected,
                "`{pointer}` in the schema this door hands a client"
            );
        }
    }

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
            .join("../../skills/temper-knowledge-base");

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

    /// **Every scalar enum in an advertised tool's input schema must be inlined, never `$ref`d.**
    ///
    /// rmcp generates input schemas with schemars' ref-based generator, so a scalar enum
    /// field emits `{"$ref": "#/$defs/Foo"}` into `$defs`. The Anthropic tool-use layer
    /// does not resolve `$ref`/`$defs`, so the model gets no type signal for that field
    /// and sends explicit `null` → `-32602: invalid type: null, expected string`.
    /// Objects tolerate an unresolved `$ref` (the model defaults to sending a map), so the
    /// invariant is narrow: a `$ref` may never resolve to a scalar-enum schema among an
    /// advertised input's top-level properties.
    ///
    /// A unit-only enum is scalar in either emitted shape: the flat
    /// `{"type":"string","enum":[…]}` form, and the `oneOf`-of-string-consts form schemars
    /// 1.2 emits (see the `tools::blobs` harness, which documents both admissible *inline*
    /// shapes). This test accepts either shape inline and flags either shape behind `$ref`.
    ///
    /// Established 2026-05-29 on `EdgeKind`/`Polarity` and applied to each enum added
    /// since — but never swept across the router: eleven older enums kept shipping `$ref`d.
    /// This walks `tool_router().list_all()` — the object a connected client actually
    /// receives — so a new tool cannot quietly regress it.
    ///
    /// **Declared remainder:** only top-level `$ref`s are walked, and `Option<Enum>` input
    /// fields exist today — eight of them (`RelationshipInput`'s target/edge_kind/polarity,
    /// `blob_manage`'s direction/edge_kind/polarity, `invocation_manage`'s disposition, and
    /// the flattened `ActInput`'s confidence band) — and pass unwalked: their `$ref` arrives
    /// inside `anyOf`, not at the property's top level, and they hold inline only because
    /// each enum separately carries its own `schemars(inline)`. An `Option<NewEnum>` without
    /// that attribute would land its `$ref` in the advertised schema and pass this test.
    /// Nested objects inside composed inputs (`run_query`'s stages) are likewise unwalked.
    /// This paragraph is where that gap is named rather than hidden.
    #[test]
    fn every_advertised_tool_input_inlines_scalar_enums() {
        // A schema is a scalar enum when it enumerates string constants, in either
        // emitted shape. Anything else — an object, a map, a free string — is fine
        // behind `$ref`.
        fn resolves_to_string_enum(resolved: &serde_json::Value) -> bool {
            if resolved.get("type").and_then(|t| t.as_str()) == Some("string")
                && resolved.get("enum").is_some()
            {
                return true;
            }
            resolved
                .get("oneOf")
                .and_then(|o| o.as_array())
                .is_some_and(|branches| {
                    !branches.is_empty()
                        && branches.iter().all(|b| {
                            b.get("const").is_some()
                                && b.get("type").and_then(|t| t.as_str()) == Some("string")
                        })
                })
        }

        let advertised = TemperMcpService::tool_router().list_all();

        let mut checked = 0usize;
        let mut offenders: Vec<String> = Vec::new();
        for tool in &advertised {
            let schema =
                serde_json::to_value(&*tool.input_schema).expect("input schema serializes");
            let defs = schema.get("$defs");
            let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else {
                continue;
            };
            for (name, prop) in props {
                checked += 1;
                let Some(target) = prop.get("$ref").and_then(|r| r.as_str()) else {
                    continue;
                };
                let leaf = target.rsplit('/').next().unwrap_or_default();
                let Some(resolved) = defs.and_then(|d| d.get(leaf)) else {
                    continue;
                };
                if resolves_to_string_enum(resolved) {
                    offenders.push(format!(
                        "{}.{name} → {target} (a scalar enum behind `$ref` reaches \
                         Anthropic tool-use as null)",
                        tool.name
                    ));
                }
            }
        }

        // A schema shape change that empties the walk must fail loudly, not pass vacuously.
        assert!(
            checked >= 30,
            "walked only {checked} input properties across {} advertised tools — the \
             properties walk has probably broken, leaving this test checking nothing",
            advertised.len()
        );
        assert!(
            offenders.is_empty(),
            "{} advertised tool input(s) carry a scalar enum behind `$ref` — add \
             `#[schemars(inline)]` to the enum:\n  {}",
            offenders.len(),
            offenders.join("\n  ")
        );
    }

    /// MCP 2026-07-28 makes `ttlMs` and `cacheScope` required on `tools/list`, and Claude
    /// Code enforces it: without them it logs `Invalid result for tools/list` and drops
    /// every tool. rmcp's constructor leaves both unset, so assert the serialized answer of
    /// the result builder in both blob postures. FAILS IF: either key is absent, or the
    /// values drift from the deployment-surface policy. That the handler answers with this
    /// builder is witnessed over the transport by `mcp_typed_object_calls_e2e` and by the
    /// byte-identical declaration fixture.
    #[test]
    fn list_tools_result_carries_ttl_ms_and_cache_scope() {
        for blob_ready in [true, false] {
            let wire = serde_json::to_value(list_tools_result(blob_ready))
                .expect("tools/list result serializes");

            assert_eq!(
                wire.get("ttlMs"),
                Some(&serde_json::json!(
                    crate::cache_policy::DEPLOYMENT_SURFACE_TTL_MS
                )),
                "tools/list must carry a numeric ttlMs (blob_ready={blob_ready}): {wire}"
            );
            assert_eq!(
                wire.get("cacheScope"),
                Some(&serde_json::json!("public")),
                "tools/list must carry cacheScope (blob_ready={blob_ready}): {wire}"
            );
            assert!(
                wire["tools"].as_array().is_some_and(|t| !t.is_empty()),
                "the stamped result still advertises the tool set: {wire}"
            );
        }
    }
}
