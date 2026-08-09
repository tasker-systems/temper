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

/// One returned stage's answer.
///
/// Neither `PartialEq` nor `Eq`, unlike its neighbours here: it holds hydrated rows, and
/// [`crate::types::resource_view::ResourceView`] derives neither while the quantities are floats.
/// The tests below compare the SERIALIZED form instead, which is what a client actually observes
/// and is the stronger assertion anyway — a field that fails to serialize is invisible to
/// structural equality.
///
/// `[renamed from StageResult — 2026-08-08]` A stage runs one act, but the thing being described is
/// the STAGE — it is keyed by the caller's stage name, and its numbers are about the set that
/// stage was handed. Naming it for the act invited exactly the ordinal-keyed trace this reshape
/// also removed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct StageResult {
    pub act: ActName,
    pub disposition: super::disposition::StageDisposition,
    /// Echoed from this act's declaration. Present iff the act orders anything.
    ///
    /// **Once per stage, rather than once per row.** The row keeps a literal field name so two
    /// acts' numbers cannot be summed by accident; this carries the RANGE so a reader is not left
    /// to assume `[0,1]`. Assuming `[0,1]` is the live mistake in this family, not a hypothetical
    /// one: `vec_norm` rescales a cosine distance as `1 - d/2` into `[0,1]` while `region_score`
    /// spans `[-0.6, 1.0]`, and neither column name says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orders_by: Option<super::act::ActQuantity>,
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
    ///
    /// There is no separate "you were clamped" flag, deliberately: ceilings are published per act,
    /// so the applied value is the whole story. Clamping to a ceiling nobody published would be
    /// the bug. This covers only terms the act ADMITS — one it does not is refused outright.
    pub terms_applied: BTreeMap<BoundTerm, i64>,
    pub narrowed_by: Vec<NarrowedBy>,
    /// How many ids this stage was handed. Zero for a stage with no input.
    pub input_ids: i64,
    /// How many of the usable ids actually contributed to what came back.
    ///
    /// The reading depends on the relation and both are honest: for a `bound`, how many of your
    /// ids are in the output; for a `seed`, how many led to something in the output.
    ///
    /// **NULL MEANS THIS ACT CANNOT REPORT IT — never zero.** Which acts can is DECLARED in
    /// [`super::act::ActDeclaration::discloses`], so it is knowable before the query runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_contributed: Option<i64>,
    /// How many did not — for ANY reason, deliberately conflated.
    ///
    /// Invisible, nonexistent and malformed are one number on purpose. Separating them, or naming
    /// this field for the invisible case alone, is a **single-probe existence oracle**: pass one
    /// id, read the counter, learn whether it exists. This shipped in T2 as `bounds_withheld`,
    /// whose name inherited [`super::disposition::StageDisposition::Withheld`]'s meaning —
    /// *material exists* — and so disclosed exactly that. The arithmetic was always harmless
    /// (`input_ids - input_contributed` is derivable either way); the leak was in the label.
    ///
    /// The caller still learns that 28 of their 40 did not contribute, which is what
    /// `composition-is-legible` asks for. They do not learn why. Decision
    /// `019fcd13-4e65-7213-ac6f-20c3c8ccfce1`.
    pub input_unusable: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::cognitive_maps::CogmapRegionRow;
    use crate::types::ids::{LensId, RegionId};
    use crate::types::query::disposition::StageDisposition;
    use crate::types::query::hits::{RegionHit, ScoreKind, Scoring};
    use crate::types::query::id_set::IdKind;
    use crate::types::query::stage::ProducedVariant;
    use std::collections::BTreeMap;

    fn region_hit() -> RegionHit {
        RegionHit {
            region: CogmapRegionRow {
                region_id: RegionId::new(),
                lens_id: LensId::new(),
                // Beside `region_score` on the same row, and NOT a rival to it: salience is an
                // INPUT to that score. Constructed here so the two-numbers-one-row case is what
                // these tests actually serialize.
                salience: 0.61,
                content_cohesion: None,
                label: Some("composable search".to_string()),
                member_count: 4,
            },
            scoring: Scoring {
                score_kind: ScoreKind::RegionScore,
                score: 0.42,
            },
        }
    }

    fn result(narrowed_by: Vec<NarrowedBy>, input_ids: i64) -> StageResult {
        StageResult {
            act: ActName::Survey,
            disposition: StageDisposition::Answered,
            orders_by: None,
            produced: StageOutput::Regions {
                hits: vec![region_hit()],
            },
            extent: Extent::Complete,
            total: None,
            terms_applied: BTreeMap::from([(BoundTerm::Regions, 3)]),
            narrowed_by,
            input_ids,
            input_contributed: None,
            input_unusable: 0,
        }
    }

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
        // Contract chaining compares KINDS. Wrapping the rows in a tagged union must not cost that:
        // the kind is still machine-checkable, now via `StageOutput::kind` rather than a bare field.
        let r = result(vec![], 0);
        assert_eq!(r.produced.kind(), IdKind::Region);
        assert_eq!(r.produced.variant(), ProducedVariant::Regions);
        assert_eq!(r.produced.len(), 1);
    }

    #[test]
    fn a_returned_stage_is_hydrated_and_can_no_longer_be_a_bare_id_set() {
        // `[decided — 2026-08-09, Pete]` Ids stay the pipe's internal currency; they are no longer
        // something a caller can ask to be handed back. Asserted on the WIRE rather than by the
        // absence of a variant, because the variant being gone is a compile-time fact this test
        // cannot restate — what a client observes is that every returned stage carries rows.
        let json = serde_json::to_value(result(vec![], 0).produced).unwrap();
        assert_eq!(json["produced"], "regions");
        assert!(json.get("hits").is_some(), "a returned stage carries rows");
        assert!(json.get("set").is_none(), "and never a bare id set: {json}");
    }

    #[test]
    fn the_score_kind_a_row_carries_is_the_one_its_act_declared() {
        // The row is self-describing AND consistent with the declaration. `score_kind` is the
        // deployed column name, which is exactly what `orders_by.field` carries — so a caller can
        // read the kind off a row and look up its RANGE on the stage without a translation table.
        let r = result(vec![], 0);
        let kinds = r.produced.score_kinds();
        assert_eq!(kinds.len(), 1);
        let declared = crate::types::query::declaration(&ActName::Survey)
            .and_then(|d| d.score_kind())
            .expect("survey orders by something");
        assert!(kinds.contains(declared.as_str()), "got: {kinds:?}");
    }

    #[test]
    fn the_dropped_input_count_is_reported_without_saying_why() {
        // Invisible / nonexistent / malformed are ONE number on purpose. Splitting them, or naming
        // a field for the invisible case alone, turns the trace into a single-probe existence
        // oracle: pass one id, read the counter, learn whether that id exists.
        let mut r = result(vec![], 40);
        r.input_unusable = 28;
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""input_unusable":28"#), "got: {json}");
        for leaky in ["invisible", "withheld", "nonexistent", "forbidden"] {
            assert!(
                !json.contains(leaky),
                "`{leaky}` discloses WHY; got: {json}"
            );
        }
    }

    #[test]
    fn a_narrowing_echoes_back_without_requiring_counts() {
        // Counts ride only where the act computes them for free — requiring them would reintroduce
        // the second query `Extent` exists to avoid. Absent is not zero.
        let r = result(
            vec![NarrowedBy {
                key: "doc_type".to_string(),
                value: "session".to_string(),
                admitted: None,
                excluded: None,
            }],
            0,
        );
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""key":"doc_type""#));
        assert!(!json.contains("admitted"), "absent, not zero: {json}");
    }

    #[test]
    fn a_result_round_trips_through_the_wire() {
        // Structural equality is unavailable (hydrated rows, floats), so the round trip is asserted
        // on the serialized form — which is the thing a client observes, and which also catches a
        // field that silently fails to deserialize.
        let r = result(vec![], 7);
        let once = serde_json::to_string(&r).unwrap();
        let twice = serde_json::to_string(
            &serde_json::from_str::<StageResult>(&once).expect("round trips"),
        )
        .unwrap();
        assert_eq!(once, twice);
    }
}
