//! Cognitive-map read-surface wire types.
//!
//! `CogmapRegionRow` is the surface tier of a materialized region — centroid-derived readouts only
//! (salience, content-cohesion, label, member_count). Member identities are NEVER carried here; the
//! interior is dereferenced per-member through `resources_visible_to` elsewhere. Mirrors the
//! `cogmap_shape` SQL return (`migrations/20260624000002_canonical_functions.sql`) field-for-field so
//! the `temper-api` read wrapper can `query_as` straight into it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::types::ids::{LensId, RegionId, ResourceId};

/// MCP/surface input for the cogmap shape read. `cogmap` is a ref (UUID or decorated
/// `sluggify(title)-<uuid>`); `lens` is an optional lens ref to narrow the read.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CogmapShapeInput {
    /// The cognitive map to read, by ref (UUID or `slug-<uuid>`).
    pub cogmap: String,
    /// Optional lens ref to filter regions; omit for all lenses.
    pub lens: Option<String>,
}

/// One non-folded region of a cognitive map under a lens, as returned by `cogmap_shape`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "cognitive_maps.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CogmapRegionRow {
    /// `kb_cogmap_regions.id` — the region's stable identity.
    pub region_id: RegionId,
    /// The lens (perspective) that produced this region.
    pub lens_id: LensId,
    /// Computed, memoized blend (telos-alignment + reference-standing + centrality); higher = more salient.
    pub salience: f64,
    /// Mean member-to-centroid cosine; `None` until the downstream readout has been computed.
    pub content_cohesion: Option<f64>,
    /// Optional agent-authored region label.
    pub label: Option<String>,
    /// Member count (the blur the surface tier exposes; identities stay interior).
    pub member_count: i32,
}

/// MCP/surface input for the per-region analytics read. `cogmap` is a ref (UUID or decorated
/// `sluggify(title)-<uuid>`); `lens` is an optional lens ref to narrow the read.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CogmapRegionMetricsInput {
    /// The cognitive map to read, by ref (UUID or `slug-<uuid>`).
    pub cogmap: String,
    /// Optional lens ref to filter regions; omit for all lenses.
    pub lens: Option<String>,
}

/// MCP/surface input for the map-level analytics read. `cogmap` is a ref (UUID or decorated form).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CogmapAnalyticsInput {
    /// The cognitive map to read, by ref (UUID or `slug-<uuid>`).
    pub cogmap: String,
}

/// The per-region analytics tier (the five materialized scalar readouts) as returned by
/// `cogmap_region_metrics`. Sibling to `CogmapRegionRow`'s surface tier; member identities are still
/// never carried. Each metric is `Option<f64>` (the columns are nullable until materialization computes
/// them; `telos_alignment` stays `None` when the telos has no embedded chunks).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "cognitive_maps.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CogmapRegionMetricsRow {
    /// `kb_cogmap_regions.id` — the region's stable identity.
    pub region_id: RegionId,
    /// The lens (perspective) that produced this region.
    pub lens_id: LensId,
    /// Internal declared-affinity mass × size.
    pub centrality: Option<f64>,
    /// Mean member-to-centroid cosine.
    pub content_cohesion: Option<f64>,
    /// Summed weight of opposed (`contradicts`) declared edges among members — tension binds, never fractures.
    pub internal_tension: Option<f64>,
    /// Summed reinforce_count over member blocks.
    pub reference_standing: Option<f64>,
    /// Cosine of the region centroid to the cogmap's telos-resource embedding.
    pub telos_alignment: Option<f64>,
}

/// Map-level staleness readout (`cogmap_staleness`): when the shape was last materialized, the latest
/// touch to the map's regions/edges, and whether the read is stale. Staleness is LEGIBLE — reported,
/// never blocking. `materialized_at` is `None` when the map has never been materialized.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "cognitive_maps.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CogmapStaleness {
    pub materialized_at: Option<DateTime<Utc>>,
    pub latest_touch: Option<DateTime<Utc>>,
    pub is_stale: bool,
}

