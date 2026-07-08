//! Wire types for the Atlas Home read (`GET /api/graph/home`) — the JTBD
//! **build / research** verb-lens footprint (Beat B). `build` = the contexts your
//! work lives in (personal + team); `research` = the cogmaps you can reach. The
//! `you` node is dropped (self implied). See
//! docs/superpowers/specs/2026-07-07-atlas-beat-b-home-reframe-spec.md.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A context the profile can build in — personal (`@me`) or team — as a home body,
/// sized by its visible resource count. `owner_ref` is the decorated owner-scope
/// (`@me`, `+team-slug`) — the Home build lens tints by it.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_home.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct HomeContext {
    pub id: Uuid,
    pub name: String,
    /// Per-owner addressable handle (`kb_contexts.slug`). Combined with
    /// `owner_ref` it forms the `context_ref` (`owner/slug`) the vault route
    /// `/vault/[owner]/[context]` resolves — so a Home build circle can route to
    /// the context's resource list rather than a bare owner-scope path (which 404s).
    pub slug: String,
    pub owner_ref: String,
    pub resource_count: i32,
    /// Most recent `updated` timestamp among the context's visible, active
    /// resources — visibility-scoped so a resource the caller can't see (or one
    /// that's been soft-deleted) never advances it. `None` when the context has
    /// no visible resources.
    pub last_active_at: Option<DateTime<Utc>>,
}

/// A reachable cogmap as a home body (research lens). `owner_ref` is the derived
/// "held-by" scope the research lens tints by — a team `+slug`, or `temper` for the
/// universal/system kernel. `team_ids` are the visible member teams it joins.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_home.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct HomeCogmap {
    pub id: Uuid,
    pub name: String,
    pub owner_ref: String,
    pub team_ids: Vec<Uuid>,
    pub region_count: i32,
    pub facet_count: i32,
}

/// The Atlas Home footprint, lensed by act: `build` = your contexts, `research` =
/// the cogmaps you can reach. Drops the `you` node (self implied).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_home.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AtlasHome {
    pub build: Vec<HomeContext>,
    pub research: Vec<HomeCogmap>,
}
