//! The per-act invocation and result envelopes.
//!
//! Base ⊕ per-act extension: `params<act>` and `meta<act>` are added by the act implementations
//! (out of scope for v0's contract task) via `#[serde(flatten)]` on a discriminated extension.

use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use super::act::ActName;
use super::filter::{EdgeFilter, ResourceFilter};
use super::id_set::IdSet;
use super::scalars::{BoundTerm, BoundsMode, Extent};

/// One act, invoked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
pub struct ActInvocation {
    pub act: ActName,
    /// The only value that crosses a stage boundary. Membership, never rank.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<IdSet>,
    /// How this act consumes `bounds`. Required whenever `bounds` is present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds_mode: Option<BoundsMode>,
    /// Act-level bound terms. A term this act does not admit is refused STATICALLY
    /// (`RefusalReason::BoundTermNotApplicable`), never reinterpreted to fit.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub terms: BTreeMap<BoundTerm, i64>,
    /// Narrowing by what a thing IS. At most one slot applies per act; supplying the other is
    /// `RefusalReason::FilterNotApplicable`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_filter: Option<ResourceFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_filter: Option<EdgeFilter>,
}

/// One act-specific threshold, and what applying it did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
pub struct NarrowedBy {
    pub key: String,
    pub value: String,
    /// Counts are carried ONLY where the act computes them for free. Requiring them would
    /// re-introduce the second query `Extent` exists to avoid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admitted: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excluded: Option<i64>,
}

/// One act's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
pub struct ActResult {
    pub act: ActName,
    /// Declared kind, so contract chaining compares kinds rather than inferring them.
    pub produced: IdSet,
    /// Complete / partial / indeterminate. NOT a total — see `Extent`.
    pub extent: Extent,
    /// Carried only by acts that can produce one WITHOUT a second query. Never by a composition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<i64>,
    /// The APPLIED value of every admitted term, beside what was asked. Generalizes the
    /// `regions_effective` pattern the audit calls "a model of an honest knob" — which existed
    /// for exactly one term and was never extended to `limit` or `depth`.
    pub terms_effective: BTreeMap<BoundTerm, i64>,
    pub narrowed_by: Vec<NarrowedBy>,
    pub bounds_in: i64,
    pub bounds_honored: i64,
    pub bounds_withheld: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::query::id_set::{IdKind, IdSet};
    use std::collections::BTreeMap;

    #[test]
    fn an_invocation_without_bounds_or_terms_omits_them() {
        let inv = ActInvocation {
            act: ActName::FindAboutAnywhere,
            bounds: None,
            bounds_mode: None,
            terms: BTreeMap::new(),
            resource_filter: None,
            edge_filter: None,
        };
        let json = serde_json::to_string(&inv).unwrap();
        assert!(!json.contains("bounds"));
        assert!(!json.contains("terms"));
        assert!(!json.contains("filter"));
        assert_eq!(serde_json::from_str::<ActInvocation>(&json).unwrap(), inv);
    }

    #[test]
    fn a_result_declares_the_kind_it_produced() {
        // `produced` is an IdSet, so an act's output kind is machine-checkable rather than
        // inferred from which act ran. This is what makes contract chaining compare kinds.
        let r = ActResult {
            act: ActName::Survey,
            produced: IdSet {
                kind: IdKind::Region,
                provenance: None,
                ids: vec![],
            },
            extent: Extent::Complete,
            total: None,
            terms_effective: BTreeMap::from([(BoundTerm::Regions, 3)]),
            narrowed_by: vec![],
            bounds_in: 0,
            bounds_honored: 0,
            bounds_withheld: 0,
        };
        assert_eq!(r.produced.kind, IdKind::Region);
        assert_eq!(
            serde_json::from_str::<ActResult>(&serde_json::to_string(&r).unwrap()).unwrap(),
            r
        );
    }

    #[test]
    fn a_result_can_report_partial_without_paying_for_a_total() {
        // The whole point of Extent: "there is more" is answerable with a limit+1 probe, where a
        // total would cost a second query on every stage of every chain.
        let r = ActResult {
            act: ActName::FindExact,
            produced: IdSet {
                kind: IdKind::Resource,
                provenance: None,
                ids: vec![],
            },
            extent: Extent::Partial,
            total: None,
            terms_effective: BTreeMap::from([(BoundTerm::Limit, 50)]),
            narrowed_by: vec![],
            bounds_in: 0,
            bounds_honored: 0,
            bounds_withheld: 0,
        };
        assert_eq!(r.extent, Extent::Partial);
        assert!(r.total.is_none(), "a partial answer owes no total");
        // The applied ceiling is visible beside what was asked, so the clamp is not silent.
        assert_eq!(r.terms_effective.get(&BoundTerm::Limit), Some(&50));
    }

    #[test]
    fn a_traversal_result_reports_indeterminate_rather_than_guessing() {
        let r = ActResult {
            act: ActName::Survey,
            produced: IdSet {
                kind: IdKind::Region,
                provenance: None,
                ids: vec![],
            },
            extent: Extent::Indeterminate {
                reason: "region-salience traversal has no size prior to its funnel width"
                    .to_string(),
            },
            total: None,
            terms_effective: BTreeMap::from([(BoundTerm::Regions, 3)]),
            narrowed_by: vec![],
            bounds_in: 0,
            bounds_honored: 0,
            bounds_withheld: 0,
        };
        assert!(matches!(r.extent, Extent::Indeterminate { .. }));
    }

    #[test]
    fn narrowed_by_records_what_a_threshold_excluded() {
        let n = NarrowedBy {
            key: "min_lexical_rank".to_string(),
            value: "0.4".to_string(),
            admitted: Some(12),
            excluded: Some(88),
        };
        // A filter may be disclosed without paying to count what it excluded.
        let cheap = NarrowedBy {
            key: "doc_type".to_string(),
            value: "task".to_string(),
            admitted: None,
            excluded: None,
        };
        assert!(!serde_json::to_string(&cheap).unwrap().contains("admitted"));
        assert_eq!(
            serde_json::from_str::<NarrowedBy>(&serde_json::to_string(&n).unwrap()).unwrap(),
            n
        );
    }
}