/// One regulation concept (`cogmap_regulation`): a concept-resource the charter `express`-edges to
/// (label e.g. `operationalized_by`), filtered to those the principal can read.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "cognitive_maps.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CogmapRegulationRow {
    pub resource_id: ResourceId,
    pub title: String,
    pub body_text: Option<String>,
    pub edge_label: String,
}

/// The map-level analytics picture as returned by `cogmap_analytics`: the telos charter resource id,
/// staleness, and the regulation set. Per-region scalar metrics are a SEPARATE read
/// (`cogmap_region_metrics`). The access gate is INSIDE the SQL: a principal who cannot read the map
/// gets zero rows, surfaced here as `None` (→ 404 at the api boundary).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "cognitive_maps.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CogmapAnalyticsRow {
    /// `kb_cogmaps.telos_resource_id` — the charter resource (NOT NULL).
    pub telos_resource_id: ResourceId,
    pub staleness: CogmapStaleness,
    pub regulation: Vec<CogmapRegulationRow>,
}

/// One block of a cognitive map's telos charter, as returned by the `resource_blocks(cogmap_telos(…),
/// …)` composition (T1 Sequence C). Mirrors the charter-read tuple field-for-field (`seq`, `role`,
/// `body_text AS body`) so the service-direct read can `query_as!` straight into it. `role` is one of
/// `statement` / `question` / `framing` (`temper-core/src/charter.rs`); a charter block always has both
/// a role and body, so neither is `Option` (see `cogmap_charter_select`'s forced-non-null SQL).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "cognitive_maps.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CharterBlock {
    /// Position within the charter (statement is 0, then questions, then framing).
    pub seq: i32,
    /// One of `statement` / `question` / `framing`.
    pub role: String,
    /// Assembled block body text (a question-with-context block is `question + "\n\n" + context`).
    pub body: String,
}

// ---------------------------------------------------------------------------
// Cogmap ↔ team binding wire types (org-provisioning Chunk 5).
//
// Binding a cognitive map to a team writes a `kb_team_cogmaps` row — it widens
// the map's least-privilege producer-intersection reach (access §4). The surface
// is SERVICE-DIRECT and admin-gated (mirrors team membership, not a Backend
// command). Full derive stack mirrors `team::AddMemberRequest`.
// ---------------------------------------------------------------------------

/// Request body for `POST /api/cognitive-maps/{id}/teams` — bind a map to a team.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "cognitive_maps.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindTeamRequest {
    /// The team to bind the cognitive map to.
    pub team_id: Uuid,
}

/// The result of binding a cognitive map to a team. `bound` is `false` when the
/// binding already existed (idempotent no-op) — the clean mirror of genesis's
/// `created` flag.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "cognitive_maps.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindTeamOutcome {
    pub cogmap_id: Uuid,
    pub team_id: Uuid,
    /// `true` when this call inserted the binding; `false` when it already existed.
    pub bound: bool,
}

