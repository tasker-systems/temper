//! Data-artifact shape registry tools — visibility-gated reads and
//! authority-gated declares over `kb_data_artifact_shapes`.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::types::data_artifact::KindOwnerInput;
use temper_core::types::data_artifact_shape::{EnforcementMode, ShapeView};
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{CogmapId, ContextId, ProfileId, ShapeId};
use temper_services::backend::substrate_read;
use temper_services::error::ApiError;
use temper_services::services::shape_service::{self, DeclareShapeServiceParams};
use temper_substrate::payloads::{
    AnchorRef, EnforcementMode as SubstrateEnforcementMode, KindOwner,
};

use crate::service::TemperMcpService;

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

pub async fn list_shapes(
    svc: &TemperMcpService,
    input: ListShapesInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;

    let anchor = parse_home_anchor(&input.home_type, &input.home_id)?;

    let shapes = substrate_read::list_shapes(pool, ProfileId::from(profile.id), anchor)
        .await
        .map_err(map_api_err)?;

    let json = serde_json::to_string_pretty(&shapes).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        json,
    )]))
}

pub async fn get_shape(
    svc: &TemperMcpService,
    input: GetShapeInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;

    let shape_id = parse_shape_ref(&input.shape_id)?;

    let shape = substrate_read::get_shape(pool, ProfileId::from(profile.id), shape_id)
        .await
        .map_err(map_api_err)?;

    match shape {
        Some(s) => {
            let json = serde_json::to_string_pretty(&s).unwrap_or_else(|_| "{}".to_string());
            Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                json,
            )]))
        }
        None => Ok(CallToolResult::success(vec![rmcp::model::Content::text(
            "Shape not found or not visible to you.".to_string(),
        )])),
    }
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
    input: DeclareShapeInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    let anchor = parse_home_anchor(&input.home_type, &input.home_id)?;
    let home = match anchor {
        HomeAnchor::Context(id) => AnchorRef::context(id),
        HomeAnchor::Cogmap(id) => AnchorRef::cogmap(id),
    };

    let kind_owner = input.kind_owner.map(|ko| match ko {
        KindOwnerInput::Profile(id) => KindOwner::Profile(id),
        KindOwnerInput::Team(id) => KindOwner::Team(id),
    });

    let enforcement = match input.enforcement {
        EnforcementMode::Advisory => SubstrateEnforcementMode::Advisory,
        EnforcementMode::Enforcing => SubstrateEnforcementMode::Enforcing,
    };

    let emitter = temper_substrate::writes::resolve_emitter(pool, profile_id, "mcp")
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("failed to resolve emitter: {e}"), None)
        })?;

    let shape_id = shape_service::declare_shape(
        pool,
        DeclareShapeServiceParams {
            home,
            kind: &input.kind,
            kind_owner,
            schema: &input.schema,
            enforcement,
            principal: profile_id,
            emitter,
        },
    )
    .await
    .map_err(|e| match e {
        ApiError::Forbidden => rmcp::ErrorData::invalid_params(
            "Not authorized to declare shapes in this home: authoring authority required."
                .to_string(),
            None,
        ),
        ApiError::NotFound(msg) => rmcp::ErrorData::invalid_params(msg, None),
        ApiError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
        other => rmcp::ErrorData::internal_error(format!("Failed to declare shape: {other}"), None),
    })?;

    let shape: Option<ShapeView> = substrate_read::get_shape(pool, profile_id, shape_id)
        .await
        .map_err(map_api_err)?;

    let shape = shape.ok_or_else(|| {
        rmcp::ErrorData::internal_error("shape declared but not retrievable".to_string(), None)
    })?;

    let json = serde_json::to_string_pretty(&shape).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        json,
    )]))
}

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
    let parts: Vec<&str> = s.split('-').collect();
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

fn map_api_err(e: ApiError) -> rmcp::ErrorData {
    rmcp::ErrorData::internal_error(e.to_string(), None)
}
