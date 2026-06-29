//! Cognitive-map tools. Reads: `cogmap_shape` (surface tier / materialized regions),
//! `cogmap_region_metrics`, `cogmap_analytics`. Write: `cogmap_create` (genesis — create a new map).

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_api::backend::DbBackend;
use temper_api::error::ApiError;
use temper_api::services::{access_service, cogmap_service};
use temper_core::error::TemperError;
use temper_core::types::cognitive_maps::{
    BindTeamRequest, CogmapAnalyticsInput, CogmapRegionMetricsInput, CogmapShapeInput,
};
use temper_core::types::ids::ProfileId;
use temper_core::types::reconcile::CreateCogmapRequest;
use temper_workflow::operations::{Backend, CreateCognitiveMap, Surface};

use crate::service::TemperMcpService;

pub async fn cogmap_shape(
    svc: &TemperMcpService,
    input: CogmapShapeInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    // Resolve refs → UUIDs (trailing-UUID-only; slug half ignored). Use the same resolver the CLI uses.
    let cogmap_id = temper_workflow::operations::parse_ref(&input.cogmap)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad cogmap ref: {e}"), None))?
        .0;
    let lens_id = match input.lens.as_deref() {
        Some(l) => Some(
            temper_workflow::operations::parse_ref(l)
                .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad lens ref: {e}"), None))?
                .0,
        ),
        None => None,
    };

    let rows = temper_api::backend::substrate_read::cogmap_shape_select(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        cogmap_id,
        lens_id,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("cogmap_shape failed: {e}"), None))?;

    let text = serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

pub async fn cogmap_region_metrics(
    svc: &TemperMcpService,
    input: CogmapRegionMetricsInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let cogmap_id = temper_workflow::operations::parse_ref(&input.cogmap)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad cogmap ref: {e}"), None))?
        .0;
    let lens_id = match input.lens.as_deref() {
        Some(l) => Some(
            temper_workflow::operations::parse_ref(l)
                .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad lens ref: {e}"), None))?
                .0,
        ),
        None => None,
    };

    let rows = temper_api::backend::substrate_read::cogmap_region_metrics_select(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        cogmap_id,
        lens_id,
    )
    .await
    .map_err(|e| {
        rmcp::ErrorData::internal_error(format!("cogmap_region_metrics failed: {e}"), None)
    })?;

    let text = serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

pub async fn cogmap_analytics(
    svc: &TemperMcpService,
    input: CogmapAnalyticsInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let cogmap_id = temper_workflow::operations::parse_ref(&input.cogmap)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad cogmap ref: {e}"), None))?
        .0;

    let got = temper_api::backend::substrate_read::cogmap_analytics_select(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        cogmap_id,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("cogmap_analytics failed: {e}"), None))?;

    match got {
        Some(analytics) => {
            let text =
                serde_json::to_string_pretty(&analytics).unwrap_or_else(|_| "{}".to_string());
            Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                text,
            )]))
        }
        None => Err(rmcp::ErrorData::invalid_params(
            "cognitive map not found or not readable".to_string(),
            None,
        )),
    }
}

// ── cogmap_create (genesis) ──────────────────────────────────────────────────

/// MCP input for cogmap_create (genesis). The MCP surface creates the map with an EMPTY charter — the
/// charter is authored prose that must be embedded client-side, and the MCP server is embed-free by
/// design (mirroring the cogmap reconcile write path). Deliver the charter afterwards with
/// `temper cogmap reconcile` (which embeds client-side).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CogmapCreateInput {
    /// The new cognitive map's name.
    pub name: String,
    /// The telos charter resource's title.
    pub telos_title: String,
    /// Optional explicit cogmap id (uuidv7). Absent ⇒ the server mints one. Supplying it makes genesis
    /// reproducible and idempotent (re-creating at the same id is a no-op).
    #[serde(default)]
    pub cogmap_id: Option<Uuid>,
    /// Optional explicit telos charter resource id (uuidv7). Absent ⇒ the server mints one.
    #[serde(default)]
    pub telos_resource_id: Option<Uuid>,
}

