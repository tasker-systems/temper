//! Data artifact tools — visibility-gated reads and auth-gated writes over
//! `kb_data_artifacts`.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::types::authorship::ActInput;
use temper_core::types::data_artifact::KindOwnerInput;
use temper_core::types::ids::{DataArtifactId, ProfileId, ResourceId};
use temper_services::backend::{substrate_read, DbBackend};
use temper_services::error::ApiError;
use temper_workflow::operations::{Backend, CommitDataArtifact, Surface};

use crate::service::TemperMcpService;

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

pub async fn list_artifacts(
    svc: &TemperMcpService,
    input: ListArtifactsInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;

    let resource_id = parse_resource_ref(&input.resource_id)?;

    let artifacts = temper_services::backend::substrate_read::list_artifacts(
        pool,
        ProfileId::from(profile.id),
        resource_id,
        input.kind.as_deref(),
        input.intent.as_deref(),
        input.include_folded.unwrap_or(false),
    )
    .await
    .map_err(map_api_err)?;

    let json = serde_json::to_string_pretty(&artifacts).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        json,
    )]))
}

pub async fn get_artifact(
    svc: &TemperMcpService,
    input: GetArtifactInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;

    let artifact_id = parse_artifact_ref(&input.artifact_id)?;

    let artifact = substrate_read::get_artifact(pool, ProfileId::from(profile.id), artifact_id)
        .await
        .map_err(map_api_err)?;

    match artifact {
        Some(a) => {
            let json = serde_json::to_string_pretty(&a).unwrap_or_else(|_| "{}".to_string());
            Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                json,
            )]))
        }
        None => Ok(CallToolResult::success(vec![rmcp::model::Content::text(
            "Artifact not found or not visible to you.".to_string(),
        )])),
    }
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
    input: CommitArtifactInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    let resource_id = parse_resource_ref(&input.resource_id)?;

    let supersedes = input
        .supersedes
        .iter()
        .map(|s| parse_artifact_ref(s))
        .collect::<Result<Vec<_>, _>>()?;

    let act = ActInput {
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
        confidence: match input.confidence.as_deref() {
            Some("tentative") => Some(temper_core::types::authorship::ConfidenceBand::Tentative),
            Some("probable") => Some(temper_core::types::authorship::ConfidenceBand::Probable),
            Some("confident") => Some(temper_core::types::authorship::ConfidenceBand::Confident),
            Some(other) => {
                return Err(rmcp::ErrorData::invalid_params(
                    format!(
                        "unrecognized confidence '{other}'; expected tentative|probable|confident"
                    ),
                    None,
                ))
            }
            None => None,
        },
        reasoning: input.reasoning,
        rationale: input.rationale,
        persona: input.persona,
        model: input.model,
    };

    let act_ctx = act
        .into_act_context()
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    let cmd = CommitDataArtifact {
        resource: resource_id,
        kind: input.kind,
        kind_owner: input.kind_owner,
        intent: input.intent,
        precedence: input.precedence,
        content: input.content,
        supersedes,
        act: act_ctx,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id);
    let out = backend
        .commit_data_artifact(cmd)
        .await
        .map_err(|e| match e {
            temper_core::error::TemperError::Forbidden
            | temper_core::error::TemperError::ForbiddenDetail(_) => {
                rmcp::ErrorData::invalid_params(
                    "Not authorized to commit artifacts to this resource: write access required."
                        .to_string(),
                    None,
                )
            }
            temper_core::error::TemperError::NotFound(msg) => {
                rmcp::ErrorData::invalid_params(msg, None)
            }
            temper_core::error::TemperError::BadRequest(msg) => {
                rmcp::ErrorData::invalid_params(msg, None)
            }
            other => {
                rmcp::ErrorData::internal_error(format!("Failed to commit artifact: {other}"), None)
            }
        })?;

    let response = temper_core::types::data_artifact::ArtifactCommitResponse {
        artifact_id: out.value.artifact_id,
        artifact: out.value,
    };
    let json = serde_json::to_string_pretty(&response).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        json,
    )]))
}

fn parse_resource_ref(s: &str) -> Result<ResourceId, rmcp::ErrorData> {
    temper_core::refs::parse_ref(s)
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

fn map_api_err(e: ApiError) -> rmcp::ErrorData {
    rmcp::ErrorData::internal_error(e.to_string(), None)
}
