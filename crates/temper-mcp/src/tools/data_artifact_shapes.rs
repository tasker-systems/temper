//! Data-artifact shape registry tools — visibility-gated reads and
//! authority-gated declares over `kb_data_artifact_shapes`.
//!
//! Execution crosses the DEPLOYED API over the wire (beat G4 — the last direct
//! cluster): `list` / `declare` cross by home — a context home rides
//! `GET|POST /api/contexts/{id}/shapes`, a cogmap home rides
//! `GET|POST /api/cognitive-maps/{id}/shapes` (the route-first pair, PR #968 —
//! until it the cogmap arm had no wire door); `get` rides `GET /api/shapes/{shape_id}`
//! on either home. Each call forwards the caller's bearer via the per-request
//! temper-client relay built from the request's `Parts`, and touches the pool nowhere.
//!
//! The emitter the direct binding passed explicitly (`resolve_emitter(…, "mcp")`)
//! is the request's resolved surface now: both declare routes read `RequestSurface`,
//! so a declare through this door attributes the caller's own `@mcp`.
//!
//! # Declared parity deltas (the direct faces pinned by
//! # `ingest_blobs_artifacts_parity_test.rs`, flipped here deliberately)
//!
//! - **`get` on an absent or invisible shape**: the direct read answered 200-text
//!   ("Shape not found or not visible to you."). The route 404s: the door renders
//!   `invalid_params` with the server's own sentence ("shape not found").
//!
//! Every other arm maps arm-for-arm: the direct map's bare NotFound/BadRequest
//! (the `invalid_params` arms carried no prefix) and the declare authority gate's
//! tool-voiced sentence ("Not authorized to declare shapes in this home: authoring
//! authority required.") keep their shapes through the door.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_client::error::ClientError;
use temper_core::types::authorship::ActInput;
use temper_core::types::data_artifact::KindOwnerInput;
use temper_core::types::data_artifact_shape::{EnforcementMode, ShapeDeclareRequest};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{CogmapId, ContextId, ShapeId};

use crate::service::{api_error_cause, AcrossAuth, TemperMcpService};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListShapesInput {
    /// The home anchor type: `"context"` or `"cogmap"`.
    pub home_type: String,
    /// The home anchor ID (UUID or decorated `slug-<uuid>` form).
    pub home_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetShapeInput {
    /// The shape ID (UUID or decorated `slug-<uuid>` form).
    pub shape_id: String,
}

// ── Tool handlers ──────────────────────────────────────────────────────────────

pub async fn list_shapes(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: ListShapesInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let anchor = parse_home_anchor(&input.home_type, &input.home_id)?;

    let shapes = svc
        .relay_client(parts)?
        .data_artifacts()
        .list_for(anchor)
        .await
        .across_auth(|e| map_err(e, "list_data_artifact_shapes"))?;

    let json = serde_json::to_string_pretty(&shapes).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(json),
    ]))
}

pub async fn get_shape(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: GetShapeInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let shape_id = parse_shape_ref(&input.shape_id)?;

    let shape = svc
        .relay_client(parts)?
        .data_artifacts()
        .get_shape(shape_id.uuid())
        .await
        .across_auth(|e| match e {
            // The route 404s an absent or invisible shape — the direct read's 200-text
            // posture flips to an error carrying the server's sentence (declared in the
            // module header).
            ClientError::NotFound { message } => rmcp::ErrorData::invalid_params(message, None),
            other => map_err(other, "get_data_artifact_shape"),
        })?;

    let json = serde_json::to_string_pretty(&shape).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(json),
    ]))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeclareShapeInput {
    /// The home anchor type: `"context"` or `"cogmap"`.
    pub home_type: String,
    /// The home anchor ID (UUID or decorated `slug-<uuid>` form).
    pub home_id: String,
    /// The bare family name, qualified by `kind_owner` (or defaulted from the home).
    pub kind: String,
    /// Override the namespace half of the family name. Omit to let the server default it.
    #[serde(default)]
    pub kind_owner: Option<KindOwnerInput>,
    /// The JSON Schema (draft 2020-12) governing this family. Validated Rust-side.
    pub schema: serde_json::Value,
    /// Whether a non-conforming commit is refused (`"enforcing"`) or merely recorded (`"advisory"`).
    pub enforcement: EnforcementMode,
    /// Correlate this act with an open invocation envelope (its UUID).
    #[serde(default)]
    pub invocation_id: Option<String>,
    /// Stitch this write into an act-grain thread (a bare UUID you mint).
    #[serde(default)]
    pub correlation_id: Option<String>,
    /// Graded authorship confidence: `"tentative"`, `"probable"`, or `"confident"`.
    #[serde(default)]
    pub confidence: Option<String>,
    /// Free-text reasoning for the act (requires confidence).
    #[serde(default)]
    pub reasoning: Option<String>,
    /// Structured rationale for the act (requires confidence).
    #[serde(default)]
    pub rationale: Option<String>,
    /// Persona/role the author acted as (requires confidence).
    #[serde(default)]
    pub persona: Option<String>,
    /// Model that authored the act (requires confidence).
    #[serde(default)]
    pub model: Option<String>,
}

