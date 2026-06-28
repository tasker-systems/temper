//! Cognitive-map read tools. `cogmap_shape` reads the surface tier (materialized regions) of a map.

use rmcp::model::CallToolResult;

use temper_core::types::cognitive_maps::{CogmapAnalyticsInput, CogmapRegionMetricsInput, CogmapShapeInput};
use temper_core::types::ids::ProfileId;

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
    .map_err(|e| rmcp::ErrorData::internal_error(format!("cogmap_region_metrics failed: {e}"), None))?;

    let text = serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(text)]))
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
            let text = serde_json::to_string_pretty(&analytics)
                .unwrap_or_else(|_| "{}".to_string());
            Ok(CallToolResult::success(vec![rmcp::model::Content::text(text)]))
        }
        None => Err(rmcp::ErrorData::invalid_params(
            "cognitive map not found or not readable".to_string(),
            None,
        )),
    }
}
