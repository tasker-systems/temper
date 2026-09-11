//! Wire types for the `/api/facets` write endpoint (`facet_set`).
//!
//! Shared between `temper-api` (server-side, OpenAPI schema source) and
//! `temper-client` (client-side, typed request builder). The structs both
//! `Serialize` (so the client can post them) and `Deserialize` (so the
//! server can extract them); both sides re-use the same struct rather than
//! string-mirroring a JSON shape.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::authorship::ActInput;
use crate::types::provenance::BlockFoldDisposition;

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
    /// The facet's typed value payload — an **object** of `key` → value marks; one property row
    /// per inner key. A map, not a bare `Value`, so the published schema says `object` and a
    /// scalar payload is refused at this boundary rather than surfacing as a database error.
    pub values: serde_json::Map<String, serde_json::Value>,
    /// Relative weight of the facet; defaults to `1.0` when omitted, matching the MCP tool and CLI
    /// (both default it) so a raw API caller need not supply it.
    #[serde(default = "default_facet_weight")]
    pub weight: f64,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship for the facet_set act.
    /// Flattened as top-level keys; all optional (empty when nothing is supplied).
    #[serde(default, flatten)]
    pub act: ActInput,
}

/// The property key that carries an edge's span qualification: one `kb_properties` row owned
/// by the edge per (endpoint, block), valued `{"endpoint": ..., "address": ...}`. Written
/// through the edge facet surfaces' keyed mode ([`EdgeFacetSetRequest::property_key`]); read
/// back like any facet.
pub const ANCHORED_AT_PROPERTY_KEY: &str = "anchored-at";

/// Which end of a relationship an `anchored-at` row qualifies — the value's `endpoint` half.
/// Source and target mean the stored columns; the row never re-points when presentation
/// around the edge changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchoredAtEndpoint {
    Source,
    Target,
}

impl AnchoredAtEndpoint {
    /// Parse the value's `endpoint` string. Anything else is refused, never guessed.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "source" => Some(Self::Source),
            "target" => Some(Self::Target),
            _ => None,
        }
    }

    /// The canonical spelling, as stored in the value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Target => "target",
        }
    }
}

/// One parsed `<resource-uuid>#<block-uuid>` anchor address — the one declared form: exactly
/// one `#`, both halves bare UUIDs already in canonical (lowercase, hyphenated) form.
///
/// A non-canonical spelling is refused rather than normalized: the row stores what the caller
/// sent, so an accepted-but-rewritten spelling would store a value the caller cannot query
/// back, and two spellings of one address would become two rows where the unique-active index
/// could have seen them as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchorAddress {
    pub resource: Uuid,
    pub block: Uuid,
}

impl AnchorAddress {
    /// Parse an address in the one declared form. Returns `None` for any other spelling —
    /// a missing or repeated `#`, a non-UUID half, or a UUID not already canonical.
    pub fn parse(address: &str) -> Option<Self> {
        let (resource_half, block_half) = address.split_once('#')?;
        if block_half.contains('#') {
            return None;
        }
        let resource = resource_half.parse::<Uuid>().ok()?;
        let block = block_half.parse::<Uuid>().ok()?;
        if resource.to_string() != resource_half || block.to_string() != block_half {
            return None;
        }
        Some(Self { resource, block })
    }
}

/// How one `anchored-at` row's address resolved — the block read's own three-state contract,
/// stated per row. `live`, `folded`, and `absent` are the block read's own state names,
/// serialized under `"state"` the same way its answer is.
///
/// Carried only on `anchored-at` rows; every other facet row states `null` for this field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AnchorAddressResolution {
    /// The addressed block resolves and is current.
    Live,
    /// The addressed block's row persists, folded away. The disposition is the block read's
    /// own gated envelope: every named successor already passed the caller's own read gate,
    /// and successors the caller cannot read are omitted entirely (no id, no count). The
    /// qualification is never re-pointed by the fold — a dangling anchor states where its
    /// content went rather than silently following it.
    Folded {
        /// The fold act the resolution walked.
        folded_by_event_id: Uuid,
        disposition: BlockFoldDisposition,
    },
    /// No block answers under the addressed resource, or the addressed resource is not
    /// readable — stated identically either way, exactly as the block read itself answers
    /// an unresolvable address. Never a computed negative: an address you cannot read tells
    /// you nothing about what it would have corroborated.
    Absent,
}

