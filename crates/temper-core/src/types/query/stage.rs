//! Stage identity, inputs, and outputs.
//!
//! One responsibility: what a node is called, what flows *into* it, and what it hands *out*. The
//! gap this whole phase exists to close lives here — [`StageInput`] lets an invocation finally
//! *declare* the upstream reference that [`super::trace::InputSource::Upstream`] could already
//! only *report*.

use serde::{Deserialize, Serialize};

use super::hits::{FtsHit, GraphHit, RegionHit, VecHit};
use super::id_set::{IdKind, IdSet};

/// A stage's name, and — because [`StageName::parse`] is the only constructor — a proof that the
/// name is a safe SQL identifier.
///
/// The compiler (beat C, Task 9) emits stage names as CTE identifiers. Parse-don't-validate is the
/// whole design: a name that cannot be constructed cannot reach SQL, so there is deliberately no
/// `new_unchecked`. The accepted shape is `[a-z][a-z0-9_]{0,62}` — a leading lowercase letter,
/// then up to 62 more lowercase-alphanumeric-or-underscore characters (63 total).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "mcp", schemars(inline))]
// Transparent on the wire (a bare string), but validated on the way in: `try_from` routes every
// deserialization through [`StageName::parse`], so a malformed name fails to parse rather than
// constructing an unsafe identifier. A plain `#[serde(transparent)]` would skip that check.
#[serde(try_from = "String", into = "String")]
pub struct StageName(String);

impl StageName {
    /// Rejects anything outside `[a-z][a-z0-9_]{0,62}`. The only constructor, so a `StageName` is
    /// evidence of validity — Task 9 relies on that for SQL identifier safety.
    pub fn parse(raw: &str) -> Option<StageName> {
        if raw.is_empty() || raw.len() > 63 {
            return None;
        }
        let mut chars = raw.chars();
        let first = chars.next()?;
        if !first.is_ascii_lowercase() {
            return None;
        }
        if chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            Some(StageName(raw.to_string()))
        } else {
            None
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for StageName {
    type Error = String;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        StageName::parse(&raw).ok_or_else(|| format!("`{raw}` is not a valid stage name"))
    }
}

impl From<StageName> for String {
    fn from(name: StageName) -> String {
        name.0
    }
}

/// What the receiving act does with the set it was handed.
///
/// Declared at the CONSUMING stage, never the producing one — the producer emits membership and
/// has no opinion about what the next act does with it.
///
/// ```text
/// bound — narrow to within this set.  Output ⊆ input.
/// seed  — grow from this set.         Output ESCAPES input.
/// ```
///
/// **A composition is not monotonically narrowing.** A pipeline may alternate:
/// `find-about-anywhere` (fuzzy, wide) → `follow-from` as a SEED (reaches beyond) → `find-exact`
/// as a BOUND (narrows again). Any code assuming a stage's output is a subset of its input is
/// wrong for half of these.
///
/// # Why this is on the wire rather than derived from the act
///
/// Across all seven acts `accepts_bounds` and `accepts_seeds` are DISJOINT, so the relation is
/// fully determined by the act and this field can only ever agree with the declaration or be
/// refused. It is carried anyway, for the negative face: a caller who writes `seed` against
/// `find-exact` meant something real — *reach beyond these* — and `find-exact` cannot. Deriving
/// the relation from the act would silently execute a NARROWING instead, answering a different
/// question than the one asked. Refusing tells them what the system cannot do.
/// `[decided — 2026-08-08, Pete]`
///
/// # The third id set, which is not on this axis
///
/// The visibility verdict is hard, applies to every stage, and is **never expressible here**. It
/// is not a `StageRelation` value and must never become one — `query_plan`'s ungated-core call
/// fixes it as a non-parameter for exactly that reason.
///
/// Replaces the incumbent `BoundsMode`, which sat on the invocation as an `Option` whose
/// "required whenever `input` is present" invariant was held by prose. That admitted a meaningless
/// state — input present, relation absent — which the validator silently read as `bound`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum StageRelation {
    /// Narrow to within this set.
    Bound,
    /// Grow from this set.
    Seed,
}

