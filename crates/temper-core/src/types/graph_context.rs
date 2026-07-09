//! Beat E — the context panorama wire types.
//!
//! A context panorama is the builder-axis sibling of the cogmap panorama: container
//! territories (goal-rooted, edge-derived) plus the residue that reaches no container.
//!
//! Residual buckets are DERIVED from a group-by key, never enumerated. `group_key`
//! defaults to `doc_type` (itself just a `kb_properties` row), so grouping by `stage`,
//! a facet, or a keyword needs no schema change. This is why there is no
//! `WHERE doc_type <> 'session'` anywhere in the read path — sessions are a bucket the
//! data produced, not a rule the designer wrote.

use serde::{Deserialize, Serialize};

use super::graph_territory::Territory;

/// One residual bucket: a distinct value of the group key, and how many otherwise
/// uncontained resources carry it.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_context.ts"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResidualBucket {
    pub value: String,
    pub count: i32,
}

/// The residue of a context, grouped. `buckets` is empty (never null) when every
/// resource reaches a container — the healthy steady state.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_context.ts"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResidualGroups {
    pub group_key: String,
    pub buckets: Vec<ResidualBucket>,
}

/// A group key the caller could have grouped by, with how much of the context it covers.
/// Lets the UI offer alternatives without the server assuming which one matters.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_context.ts"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct GroupKeyMeta {
    pub key: String,
    pub distinct_values: i32,
    pub coverage: i32,
}

/// Tier-0 of the context door.
///
/// `containers` carry `TerritoryKind::Context` — NOT a new variant. `kind` selects the
/// tint, and tint encodes the AXIS (spec D6): a goal container sits on the builder axis,
/// so it is `Context`-tinted even though it is rooted at a goal. `label` is the goal title.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "graph_context.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ContextPanorama {
    pub containers: Vec<Territory>,
    pub residual: ResidualGroups,
    pub group_keys: Vec<GroupKeyMeta>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn residual_bucket_round_trips() {
        let g = ResidualGroups {
            group_key: "doc_type".into(),
            buckets: vec![ResidualBucket {
                value: "session".into(),
                count: 395,
            }],
        };
        let json = serde_json::to_string(&g).expect("serialize");
        assert_eq!(
            json,
            r#"{"group_key":"doc_type","buckets":[{"value":"session","count":395}]}"#
        );
        let back: ResidualGroups = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, g);
    }

    #[test]
    fn empty_residual_serializes_as_empty_array_not_null() {
        // A well-edged context has NO residuals; the tray must render "nothing",
        // never crash on a null (spec D2: the tray shrinks to nothing).
        let g = ResidualGroups {
            group_key: "doc_type".into(),
            buckets: vec![],
        };
        assert_eq!(
            serde_json::to_string(&g).unwrap(),
            r#"{"group_key":"doc_type","buckets":[]}"#
        );
    }
}
