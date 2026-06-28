//! Cognitive-map read-surface wire types.
//!
//! `CogmapRegionRow` is the surface tier of a materialized region — centroid-derived readouts only
//! (salience, content-cohesion, label, member_count). Member identities are NEVER carried here; the
//! interior is dereferenced per-member through `resources_visible_to` elsewhere. Mirrors the
//! `cogmap_shape` SQL return (`migrations/20260624000002_canonical_functions.sql`) field-for-field so
//! the `temper-api` read wrapper can `query_as` straight into it.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// One non-folded region of a cognitive map under a lens, as returned by `cogmap_shape`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "cognitive_maps.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CogmapRegionRow {
    /// `kb_cogmap_regions.id` — the region's stable identity.
    pub region_id: Uuid,
    /// The lens (perspective) that produced this region.
    pub lens_id: Uuid,
    /// Computed, memoized blend (telos-alignment + reference-standing + centrality); higher = more salient.
    pub salience: f64,
    /// Mean member-to-centroid cosine; `None` until the downstream readout has been computed.
    pub content_cohesion: Option<f64>,
    /// Optional agent-authored region label.
    pub label: Option<String>,
    /// Member count (the blur the surface tier exposes; identities stay interior).
    pub member_count: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn cogmap_region_row_serde_roundtrip_preserves_nullables() {
        let row = CogmapRegionRow {
            region_id: Uuid::from_u128(1),
            lens_id: Uuid::from_u128(2),
            salience: 0.75,
            content_cohesion: None,
            label: Some("Migration tooling".to_string()),
            member_count: 4,
        };
        let json = serde_json::to_string(&row).expect("serialize");
        // null nullable + present nullable both survive the round-trip
        assert!(json.contains("\"content_cohesion\":null"), "json: {json}");
        assert!(json.contains("\"label\":\"Migration tooling\""), "json: {json}");
        let back: CogmapRegionRow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, row);
    }
}