/// Where a stage's set comes from, and WHAT IT IS FOR.
///
/// This is the type that closes the gap. Before it, an invocation could carry only a literal
/// `bounds: Option<IdSet>` and had no way to name a producing stage.
///
/// [`StageRelation`] is a field of every variant rather than an `Option` on the invocation, so
/// "an input with no declared relation" is **unrepresentable** rather than merely invalid. The
/// relation belongs to the edge, not to the stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", tag = "from")]
pub enum StageInput {
    /// A literal id set the caller supplied — the incumbent `bounds` case, now one input variant.
    Caller {
        #[serde(rename = "as")]
        relation: StageRelation,
        ids: IdSet,
    },
    /// The `produced` set of an earlier stage, named rather than copied.
    Upstream {
        #[serde(rename = "as")]
        relation: StageRelation,
        stage: StageName,
    },
}

impl StageInput {
    /// What the receiving act does with this set. Total, because every variant carries one —
    /// callers read the relation without matching on where the set came from, which are two
    /// independent questions.
    pub fn relation(&self) -> StageRelation {
        match self {
            StageInput::Caller { relation, .. } | StageInput::Upstream { relation, .. } => {
                *relation
            }
        }
    }
}

/// What a RETURNED stage produced, tagged by currency AND quantity.
///
/// # There is no `ids` variant
///
/// `[decided — 2026-08-09, Pete]` A returned stage is always hydrated: resource hits or region
/// hits, never a bare id set.
///
/// Ids remain the pipe's **internal** currency — the only value that crosses a stage boundary is
/// membership, and an intermediate stage that merely feeds a downstream one is never hydrated at
/// all. That is unchanged, and it is what keeps `no-cross-act-ranking` structural. What is gone is
/// `ids` as a thing a caller can ASK to be given back.
///
/// Subselecting a result set is a client concern with a good answer already (`temper … | jq`), and
/// the principled version is a known later door: GraphQL over this surface, where a caller opts
/// into exactly the parts they want. Shipping one coarse "just ids" toggle now would occupy the
/// space that door is for, and it would be the only variant whose shape a caller could not read off
/// the act declarations.
///
/// # Why the quantity is in the tag, not just the row
///
/// `[decided during build — 2026-08-09]` The contract drafted this as one `resource_hits` variant
/// holding a per-item union of the three hit types. Four flat variants instead, for two reasons:
///
/// * A union on the ARRAY ITEM permits a list holding an [`FtsHit`] beside a [`VecHit`], policed
///   only by a sentence saying it must not. A stage runs ONE act and an act produces ONE quantity,
///   so homogeneity is a property of the type here rather than of everyone remembering.
///
/// * It makes `/api/query/validate` self-sufficient. Told `resource_hits`, a caller still does not
///   know whether to read `fts_norm` or `vec_norm` and must cross-reference `orders_by`. Told
///   `vec_hits`, they know — and telling them the whole answer before they run anything is that
///   route's entire purpose.
///
/// The tag is DERIVABLE BEFORE EXECUTION from the act's declared `produces` and `orders_by`, which
/// is what makes that possible: this union has no member an act cannot declare in advance.
///
/// Neither `PartialEq` nor `Eq`: [`crate::types::resource_view::ResourceView`] derives neither, and
/// the quantities are floats.
/// Tests compare the serialized form, which is the thing a client actually observes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", tag = "produced")]
pub enum StageOutput {
    /// Produced by `find-exact`.
    FtsHits { hits: Vec<FtsHit> },
    /// Produced by `find-about-anywhere` and `find-about-within`.
    VecHits { hits: Vec<VecHit> },
    /// Produced by `follow-from`.
    GraphHits { hits: Vec<GraphHit> },
    /// Produced by `survey`.
    RegionHits { hits: Vec<RegionHit> },
}

/// Which [`StageOutput`] variant a stage carries — the tag, as a value.
///
/// It exists so `/api/query/validate` can PROMISE a variant using the same type the response
/// REPORTS, rather than a parallel enum that would drift. [`StageOutput::variant`] answers it from
/// an actual output; [`super::act::ActDeclaration::produced_variant`] predicts it from a
/// declaration, and a test asserts the two agree for every act.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ProducedVariant {
    FtsHits,
    VecHits,
    GraphHits,
    RegionHits,
}

impl StageOutput {
    /// Which variant this is — the same value `/api/query/validate` promised in advance.
    pub fn variant(&self) -> ProducedVariant {
        match self {
            StageOutput::FtsHits { .. } => ProducedVariant::FtsHits,
            StageOutput::VecHits { .. } => ProducedVariant::VecHits,
            StageOutput::GraphHits { .. } => ProducedVariant::GraphHits,
            StageOutput::RegionHits { .. } => ProducedVariant::RegionHits,
        }
    }