/// Whether an `anchored-at` row agrees with the anchored block's own attribution, stated
/// where an edge declares a direction and the row anchors the declared side. The comparison
/// runs against the block's live attribution only — a corrected (retracted) attribution row
/// never corroborates — and carried rows corroborate like direct ones.
///
/// `null` is rendered, never a computed negative: a row whose edge declares no direction, a
/// row anchored off the declared side, and a row whose address did not resolve `live` all
/// state `null`, so an edge kind can gain its direction additively and old readers survive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AnchorVerdict {
    /// The block's live attribution is non-empty and names the edge's peer — the two
    /// records agree.
    Corroborated,
    /// The block's live attribution is non-empty and does not name the edge's peer — the
    /// two records disagree about where this relationship manifests. Rendered by name,
    /// never smoothed: nothing on either side is rewritten or dropped.
    Divergent,
    /// The block carries no live attribution rows — an absence of testimony, never
    /// readable as agreement or disagreement.
    Unattributed,
}

/// Request body for `POST /api/relationships/{edge_handle}/facets` — a facet whose owner is an
/// **edge**.
///
/// A separate type from [`FacetSetRequest`] rather than an optional `edge` field on it, because the
/// owner is not a payload choice: it is in the path, and it selects a different authorization gate
/// (the edge's own mutability clauses, not `can_modify_resource`). Two shapes that can each be
/// parsed into exactly one owner beat one shape carrying two optional ids that must then be
/// validated into exactly one.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct EdgeFacetSetRequest {
    /// The facet's typed value payload — an **object** of `key` → value marks; same constraint as
    /// [`FacetSetRequest::values`].
    pub values: serde_json::Map<String, serde_json::Value>,
    /// Optional property key for a keyed single-row write (e.g. `anchored-at`): asserts `values`
    /// as ONE row under this key instead of the clustering `facet` verb. Omitted, the write is
    /// an ordinary facet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property_key: Option<String>,
    /// Relative weight of the facet; defaults to `1.0` when omitted, matching [`FacetSetRequest`].
    #[serde(default = "default_facet_weight")]
    pub weight: f64,
    /// Per-act correlation (`invocation_id`) + discrete agent authorship for the facet_set act.
    #[serde(default, flatten)]
    pub act: ActInput,
}

/// One property row owned by an edge, as read back by
/// `GET /api/relationships/{edge_handle}/facets`.
///
/// **Carries its author, because an edge facet is an evidential claim.** The use case this exists
/// for is *"this task witnesses clause X of goal G"* on an `advances` edge — a statement a later
/// reader weighs. Anyone with source-write and container-write on the edge may write one, which is
/// not the same set as the edge's asserter, so an unattributed row would let a planted claim read
/// identically to a steward's.
///
/// Attribution follows the precedent [`crate::types::citation_audit::CitationAuditRow`] set:
/// identity travels on the emitting event (`kb_events.emitter_entity_id → kb_entities.profile_id`),
/// and the row carries the profile **plus** its two human-readable `kb_profiles` columns so a
/// caller never needs a second round trip to name an author.
///
/// **`authored_by_event_id` is the replay-stable identity**, not `property_id` — a property row is
/// a masked surrogate whose id a replay re-mints, exactly as an audit's is.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct EdgeFacetRow {
    pub property_id: Uuid,
    /// `kb_properties.property_key`. `"facet"` for a clustering facet written by `facet_set`; an
    /// arbitrary key for a single-valued property written by `property_set`.
    pub property_key: String,
    pub value: serde_json::Value,
    pub weight: f64,
    /// `kb_properties.asserted_by_event_id` — the act that wrote this facet, and the row's
    /// replay-stable identity.
    pub authored_by_event_id: Uuid,
    /// The profile behind that act's emitter entity. `None` only if the emitter has no profile,
    /// which no live write path produces — carried as an `Option` rather than fabricating an id.
    pub authored_by_profile_id: Option<Uuid>,
    pub authored_by_handle: Option<String>,
    pub authored_by_display_name: Option<String>,
    /// For an `anchored-at` row: how the row's address resolved — `live`, `folded` (with the
    /// gated disposition), or `absent`. `null` for every other facet row.
    #[serde(default)]
    pub address_resolution: Option<AnchorAddressResolution>,
    /// For an `anchored-at` row resolving `live`: whether the anchored block's own live
    /// attribution corroborates the qualification (`corroborated`), disagrees with it
    /// (`divergent`), or is absent (`unattributed`). `null` when the row is not
    /// `anchored-at`, its address did not resolve `live`, or the edge declares no verdict
    /// direction for the row's anchored side.
    #[serde(default)]
    pub verdict: Option<AnchorVerdict>,
}

/// The live facets of one edge. Folded rows are excluded: folding an edge cascades to the
/// properties it owns, so a folded property here would mean a retracted relationship still
/// carrying live qualifiers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct EdgeFacetsResponse {
    pub edge_handle: Uuid,
    pub facets: Vec<EdgeFacetRow>,
}

