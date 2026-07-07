//! R2 Tier-0 "territory overview" wire types — region + context territories,
//! orphan salient nodes (sparsity fallback), and aggregated cross-territory
//! bridges. See `graph_service::cogmap_panorama`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A territory on the Atlas panorama: a region, a context, or a cogmap.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_territory.ts"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum TerritoryKind {
    Region,
    Context,
    Cogmap,
}

/// A tinted, sized territory (Tier-0 aggregate). `salience` sizes regions;
/// `member_count` sizes contexts/cogmaps.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_territory.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct Territory {
    pub id: Uuid,
    pub kind: TerritoryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub member_count: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salience: Option<f64>,
    /// Region content cohesion (`content_cohesion`: mean member-to-centroid cosine).
    /// Sizes nothing — surfaced in the region hover card. None for contexts/cogmaps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coherence: Option<f64>,
    /// Cogmap/context this territory belongs to (for drill-in addressing).
    pub anchor_id: Uuid,
}

/// An aggregated cross-territory bridge (Tier-0).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_territory.ts"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct Bridge {
    pub source_territory: Uuid,
    pub target_territory: Uuid,
    pub edge_count: i32,
}

/// A high-degree standalone node surfaced where its cogmap home has no region
/// (sparsity rule). `doc_type` is optional/free-form.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_territory.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct OrphanNode {
    pub id: Uuid,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    pub degree: i32,
    pub anchor_id: Uuid,
    /// Human name of the home cogmap (`kb_cogmaps.name`), for the sparse territory label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_label: Option<String>,
}

/// The whole Tier-0 panorama for a team scope.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_territory.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct TerritoryOverview {
    pub territories: Vec<Territory>,
    pub orphan_nodes: Vec<OrphanNode>,
    pub bridges: Vec<Bridge>,
}

/// A member of a region's interior (resolved per-member through resources_visible_to).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_territory.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct RegionMember {
    pub id: Uuid,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affinity: Option<f64>,
}

/// R3 territory drill-in: region label + top-N members (visibility-scoped).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_territory.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct TerritorySlice {
    pub region_id: Uuid,
    /// The region's human label (`kb_cogmap_regions.label`); may be null.
    pub label: Option<String>,
    pub members: Vec<RegionMember>,
}
