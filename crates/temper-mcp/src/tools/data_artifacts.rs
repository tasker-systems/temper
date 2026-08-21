//! Data artifact tools — visibility-gated reads over `kb_data_artifacts`.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::types::ids::{DataArtifactId, ProfileId, ResourceId};
use temper_services::backend::substrate_read;
use temper_services::error::ApiError;

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
