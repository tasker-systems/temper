//! The predicate layer. Bounds are membership and terms are magnitude; neither can narrow by what
//! a thing IS. Everything carries `kb_properties` (`doc_type`, `tags`, `facet`) and every edge
//! carries BOTH an `edge_kind` and a `label`.
//!
//! Typed slots, deliberately NOT a generic `{field, op, value}` grammar: a general predicate
//! language would be more expressive and would immediately re-open every conflation this contract
//! exists to close.

use serde::{Deserialize, Serialize};

// The four members of the DDL's `edge_kind` enum
// (`migrations/20260624000001_canonical_schema.sql:95`) are ALREADY modelled, and re-used here
// rather than restated. `types::graph::EdgeKind` is `sqlx::Type`-bound to that DDL, so it is the
// copy a schema change breaks — which is exactly why the contract must not carry a second one.
// Its closedness is the fix for the audit's #1 finding: an edge `label` such as `advances` cannot
// be passed here.
use crate::types::graph::EdgeKind;

/// Narrowing over edges. `edge_kinds` and `labels` are DIFFERENT AXES and are never merged: the
/// kind is a closed DDL enum, the label is free text the caller actually sees on every edge.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
pub struct EdgeFilter {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edge_kinds: Vec<EdgeKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

/// One `kb_properties` facet predicate, at the inner-key grain the facet model uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
pub struct FacetPredicate {
    pub key: String,
    pub value: String,
}

/// Narrowing over resources. Every field is AND-composed; an unset field narrows nothing.
///
/// An unknown value on a closed vocabulary (`doc_type`, `stage`, `status`) is a REFUSAL
/// (`RefusalReason::UnknownFilterValue`), never a confident empty page — the audit found four
/// filters that accept nonsense and return one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
pub struct ResourceFilter {
    /// `kb_properties` where `property_key = 'doc_type'`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub doc_type: Vec<String>,
    /// `kb_properties` where `property_key = 'tags'`. AND-containment.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// `kb_properties` where `property_key = 'facet'`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facets: Vec<FacetPredicate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_contains: Option<String>,
}

/// Which filter slot an act admits. An unadmitted filter is DECLINED, never ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
#[serde(rename_all = "snake_case")]
pub enum FilterField {
    Resource,
    Edge,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_kind_is_closed_at_the_four_the_ddl_declares() {
        // migrations/20260624000001_canonical_schema.sql:95
        //   CREATE TYPE edge_kind AS ENUM ('express', 'contains', 'leads_to', 'near');
        //
        // Regression cover over the INCUMBENT `types::graph::EdgeKind`, which this contract
        // re-uses rather than restates. These assertions are what make the re-use safe: they fail
        // if the shared type ever stops having the properties the contract depends on.
        for (k, j) in [
            (EdgeKind::Express, "\"express\""),
            (EdgeKind::Contains, "\"contains\""),
            (EdgeKind::LeadsTo, "\"leads_to\""),
            (EdgeKind::Near, "\"near\""),
        ] {
            assert_eq!(serde_json::to_string(&k).unwrap(), j);
        }
    }

    #[test]
    fn a_label_cannot_be_passed_as_an_edge_kind() {
        // THE audit's #1 finding, fixed at the type level. `advances` is a real LABEL that appears
        // on real edges; it is not an edge_kind. Today `--edge-type advances` silently narrows to
        // nothing with reason: ok. Here it cannot be constructed at all.
        assert!(serde_json::from_str::<EdgeKind>("\"advances\"").is_err());
        assert!(serde_json::from_str::<EdgeKind>("\"derived_from\"").is_err());
    }

    #[test]
    fn labels_and_edge_kinds_are_separate_fields_on_the_filter() {
        // Separate slots, different types — so the caller who means "advances" has exactly one
        // place to put it, and it is the right one.
        let f = EdgeFilter {
            edge_kinds: vec![EdgeKind::LeadsTo],
            labels: vec!["advances".to_string()],
        };
        let back: EdgeFilter = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
        assert_eq!(back.edge_kinds, vec![EdgeKind::LeadsTo]);
        assert_eq!(back.labels, vec!["advances".to_string()]);
    }

    #[test]
    fn an_empty_filter_serializes_to_nothing() {
        let f = ResourceFilter::default();
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, "{}", "an unset filter must not emit empty arrays");
        assert_eq!(serde_json::from_str::<ResourceFilter>("{}").unwrap(), f);
    }

    #[test]
    fn resource_filters_compose_and_round_trip() {
        // filters-compose-to-narrow: several predicates on one request, AND semantics.
        let f = ResourceFilter {
            doc_type: vec!["task".to_string()],
            tags: vec!["search".to_string(), "ci".to_string()],
            facets: vec![FacetPredicate {
                key: "domain".to_string(),
                value: "search".to_string(),
            }],
            stage: Some("in-progress".to_string()),
            ..Default::default()
        };
        assert_eq!(
            serde_json::from_str::<ResourceFilter>(&serde_json::to_string(&f).unwrap()).unwrap(),
            f
        );
    }

    #[test]
    fn filter_fields_name_the_two_slots_an_act_may_admit() {
        assert_eq!(
            serde_json::to_string(&FilterField::Resource).unwrap(),
            "\"resource\""
        );
        assert_eq!(
            serde_json::to_string(&FilterField::Edge).unwrap(),
            "\"edge\""
        );
    }
}
