//! The per-act invocation and result envelopes.
//!
//! Base ⊕ per-act extension: `params<act>` and `meta<act>` are added by the act implementations
//! (out of scope for v0's contract task) via `#[serde(flatten)]` on a discriminated extension.

use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use super::act::ActName;
use super::filter::{EdgeFilter, PropertyPredicate, ResourceFilter};
use super::scalars::{BoundTerm, Extent};
use super::stage::{StageInput, StageName, StageOutput};

/// One act, invoked — a named node in the composition DAG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ActInvocation {
    /// This node's name, referenced by downstream stages and by `returns`.
    pub name: StageName,
    pub act: ActName,
    /// Where this stage's set comes from, and what this act does with it: caller-supplied ids or
    /// an upstream stage, each carrying its own [`super::stage::StageRelation`]. Absent for a root
    /// act that takes no incoming set (e.g. `find-exact`). Replaces the incumbent literal
    /// `bounds: Option<IdSet>`, whose caller case survives as [`StageInput::Caller`].
    ///
    /// There is deliberately no sibling `bounds_mode` here. It was an `Option<BoundsMode>` whose
    /// "required whenever `input` is present" invariant lived in prose, which admitted a
    /// meaningless state the validator then read as `bound`. The relation belongs to the edge, and
    /// nesting it there makes the meaningless state unrepresentable rather than merely invalid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<StageInput>,
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
    /// Narrowing by what a thing IS in `kb_properties`: open key space, closed operator set. An
    /// unknown subject or an empty key/value is refused statically (spec §12).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<PropertyPredicate>,
}

/// One act-specific threshold, and what applying it did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
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
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ActResult {
    pub act: ActName,
    /// What this stage produced, as a tagged union. Declared kind, so contract chaining compares
    /// kinds rather than inferring them — through [`StageOutput::kind`] rather than a bare field.
    pub produced: StageOutput,
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
    /// How many ids this stage was handed.
    pub bounds_in: i64,
    /// How many of them contributed.
    pub bounds_honored: i64,
    /// How many did not — for ANY reason, deliberately conflated.
    ///
    /// Invisible, nonexistent and malformed are one number on purpose. Separating them, or naming
    /// this field for the invisible case alone, is a **single-probe existence oracle**: pass one
    /// id, read the counter, learn whether it exists. This shipped in T2 as `bounds_withheld`,
    /// whose name inherited [`super::disposition::StageDisposition::Withheld`]'s meaning —
    /// *material exists* — and so disclosed exactly that. The arithmetic was always harmless
    /// (`bounds_in - bounds_honored` is derivable either way); the leak was in the label.
    ///
    /// The caller still learns that 28 of their 40 did not contribute, which is what
    /// `composition-is-legible` asks for. They do not learn why. Decision
    /// `019fcd13-4e65-7213-ac6f-20c3c8ccfce1`.
    pub bounds_dropped: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::query::id_set::{IdKind, IdSet};
    use crate::types::query::stage::StageName;
    use std::collections::BTreeMap;

    #[test]
    fn an_invocation_without_input_or_terms_omits_them() {
        let inv = ActInvocation {
            name: StageName::parse("wide").unwrap(),
            act: ActName::FindAboutAnywhere,
            input: None,
            terms: BTreeMap::new(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        };
        let json = serde_json::to_string(&inv).unwrap();
        assert!(!json.contains("input"));
        assert!(!json.contains("terms"));
        assert!(!json.contains("filter"));
        assert_eq!(serde_json::from_str::<ActInvocation>(&json).unwrap(), inv);
    }

    #[test]
    fn a_result_still_declares_the_kind_it_produced_through_the_union() {
        // Contract chaining compares KINDS. Wrapping the set in a tagged union must not cost that:
        // the kind is still machine-checkable, now via `StageOutput::kind` rather than a bare field.
        let r = ActResult {
            act: ActName::Survey,
            produced: StageOutput::Ids {
                set: IdSet {
                    kind: IdKind::Region,
                    provenance: None,
                    ids: vec![],
                },
            },
            extent: Extent::Complete,
            total: None,
            terms_effective: BTreeMap::from([(BoundTerm::Regions, 3)]),
            narrowed_by: vec![],
            bounds_in: 0,
            bounds_honored: 0,
            bounds_dropped: 0,
        };
        assert_eq!(r.produced.kind(), IdKind::Region);
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
            produced: StageOutput::Ids {
                set: IdSet {
                    kind: IdKind::Resource,
                    provenance: None,
                    ids: vec![],
                },
            },
            extent: Extent::Partial,
            total: None,
            terms_effective: BTreeMap::from([(BoundTerm::Limit, 50)]),
            narrowed_by: vec![],
            bounds_in: 0,
            bounds_honored: 0,
            bounds_dropped: 0,
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
            produced: StageOutput::Ids {
                set: IdSet {
                    kind: IdKind::Region,
                    provenance: None,
                    ids: vec![],
                },
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
            bounds_dropped: 0,
        };
        assert!(matches!(r.extent, Extent::Indeterminate { .. }));
    }

    #[test]
    fn the_dropped_count_cannot_be_read_as_an_existence_oracle() {
        // Two callers each name one id they cannot see: one id exists, one does not. The wire
        // form must be IDENTICAL, or a single probe distinguishes them. This is the property the
        // field's name used to break — `bounds_withheld` inherited StageDisposition::Withheld's
        // meaning ("material exists") and so answered the question by labelling it.
        let invisible_but_real = ActResult {
            act: ActName::FindExact,
            produced: StageOutput::Ids {
                set: IdSet {
                    kind: IdKind::Resource,
                    provenance: None,
                    ids: vec![],
                },
            },
            extent: Extent::Complete,
            total: None,
            terms_effective: BTreeMap::new(),
            narrowed_by: vec![],
            bounds_in: 1,
            bounds_honored: 0,
            bounds_dropped: 1,
        };
        let never_existed = ActResult {
            bounds_dropped: 1,
            ..invisible_but_real.clone()
        };
        assert_eq!(
            serde_json::to_string(&invisible_but_real).unwrap(),
            serde_json::to_string(&never_existed).unwrap(),
            "an invisible id and a nonexistent one must be indistinguishable on the wire"
        );
        // And no field anywhere in the rendering is named for the invisible case alone.
        let json = serde_json::to_string(&invisible_but_real).unwrap();
        assert!(!json.contains("withheld"), "no withheld-shaped counter");
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
