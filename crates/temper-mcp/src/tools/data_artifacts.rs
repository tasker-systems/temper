//! Data artifact tools — visibility-gated reads and auth-gated writes over
//! `kb_data_artifacts`.
//!
//! Execution crosses the DEPLOYED API over the wire (beat G4 — the last direct
//! cluster): `list` → `GET /api/resources/{id}/artifacts`, `get` →
//! `GET /api/data-artifacts/{artifact_id}` (the flat read, route-first in PR #968 —
//! the tool carries no `resource_id` and the nested route could not be its door),
//! `commit` → `POST /api/resources/{id}/artifacts`. Each call forwards the caller's
//! bearer via the per-request temper-client relay built from the request's `Parts`.
//!
//! The per-act envelope (`confidence` / `persona` / `reasoning` / `rationale` /
//! `model` / `invocation_id` / `correlation_id`) maps STRAIGHT into the wire
//! request's `act` — never defaulted, never dropped (the G3c act-ride discipline).
//!
//! # Declared parity deltas (the direct faces pinned by
//! # `ingest_blobs_artifacts_parity_test.rs`, flipped here deliberately)
//!
//! - **`get` on an absent or invisible artifact**: the direct read answered
//!   200-text ("Artifact not found or not visible to you.") — a success-shaped
//!   absence. The flat route 404s (the leak-safe posture, folded rows included): the
//!   door renders `invalid_params` with the server's own sentence ("artifact not
//!   found").
//!
//! Every other arm maps arm-for-arm: commit's write-access refusal keeps the tool's
//! own sentence under `invalid_params`; the typed `DataArtifactRefusal` 400 travels
//! under its own code and renders `invalid_params` with the refusal's own words
//! (mapped from the DEDICATED client variant, never the generic 400 catch-all);
//! `NotFound` answers the server's sentence bare (the direct map's bare sentences,
//! no prefix to drop — the direct map never prefixed this family's NotFound/BadRequest
//! arms).

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_client::error::ClientError;
use temper_core::types::authorship::ActInput;
use temper_core::types::data_artifact::{
    ArtifactCommitRequest, ArtifactListParams, KindOwnerInput,
};
use temper_core::types::ids::{DataArtifactId, ResourceId};

use crate::service::{api_error_cause, AcrossAuth, TemperMcpService};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListArtifactsInput {
    /// The resource ID (UUID or decorated `slug-<uuid>` form).
    pub resource_id: String,
    /// Filter by the bare family name (e.g. `"measurement"`).
    #[serde(default)]
    pub kind: Option<String>,
    /// Filter by selection intent: `"current"`, `"member"`, or `"pinned"`.
    #[serde(default)]
    pub intent: Option<String>,
    /// Include folded (superseded) artifacts. Default: false.
    #[serde(default)]
    pub include_folded: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetArtifactInput {
    /// The artifact ID (UUID or decorated `slug-<uuid>` form).
    pub artifact_id: String,
}

// ── Tool handlers ──────────────────────────────────────────────────────────────

pub async fn list_artifacts(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: ListArtifactsInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let resource_id = parse_resource_ref(&input.resource_id)?;

    let artifacts = svc
        .relay_client(parts)?
        .data_artifacts()
        .list(
            resource_id.uuid(),
            &ArtifactListParams {
                kind: input.kind,
                intent: input.intent,
                include_folded: input.include_folded,
                counts: None,
            },
        )
        .await
        .across_auth(|e| map_err(e, "list_data_artifacts"))?;

    let json = serde_json::to_string_pretty(&artifacts).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(json),
    ]))
}

