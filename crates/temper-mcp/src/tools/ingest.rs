//! Segmented (multi-block) ingest tools — the MCP surface's answer to a body too large for one
//! call. `ingest_begin` creates the resource with segment 0; `ingest_append` lands each further
//! segment; `ingest_finalize` declares the session complete; `ingest_blocks` reads the landed set
//! back, which is how a stateless caller resumes after an interruption.
//!
//! Unlike the CLI, an MCP caller has no chunker and no embedder: it omits `chunks_packed` entirely
//! and the server chunks the segment text itself, carrying the heading breadcrumb across the block
//! boundary so `header_path` stays continuous.
//!
//! Integrity is per-segment on the way in (`content_hash`, verified server-side) plus the opaque
//! `body_hash` echoed back at finalize. Nothing here asks the caller to compute a merkle it has no
//! way to derive.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;

use temper_core::error::TemperError;
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_core::types::ingest::{AppendBlockPayload, FinalizePayload, SegmentedBegin};
use temper_services::backend::DbBackend;
use temper_workflow::operations::{Backend, Surface};

use crate::service::TemperMcpService;
use crate::tools::resources::{build_create_command, CreateResourceInput};

/// The segment budget a caller gets when it does not name one. Matches the CLI's default so a
/// resumed session re-derives identical boundaries. Recorded, never enforced.
const DEFAULT_BLOCK_BUDGET: u32 = 262_144;

// ── Helpers ────────────────────────────────────────────────────────────────────

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

fn map_err(e: TemperError, action: &str) -> rmcp::ErrorData {
    match e {
        TemperError::NotFound(_) => {
            rmcp::ErrorData::invalid_params(format!("{action}: resource not found"), None)
        }
        TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        TemperError::Forbidden => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{action}: cannot modify this resource"),
            None,
        ),
        other => rmcp::ErrorData::internal_error(format!("{action}: {other}"), None),
    }
}

fn parse_resource(s: &str) -> Result<ResourceId, rmcp::ErrorData> {
    let uuid = temper_workflow::operations::parse_ref(s)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad resource ref: {e}"), None))?
        .0;
    Ok(ResourceId::from(uuid))
}

// ── Inputs ─────────────────────────────────────────────────────────────────────

/// MCP input for `ingest_begin` — every `create_resource` field, plus the segmented-session hints.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct IngestBeginInput {
    #[serde(flatten)]
    pub create: CreateResourceInput,
    /// Bare-hex sha256 of `content` — this segment's transit-integrity check. A mismatch is
    /// rejected.
    pub content_hash: String,
    /// Bytes of text this session's segment boundaries were cut at. Recorded so a resume re-derives
    /// identical boundaries; never enforced server-side. Defaults to 262144.
    #[serde(default)]
    pub block_budget: Option<u32>,
    /// Best-effort total segment count, if known upfront. Informational only.
    #[serde(default)]
    pub total_blocks_hint: Option<u32>,
    /// sha256 of the whole source, when the source has a stable identity (a file on disk). Omit when
    /// composing content in-context — there is nothing stable to hash.
    #[serde(default)]
    pub source_hash: Option<String>,
}

/// MCP input for `ingest_append`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct IngestAppendInput {
    /// Resource ref returned by `ingest_begin` (UUID or decorated `slug-<uuid>`).
    pub resource: String,
    /// Zero-based segment index. Segment 0 landed at begin, so appends start at 1 and go in order.
    pub seq: u32,
    /// This segment's markdown text.
    pub content: String,
    /// Bare-hex sha256 of `content`. A mismatch is rejected before anything lands.
    pub content_hash: String,
    /// Optional block-provenance sources this segment was distilled from — recorded against this
    /// appended content block. Same value grammar as `create_resource`'s `sources` / `resource
    /// create --sources`: each entry is a resource ref (UUID or decorated `slug-<uuid>`) or an
    /// http/https URL. Omit for an un-attributed append.
    #[serde(default)]
    pub sources: Option<Vec<String>>,
}

