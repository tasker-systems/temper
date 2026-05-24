//! Typed payloads for the `relationship_*` event family — the structured
//! shape the edge-projection builder reads out of `kb_events.payload`.
//!
//! `relationship_asserted` is the lifecycle root: its event id becomes the
//! `correlation_id` shared by every later event for that edge. The projection
//! builder keys on `correlation_id`, not on ledger `references`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::graph::{EdgeKind, Polarity};

/// The target endpoint of an asserted relationship — a resolved resource id,
/// or an unresolved slug (forward reference). A slug target projects no edge
/// until a resource with that slug exists; this replaces `kb_deferred_edges`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum TargetEndpoint {
    Resource(Uuid),
    Slug(String),
}

/// `relationship_asserted` — genesis of a relationship. Topic class: Declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipAsserted {
    pub source_resource_id: Uuid,
    pub target: TargetEndpoint,
    pub edge_kind: EdgeKind,
    pub polarity: Polarity,
    pub label: String,
    pub weight: f64,
}

/// `relationship_retyped` — change the structural kind and polarity. Declaration.
///
/// `label` is intentionally absent: no surface exposes label-on-retype, and
/// the projection apply never updated it. Relabel-on-retype is future scope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipRetyped {
    pub edge_kind: EdgeKind,
    pub polarity: Polarity,
}

/// `relationship_reweighted` — change the weight. Declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipReweighted {
    pub weight: f64,
}

/// `relationship_folded` — edge preserved but removed from the default
/// projection. The retraction mechanism: "no longer current, but not wrong".
/// Topic class: Deformation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipFolded {
    /// Optional human note on why the edge was folded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `relationship_decayed` — schema only in phases 1-2; mechanics are phase 4.
/// Topic class: Deformation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipDecayed {
    /// Multiplicative decay factor applied to the edge weight (0.0..1.0).
    pub factor: f64,
}

/// `relationship_corrected` — the edge was *wrong*; carries a scar.
/// Schema only in phases 1-2; mechanics are phase 4. Topic class: Judgment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipCorrected {
    /// Structured account of the wrongness — the scar.
    pub scar: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asserted_round_trips_with_slug_target() {
        let p = RelationshipAsserted {
            source_resource_id: Uuid::nil(),
            target: TargetEndpoint::Slug("some-goal".into()),
            edge_kind: EdgeKind::Contains,
            polarity: Polarity::Forward,
            label: "parent_of".into(),
            weight: 1.0,
        };
        let v = serde_json::to_value(&p).unwrap();
        let back: RelationshipAsserted = serde_json::from_value(v).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn target_endpoint_resource_round_trips() {
        let t = TargetEndpoint::Resource(Uuid::nil());
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(serde_json::from_value::<TargetEndpoint>(v).unwrap(), t);
    }

    #[test]
    fn folded_reason_is_optional() {
        let v = serde_json::json!({});
        let p: RelationshipFolded = serde_json::from_value(v).unwrap();
        assert!(p.reason.is_none());
    }
}
