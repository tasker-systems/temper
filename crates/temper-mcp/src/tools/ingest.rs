//! Segmented (multi-block) ingest tools — the MCP surface's answer to a body too large for one
//! call. `ingest_begin` creates the resource with segment 0; `ingest_append` lands each further
//! segment; `ingest_finalize` declares the session complete; `ingest_blocks` reads the landed set
//! back, which is how a stateless caller resumes after an interruption.
//!
//! Execution crosses the DEPLOYED API over the wire (beat G4 — the last direct cluster):
//! begin → `POST /api/ingest` (the segmented branch: `IngestPayload.segmented`), append →
//! `POST /api/resources/{id}/blocks`, finalize → `POST /api/resources/{id}/finalize`, blocks
//! → `GET /api/resources/{id}/blocks`. Each call forwards the caller's bearer via the
//! per-request temper-client relay built from the request's `Parts`, and touches the pool
//! nowhere.
//!
//! Unlike the CLI, an MCP caller has no chunker and no embedder: it omits `chunks_packed`
//! entirely, so the server chunks each segment itself and carries the heading breadcrumb
//! across the block boundary so `header_path` stays continuous — the server-side path the
//! deployed route runs on this relay's forwarded payloads.
//!
//! Integrity is per-segment on the way in (`content_hash`, verified server-side) plus the
//! opaque `body_hash` echoed back at finalize. Nothing here asks the caller to compute a
//! merkle it has no way to derive.
//!
//! # Declared parity deltas (the direct faces pinned by
//! # `ingest_blobs_artifacts_parity_test.rs`, flipped here deliberately)
//!
//! - **Finalize's expectation-mismatch Conflict** (`expected_blocks`,
//!   `expected_body_hash`): the direct map had no `Conflict` arm (the catch-all rendered
//!   it `internal_error`); the wire's 409 renders `invalid_params` with the server's own
//!   sentence. The append path's occupied-seq re-write is NOT a delta — both sides refuse
//!   `internal_error` (the append route bridges the raise generically, never a typed 409);
//!   named at the pin, not declared here.
//! - **Finalize's whole-content-hash mismatch** (`expected_content_hash` — NOT reachable
//!   from MCP today: the tool sends `None`, declared below): the wire answers 422
//!   `CONTENT_INTEGRITY` typed (`ClientError::ContentIntegrity`) and renders
//!   `invalid_params` with the server's sentence — the G3c closed-invocation family. Named
//!   (the arm exists, the tool never fires it); not a reachable parity face.
//!
//! The surface-side checks stay MCP-local (pre-wire): begin's "requires content" arm and
//! "content_hash does not match content" arm are the tool's own input assertions, unchanged
//! either side of the door. The dispatch requirement arms are byte-exact.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_client::error::ClientError;
use temper_core::types::ingest::{
    AppendBlockPayload, FinalizePayload, IngestPayload, SegmentedBegin,
};

use crate::service::{api_error_cause, AcrossAuth, TemperMcpService};
use crate::tools::resources::CreateResourceInput;

/// The segment budget a caller gets when it does not name one. Matches the CLI's default so a
/// resumed session re-derives identical boundaries. Recorded, never enforced.
const DEFAULT_BLOCK_BUDGET: u32 = 262_144;

// ── Helpers ────────────────────────────────────────────────────────────────────

fn to_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

/// Wire refusals to rmcp, in the G3c/G3d mapping idiom (see the module header for the
/// declared deltas).
fn map_err(e: ClientError, action: &str) -> rmcp::ErrorData {
    match e {
        ClientError::ResourceErased { id } => crate::tools::resources::erased_error(id),
        ClientError::NotFound { message } => {
            rmcp::ErrorData::invalid_params(format!("{action}: {message}"), None)
        }
        ClientError::Server {
            status: 400,
            message,
        } => rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None),
        ClientError::Server {
            status: 422,
            message,
        } => rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None),
        ClientError::ContentIntegrity { message } => {
            rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None)
        }
        ClientError::Conflict { message } | ClientError::IngestEnded { message } => {
            rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None)
        }
        // A refusal that named the capability it withheld — carry the gate's own sentence. The
        // terse arm below stays exactly as it was, for the caller who may not even read the subject.
        ClientError::ForbiddenDetail { message } => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{action}: {message}"),
            None,
        ),
        ClientError::Forbidden => rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_REQUEST,
            format!("{action}: cannot modify this resource"),
            None,
        ),
        other => rmcp::ErrorData::internal_error(format!("{action}: {other}"), None),
    }
}