/// The result of unbinding a cognitive map from a team. `unbound` is `false` when
/// no binding existed (no-op safe).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "cognitive_maps.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnbindTeamOutcome {
    pub cogmap_id: Uuid,
    pub team_id: Uuid,
    /// `true` when this call deleted a binding; `false` when none existed.
    pub unbound: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn bind_team_request_serde_roundtrip() {
        let team_id = Uuid::now_v7();
        let req = BindTeamRequest { team_id };
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(json.contains("\"team_id\""), "json: {json}");
        let back: BindTeamRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.team_id, team_id);
    }

    #[test]
    fn bind_team_outcome_serde_roundtrip() {
        let outcome = BindTeamOutcome {
            cogmap_id: Uuid::now_v7(),
            team_id: Uuid::now_v7(),
            bound: true,
        };
        let json = serde_json::to_string(&outcome).expect("serialize");
        assert!(json.contains("\"bound\":true"), "json: {json}");
        let back: BindTeamOutcome = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, outcome);
    }

    #[test]
    fn unbind_team_outcome_serde_roundtrip() {
        let outcome = UnbindTeamOutcome {
            cogmap_id: Uuid::now_v7(),
            team_id: Uuid::now_v7(),
            unbound: false,
        };
        let json = serde_json::to_string(&outcome).expect("serialize");
        assert!(json.contains("\"unbound\":false"), "json: {json}");
        let back: UnbindTeamOutcome = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, outcome);
    }

    #[test]
    fn cogmap_shape_input_serde_roundtrip() {
        // cogmap + lens present
        let with_lens = CogmapShapeInput {
            cogmap: "my-map-00000000-0000-0000-0000-000000000042".to_string(),
            lens: Some("00000000-0000-0000-0000-000000000007".to_string()),
        };
        let json = serde_json::to_string(&with_lens).expect("serialize with lens");
        let back: CogmapShapeInput = serde_json::from_str(&json).expect("deserialize with lens");
        assert_eq!(back.cogmap, with_lens.cogmap);
        assert_eq!(back.lens, with_lens.lens);

        // lens: None serializes to null and round-trips correctly
        let no_lens = CogmapShapeInput {
            cogmap: "bare-uuid-00000000-0000-0000-0000-000000000001".to_string(),
            lens: None,
        };
        let json2 = serde_json::to_string(&no_lens).expect("serialize no lens");
        let back2: CogmapShapeInput = serde_json::from_str(&json2).expect("deserialize no lens");
        assert_eq!(back2.cogmap, no_lens.cogmap);
        assert!(back2.lens.is_none());
    }

    #[test]
    fn cogmap_region_row_serde_roundtrip_preserves_nullables() {
        let row = CogmapRegionRow {
            region_id: Uuid::from_u128(1).into(),
            lens_id: Uuid::from_u128(2).into(),
            salience: 0.75,
            content_cohesion: None,
            label: Some("Migration tooling".to_string()),
            member_count: 4,
        };
        let json = serde_json::to_string(&row).expect("serialize");
        // null nullable + present nullable both survive the round-trip
        assert!(json.contains("\"content_cohesion\":null"), "json: {json}");
        assert!(
            json.contains("\"label\":\"Migration tooling\""),
            "json: {json}"
        );
        let back: CogmapRegionRow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, row);
    }

    #[test]
    fn cogmap_region_metrics_row_serde_roundtrip_preserves_nullables() {
        let row = CogmapRegionMetricsRow {
            region_id: Uuid::from_u128(1).into(),
            lens_id: Uuid::from_u128(2).into(),
            centrality: Some(4.0),
            content_cohesion: None,
            internal_tension: Some(1.5),
            reference_standing: Some(0.0),
            telos_alignment: None,
        };
        let json = serde_json::to_string(&row).expect("serialize");
        assert!(json.contains("\"content_cohesion\":null"), "json: {json}");
        assert!(json.contains("\"internal_tension\":1.5"), "json: {json}");
        let back: CogmapRegionMetricsRow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, row);
    }

    #[test]
    fn cogmap_analytics_row_nests_staleness_and_regulation() {
        let row = CogmapAnalyticsRow {
            telos_resource_id: Uuid::from_u128(9).into(),
            staleness: CogmapStaleness {
                materialized_at: None,
                latest_touch: None,
                is_stale: true,
            },
            regulation: vec![CogmapRegulationRow {
                resource_id: Uuid::from_u128(3).into(),
                title: "Deploy safely".to_string(),
                body_text: Some("body".to_string()),
                edge_label: "operationalized_by".to_string(),
            }],
        };
        let json = serde_json::to_string(&row).expect("serialize");
        assert!(json.contains("\"is_stale\":true"), "json: {json}");
        assert!(
            json.contains("\"edge_label\":\"operationalized_by\""),
            "json: {json}"
        );
        let back: CogmapAnalyticsRow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, row);
    }
}
