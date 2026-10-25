//! The batch read's wire — one ask in, the matching artifacts out, whole, in bounded pages (spec
//! `temper-artifacts:specs/2026-10-19-asking-artifacts-by-what-they-are-design.md` §7).
//!
//! A read, not an act: it never enters a composition, and its result is artifacts, never an
//! `IdSet` or a `StageInput` (§9). [`AskRequest::refusals`] is the half of its refusals that needs
//! no shape — the page bound and the request's value budget — decided here, offline, and returned
//! beside whatever the ask's own validation refuses.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{ArtifactAsk, ShapeIdentity, ShapeSuccessor};
use crate::types::data_artifact::{
    ArtifactCursor, ArtifactView, ARTIFACT_DRAIN_PAGE_ROWS, MAX_ARTIFACT_PAGE_ROWS,
};
use crate::types::ids::{DataArtifactId, ResourceId, ShapeId};
use crate::types::query::filter::MAX_COMPOSITION_PROPERTY_VALUE_BYTES;
use crate::types::query::{ArtifactAskPopulation, PlanRefusal, RefusalReason};

/// One page of a batch read: the ask, re-sent whole on every page, and where to continue.
///
/// **The continuation carries position, never authority** (§7.3): every page runs the ask's
/// validation again for the asker, so a cursor handed to another principal, or replayed after
/// access is revoked, reveals nothing a first page would not.
///
/// **Strict at its own top level**, like [`ArtifactAsk`]: an unknown field beside `ask`, `limit`
/// and `after` is a body that cannot be read, never a field that silently changes nothing.
/// `after` is one [`ArtifactCursor`] object, so half a position is a body that cannot be read
/// either; the cursor itself reads only its two fields and ignores any other, because it is also a
/// response type that a client must read from a newer server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AskRequest {
    /// The question, against its governing shape.
    pub ask: ArtifactAsk,
    /// How many matching artifacts this page examines — served and oversized alike. At least 1
    /// and at most `MAX_ARTIFACT_PAGE_ROWS`, which is 200; a value outside that range is refused
    /// (`page_limit_out_of_range`), never clamped. Absent: `ARTIFACT_DRAIN_PAGE_ROWS`, which is
    /// 50.
    #[serde(default)]
    #[cfg_attr(feature = "typescript", ts(optional))]
    pub limit: Option<u32>,
    /// Continue after this position: the previous page's `next`, unchanged, with the same ask.
    /// Absent: the first page.
    #[serde(default)]
    #[cfg_attr(feature = "typescript", ts(optional))]
    pub after: Option<ArtifactCursor>,
}

/// One page of the artifacts matching an ask, in `(created, artifact_id)` order, oldest first.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AskResponse {
    /// The shape version this page was validated against: on the first page the version the ask
    /// named, and on every later page that same version, even once amended (see `amended_since`).
    pub shape: AskedShape,
    /// The matching artifacts that fit this page's byte budget and depth, each whole: `content` is
    /// the stored content, absent for an artifact committed with no bytes. `resource_id` is
    /// attribution, not a set.
    pub artifacts: Vec<ArtifactView>,
    /// Matching artifacts too large for any page's byte budget, or too deeply nested for a page
    /// (content more than 96 arrays or objects deep), in order among the rest, by id and size and
    /// without content. Each is read whole by id (`GET /api/data-artifacts/{artifact_id}`); on the
    /// hosted deployment that read is refused too when the response carrying the artifact, with
    /// its content as storage serves it, is longer than the platform's response cap (see
    /// `OversizedArtifact`).
    pub oversized: Vec<OversizedArtifact>,
    /// What the ask counted over its whole population, on the first page; `null` on every later
    /// page. These are exact counts over the whole counted set: to count them, the first page
    /// matches every governed artifact of the home, content included, and the read budget that
    /// bounds the page's own rows does not bound that. They are read in a statement of their own
    /// beside the page's, so a commit landing between the two can make `matched` differ from the
    /// rows by that commit; they describe the population, not the page.
    // utoipa reads `Option` as "may be omitted"; the key is always written, `null` included, so
    // the published contract says required (the `located_at` rule in `query/hits.rs`).
    #[cfg_attr(feature = "web-api", schema(required = true))]
    pub population: Option<ArtifactAskPopulation>,
    /// The order rows arrive in. Enumeration order, never rank.
    pub ordered_by: AskOrder,
    /// Where to continue, or `null` when the enumeration is over. Pass it back unchanged as the
    /// next request's `after`, with the same ask. Only `null` ends an enumeration: a page also
    /// stops when it has read as much as one page may read, and then `next` is the last artifact it
    /// read, matched or not, so a page can carry no artifacts and still a `next`.
    // Always written, `null` included: required in the published contract, as `population`.
    #[cfg_attr(feature = "web-api", schema(required = true))]
    pub next: Option<ArtifactCursor>,
    /// The version of the shape's lineage now in force, when the version this enumeration was
    /// validated against has been amended since its first page; `null` otherwise. A continuation
    /// is validated against the version the first page named, but each row's `shape_state`, and
    /// any `shape_states` narrowing, are read against the version now in force, whose verdicts
    /// reconciliation may not yet have written. So an ask narrowed by `shape_states` should start
    /// again from a first page, asking the version `amended_since` names, when it appears.
    // Always written, `null` included: required in the published contract, as `population`.
    #[cfg_attr(feature = "web-api", schema(required = true))]
    pub amended_since: Option<ShapeSuccessor>,
}

