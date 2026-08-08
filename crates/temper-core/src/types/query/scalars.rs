//! Base-envelope scalars shared by every act.

use serde::{Deserialize, Serialize};

/// Whether the caller received everything that matched.
///
/// NOT a total. A total costs a second query — the standing tax of pagination — and across a chain
/// that tax is paid per stage; for a whole composition it is not even well-defined, because each
/// stage's output is the next stage's domain. `Partial` is answerable with a `limit + 1` probe.
///
/// This is what `every-bound-a-read-applies-is-visible-in-its-answer` actually asks for: the
/// ability to distinguish "this is everything" from "this is some of it".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", tag = "extent")]
pub enum Extent {
    /// Everything that matched is here.
    Complete,
    /// More exists beyond what was returned.
    Partial,
    /// Neither is determinable: the candidate set is *produced by* the bound rather than selected
    /// under it. `survey` is the worked case — a region-salience traversal has no size prior to
    /// its own funnel width.
    Indeterminate { reason: String },
}

/// A bound term. CLOSED, and each term has exactly one meaning on every read: `limit` is rows,
/// `offset` is rows skipped, `regions` is funnel width.
///
/// An act that cannot serve a term does NOT reinterpret it — it declines it
/// (`RefusalReason::BoundTermNotApplicable`), decided statically against the schema before
/// execution. That is `the-same-bound-term-means-the-same-thing-on-every-read` by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum BoundTerm {
    /// Rows returned.
    Limit,
    /// Rows skipped.
    Offset,
    /// Funnel width, in regions. `survey`'s only bound term — it has no rows to limit.
    Regions,
}

// `BoundsMode` used to live here. It is now `StageRelation`, in `stage.rs` beside `StageInput`,
// because the relation is a property of the EDGE rather than of the stage — carrying it here, as a
// base-envelope scalar, is what made an `Option` on the invocation look reasonable.

/// How much per-resource meta the trace retains (design §4.4, tier 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum MetaDetail {
    /// Per-resource meta only for ids in the final result set. Bounded by the caller's own limit.
    #[default]
    Surviving,
    /// Every id at every stage, including ids dropped mid-composition. The diagnostic mode.
    Full,
    /// Tier 1 only.
    None,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extent_distinguishes_complete_from_partial_from_uncountable() {
        // The clause needs "this is everything" vs "this is some of it" — NOT a total. `Partial`
        // is answerable with a limit+1 probe; a total would cost a second query per stage.
        let all = [
            Extent::Complete,
            Extent::Partial,
            Extent::Indeterminate {
                reason: "candidate set is produced by the bound".to_string(),
            },
        ];
        let rendered: Vec<String> = all
            .iter()
            .map(|e| serde_json::to_string(e).unwrap())
            .collect();
        // All three are mutually distinguishable.
        assert_eq!(rendered.len(), 3);
        assert_ne!(rendered[0], rendered[1]);
        assert_ne!(rendered[1], rendered[2]);
        for (e, j) in all.iter().zip(rendered.iter()) {
            assert_eq!(&serde_json::from_str::<Extent>(j).unwrap(), e);
        }
    }

    #[test]
    fn extent_never_serializes_as_bare_null() {
        // A nullable would collapse "complete" with "could not tell" — the
        // is_stale-on-a-never-materialized-map ambiguity, one family over.
        assert_ne!(serde_json::to_string(&Extent::Complete).unwrap(), "null");
    }

    #[test]
    fn bound_terms_each_have_exactly_one_meaning() {
        // `limit` means rows, always. `regions` means funnel width, always. A term is never
        // reinterpreted per act — an act that cannot serve a term DECLINES it (Task 4/5).
        assert_eq!(
            serde_json::to_string(&BoundTerm::Limit).unwrap(),
            "\"limit\""
        );
        assert_eq!(
            serde_json::to_string(&BoundTerm::Offset).unwrap(),
            "\"offset\""
        );
        assert_eq!(
            serde_json::to_string(&BoundTerm::Regions).unwrap(),
            "\"regions\""
        );
    }

    #[test]
    fn bound_term_is_closed_unknown_terms_fail_to_parse() {
        // Closed on purpose: a term whose meaning is not fixed by the contract is exactly the
        // thing `the-same-bound-term-means-the-same-thing-on-every-read` forbids.
        assert!(serde_json::from_str::<BoundTerm>("\"page_size\"").is_err());
    }

    // `bounds_mode_round_trips_both_directions` moved with the type — see
    // `stage::tests::the_relation_rides_the_wire_as_the_edge_word_callers_write`, which pins the
    // same two spellings plus the `as` position the round-trip alone could not see.

    #[test]
    fn meta_detail_defaults_to_surviving() {
        assert_eq!(MetaDetail::default(), MetaDetail::Surviving);
    }
}