fn parse_resource(s: &str) -> Result<Uuid, rmcp::ErrorData> {
    temper_workflow::operations::parse_ref(s)
        .map(|p| p.uuid())
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad resource ref: {e}"), None))
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

// ── Wire-shape builders ───────────────────────────────────────────────────────
//
// The wire request is built straight from the input. What stays behind the door:
// the surface-side hash check on segment 0 (declared below, MCP-local), and the
// `Surface::Mcp` stamp the direct command carried (now the carrier).

/// The wire `IngestPayload` for a begin call: the create fields, the segment's raw
/// text, and the segmented-begin discriminator. The direct binding's home refusals
/// are carried MCP-local (the `build_create_command` shape they came from): exactly
/// one of `context_ref` / `cogmap`, the goal ref's parse failure a hard error, the
/// sources' classifier failure a hard error ("never a silent drop"), and a
/// managed_meta serialization failure an internal error — none silently defaulted.
fn wire_begin(input: IngestBeginInput) -> Result<IngestPayload, rmcp::ErrorData> {
    let create = input.create;
    let home_cogmap_id = create
        .cogmap
        .as_deref()
        .map(|r| {
            temper_workflow::operations::parse_ref(r)
                .map(|p| p.uuid())
                .map_err(|e| {
                    rmcp::ErrorData::invalid_params(format!("invalid cogmap ref: {e}"), None)
                })
        })
        .transpose()?;
    match (&create.context_ref, home_cogmap_id) {
        (Some(_), Some(_)) => {
            return Err(rmcp::ErrorData::invalid_params(
                "context_ref and cogmap are mutually exclusive; supply exactly one home"
                    .to_string(),
                None,
            ));
        }
        (None, None) => {
            return Err(rmcp::ErrorData::invalid_params(
                "no home specified — supply exactly one of context_ref or cogmap".to_string(),
                None,
            ));
        }
        _ => {}
    }
    let goal = create
        .goal
        .as_deref()
        .map(|r| {
            temper_workflow::operations::parse_ref(r)
                .map(|p| p.uuid())
                .map_err(|e| {
                    rmcp::ErrorData::invalid_params(format!("invalid goal ref: {e}"), None)
                })
        })
        .transpose()?;
    let managed_meta = create
        .managed_meta
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("managed_meta serialization failed: {e}"), None)
        })?;
    let sources = create
        .sources
        .unwrap_or_default()
        .iter()
        .map(|s| temper_workflow::operations::resolve_provenance_source(s))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            rmcp::ErrorData::invalid_params(format!("invalid sources value: {e}"), None)
        })?;
    Ok(IngestPayload {
        title: create.title,
        origin_uri: create
            .origin_uri
            .unwrap_or_else(|| format!("mcp://agent/{}", Uuid::new_v4())),
        context_ref: create.context_ref.unwrap_or_default(),
        home_cogmap_id,
        doc_type_name: create.doc_type_name,
        goal,
        content_hash: None,
        idempotency_key: create.idempotency_key,
        content: create.content.unwrap_or_default(),
        metadata: None,
        managed_meta,
        open_meta: create.open_meta,
        chunks_packed: None,
        sources,
        act: create.act,
        segmented: Some(SegmentedBegin {
            total_blocks_hint: input.total_blocks_hint,
            block_budget: input.block_budget.unwrap_or(DEFAULT_BLOCK_BUDGET),
            source_hash: input.source_hash,
        }),
    })
}

// ── Tool handlers ──────────────────────────────────────────────────────────────

pub async fn ingest_begin(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: IngestBeginInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
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

    let out = svc
        .relay_client(parts)?
        .ingest()
        .begin_segmented(&wire_begin(input)?)
        .await
        .across_auth(|e| map_err(e, "ingest_begin"))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&out)),
    ]))
}