/// One facet row owned by a **resource**, as read back by `GET /api/resources/{id}/facets`.
///
/// **Deliberately not [`EdgeFacetRow`], and not an alias of it.** The two look alike and are not
/// interchangeable: the gates differ (`resources_visible_to` vs `edges_visible_to`), an edge's facets
/// fold *with the edge* while a resource's do not, and a resource's `kb_properties` rows are shared
/// with the frontmatter tiers where an edge's are not. Two types that must be able to diverge beat
/// one type that makes divergence a breaking change.
///
/// **Why this carries `created` where [`EdgeFacetRow`] does not.** `facet_set` appends rather than
/// upserts, so one logical facet asserted twice leaves two live rows (task
/// `019f6d08-2b55-7ee0-b9ac-1959cf4d736b`; 30 resources in production carry 2 or 3). The whole point
/// of this read is that a caller can tell that case from a genuinely multi-valued facet, and a
/// timestamp per row is what makes the supersession legible without the read taking a position on
/// how supersession *should* work — a fork this read deliberately does not settle.
///
/// **`property_key` rides on the row even though the read is scoped to `"facet"`.** It costs one
/// field now and makes widening the read to another property key additive later, rather than a wire
/// break. The read is scoped rather than general because a resource's other property rows *are* its
/// frontmatter — `get_meta` already serves them — so returning them all would be a second,
/// divergent copy of an existing surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceFacetRow {
    /// `kb_properties.id`. A **masked surrogate** — a replay re-mints it, so the replay-stable
    /// identity is `authored_by_event_id`, exactly as for an edge facet or a citation audit.
    pub property_id: Uuid,
    /// `kb_properties.property_key` — always `"facet"` for rows this read returns. Carried so the
    /// row shape survives widening the read past that one key.
    pub property_key: String,
    /// The facet's value payload, verbatim. An object, not a scalar, in every production row
    /// observed: `{"node_label": "concern", "status": "open", "severity": "high"}`.
    pub value: serde_json::Value,
    /// `kb_properties.weight`. **Load-bearing, and precisely what the frontmatter collapse drops**:
    /// the region producer clusters on it (`temper-substrate/src/substrate.rs:110-122` →
    /// `expand_facets`), and 128 production rows carry a non-default value.
    pub weight: f64,
    /// When the facet was asserted. What lets a caller order two live rows for one logical facet.
    pub created: DateTime<Utc>,
    /// `kb_properties.asserted_by_event_id` — the act that wrote this facet, and the row's
    /// replay-stable identity.
    pub authored_by_event_id: Uuid,
    /// The profile behind that act's emitter entity. `None` only if the emitter has no profile,
    /// which no live write path produces — carried as an `Option` rather than fabricating an id,
    /// matching [`EdgeFacetRow`].
    pub authored_by_profile_id: Option<Uuid>,
    pub authored_by_handle: Option<String>,
    pub authored_by_display_name: Option<String>,
}

/// The live facets of one resource, oldest-first within each key.
///
/// **No server-side collapse.** One element per live `kb_properties` row. `get_meta` already offers
/// the collapsed view (`open_meta.facet`, newest-wins, weight discarded); duplicating that here would
/// reproduce the very concealment this read exists to end.
///
/// An empty `facets` list means *readable, and nothing asserted* — distinct from the `404` a caller
/// who may not read the resource receives. See `facet_service::list_resource_facets` for why that
/// distinction is safe rather than an existence oracle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceFacetsResponse {
    pub resource: Uuid,
    pub facets: Vec<ResourceFacetRow>,
}

