//! Per-stage disclosure. Tier 1 of design §4.4 — mandatory, O(stages), never truncated.

use serde::{Deserialize, Serialize};

use super::act::ActName;
use super::disposition::StageDisposition;
use super::envelope::NarrowedBy;
use super::scalars::MetaDetail;

/// Where a stage's bounds came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
#[serde(rename_all = "snake_case", tag = "source")]
pub enum BoundsSource {
    /// Verbatim from an earlier stage's `produced` set.
    Upstream { stage: u32 },
    /// Produced by a jaq expression between stages — i.e. the caller sub-selected, and the
    /// bounds no longer equal any act's output.
    Expression,
    /// Supplied directly by the caller.
    Caller,
}

/// A per-resource meta budget that bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
pub struct MetaTruncated {
    pub stage: u32,
    pub retained: i64,
    pub dropped: i64,
}

/// One stage's mandatory disclosure. Exists whether or not the stage produced a result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
pub struct StageTrace {
    pub stage: u32,
    pub act: ActName,
    pub disposition: StageDisposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds_source: Option<BoundsSource>,
    pub bounds_in: i64,
    pub bounds_honored: i64,
    pub bounds_withheld: i64,
    pub narrowed_by: Vec<NarrowedBy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_truncated: Option<MetaTruncated>,
}

/// The whole composition's disclosure: an ordered per-stage record array.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
pub struct CompositionTrace {
    pub meta_detail: MetaDetail,
    pub stages: Vec<StageTrace>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::query::disposition::StageDisposition;

    #[test]
    fn a_refused_stage_still_has_a_trace_entry() {
        // The reason disclosure lives in the envelope rather than in each act's response: a
        // refused stage has no result to attach disclosure to.
        let t = StageTrace {
            stage: 2,
            act: ActName::FindAboutWithin,
            disposition: StageDisposition::Refused,
            bounds_source: Some(BoundsSource::Upstream { stage: 1 }),
            bounds_in: 40,
            bounds_honored: 0,
            bounds_withheld: 0,
            narrowed_by: vec![],
            meta_truncated: None,
        };
        assert_eq!(t.disposition, StageDisposition::Refused);
        assert_eq!(
            serde_json::from_str::<StageTrace>(&serde_json::to_string(&t).unwrap()).unwrap(),
            t
        );
    }

    #[test]
    fn bounds_source_distinguishes_upstream_from_an_expression() {
        // When jaq post-filters between stages, the next stage's bounds no longer equal the
        // upstream act's produced set. Not forbidden — DISCLOSED.
        let up = BoundsSource::Upstream { stage: 1 };
        let ex = BoundsSource::Expression;
        let ca = BoundsSource::Caller;
        assert_ne!(
            serde_json::to_string(&up).unwrap(),
            serde_json::to_string(&ex).unwrap()
        );
        assert_ne!(
            serde_json::to_string(&ex).unwrap(),
            serde_json::to_string(&ca).unwrap()
        );
    }

    #[test]
    fn a_truncated_meta_budget_is_always_disclosed() {
        // ORPHAN_LIMIT = 50 truncates with no response flag and no server log. The contract may
        // decline to carry detail; it may never do so silently.
        let m = MetaTruncated {
            stage: 3,
            retained: 50,
            dropped: 412,
        };
        let t = StageTrace {
            stage: 3,
            act: ActName::FollowFrom,
            disposition: StageDisposition::Answered,
            bounds_source: None,
            bounds_in: 0,
            bounds_honored: 0,
            bounds_withheld: 0,
            narrowed_by: vec![],
            meta_truncated: Some(m),
        };
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains("meta_truncated"));
        assert_eq!(serde_json::from_str::<StageTrace>(&json).unwrap(), t);
    }

    #[test]
    fn a_composition_trace_is_ordered_and_carries_its_detail_level() {
        let c = CompositionTrace {
            meta_detail: MetaDetail::Surviving,
            stages: vec![],
        };
        assert_eq!(c.meta_detail, MetaDetail::Surviving);
        assert_eq!(
            serde_json::from_str::<CompositionTrace>(&serde_json::to_string(&c).unwrap()).unwrap(),
            c
        );
    }
}