    /// The kind of thing this stage produced. Contract chaining compares kinds, so wrapping the
    /// rows must not cost that comparison.
    pub fn kind(&self) -> IdKind {
        match self {
            StageOutput::FtsHits { .. }
            | StageOutput::VecHits { .. }
            | StageOutput::GraphHits { .. } => IdKind::Resource,
            StageOutput::RegionHits { .. } => IdKind::Region,
        }
    }

    /// The wire tag, which is also what `/api/query/validate` promises in advance.
    ///
    /// Derived through serde rather than a second match, so the value a caller is promised and the
    /// value they receive cannot drift — two hand-written lists is the `ADMIN_EVENT_TYPES` failure.
    pub fn produced_tag(&self) -> &'static str {
        match self {
            StageOutput::FtsHits { .. } => "fts_hits",
            StageOutput::VecHits { .. } => "vec_hits",
            StageOutput::GraphHits { .. } => "graph_hits",
            StageOutput::RegionHits { .. } => "region_hits",
        }
    }

    pub fn len(&self) -> usize {
        match self {
            StageOutput::FtsHits { hits } => hits.len(),
            StageOutput::VecHits { hits } => hits.len(),
            StageOutput::GraphHits { hits } => hits.len(),
            StageOutput::RegionHits { hits } => hits.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::query::id_set::{IdKind, IdSet};

    #[test]
    fn a_stage_name_is_a_safe_sql_identifier_or_it_does_not_exist() {
        // Task 9 emits stage names as CTE identifiers. The type is the gate: if a name cannot be
        // constructed, it cannot reach SQL. This is parse-don't-validate, and it is the reason
        // there is no `StageName::new_unchecked`.
        assert!(StageName::parse("hits").is_some());
        assert!(StageName::parse("wide_arm_2").is_some());
        assert!(StageName::parse("Hits").is_none(), "uppercase rejected");
        assert!(
            StageName::parse("2hits").is_none(),
            "must start with a letter"
        );
        assert!(StageName::parse("hits-2").is_none(), "hyphen rejected");
        assert!(StageName::parse("hits\"; DROP TABLE kb_resources; --").is_none());
        assert!(StageName::parse("").is_none());
        assert!(
            StageName::parse(&"a".repeat(64)).is_none(),
            "63 is the ceiling"
        );
    }

    #[test]
    fn an_input_distinguishes_caller_ids_from_an_upstream_reference() {
        // THE gap this whole phase exists to close: the invocation side can finally declare what
        // BoundsSource has always been able to report.
        let caller = StageInput::Caller {
            relation: StageRelation::Bound,
            ids: IdSet {
                kind: IdKind::Resource,
                provenance: None,
                ids: vec![],
            },
        };
        let upstream = StageInput::Upstream {
            relation: StageRelation::Seed,
            stage: StageName::parse("hits").unwrap(),
        };
        assert_ne!(
            serde_json::to_string(&caller).unwrap(),
            serde_json::to_string(&upstream).unwrap()
        );
        for v in [caller, upstream] {
            assert_eq!(
                serde_json::from_str::<StageInput>(&serde_json::to_string(&v).unwrap()).unwrap(),
                v
            );
        }
    }

    #[test]
    fn an_input_with_no_declared_relation_does_not_deserialize() {
        // The whole reason the relation moved off the invocation and into the input. As
        // `ActInvocation.bounds_mode: Option<BoundsMode>` the invariant "required whenever `input`
        // is present" was held by PROSE, and the meaningless state it admitted was not inert: the
        // validator read `matches!(bounds_mode, Some(Seed))`, so an absent relation silently
        // classified as a BOUND and was checked against the wrong acceptance list.
        //
        // Now the state cannot be constructed and cannot arrive over the wire. This asserts the
        // wire half; the type half is enforced by the compiler at every construction site.
        assert!(
            serde_json::from_str::<StageInput>(r#"{"from":"upstream","stage":"hits"}"#).is_err(),
            "an upstream input must declare what the receiving act does with the set"
        );
        assert!(serde_json::from_str::<StageInput>(
            r#"{"from":"caller","ids":{"kind":"resource","ids":[]}}"#
        )
        .is_err());
    }

    #[test]
    fn the_relation_is_readable_without_asking_where_the_set_came_from() {
        // Two independent questions — what the act does with the set, and who produced it. A
        // consumer that needs only the first should not have to match on the second, because that
        // is how the two get accidentally coupled.
        let up = StageInput::Upstream {
            relation: StageRelation::Seed,
            stage: StageName::parse("hits").unwrap(),
        };
        assert_eq!(up.relation(), StageRelation::Seed);
        assert_eq!(
            StageInput::Caller {
                relation: StageRelation::Bound,
                ids: IdSet {
                    kind: IdKind::Resource,
                    provenance: None,
                    ids: vec![],
                },
            }
            .relation(),
            StageRelation::Bound
        );
    }

    #[test]
    fn the_relation_rides_the_wire_as_the_edge_word_callers_write() {
        // `as` in JSON, `relation` in Rust — `as` is a Rust keyword, and `r#as` at every match site
        // would be a worse read than one rename attribute. The trace echoes the same word back
        // under `relation`, so a caller sees `"as": "seed"` going out and `"relation": "seed"`
        // coming back; both name the same edge.
        let up = StageInput::Upstream {
            relation: StageRelation::Seed,
            stage: StageName::parse("hits").unwrap(),
        };
        let json = serde_json::to_string(&up).unwrap();
        assert!(json.contains(r#""as":"seed""#), "got: {json}");
        assert!(!json.contains("relation"));
    }

    #[test]
    fn a_stage_name_round_trips_through_the_wire_as_a_bare_string() {
        // Transparent on the wire so a plan reads as JSON a human wrote, not as a tagged wrapper.
        let n = StageName::parse("near").unwrap();
        assert_eq!(serde_json::to_string(&n).unwrap(), "\"near\"");
        assert_eq!(serde_json::from_str::<StageName>("\"near\"").unwrap(), n);
        assert!(
            serde_json::from_str::<StageName>("\"Near\"").is_err(),
            "validation applies on deserialize"
        );
    }

    #[test]
    fn a_stage_output_is_tagged_and_the_tag_carries_the_quantity_too() {
        // Tagging from day one is what made this reshape additive rather than breaking: the union
        // grew from one member to four without a client having to learn a new envelope.
        //
        // The tag names the QUANTITY, not just the currency, and that is the load-bearing half.
        // `resource_hits` would leave a caller unable to tell whether to read `fts_norm` or
        // `vec_norm`; `vec_hits` tells them outright, which is what lets `/api/query/validate`
        // answer completely before anything runs.
        let o = StageOutput::VecHits { hits: vec![] };
        let json = serde_json::to_value(&o).unwrap();
        assert_eq!(json["produced"], "vec_hits");
        assert_eq!(o.kind(), IdKind::Resource);
        assert!(o.is_empty());
    }

    #[test]
    fn two_acts_over_the_same_currency_still_land_in_different_variants() {
        // `no-cross-act-ranking`, made structural one level further down. `find-exact` and
        // `follow-from` both produce RESOURCES, so a single `resource_hits` variant would put
        // `fts_norm` and `graph_score` rows in lists of the same type — and the only thing stopping
        // someone concatenating them would be a comment. These cannot be concatenated: they are
        // different variants holding different row types.
        let fts = StageOutput::FtsHits { hits: vec![] };
        let graph = StageOutput::GraphHits { hits: vec![] };
        assert_eq!(fts.kind(), graph.kind(), "same currency");
        assert_ne!(
            fts.produced_tag(),
            graph.produced_tag(),
            "and still not the same shape"
        );
    }

    #[test]
    fn there_is_no_returnable_bare_id_set() {
        // `[decided — 2026-08-09, Pete]` Ids stay the pipe's internal currency and are no longer
        // something a caller can ask to be handed back — subselection is `jq` today and a GraphQL
        // door later. Every variant hydrates, asserted over the whole union so a fifth variant
        // added later has to face this rule rather than slip past it.
        for o in [
            StageOutput::FtsHits { hits: vec![] },
            StageOutput::VecHits { hits: vec![] },
            StageOutput::GraphHits { hits: vec![] },
            StageOutput::RegionHits { hits: vec![] },
        ] {
            let json = serde_json::to_value(&o).unwrap();
            assert!(
                json.get("hits").is_some(),
                "{} must carry rows: {json}",
                o.produced_tag()
            );
            assert!(
                json.get("set").is_none(),
                "{} must not carry a bare id set: {json}",
                o.produced_tag()
            );
        }
    }
}