/// Genesis (create) a new cognitive map. AUTH BEFORE WRITE: genesis is system-admin-only — the gate
/// lives on the surface (the backend command does not gate), so the MCP tool checks `is_system_admin`
/// here, mirroring the HTTP handler. The map is born with an EMPTY charter (see [`CogmapCreateInput`]).
pub async fn cogmap_create(
    svc: &TemperMcpService,
    input: CogmapCreateInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    // Auth before write: genesis is system-admin-only.
    let is_admin = access_service::is_system_admin(pool, profile_id)
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("admin check failed: {e}"), None))?;
    if !is_admin {
        return Err(rmcp::ErrorData::invalid_params(
            "cognitive-map genesis requires system-admin".to_string(),
            None,
        ));
    }

    let cmd = CreateCognitiveMap {
        request: CreateCogmapRequest {
            cogmap_id: input.cogmap_id,
            telos_resource_id: input.telos_resource_id,
            name: input.name,
            telos_title: input.telos_title,
            // Empty charter — the MCP server is embed-free; deliver the charter via reconcile.
            telos: None,
        },
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(pool.clone(), profile_id);
    let out = backend
        .create_cognitive_map(cmd)
        .await
        .map_err(|e| match e {
            TemperError::Forbidden => rmcp::ErrorData::invalid_params(
                "not authorized to create cognitive maps".to_string(),
                None,
            ),
            TemperError::Conflict(msg) => rmcp::ErrorData::invalid_params(msg, None),
            TemperError::BadRequest(msg) => rmcp::ErrorData::invalid_params(msg, None),
            other => rmcp::ErrorData::internal_error(
                format!("Failed to create cognitive map: {other}"),
                None,
            ),
        })?;

    let text = serde_json::to_string_pretty(&out.value).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

// ── cogmap_bind / cogmap_unbind (service-direct) ─────────────────────────────

/// MCP input for cogmap_bind / cogmap_unbind. `cogmap` is a ref (UUID or decorated `slug-<uuid>`);
/// `team_id` is the team's raw UUID (id-based, mirroring the HTTP wire shape).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CogmapBindInput {
    /// The cognitive map to bind, by ref (UUID or `slug-<uuid>`).
    pub cogmap: String,
    /// The team's UUID.
    pub team_id: Uuid,
}

/// Map a service `ApiError` to an rmcp protocol error. `Forbidden` ⇒ invalid_params (the admin gate);
/// everything else ⇒ internal_error.
fn map_api_error(context: &str, err: ApiError) -> rmcp::ErrorData {
    match err {
        ApiError::Forbidden => {
            rmcp::ErrorData::invalid_params(format!("{context} requires system-admin"), None)
        }
        other => rmcp::ErrorData::internal_error(format!("{context} failed: {other}"), None),
    }
}

/// Bind a cognitive map to a team. SERVICE-DIRECT (binding is not a Backend command) — calls
/// `cogmap_service::bind_team` directly, which enforces the `is_system_admin` gate before any write.
pub async fn cogmap_bind(
    svc: &TemperMcpService,
    input: CogmapBindInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let cogmap_id = temper_workflow::operations::parse_ref(&input.cogmap)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad cogmap ref: {e}"), None))?
        .0;

    let outcome = cogmap_service::bind_team(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        cogmap_id,
        &BindTeamRequest {
            team_id: input.team_id,
        },
    )
    .await
    .map_err(|e| map_api_error("cogmap_bind", e))?;

    let text = serde_json::to_string_pretty(&outcome).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

/// Unbind a cognitive map from a team. SERVICE-DIRECT, admin-gated (see [`cogmap_bind`]).
pub async fn cogmap_unbind(
    svc: &TemperMcpService,
    input: CogmapBindInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let cogmap_id = temper_workflow::operations::parse_ref(&input.cogmap)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad cogmap ref: {e}"), None))?
        .0;

    let outcome = cogmap_service::unbind_team(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        cogmap_id,
        input.team_id,
    )
    .await
    .map_err(|e| map_api_error("cogmap_unbind", e))?;

    let text = serde_json::to_string_pretty(&outcome).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cogmap_bind_input_deserializes() {
        let id = Uuid::now_v7();
        let raw = serde_json::json!({ "cogmap": "m", "team_id": id.to_string() });
        let input: CogmapBindInput = serde_json::from_value(raw).expect("bind input");
        assert_eq!(input.cogmap, "m");
        assert_eq!(input.team_id, id);
    }

    #[test]
    fn cogmap_create_input_deserializes_minimal() {
        let raw = serde_json::json!({ "name": "M", "telos_title": "T" });
        let input: CogmapCreateInput = serde_json::from_value(raw).expect("minimal input");
        assert_eq!(input.name, "M");
        assert_eq!(input.telos_title, "T");
        assert!(input.cogmap_id.is_none());
        assert!(input.telos_resource_id.is_none());
    }

    #[test]
    fn cogmap_create_input_accepts_explicit_ids() {
        let id = Uuid::now_v7();
        let raw = serde_json::json!({
            "name": "M", "telos_title": "T",
            "cogmap_id": id.to_string(),
        });
        let input: CogmapCreateInput = serde_json::from_value(raw).expect("input with id");
        assert_eq!(input.cogmap_id, Some(id));
    }
}