pub async fn ingest_append(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: IngestAppendInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
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

    let out = svc
        .relay_client(parts)?
        .ingest()
        .append_block(
            resource,
            &AppendBlockPayload {
                seq: input.seq,
                content: input.content,
                content_hash: input.content_hash,
                // No chunker, no embedder on this surface: the server chunks `content`.
                chunks_packed: None,
                sources,
            },
        )
        .await
        .across_auth(|e| map_err(e, "ingest_append"))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&out)),
    ]))
}

pub async fn ingest_finalize(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: IngestFinalizeInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let resource = parse_resource(&input.resource)?;

    svc.relay_client(parts)?
        .ingest()
        .finalize(
            resource,
            &FinalizePayload {
                expected_blocks: input.expected_blocks,
                expected_body_hash: input.expected_body_hash,
                // MCP is honestly EXEMPT, not covered: its finalize tool never sees the whole body,
                // only per-block content, so it cannot supply a whole-body integrity hash. `None`
                // skips the check — say so rather than imply a guarantee we don't have.
                expected_content_hash: None,
            },
        )
        .await
        .across_auth(|e| map_err(e, "ingest_finalize"))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(format!(
            "Finalized {} ({} blocks).",
            input.resource, input.expected_blocks
        )),
    ]))
}

pub async fn ingest_blocks(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: IngestBlocksInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let resource = parse_resource(&input.resource)?;

    let out = svc
        .relay_client(parts)?
        .ingest()
        .list_blocks(resource)
        .await
        .across_auth(|e| map_err(e, "ingest_blocks"))?;

    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(to_text(&out)),
    ]))
}

// ── Consolidated tool (4→1) ─────────────────────────────────────────────────────

/// The segmented-ingest action to perform.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum IngestAction {
    /// Begin a segmented ingest — land segment 0 and create the resource.
    Begin,
    /// Append one segment to an in-progress ingest.
    Append,
    /// Declare the ingest complete.
    Finalize,
    /// Read back the segments that have landed (for resume after interruption).
    Blocks,
}

/// Consolidated segmented-ingest tool — one write tool with an `action` discriminator.
///
/// Collapses `ingest_begin`, `ingest_append`, `ingest_finalize`, and `ingest_blocks`
/// into a single MCP tool. The `action` field selects which lifecycle step to perform.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SegmentedIngestInput {
    /// Which ingest action to perform.
    pub action: IngestAction,
    /// Every `create_resource` field, plus segmented-session hints. Required for `begin`; ignored otherwise.
    #[serde(flatten)]
    #[serde(default)]
    pub create: Option<CreateResourceInput>,
    /// Bare-hex sha256 of segment 0's content. Required for `begin`; ignored otherwise.
    #[serde(default)]
    pub content_hash: Option<String>,
    /// Best-effort total segment count. Used with `begin`.
    #[serde(default)]
    pub total_blocks_hint: Option<u32>,
    /// Bytes of text this session's segment boundaries were cut at. Used with `begin`.
    #[serde(default)]
    pub block_budget: Option<u32>,
    /// sha256 of the whole source. Used with `begin`.
    #[serde(default)]
    pub source_hash: Option<String>,
    /// Resource ref returned by `ingest_begin`. Required for `append`, `finalize`, `blocks`; ignored for `begin`.
    #[serde(default)]
    pub resource: Option<String>,
    /// Zero-based segment index. Required for `append` (starts at 1).
    #[serde(default)]
    pub seq: Option<u32>,
    /// This segment's markdown text. Required for `append`.
    #[serde(default)]
    pub content: Option<String>,
    /// Optional per-segment provenance sources. Used with `append`.
    #[serde(default)]
    pub sources: Option<Vec<String>>,
    /// Total landed segments counting segment 0. Required for `finalize`.
    #[serde(default)]
    pub expected_blocks: Option<u32>,
    /// The opaque `body_hash` from your most recent append/blocks response. Required for `finalize`.
    #[serde(default)]
    pub expected_body_hash: Option<String>,
}

