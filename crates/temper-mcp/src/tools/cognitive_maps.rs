//! Cognitive-map tools. Reads: `cogmap_shape` (surface tier / materialized regions),
//! `cogmap_region_metrics`, `cogmap_analytics`. Write: `cogmap_create` (genesis — create a new map).

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::types::cognitive_maps::{
    BindTeamRequest, CogmapAnalyticsInput, CogmapRegionMetricsInput, CogmapShapeInput,
    GrantCapabilityRequest, RevokeCapabilityRequest,
};
use temper_core::types::ids::{CogmapId, ProfileId};
use temper_core::types::materialize::{
    MaterializeAck, MaterializeDeltaInput, MaterializeTriggerInput,
};
use temper_core::types::reconcile::CreateCogmapRequest;
use temper_services::backend::DbBackend;
use temper_services::error::ApiError;
use temper_services::services::{access_service, cogmap_service, materialize_service};
use temper_workflow::operations::{Backend, CreateCognitiveMap, MaterializeOnThreshold, Surface};

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

    let rows = temper_services::backend::substrate_read::cogmap_shape_select(
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

    let rows = temper_services::backend::substrate_read::cogmap_region_metrics_select(
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

    let got = temper_services::backend::substrate_read::cogmap_analytics_select(
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

/// MCP input for `cogmap_read_charter`. `cogmap` is a ref (UUID or decorated `slug-<uuid>`).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CogmapReadCharterInput {
    /// The cognitive map whose telos/charter to read, by ref (UUID or `slug-<uuid>`).
    pub cogmap: String,
}

/// Read a cognitive map's telos/charter blocks (statement / questions / framing) in seq order — the
/// steward orients on this before acting. Service-direct (reads bypass the Backend trait); the access
/// gate lives in the SQL (`cogmap_charter_select`) — a principal who cannot read the charter resource
/// gets an empty vec, never an error.
pub async fn cogmap_read_charter(
    svc: &TemperMcpService,
    input: CogmapReadCharterInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;

    let cogmap_id = temper_workflow::operations::parse_ref(&input.cogmap)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad cogmap ref: {e}"), None))?
        .0;

    let rows = temper_services::backend::substrate_read::cogmap_charter_select(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        cogmap_id,
    )
    .await
    .map_err(|e| {
        rmcp::ErrorData::internal_error(format!("cogmap_read_charter failed: {e}"), None)
    })?;

    let text = serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
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

/// Genesis (create) a new cognitive map. Any authenticated profile may create a NON-RESERVED map and
/// becomes its grant-holder (the backend mints a read+write+grant on the new map). The reserved-id
/// guard lives in the backend: a caller-supplied `cogmap_id`/`telos_resource_id` is honored only for a
/// system-admin, so a non-admin can never place a map at a chosen (e.g. reserved) id. The map is born
/// with an EMPTY charter (see [`CogmapCreateInput`]).
pub async fn cogmap_create(
    svc: &TemperMcpService,
    input: CogmapCreateInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

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

// ── cogmap_materialize_delta (read) / cogmap_materialize (trigger) ───────────

/// Read a cogmap's materialize delta: how many formation events have landed since the last
/// materialize, and whether that clears the threshold. Service-direct (gates on
/// `anchor_readable_by_profile`).
pub async fn cogmap_materialize_delta(
    svc: &TemperMcpService,
    input: MaterializeDeltaInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let cogmap = temper_workflow::operations::parse_ref(&input.cogmap)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad cogmap ref: {e}"), None))?
        .0;

    let delta = materialize_service::materialize_delta(
        &svc.api_state.pool,
        ProfileId::from(profile.id),
        CogmapId::from(cogmap),
        input.threshold,
    )
    .await
    .map_err(|e| map_api_error("cogmap_materialize_delta", e))?;

    let text = serde_json::to_string_pretty(&delta).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

/// Re-materialize a cogmap's regions when its formation delta clears the threshold; a no-op below.
/// Dispatches through `DbBackend` (auth-before-write + the threshold gate live there).
///
/// CLI equivalent: `temper cogmap materialize <ref> [--threshold N]`.
pub async fn cogmap_materialize(
    svc: &TemperMcpService,
    input: MaterializeTriggerInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let cogmap = temper_workflow::operations::parse_ref(&input.cogmap)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad cogmap ref: {e}"), None))?
        .0;

    let cmd = MaterializeOnThreshold {
        cogmap: CogmapId::from(cogmap),
        threshold: input.threshold,
        origin: Surface::Mcp,
    };

    let backend = DbBackend::new(svc.api_state.pool.clone(), ProfileId::from(profile.id));
    let out = backend
        .materialize_on_threshold(cmd)
        .await
        .map_err(|e| match e {
            TemperError::Forbidden => rmcp::ErrorData::invalid_params(
                "cogmap_materialize: cannot author this cognitive map".to_string(),
                None,
            ),
            TemperError::NotFound(_) => rmcp::ErrorData::invalid_params(
                "cogmap_materialize: cognitive map not found".to_string(),
                None,
            ),
            other => rmcp::ErrorData::internal_error(format!("cogmap_materialize: {other}"), None),
        })?;

    let ack: MaterializeAck = out.value;
    let text = serde_json::to_string_pretty(&ack).unwrap_or_else(|_| "{}".to_string());
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

// ── cogmap_grant / cogmap_revoke (service-direct) ────────────────────────────

/// MCP input for cogmap_grant. `cogmap` is a ref; exactly one of `to_profile`/`to_team` names the
/// principal (raw UUID). Capability flags select which rights to grant (`read` is implied by
/// `write`/`grant` — coherence). At least one capability must be set.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CogmapGrantInput {
    /// The cognitive map, by ref (UUID or `slug-<uuid>`).
    pub cogmap: String,
    /// Grant to this profile (UUID). Mutually exclusive with `to_team`.
    #[serde(default)]
    pub to_profile: Option<Uuid>,
    /// Grant to this team (UUID). Mutually exclusive with `to_profile`.
    #[serde(default)]
    pub to_team: Option<Uuid>,
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub write: bool,
    #[serde(default)]
    pub grant: bool,
}

/// MCP input for cogmap_revoke. `cogmap` is a ref; exactly one of `from_profile`/`from_team` names
/// the principal whose grant to delete.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CogmapRevokeInput {
    pub cogmap: String,
    #[serde(default)]
    pub from_profile: Option<Uuid>,
    #[serde(default)]
    pub from_team: Option<Uuid>,
}

/// Resolve exactly one of (profile, team) into a `(principal_table, principal_id)` pair.
fn resolve_principal(
    profile: Option<Uuid>,
    team: Option<Uuid>,
) -> Result<(String, Uuid), rmcp::ErrorData> {
    match (profile, team) {
        (Some(p), None) => Ok(("kb_profiles".to_string(), p)),
        (None, Some(t)) => Ok(("kb_teams".to_string(), t)),
        (Some(_), Some(_)) => Err(rmcp::ErrorData::invalid_params(
            "supply exactly one principal, not both a profile and a team".to_string(),
            None,
        )),
        (None, None) => Err(rmcp::ErrorData::invalid_params(
            "no principal — supply exactly one of a profile or a team".to_string(),
            None,
        )),
    }
}

/// Grant a capability on a cognitive map. SERVICE-DIRECT, gated by `is_system_admin OR can_grant`
/// (see `access_service::grant_capability`). `read` is forced on when `write`/`grant` is set
/// (coherence: you cannot write/grant what you cannot read).
pub async fn cogmap_grant(
    svc: &TemperMcpService,
    input: CogmapGrantInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let cogmap_id = temper_workflow::operations::parse_ref(&input.cogmap)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad cogmap ref: {e}"), None))?
        .0;
    let (principal_table, principal_id) = resolve_principal(input.to_profile, input.to_team)?;

    if !(input.read || input.write || input.grant) {
        return Err(rmcp::ErrorData::invalid_params(
            "no capability selected — set at least one of read/write/grant".to_string(),
            None,
        ));
    }
    let req = GrantCapabilityRequest {
        subject_table: "kb_cogmaps".to_string(),
        subject_id: cogmap_id,
        principal_table,
        principal_id,
        can_read: input.read || input.write || input.grant, // coherence: write|grant ⇒ read
        can_write: input.write,
        can_delete: false,
        can_grant: input.grant,
    };

    let outcome =
        access_service::grant_capability(&svc.api_state.pool, ProfileId::from(profile.id), &req)
            .await
            .map_err(|e| map_api_error("cogmap_grant", e))?;

    let text = serde_json::to_string_pretty(&outcome).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

/// Revoke a capability grant on a cognitive map. SERVICE-DIRECT, admin/can_grant-gated (see
/// [`cogmap_grant`]). Absent grant ⇒ no-op success.
pub async fn cogmap_revoke(
    svc: &TemperMcpService,
    input: CogmapRevokeInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let cogmap_id = temper_workflow::operations::parse_ref(&input.cogmap)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("bad cogmap ref: {e}"), None))?
        .0;
    let (principal_table, principal_id) = resolve_principal(input.from_profile, input.from_team)?;

    let req = RevokeCapabilityRequest {
        subject_table: "kb_cogmaps".to_string(),
        subject_id: cogmap_id,
        principal_table,
        principal_id,
    };
    let outcome =
        access_service::revoke_capability(&svc.api_state.pool, ProfileId::from(profile.id), &req)
            .await
            .map_err(|e| map_api_error("cogmap_revoke", e))?;

    let text = serde_json::to_string_pretty(&outcome).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cogmap_grant_input_deserializes() {
        let id = Uuid::now_v7();
        let raw = serde_json::json!({ "cogmap": "m", "to_profile": id.to_string(), "write": true });
        let input: CogmapGrantInput = serde_json::from_value(raw).unwrap();
        assert_eq!(input.to_profile, Some(id));
        assert!(input.write);
        assert!(!input.grant);
    }

    #[test]
    fn cogmap_read_charter_input_deserializes() {
        let raw = serde_json::json!({ "cogmap": "m" });
        let input: CogmapReadCharterInput = serde_json::from_value(raw).expect("charter input");
        assert_eq!(input.cogmap, "m");
    }

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
