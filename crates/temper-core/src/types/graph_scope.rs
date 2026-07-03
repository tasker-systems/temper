//! Wire types for the Graph Atlas team-graph-scope read (R1).
//! See docs/superpowers/specs/2026-07-03-temper-ui-graph-visualization-atlas-design.md.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A team identity as it appears in the scope view (self, an ancestor, or a zone header).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_scope.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct TeamRef {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
}

/// An enterable child-team zone: a door the profile may drill into, with a size hint.
/// `resource_count` is the number of resources the profile would see within the child's
/// scope (child + its ancestors), i.e. `count(resources_in_team_scope(profile, child))`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_scope.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct TeamZone {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub resource_count: i32,
}

/// The team-scoped navigation frame for the graph view: the scope team, its reachable
/// ancestor set (DAG up-set, excludes self — presented as chips, not a linear path),
/// and the enterable child zones.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_scope.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct TeamScopeView {
    pub team: TeamRef,
    pub ancestors: Vec<TeamRef>,
    pub zones: Vec<TeamZone>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_scope_view_round_trips() {
        let view = TeamScopeView {
            team: TeamRef {
                id: Uuid::nil(),
                slug: "eng".into(),
                name: "Engineering".into(),
            },
            ancestors: vec![TeamRef {
                id: Uuid::nil(),
                slug: "epd".into(),
                name: "EPD".into(),
            }],
            zones: vec![TeamZone {
                id: Uuid::nil(),
                slug: "squad-a".into(),
                name: "Squad A".into(),
                resource_count: 142,
            }],
        };
        let json = serde_json::to_string(&view).unwrap();
        let back: TeamScopeView = serde_json::from_str(&json).unwrap();
        assert_eq!(view, back);
    }
}
