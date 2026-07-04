//! Wire types for the Atlas Home read (`GET /api/graph/home`) — the
//! you→teams→cogmaps membership graph. See
//! docs/superpowers/specs/2026-07-04-graph-atlas-atlas-home-design.md.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A member team as a home door, with size hints.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_home.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct HomeTeam {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub resource_count: i32,
    pub cogmap_count: i32,
}

/// A visible cogmap as a home door. `team_ids` are the visible member teams this
/// cogmap joins — i.e. the bipartite team→cogmap edges (a shared cogmap lists >1).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_home.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct HomeCogmap {
    pub id: Uuid,
    pub name: String,
    pub team_ids: Vec<Uuid>,
    pub region_count: i32,
    pub facet_count: i32,
}

/// The full membership home: you → member teams → visible cogmaps.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_home.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AtlasHome {
    pub teams: Vec<HomeTeam>,
    pub cogmaps: Vec<HomeCogmap>,
}
