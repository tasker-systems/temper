//! `temper cogmap shape` business logic — thin wrapper over the cognitive-maps client. Cloud-only.

use temper_core::types::cognitive_maps::{
    CogmapAnalyticsRow, CogmapRegionMetricsRow, CogmapRegionRow,
};

use crate::error::Result;

/// Call the shape API for the given cogmap (and optional lens), both already resolved to UUIDs.
pub async fn shape_api(
    client: &temper_client::TemperClient,
    cogmap_id: uuid::Uuid,
    lens_id: Option<uuid::Uuid>,
) -> Result<Vec<CogmapRegionRow>> {
    client
        .cognitive_maps()
        .shape(cogmap_id, lens_id)
        .await
        .map_err(crate::commands::client_err)
}

/// Call the region-metrics API for the given cogmap (and optional lens), both resolved to UUIDs.
pub async fn region_metrics_api(
    client: &temper_client::TemperClient,
    cogmap_id: uuid::Uuid,
    lens_id: Option<uuid::Uuid>,
) -> Result<Vec<CogmapRegionMetricsRow>> {
    client
        .cognitive_maps()
        .region_metrics(cogmap_id, lens_id)
        .await
        .map_err(crate::commands::client_err)
}

/// Call the analytics API for the given cogmap (resolved to a UUID).
pub async fn analytics_api(
    client: &temper_client::TemperClient,
    cogmap_id: uuid::Uuid,
) -> Result<CogmapAnalyticsRow> {
    client
        .cognitive_maps()
        .analytics(cogmap_id)
        .await
        .map_err(crate::commands::client_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn render_region_metrics_rows_json_is_passthrough_array() {
        use temper_core::types::cognitive_maps::CogmapRegionMetricsRow;
        let rows: Vec<CogmapRegionMetricsRow> = vec![CogmapRegionMetricsRow {
            region_id: Uuid::from_u128(1),
            lens_id: Uuid::from_u128(2),
            centrality: Some(4.0),
            content_cohesion: None,
            internal_tension: Some(1.5),
            reference_standing: Some(7.0),
            telos_alignment: None,
        }];
        let out =
            crate::format::render(&rows, crate::format::OutputFormat::Json).expect("json render");
        assert!(out.starts_with('['), "json should be an array: {out}");
        assert!(out.contains("\"internal_tension\""), "json: {out}");
    }

    #[test]
    fn render_shape_rows_json_is_passthrough_array() {
        let rows: Vec<CogmapRegionRow> = vec![CogmapRegionRow {
            region_id: Uuid::from_u128(1),
            lens_id: Uuid::from_u128(2),
            salience: 0.5,
            content_cohesion: None,
            label: Some("region".to_string()),
            member_count: 2,
        }];
        let out =
            crate::format::render(&rows, crate::format::OutputFormat::Json).expect("json render");
        assert!(out.starts_with('['), "json should be an array: {out}");
        assert!(out.contains("\"region_id\""), "json: {out}");
        assert!(out.contains("\"member_count\""), "json: {out}");
    }
}
