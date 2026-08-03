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
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
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
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
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
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
pub struct IdSet {
    pub kind: IdKind,
    /// Required for `region`; absent for every other kind today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<IdProvenance>,
    pub ids: Vec<Uuid>,
}

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
