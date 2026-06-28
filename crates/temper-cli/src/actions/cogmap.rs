//! `temper cogmap shape` business logic — thin wrapper over the cognitive-maps client. Cloud-only.

use temper_core::types::cognitive_maps::CogmapRegionRow;

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

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

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