/// The shape version a batch page was validated against, and the home its population ranges over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct AskedShape {
    pub shape_id: ShapeId,
    pub shape_version: i32,
    /// `"kb_contexts"` or `"kb_cogmaps"`.
    pub home_anchor_table: String,
    pub home_anchor_id: Uuid,
}

impl From<&ShapeIdentity> for AskedShape {
    fn from(shape: &ShapeIdentity) -> AskedShape {
        AskedShape {
            shape_id: shape.shape_id,
            shape_version: shape.shape_version,
            home_anchor_table: shape.home_anchor_table.clone(),
            home_anchor_id: shape.home_anchor_id,
        }
    }
}

/// A matching artifact too large for a page's byte budget by itself, its content and the rest of its
/// row together, or too deeply nested for a page, its content more than 96 arrays or objects deep:
/// named, sized, and left to be read by id. A page holds content to that depth so that every
/// first-party client parses every page. A read by id writes the content less deeply, but a
/// client's own nesting limit can refuse that read too when the content nests very deeply. And a
/// read by id is not held to a page's byte budget: on the hosted deployment it is refused when the
/// response carrying the artifact, with its content as storage serves it, is longer than the
/// platform's response cap, which content committed within the request cap can reach, because
/// storage prints some numbers longer than they were committed (see `content_bytes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct OversizedArtifact {
    pub artifact_id: DataArtifactId,
    pub resource_id: ResourceId,
    /// The artifact's stored `content_bytes`: its content's length in compact JSON as committed.
    /// The content a page or a read by id serves can be longer, because storage prints a number
    /// from 1e16 up to the 64-bit integer limits in full (`1e19`, committed as the five bytes
    /// `1e+19`, is served as twenty digits). A page measures what it would write, so an artifact
    /// can be named here though its `content_bytes` alone would fit a page.
    pub content_bytes: i64,
}

/// The order a batch read returns rows in: `created`, oldest first, ties broken by artifact id.
/// The one order v0 serves; closed, so a reader can match it exhaustively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum AskOrder {
    Created,
}

impl AskRequest {
    /// The matching rows this page examines: `limit`, or [`ARTIFACT_DRAIN_PAGE_ROWS`] when absent.
    /// Within `1..=MAX_ARTIFACT_PAGE_ROWS` only for a request [`AskRequest::refusals`] admits.
    pub fn rows(&self) -> u32 {
        self.limit.unwrap_or(ARTIFACT_DRAIN_PAGE_ROWS)
    }

