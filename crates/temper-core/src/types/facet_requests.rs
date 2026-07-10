//! Wire types for the `/api/facets` write endpoint (`facet_set`).
//!
//! Shared between `temper-api` (server-side, OpenAPI schema source) and
//! `temper-client` (client-side, typed request builder). The structs both
//! `Serialize` (so the client can post them) and `Deserialize` (so the
//! server can extract them); both sides re-use the same struct rather than
//! string-mirroring a JSON shape.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::authorship::ActInput;

/// Default facet weight when a request omits it (matches the MCP/CLI default).
fn default_facet_weight() -> f64 {
    1.0
}

/// Request body for `POST /api/facets`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct FacetSetRequest {
    /// The resource whose facet property is being set — a pre-resolved id.
    pub resource: Uuid,
    /// The facet's typed value payload.
    pub values: serde_json::Value,
    /// Relative weight of the facet; defaults to `1.0` when omitted, matching the MCP tool and CLI
    /// (both default it) so a raw API caller need not supply it.
    #[serde(default = "default_facet_weight")]
    pub weight: f64,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship for the facet_set act.
    /// Flattened as top-level keys; all optional (empty when nothing is supplied).
    #[serde(default, flatten)]
    pub act: ActInput,
}

/// Acknowledgement returned by the facet write endpoint.
///
/// `id` duplicates `property_id` — see `InvocationAck::id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct FacetAck {
    pub id: Uuid,
    pub property_id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::authorship::ConfidenceBand;

    #[test]
    fn facet_set_request_round_trips_without_act() {
        let req = FacetSetRequest {
            resource: Uuid::nil(),
            values: serde_json::json!({"summary": "example"}),
            weight: 1.0,
            act: ActInput::default(),
        };
        let v = serde_json::to_value(&req).unwrap();
        // Empty act fields skip-serialize, so the wire stays minimal.
        assert!(v.get("invocation_id").is_none());
        assert!(v.get("confidence").is_none());
        let back: FacetSetRequest = serde_json::from_value(v).unwrap();
        assert_eq!(back.resource, req.resource);
        assert_eq!(back.values, req.values);
        assert_eq!(back.weight, req.weight);
        assert_eq!(back.act, req.act);
    }

    #[test]
    fn facet_set_request_round_trips_with_flattened_act() {
        let req = FacetSetRequest {
            resource: Uuid::nil(),
            values: serde_json::json!({"summary": "example"}),
            weight: 0.5,
            act: ActInput {
                invocation_id: None,
                correlation_id: None,
                reasoning: Some("because X".into()),
                confidence: Some(ConfidenceBand::Probable),
                rationale: None,
                persona: None,
                model: None,
            },
        };
        let v = serde_json::to_value(&req).unwrap();
        // The act fields appear as top-level keys, not nested under an `act` object.
        assert_eq!(v["reasoning"], "because X");
        assert_eq!(v["confidence"], "probable");
        assert!(v.get("act").is_none());
        let back: FacetSetRequest = serde_json::from_value(v).unwrap();
        assert_eq!(back.act, req.act);
    }

    #[test]
    fn facet_set_request_defaults_weight_when_omitted() {
        // A raw API caller may omit `weight`; it defaults to 1.0 (matching MCP/CLI).
        let wire = serde_json::json!({
            "resource": Uuid::nil(),
            "values": {"summary": "example"},
        });
        let req: FacetSetRequest = serde_json::from_value(wire).unwrap();
        assert_eq!(req.weight, 1.0);
    }

    #[test]
    fn facet_ack_round_trips() {
        let ack = FacetAck {
            id: Uuid::nil(),
            property_id: Uuid::nil(),
        };
        let v = serde_json::to_value(&ack).unwrap();
        let back: FacetAck = serde_json::from_value(v).unwrap();
        assert_eq!(back.property_id, ack.property_id);
    }

    #[test]
    fn facet_ack_carries_both_id_and_property_id() {
        let pid = uuid::Uuid::nil();
        let ack = FacetAck {
            id: pid,
            property_id: pid,
        };
        let v = serde_json::to_value(&ack).expect("serialize");
        assert_eq!(v["id"], v["property_id"], "id must alias property_id");
        assert!(v.get("id").is_some(), "generic `id` key present: {v}");
    }
}