pub async fn get_artifact(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: GetArtifactInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let artifact_id = parse_artifact_ref(&input.artifact_id)?;

    let artifact = svc
        .relay_client(parts)?
        .data_artifacts()
        .get_by_id(artifact_id.uuid())
        .await
        .across_auth(|e| match e {
            // The flat route 404s an absent or invisible artifact — the direct read's
            // 200-text posture flips to an error with the server's own sentence
            // (declared in the module header).
            ClientError::NotFound { message } => rmcp::ErrorData::invalid_params(message, None),
            ClientError::ResourceErased { id } => crate::tools::resources::erased_error(id),
            other => map_err(other, "get_data_artifact"),
        })?;

    let json = serde_json::to_string_pretty(&artifact).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(json),
    ]))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CommitArtifactInput {
    /// The resource ID (UUID or decorated `slug-<uuid>` form) that owns the artifact.
    pub resource_id: String,
    /// The bare family name, qualified by `kind_owner` (or defaulted from the resource's home).
    pub kind: String,
    /// Override the namespace half of the family name. Omit to let the server default it.
    #[serde(default)]
    pub kind_owner: Option<KindOwnerInput>,
    /// Selection intent: `"current"`, `"member"`, or `"pinned"`.
    pub intent: String,
    /// Ordering among peers. Meaningful for `member`; carried for all. Default: 0.0.
    #[serde(default)]
    pub precedence: f64,
    /// The structured payload as a JSON value. Hashed and stored verbatim.
    pub content: serde_json::Value,
    /// Artifacts this one replaces, by ID (UUID or decorated `slug-<uuid>` form). Empty = replaces nothing.
    #[serde(default)]
    pub supersedes: Vec<String>,
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

pub async fn commit_artifact(
    svc: &TemperMcpService,
    parts: &http::request::Parts,
    input: CommitArtifactInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let resource_id = parse_resource_ref(&input.resource_id)?;

    let supersedes = input
        .supersedes
        .iter()
        .map(|s| parse_artifact_ref(s))
        .collect::<Result<Vec<_>, _>>()?;

    // The act envelope is built from the input's discrete authorship strings into
    // the wire's typed fields: the confidence band parses HERE (the tool's own
    // vocabulary refusal stays pre-wire), and the request carries the typed act —
    // never defaulted, never dropped (the G3c act-ride discipline).
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

    let request = ArtifactCommitRequest {
        kind: input.kind,
        kind_owner: input.kind_owner,
        intent: input.intent,
        precedence: input.precedence,
        content: input.content,
        supersedes,
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

    let response = svc
        .relay_client(parts)?
        .data_artifacts()
        .commit(resource_id.uuid(), &request)
        .await
        .across_auth(|e| match e {
            // The read-on-write commit against an unwritable resource keeps the tool's own
            // write-access sentence under invalid_params — the door's 403 renders bare
            // otherwise, and an agent repairing an auth gate reads the capability it lacks.
            ClientError::Forbidden | ClientError::ForbiddenDetail { .. } => {
                rmcp::ErrorData::invalid_params(
                    "Not authorized to commit artifacts to this resource: write access required."
                        .to_string(),
                    None,
                )
            }
            other => map_err(other, "commit_data_artifact"),
        })?;

    let json = serde_json::to_string_pretty(&response).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(json),
    ]))
}

fn parse_resource_ref(s: &str) -> Result<ResourceId, rmcp::ErrorData> {
    temper_core::refs::parse_ref(s)
        .map(|p| ResourceId::from(p.uuid()))
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))
}

fn parse_artifact_ref(s: &str) -> Result<DataArtifactId, rmcp::ErrorData> {
    let s = s.trim();
    if let Ok(id) = Uuid::parse_str(s) {
        return Ok(DataArtifactId::from(id));
    }
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() >= 5 {
        let tail = parts[parts.len() - 5..].join("-");
        if let Ok(id) = Uuid::parse_str(&tail) {
            return Ok(DataArtifactId::from(id));
        }
    }
    Err(rmcp::ErrorData::invalid_params(
        format!("not an artifact ref (expected a UUID or `slug-<uuid>`): {s:?}"),
        None,
    ))
}

/// Wire refusals to rmcp. See the module header for the declared delta and the arm
/// table's sources.
fn map_err(e: ClientError, action: &str) -> rmcp::ErrorData {
    match e {
        ClientError::NotFound { message } => rmcp::ErrorData::invalid_params(message, None),
        ClientError::ResourceErased { id } => crate::tools::resources::erased_error(id),
        ClientError::Server {
            status: 400,
            message,
        } => rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None),
        ClientError::Conflict { message } => {
            rmcp::ErrorData::invalid_params(api_error_cause(&message).to_string(), None)
        }
        // The DEDICATED wire refusal (the 400 whose code is
        // `DATA_ARTIFACT_REFUSAL_CODE`): the refusal's own words are the message —
        // the SQL wrapper's vocabulary or the enforcing-shape verdict's detail, never
        // the catch-all.
        ClientError::DataArtifactRefusal { message } => {
            rmcp::ErrorData::invalid_params(message, None)
        }
        other => rmcp::ErrorData::internal_error(format!("{action}: {other}"), None),
    }
}