pub async fn declare_shape(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: DeclareShapeInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let anchor = parse_home_anchor(&input.home_type, &input.home_id)?;

    // The act envelope assembles into the wire's typed act — never defaulted, never
    // dropped (the G3c act-ride discipline). The route accepts the envelope today;
    // that it does not yet reach the declare event is the separately-filed defect
    // task 01a0e2f0-5bdc-7b00-94d2-0fbf9d141df0 — BOTH doors drop it, so parity
    // holds with the defect inherited.
    let confidence = match input.confidence.as_deref() {
        Some("tentative") => Some(temper_core::types::authorship::ConfidenceBand::Tentative),
        Some("probable") => Some(temper_core::types::authorship::ConfidenceBand::Probable),
        Some("confident") => Some(temper_core::types::authorship::ConfidenceBand::Confident),
        Some(other) => {
            return Err(rmcp::ErrorData::invalid_params(
                format!("unrecognized confidence '{other}'; expected tentative|probable|confident"),
                None,
            ))
        }
        None => None,
    };

    let request = ShapeDeclareRequest {
        kind: input.kind,
        kind_owner: input.kind_owner,
        schema: input.schema,
        enforcement: input.enforcement,
        act: ActInput {
            invocation_id: input
                .invocation_id
                .as_deref()
                .map(|s| {
                    temper_core::refs::parse_ref(s)
                        .map(|id| temper_core::types::ids::InvocationId::from(id.0))
                })
                .transpose()
                .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?,
            correlation_id: input
                .correlation_id
                .as_deref()
                .map(|s| {
                    temper_core::refs::parse_ref(s)
                        .map(|id| temper_core::types::ids::CorrelationId::from(id.0))
                })
                .transpose()
                .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?,
            confidence,
            reasoning: input.reasoning,
            rationale: input.rationale,
            persona: input.persona,
            model: input.model,
        },
    };

    let shape = svc
        .relay_client(parts)?
        .data_artifacts()
        .declare_for(anchor, &request)
        .await
        .across_auth(|e| match e {
            // The authority gate ("a caller who cannot author the home is refused with
            // 403") keeps the tool's own requirement sentence — the G3c disclosure
            // dialect: a principal with read standing reads back what they would need.
            ClientError::Forbidden => rmcp::ErrorData::invalid_params(
                "Not authorized to declare shapes in this home: authoring authority required."
                    .to_string(),
                None,
            ),
            other => map_err(other, "declare_data_artifact_shape"),
        })?;

    let json = serde_json::to_string_pretty(&shape).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(json),
    ]))
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Resolve a home anchor from the input's two fields (`home_type` vocabulary
/// guard at the parse callsite). The result feeds the client's keyed dispatch
/// (`data_artifacts().list_for/declare_for` below).
fn parse_home_anchor(home_type: &str, home_id: &str) -> Result<HomeAnchor, rmcp::ErrorData> {
    let id = parse_uuid_ref(home_id)?;
    match home_type {
        "context" => Ok(HomeAnchor::Context(ContextId::from(id))),
        "cogmap" => Ok(HomeAnchor::Cogmap(CogmapId::from(id))),
        other => Err(rmcp::ErrorData::invalid_params(
            format!("unrecognized home_type '{other}'; expected 'context' or 'cogmap'"),
            None,
        )),
    }
}

fn parse_uuid_ref(s: &str) -> Result<Uuid, rmcp::ErrorData> {
    let s = s.trim();
    if let Ok(id) = Uuid::parse_str(s) {
        return Ok(id);
    }
    let parts = s.split('-').collect::<Vec<_>>();
    if parts.len() >= 5 {
        let tail = parts[parts.len() - 5..].join("-");
        if let Ok(id) = Uuid::parse_str(&tail) {
            return Ok(id);
        }
    }
    Err(rmcp::ErrorData::invalid_params(
        format!("not a UUID or `slug-<uuid>`: {s:?}"),
        None,
    ))
}

fn parse_shape_ref(s: &str) -> Result<ShapeId, rmcp::ErrorData> {
    Ok(ShapeId::from(parse_uuid_ref(s)?))
}

fn map_err(e: ClientError, action: &str) -> rmcp::ErrorData {
    match e {
        ClientError::NotFound { message } => rmcp::ErrorData::invalid_params(message, None),
        ClientError::Server {
            status: 400,
            message,
        } => rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None),
        ClientError::Conflict { message } => {
            rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None)
        }
        // The dedicated wire refusal: the SQL wrapper's own vocabulary or the
        // enforcing-shape verdict's detail — the caller sees the refusal, not an
        // internal error.
        ClientError::DataArtifactRefusal { message } => {
            rmcp::ErrorData::invalid_params(message, None)
        }
        other => rmcp::ErrorData::internal_error(format!("{action}: {other}"), None),
    }
}