/// Acknowledgement returned by the facet write endpoint — **every row the assert wrote**.
///
/// A facet is stored one row per inner key (migration `20260730000010`), so `{status: open,
/// as_of: X}` is two rows and a singular ack could only name one of them. Which one it named would
/// be arbitrary, and a caller reading a single id back from a two-mark write would have a value
/// that *reads as complete* — precisely the defect `GET /api/resources/{id}/facets` shipped to end
/// (goal `019fafd9-a978-7860-ae39-23958b4471b8`). So the ack is plural at the type level: there is
/// no shape in which it can under-report.
///
/// Ordered as written — one entry per inner key of the asserted object, in the order the projector
/// walked them. A non-facet property write yields exactly one entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct FacetAck {
    /// The rows written. Never empty: an assert that names no mark is refused upstream rather than
    /// acknowledged with nothing (`facet_object_has_keys` in `db_backend`).
    pub property_ids: Vec<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::authorship::ConfidenceBand;

    fn values_example() -> serde_json::Map<String, serde_json::Value> {
        serde_json::json!({"summary": "example"})
            .as_object()
            .expect("example payload is an object")
            .clone()
    }

    /// The typed-object wire contract: a scalar `values` payload must be refused at
    /// deserialization — never forwarded to the database to learn the shape from an error.
    #[test]
    fn facet_set_request_rejects_scalar_values_payloads() {
        for payload in [
            serde_json::json!("a string"),
            serde_json::json!(7),
            serde_json::json!([1, 2]),
        ] {
            let wire = serde_json::json!({
                "resource": Uuid::nil(),
                "values": payload,
            });
            assert!(
                serde_json::from_value::<FacetSetRequest>(wire).is_err(),
                "scalar `values` ({payload}) must not deserialize"
            );
        }
    }

    #[test]
    fn facet_set_request_round_trips_without_act() {
        let req = FacetSetRequest {
            resource: Uuid::nil(),
            values: values_example(),
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
            values: values_example(),
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
            property_ids: vec![Uuid::nil()],
        };
        let v = serde_json::to_value(&ack).unwrap();
        let back: FacetAck = serde_json::from_value(v).unwrap();
        assert_eq!(back.property_ids, ack.property_ids);
    }

    /// The reason this type is plural. A two-key assert writes two rows, and the ack has to be able
    /// to say so — under the old singular shape this information had nowhere to go, so the caller
    /// received one id and no indication that a second row existed.
    #[test]
    fn facet_ack_reports_every_row_a_multi_key_assert_wrote() {
        let ack = FacetAck {
            property_ids: vec![
                Uuid::from_u128(1), // {"status": "open"}
                Uuid::from_u128(2), // {"as_of": "X"}
            ],
        };
        let v = serde_json::to_value(&ack).expect("serialize");
        assert_eq!(
            v["property_ids"].as_array().map(Vec::len),
            Some(2),
            "both rows must be named on the wire: {v}"
        );
        let back: FacetAck = serde_json::from_value(v).unwrap();
        assert_eq!(back.property_ids, ack.property_ids, "order is as written");
    }

    fn plain_facet_row() -> EdgeFacetRow {
        EdgeFacetRow {
            property_id: Uuid::nil(),
            property_key: "facet".to_string(),
            value: serde_json::json!({"status": "open"}),
            weight: 1.0,
            authored_by_event_id: Uuid::nil(),
            authored_by_profile_id: None,
            authored_by_handle: None,
            authored_by_display_name: None,
            address_resolution: None,
            verdict: None,
        }
    }

    /// The row's own wire contract for the two anchored-at fields: they serialize as
    /// PRESENT-but-null (matching the sibling author fields — no skip-serializing), and a
    /// payload that omits them deserializes to `None`, so a new reader reading an old
    /// writer's wire parses.
    #[test]
    fn edge_facet_row_states_resolution_and_verdict_null_never_absent() {
        let v = serde_json::to_value(plain_facet_row()).unwrap();
        assert!(
            v.get("address_resolution").is_some(),
            "the field must be present, never absent: {v}"
        );
        assert_eq!(v["address_resolution"], serde_json::Value::Null);
        assert!(v.get("verdict").is_some(), "present, never absent: {v}");
        assert_eq!(v["verdict"], serde_json::Value::Null);

        let mut wire = v.clone();
        wire.as_object_mut().unwrap().remove("address_resolution");
        wire.as_object_mut().unwrap().remove("verdict");
        let back: EdgeFacetRow = serde_json::from_value(wire).unwrap();
        assert_eq!(back.address_resolution, None);
        assert_eq!(back.verdict, None);
    }

    /// The resolution states the block read's own state names under `"state"`, and the
    /// folded arm carries the gated disposition envelope verbatim.
    #[test]
    fn anchor_resolution_round_trips_the_block_read_state_names() {
        let live = serde_json::to_value(AnchorAddressResolution::Live).unwrap();
        assert_eq!(live["state"], "live");

        let absent = serde_json::to_value(AnchorAddressResolution::Absent).unwrap();
        assert_eq!(absent["state"], "absent");

        let folded = AnchorAddressResolution::Folded {
            folded_by_event_id: Uuid::nil(),
            disposition: BlockFoldDisposition::Unrecorded,
        };
        let v = serde_json::to_value(&folded).unwrap();
        assert_eq!(v["state"], "folded");
        assert_eq!(v["disposition"]["disposition"], "unrecorded");
        let back: AnchorAddressResolution = serde_json::from_value(v).unwrap();
        assert_eq!(back, folded);
    }

    /// The verdict is a plain name on the wire — no payload to misread as a computation.
    #[test]
    fn anchor_verdict_round_trips_by_name() {
        for (verdict, word) in [
            (AnchorVerdict::Corroborated, "corroborated"),
            (AnchorVerdict::Divergent, "divergent"),
            (AnchorVerdict::Unattributed, "unattributed"),
        ] {
            let v = serde_json::to_value(verdict).unwrap();
            assert_eq!(v, serde_json::json!(word));
            let back: AnchorVerdict = serde_json::from_value(v).unwrap();
            assert_eq!(back, verdict);
        }
    }
}