    /// Every refusal this request earns without its shape: a `limit` outside
    /// `1..=MAX_ARTIFACT_PAGE_ROWS` (`page_limit_out_of_range`), and predicate values past the
    /// per-request budget (`property_value_budget_exceeded`, counted by
    /// [`ArtifactAsk::value_bytes`] against `MAX_COMPOSITION_PROPERTY_VALUE_BYTES`, the budget a
    /// composition's values share — spec §3.4). Each names the body field at fault and is
    /// returned beside the ask's own refusals, never instead of them. `stage` is `None`: a batch
    /// read has no stages.
    pub fn refusals(&self) -> Vec<PlanRefusal> {
        let mut errs = Vec::new();
        match self.limit {
            Some(0) => errs.push(refused(
                RefusalReason::PageLimitOutOfRange,
                "`limit` must be at least 1; a page of zero artifacts cannot make progress"
                    .to_string(),
            )),
            Some(limit) if limit > MAX_ARTIFACT_PAGE_ROWS => errs.push(refused(
                RefusalReason::PageLimitOutOfRange,
                format!(
                    "`limit` {limit} is above the ceiling `MAX_ARTIFACT_PAGE_ROWS` \
                     ({MAX_ARTIFACT_PAGE_ROWS}); ask for at most {MAX_ARTIFACT_PAGE_ROWS} and \
                     continue with `after`"
                ),
            )),
            _ => {}
        }
        let value_bytes = self.ask.value_bytes();
        if value_bytes > MAX_COMPOSITION_PROPERTY_VALUE_BYTES {
            errs.push(refused(
                RefusalReason::PropertyValueBudgetExceeded,
                format!(
                    "the predicate values of `ask` may total at most \
                     {MAX_COMPOSITION_PROPERTY_VALUE_BYTES} bytes; this one carries {value_bytes}"
                ),
            ));
        }
        errs
    }
}

