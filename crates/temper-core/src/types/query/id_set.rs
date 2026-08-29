//! The query-envelope currency: a typed, tagged set of ids.
//!
//! The tag is carried as DATA, not as a Rust newtype. `crates/temper-core/src/types/ids.rs`
//! already defines 17 typed ids, but the `define_id!` macro applies `#[serde(transparent)]`, so
//! every one of them serializes as a bare uuid string. This contract is a wire contract and jaq
//! operates on JSON — a newtype would give the chaining check nothing to check.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::ids::{CogmapId, ContextId};

/// What an [`IdSet`]'s ids name.
///
/// OPEN vocabulary: an unrecognized kind parses into [`IdKind::Other`] so the act layer can
/// refuse it with a reason. Domain-named, never table-named — this deliberately diverges from
/// `LedgerRefKind`, which renames every variant to its SQL table.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum IdKind {
    Resource,
    Region,
    Cogmap,
    Context,
    /// An unrecognized kind. Never constructed by this crate — only by deserializing a producer
    /// newer than this consumer.
    #[serde(untagged)]
    Other(String),
}

impl IdKind {
    /// Whether this is a kind v0 closed the vocabulary at (design §3.1.1).
    pub fn is_known(&self) -> bool {
        !matches!(self, IdKind::Other(_))
    }
}

/// Which anchor produced a set of region ids.
///
/// Load-bearing for exactly one kind today. Mirrors the shape of
/// [`crate::types::home::HomeAnchor`] without reusing it: that type has no wire derives and is
/// deliberately internal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", tag = "anchor", content = "id")]
pub enum IdProvenance {
    Cogmap(CogmapId),
    Context(ContextId),
}

impl IdProvenance {
    pub fn is_context(&self) -> bool {
        matches!(self, IdProvenance::Context(_))
    }
}

/// The one value that crosses a stage boundary. Membership, never rank.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct IdSet {
    pub kind: IdKind,
    /// Required for `region`; absent for every other kind today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<IdProvenance>,
    /// The ids themselves — at most [`MAX_ID_SET_IDS`] of them, refused as
    /// [`crate::types::query::disposition::RefusalReason::TooManyIds`].
    // Published on BOTH doors — `utoipa` for `openapi.json` and the SDKs, `schemars` for the MCP
    // tool's input schema. See `Composition::stages` for why the pair is not duplication.
    // `max_items` is what makes that refusal legal in the SHAPE pass, exactly as it is for
    // `Composition::stages` — a client refuses what the contract forbids, never what one
    // deployment chose. See the comment on that field for the full argument.
    #[cfg_attr(feature = "web-api", schema(max_items = 256))]
    #[cfg_attr(feature = "mcp", schemars(length(max = 256)))]
    pub ids: Vec<Uuid>,
}

/// The most ids one [`IdSet`] may carry.
///
/// # It bounds a PRODUCT, not a list
///
/// Every stage discloses how many of the ids it was handed it could not use, and that number is
/// computed by comparing the caller's set against the set this principal can see — so the work is
/// `|caller ids| × |visible ids|`, with the caller choosing the second factor. Same shape as
/// `MAX_PER_CANDIDATE_PROBES`, which was measured rather than argued: one predicate carrying 2,000
/// values against 3,761 live rows discarded 2.5 million join-filter rows in 1,628 ms. 256 against a
/// corpus of that order is roughly a million comparisons per stage — an order below the measured
/// cliff, and orders above any question anyone asks, since a caller pipes ids they received from a
/// prior read and a page is 50.
///
/// # Distinct from the anchor's cardinality, which this does not retire
///
/// `AnchorTakesOneId` refuses a cogmap or context bound carrying more than one id, because today's
/// fragments take an `(anchor_table, anchor_id)` pair. That is a limitation of what this server has
/// BUILT — an `anchor_ids uuid[]` retires it — so it lives in the capability pass and may move.
/// This cap is a fact about the contract and cannot move without a wire change. The two coexist:
/// a two-id cogmap bound meets the first, a 300-id resource bound meets the second.
pub const MAX_ID_SET_IDS: usize = 256;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_serializes_domain_named_not_table_named() {
        assert_eq!(
            serde_json::to_string(&IdKind::Resource).unwrap(),
            "\"resource\""
        );
        assert_eq!(
            serde_json::to_string(&IdKind::Region).unwrap(),
            "\"region\""
        );
        assert_eq!(
            serde_json::to_string(&IdKind::Cogmap).unwrap(),
            "\"cogmap\""
        );
        assert_eq!(
            serde_json::to_string(&IdKind::Context).unwrap(),
            "\"context\""
        );
    }

    #[test]
    fn unknown_kind_deserializes_rather_than_erroring() {
        // The vocabulary is OPEN: a newer producer must not break an older consumer, and the
        // refusal must be renderable with a reason rather than surfacing as a parse error.
        let k: IdKind = serde_json::from_str("\"block\"").expect("unknown kind must parse");
        assert_eq!(k, IdKind::Other("block".to_string()));
        assert!(!k.is_known());
    }

    #[test]
    fn id_set_round_trips_without_provenance() {
        let u = uuid::Uuid::now_v7();
        let s = IdSet {
            kind: IdKind::Resource,
            provenance: None,
            ids: vec![u],
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            !json.contains("provenance"),
            "absent provenance must not serialize"
        );
        assert_eq!(serde_json::from_str::<IdSet>(&json).unwrap(), s);
    }

    #[test]
    fn region_set_carries_its_anchor_provenance() {
        // Context regions and cogmap regions are both `region` and are NOT interchangeable:
        // graph_region_composition gates on cogmap_readable_by_profile and a context region's
        // cogmap_id is NULL. The kind tag alone would admit a chain that always 404s.
        let m = CogmapId::new();
        let s = IdSet {
            kind: IdKind::Region,
            provenance: Some(IdProvenance::Cogmap(m)),
            ids: vec![uuid::Uuid::now_v7()],
        };
        let back: IdSet = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        assert!(!back.provenance.unwrap().is_context());
    }

    #[test]
    fn provenance_distinguishes_the_two_region_anchors() {
        let c = IdProvenance::Context(ContextId::new());
        let m = IdProvenance::Cogmap(CogmapId::new());
        assert!(c.is_context());
        assert!(!m.is_context());
        assert_ne!(
            serde_json::to_string(&c).unwrap(),
            serde_json::to_string(&m).unwrap()
        );
    }
}