/// MCP input for `ingest_finalize`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct IngestFinalizeInput {
    pub resource: String,
    /// Total landed segments, counting segment 0.
    pub expected_blocks: u32,
    /// The `body_hash` from your most recent `ingest_append` / `ingest_blocks` response. Opaque —
    /// echo it back verbatim; do not parse or recompute it.
    pub expected_body_hash: String,
}

/// MCP input for `ingest_blocks`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct IngestBlocksInput {
    pub resource: String,
}

// ── Tool handlers ──────────────────────────────────────────────────────────────

pub async fn ingest_begin(
    svc: &TemperMcpService,
    input: IngestBeginInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let profile_id = ProfileId::from(profile.id);

    // Segment 0's integrity is checked here, on the surface: it travels as the create body, so the
    // append path's `validate_append` never sees it.
    let content = input.create.content.as_deref().unwrap_or_default();
    if content.is_empty() {
        return Err(rmcp::ErrorData::invalid_params(
            "ingest_begin requires content — segment 0's text".to_owned(),
            None,
        ));
    }
    if temper_core::hash::sha256_hex(content.as_bytes()) != input.content_hash {
        return Err(rmcp::ErrorData::invalid_params(
            "content_hash does not match content".to_owned(),
            None,
        ));
    }

    let seg = SegmentedBegin {
        total_blocks_hint: input.total_blocks_hint,
        block_budget: input.block_budget.unwrap_or(DEFAULT_BLOCK_BUDGET),
        source_hash: input.source_hash,
    };
    let cmd = build_create_command(svc, profile_id, input.create).await?;

    let backend = DbBackend::new(svc.api_state.pool.clone(), profile_id);
    let out = backend
        .begin_segmented_ingest(cmd, seg)
        .await
        .map_err(|e| map_err(e, "ingest_begin"))?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&out.value),
    )]))
}

pub async fn ingest_append(
    svc: &TemperMcpService,
    input: IngestAppendInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let profile_id = ProfileId::from(profile.id);
    let resource = parse_resource(&input.resource)?;

    // Classify each source (resource ref → Resource, http/https URL → Remote) with the same shared
    // resolver the CLI and `create_resource` use; an unparseable value is a hard error, never a
    // silent drop.
    let sources = input
        .sources
        .unwrap_or_default()
        .iter()
        .map(|s| temper_workflow::operations::resolve_provenance_source(s))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            rmcp::ErrorData::invalid_params(format!("invalid sources value: {e}"), None)
        })?;

    let backend = DbBackend::new(svc.api_state.pool.clone(), profile_id);
    let out = backend
        .append_block(
            resource,
            AppendBlockPayload {
                seq: input.seq,
                content: input.content,
                content_hash: input.content_hash,
                // No chunker, no embedder on this surface: the server chunks `content`.
                chunks_packed: None,
                sources,
            },
            Surface::Mcp,
        )
        .await
        .map_err(|e| map_err(e, "ingest_append"))?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&out.value),
    )]))
}

pub async fn ingest_finalize(
    svc: &TemperMcpService,
    input: IngestFinalizeInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let profile_id = ProfileId::from(profile.id);
    let resource = parse_resource(&input.resource)?;

    let backend = DbBackend::new(svc.api_state.pool.clone(), profile_id);
    backend
        .finalize_ingest(
            resource,
            FinalizePayload {
                expected_blocks: input.expected_blocks,
                expected_body_hash: input.expected_body_hash,
            },
            Surface::Mcp,
        )
        .await
        .map_err(|e| map_err(e, "ingest_finalize"))?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        format!(
            "Finalized {} ({} blocks).",
            input.resource, input.expected_blocks
        ),
    )]))
}

pub async fn ingest_blocks(
    svc: &TemperMcpService,
    input: IngestBlocksInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let profile_id = ProfileId::from(profile.id);
    let resource = parse_resource(&input.resource)?;

    let backend = DbBackend::new(svc.api_state.pool.clone(), profile_id);
    let out = backend
        .list_blocks(resource)
        .await
        .map_err(|e| map_err(e, "ingest_blocks"))?;

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        to_text(&out.value),
    )]))
}