/// Map a wire-level [`SegmentedIngestInput`] of action `begin` onto the begin
/// handler's input — the requirement arms first, then the wire-collision repair:
/// the consolidated shape carries `content` (and `sources`) as top-level fields for
/// APPEND, and serde binds a caller's segment-0 text to the outer slot because the
/// outer field shares the JSON key with the flattened `create.content` — a begin by
/// the advertised shape therefore arrived with `create.content: None` and refused
/// "ingest_begin requires content" at the surface's integrity check. For begin the
/// outer slot IS the segment text: honor it when the flattened side is absent.
/// Append-begin shared keys arriving on BOTH sides keep the flattened value (never
/// silently overwrite an explicitly nested `create.content`).
fn begin_input_from(input: SegmentedIngestInput) -> Result<IngestBeginInput, rmcp::ErrorData> {
    let SegmentedIngestInput {
        action: _,
        create,
        content_hash,
        block_budget,
        total_blocks_hint,
        source_hash,
        content,
        sources,
        ..
    } = input;
    let mut create = create.ok_or_else(|| {
        rmcp::ErrorData::invalid_params("begin requires create fields".to_string(), None)
    })?;
    if create.content.is_none() {
        create.content = content;
    }
    if create.sources.is_none() {
        create.sources = sources;
    }
    let content_hash = content_hash.ok_or_else(|| {
        rmcp::ErrorData::invalid_params("begin requires `content_hash`".to_string(), None)
    })?;
    Ok(IngestBeginInput {
        create,
        content_hash,
        block_budget,
        total_blocks_hint,
        source_hash,
    })
}

/// Dispatch the consolidated segmented-ingest tool.
pub async fn segmented_ingest(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: SegmentedIngestInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match input.action {
        IngestAction::Begin => ingest_begin(svc, parts, begin_input_from(input)?).await,
        IngestAction::Append => {
            let resource = input.resource.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("append requires `resource`".to_string(), None)
            })?;
            let seq = input.seq.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("append requires `seq`".to_string(), None)
            })?;
            let content = input.content.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("append requires `content`".to_string(), None)
            })?;
            let content_hash = input.content_hash.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("append requires `content_hash`".to_string(), None)
            })?;
            ingest_append(
                svc,
                parts,
                IngestAppendInput {
                    resource,
                    seq,
                    content,
                    content_hash,
                    sources: input.sources,
                },
            )
            .await
        }
        IngestAction::Finalize => {
            let resource = input.resource.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("finalize requires `resource`".to_string(), None)
            })?;
            let expected_blocks = input.expected_blocks.ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    "finalize requires `expected_blocks`".to_string(),
                    None,
                )
            })?;
            let expected_body_hash = input.expected_body_hash.ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    "finalize requires `expected_body_hash`".to_string(),
                    None,
                )
            })?;
            ingest_finalize(
                svc,
                parts,
                IngestFinalizeInput {
                    resource,
                    expected_blocks,
                    expected_body_hash,
                },
            )
            .await
        }
        IngestAction::Blocks => {
            let resource = input.resource.ok_or_else(|| {
                rmcp::ErrorData::invalid_params("blocks requires `resource`".to_string(), None)
            })?;
            ingest_blocks(svc, parts, IngestBlocksInput { resource }).await
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// The consolidated tool's wire shape must route a begin call's `content` into
    /// `create.content` — `#[serde(flatten)] Option<CreateResourceInput>` collects the
    /// leftovers, but the outer `content` field (append's) shares the JSON key, so
    /// raw serde binds it outer and `create.content` arrives None (the defect the
    /// parity suite probed red at author time). The dispatcher's reshape honors the
    /// outer slot as begin's segment text — this pins the repair.
    #[test]
    fn the_consolidated_begin_input_routes_content_into_create() {
        let input: SegmentedIngestInput = serde_json::from_value(serde_json::json!({
            "action": "begin",
            "context_ref": "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
            "doc_type_name": "research",
            "title": "probe",
            "content": "segment zero",
            "content_hash": "deadbeef"
        }))
        .expect("begin input deserializes");
        assert!(
            input
                .create
                .as_ref()
                .expect("create parsed")
                .content
                .is_none(),
            "serde binds the shared key outer — the raw parse is the collision"
        );
        let begin = begin_input_from(input).expect("the reshape answers");
        assert_eq!(begin.create.content.as_deref(), Some("segment zero"));
        assert_eq!(begin.content_hash.as_str(), "deadbeef");
    }
}