fn refused(reason: RefusalReason, detail: String) -> PlanRefusal {
    PlanRefusal {
        stage: None,
        reason,
        detail,
        teaches: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::artifact_ask::{ArtifactPredicate, ShapeContext};
    use crate::types::query::PropertyOp;
    use chrono::{DateTime, Utc};
    use serde_json::json;

    fn ask() -> ArtifactAsk {
        ArtifactAsk {
            shape: ShapeId::from(Uuid::from_u128(7)),
            predicates: vec![],
            intent: vec![],
            include_folded: false,
            shape_states: vec![],
        }
    }

    fn request(limit: Option<u32>) -> AskRequest {
        AskRequest {
            ask: ask(),
            limit,
            after: None,
        }
    }

    fn reasons(rs: &[PlanRefusal]) -> Vec<RefusalReason> {
        rs.iter().map(|r| r.reason.clone()).collect()
    }

    fn cursor() -> ArtifactCursor {
        ArtifactCursor {
            created: DateTime::parse_from_rfc3339("2026-10-24T09:15:30.123456Z")
                .expect("literal parses")
                .with_timezone(&Utc),
            artifact_id: DataArtifactId::from(Uuid::from_u128(9)),
        }
    }

    /// **A limit outside `1..=MAX_ARTIFACT_PAGE_ROWS` is refused by name, with no stage, naming the
    /// body field `limit`; one inside it, or none, is not.** Bites: the `Some(0)` arm removed (zero
    /// is admitted), or the ceiling compared with `>=` (200 is refused).
    #[test]
    fn a_limit_outside_the_page_range_is_refused_by_name_and_never_clamped() {
        for limit in [0, MAX_ARTIFACT_PAGE_ROWS + 1] {
            let rs = request(Some(limit)).refusals();
            assert_eq!(
                reasons(&rs),
                [RefusalReason::PageLimitOutOfRange],
                "{limit}"
            );
            assert_eq!(rs[0].stage, None);
            assert!(rs[0].detail.contains("`limit`"), "{}", rs[0].detail);
            assert!(rs[0].teaches.is_none());
        }
        for limit in [Some(1), Some(MAX_ARTIFACT_PAGE_ROWS), None] {
            assert!(request(limit).refusals().is_empty(), "{limit:?}");
        }
        assert_eq!(
            serde_json::to_value(RefusalReason::PageLimitOutOfRange).unwrap(),
            json!("page_limit_out_of_range")
        );
    }

    /// **Absent, the page is the drain size; present, it is the caller's.** Bite: `rows` answering
    /// `MAX_ARTIFACT_PAGE_ROWS` for an absent limit.
    #[test]
    fn an_absent_limit_examines_the_drain_page() {
        assert_eq!(request(None).rows(), ARTIFACT_DRAIN_PAGE_ROWS);
        assert_eq!(request(Some(3)).rows(), 3);
    }

    /// **A paging refusal travels beside the ask's own, never instead of it.** An ask whose shape
    /// is not in the context is refused `shape_not_askable` by the session; the same request's
    /// out-of-range `limit` is refused by [`AskRequest::refusals`]; the two lists are disjoint and
    /// together are what the door returns. The concatenation itself is the service's; this is the
    /// offline half. Bite: `refusals` reading the limit only when the ask validates.
    #[test]
    fn a_paging_refusal_is_decided_beside_an_ask_refusal() {
        let req = request(Some(0));
        let ask_refusals = ShapeContext::empty()
            .session()
            .validate(&req.ask)
            .expect_err("no shape loaded");
        assert_eq!(reasons(&ask_refusals), [RefusalReason::ShapeNotAskable]);
        assert_eq!(
            reasons(&req.refusals()),
            [RefusalReason::PageLimitOutOfRange]
        );
    }

    /// **The request's predicate values share one budget**, measured as a composition's are.
    /// Thirty-three values of 16,000 bytes are over 512 KiB; thirty-two are not. Bite: the budget
    /// check removed, or compared against a per-value cap.
    #[test]
    fn predicate_values_past_the_request_budget_are_refused() {
        let with_values = |n: usize| AskRequest {
            ask: ArtifactAsk {
                predicates: vec![ArtifactPredicate {
                    path: vec!["unit".to_string()],
                    op: PropertyOp::Contains {
                        values: (0..n).map(|i| json!(format!("{i:0>15998}"))).collect(),
                    },
                }],
                ..ask()
            },
            limit: None,
            after: None,
        };
        assert!(with_values(32).refusals().is_empty());
        let rs = with_values(33).refusals();
        assert_eq!(reasons(&rs), [RefusalReason::PropertyValueBudgetExceeded]);
        assert!(rs[0].detail.contains("`ask`"), "{}", rs[0].detail);
    }

    /// **The request is strict, and its position is one object.** An unknown field (an `order`
    /// v0 does not serve) and a cursor missing a half both fail to read; a whole cursor reads.
    /// Bites: `deny_unknown_fields` removed, or `after` split into two optional fields.
    #[test]
    fn a_request_reads_only_what_it_declares_and_its_position_whole() {
        let base = json!({ "ask": { "shape": Uuid::from_u128(7) } });
        assert!(serde_json::from_value::<AskRequest>(base.clone()).is_ok());

        let mut ordered = base.clone();
        ordered["order"] = json!("desc");
        assert!(serde_json::from_value::<AskRequest>(ordered).is_err());

        let mut half = base.clone();
        half["after"] = json!({ "created": "2026-10-24T09:15:30.123456Z" });
        assert!(serde_json::from_value::<AskRequest>(half).is_err());

        let mut whole = base;
        whole["after"] = serde_json::to_value(cursor()).unwrap();
        let read: AskRequest = serde_json::from_value(whole).unwrap();
        assert_eq!(read.after, Some(cursor()));
    }

    /// **`next`, `population` and `amended_since` are on every page, `null` when there is nothing to
    /// say; the order is named.** A reader tells the last page from an answer that lost its
    /// continuation. Each key must be present and `null`, so a key left out fails. Bites:
    /// `#[serde(skip_serializing_if = "Option::is_none")]` on any of the three (its key is then
    /// absent), or `ordered_by` renamed.
    #[test]
    fn the_response_names_every_continuation_field_and_its_order() {
        let shape = ShapeIdentity {
            shape_id: ShapeId::from(Uuid::from_u128(7)),
            shape_version: 2,
            home_anchor_table: "kb_contexts".to_string(),
            home_anchor_id: Uuid::from_u128(1),
            kind_owner_table: "kb_profiles".to_string(),
            kind_owner_id: Uuid::from_u128(2),
            artifact_kind: "measurement".to_string(),
        };
        let last = serde_json::to_value(AskResponse {
            shape: AskedShape::from(&shape),
            artifacts: vec![],
            oversized: vec![],
            population: None,
            ordered_by: AskOrder::Created,
            next: None,
            amended_since: None,
        })
        .unwrap();
        for key in ["population", "next", "amended_since"] {
            assert!(
                last.get(key).is_some_and(serde_json::Value::is_null),
                "{key} present and null in {last}"
            );
        }
        assert_eq!(last["ordered_by"], json!("created"));
        assert_eq!(
            last["shape"],
            json!({
                "shape_id": Uuid::from_u128(7),
                "shape_version": 2,
                "home_anchor_table": "kb_contexts",
                "home_anchor_id": Uuid::from_u128(1),
            })
        );
        assert!(serde_json::from_value::<AskOrder>(json!("rank")).is_err());
    }

    /// **The masquerade boundary, the batch direction (spec §9): a batch answer cannot enter a
    /// composition as a set, and a composition returns no artifact section.** A full page — rows,
    /// an oversized entry, a population and a continuation — fails to read as a `StageInput` or as
    /// an `IdSet`, so nothing in the composition grammar accepts it; and `/api/query` still admits
    /// `OpenMeta` alone. Bites: an `IdSet` that reads anything (`#[serde(default)]` on `kind` and
    /// `ids`), or a second admitted section.
    #[test]
    fn a_batch_answer_is_never_a_set_and_a_composition_returns_no_artifacts() {
        use crate::types::query::{IdSet, ReturnSpec, StageInput};
        use crate::types::resource_view::ResourceSection;

        let shape = ShapeIdentity {
            shape_id: ShapeId::from(Uuid::from_u128(7)),
            shape_version: 1,
            home_anchor_table: "kb_contexts".to_string(),
            home_anchor_id: Uuid::from_u128(1),
            kind_owner_table: "kb_profiles".to_string(),
            kind_owner_id: Uuid::from_u128(2),
            artifact_kind: "measurement".to_string(),
        };
        let row = ArtifactView {
            artifact_id: DataArtifactId::from(Uuid::from_u128(10)),
            resource_id: ResourceId::from(Uuid::from_u128(11)),
            kind_owner_table: "kb_profiles".to_string(),
            kind_owner_id: Uuid::from_u128(2),
            artifact_kind: "measurement".to_string(),
            intent: "member".to_string(),
            precedence: 0.0,
            content_hash: "0".repeat(64),
            content_bytes: 13,
            shape_state: "declared_satisfied".to_string(),
            is_folded: false,
            created: cursor().created,
            content: Some(json!({ "unit": "ms" })),
        };
        let page = serde_json::to_value(AskResponse {
            shape: AskedShape::from(&shape),
            artifacts: vec![row],
            oversized: vec![OversizedArtifact {
                artifact_id: DataArtifactId::from(Uuid::from_u128(12)),
                resource_id: ResourceId::from(Uuid::from_u128(11)),
                content_bytes: 3_000_000,
            }],
            population: Some(ArtifactAskPopulation {
                declared_satisfied: 2,
                declared_not_satisfied: 0,
                declared_not_yet_checked: 0,
                matched: 2,
            }),
            ordered_by: AskOrder::Created,
            next: Some(cursor()),
            amended_since: Some(ShapeSuccessor {
                shape_id: ShapeId::from(Uuid::from_u128(8)),
                shape_version: 2,
            }),
        })
        .unwrap();
        assert!(
            serde_json::from_value::<StageInput>(page.clone()).is_err(),
            "a batch answer is not a stage input"
        );
        assert!(
            serde_json::from_value::<IdSet>(page).is_err(),
            "a batch answer is not an id set"
        );
        assert_eq!(ReturnSpec::ADMITTED_SECTIONS, [ResourceSection::OpenMeta]);
    }
}
