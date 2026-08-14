//! Stage identity, inputs, and outputs.
//!
//! One responsibility: what a node is called, what flows *into* it, and what it hands *out*. The
//! gap this whole phase exists to close lives here — [`StageInput`] lets an invocation finally
//! *declare* the upstream reference that [`super::trace::InputSource::Upstream`] could already
//! only *report*.

use serde::{Deserialize, Serialize};

use super::hits::{RegionHit, ResourceHit};
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
// The published contract must carry the pattern, and the derive does not infer it from
// [`StageName::parse`]. Without this the generated spec says "string" — so a client generated from
// it can build a name the server is guaranteed to reject, and learns that only at the door. The
// constraint is not cosmetic: a leading lowercase letter is what makes a caller-chosen stage unable
// to shadow a compiler-internal relation, and the compiler relies on it for SQL identifier safety.
// Keep this in step with `parse`; the two state one rule in two languages.
#[cfg_attr(feature = "web-api", schema(pattern = "^[a-z][a-z0-9_]{0,62}$"))]
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

/// What a RETURNED stage produced, tagged by CURRENCY.
///
/// # Two variants, and the tag answers exactly one question
///
/// `[decided — 2026-08-09, Pete]` What was produced is resources, or regions. How they were scored
/// is a different question, answered per row by [`super::hits::Scoring::score_kind`].
///
/// An earlier draft split this four ways — `fts_hits`, `vec_hits`, `graph_hits` — so that two acts'
/// rows could not share a list type. That guard was real but the name was a misnomer: `vec_hits`
/// describes the scoring, not what was produced, and it meant **a row could not be read without
/// knowing which envelope carried it**. The quantity was the field name, so the envelope was load-
/// bearing for interpreting the row. Now every hit is self-describing and there is one shape per
/// currency instead of one per act.
///
/// What still prevents `unified_search`'s conflation is structural and unchanged: arms are keyed
/// separately in the response, and there is no merged ordered list for two acts' rows to fall into.
/// See the module note on [`super::hits`] for the full argument.
///
/// # There is no `ids` variant
///
/// A returned stage is always hydrated. Ids remain the pipe's **internal** currency — the only
/// value crossing a stage boundary is membership, and an intermediate stage that merely feeds a
/// downstream one is never hydrated at all. What is gone is `ids` as a thing a caller can ASK to be
/// given back: subselecting a result set is a client concern with a good answer already
/// (`temper … | jq`), and the principled version is a known later door (GraphQL over this surface).
/// Shipping one coarse "just ids" toggle now would occupy the space that door is for.
///
/// Neither `PartialEq` nor `Eq`: [`crate::types::resource_view::ResourceView`] derives neither, and
/// scores are floats. Tests compare the serialized form, which is what a client observes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", tag = "produced")]
pub enum StageOutput {
    /// Produced by `find-exact`, both `find-about-*` acts, and `follow-from`.
    Resources { hits: Vec<ResourceHit> },
    /// Produced by `survey`.
    Regions { hits: Vec<RegionHit> },
}

/// Which [`StageOutput`] variant a stage carries — the tag, as a value.
///
/// It exists so `/api/query/validate` can PROMISE a variant using the same type the response
/// REPORTS, rather than a parallel enum that would drift. [`StageOutput::variant`] answers it from
/// an actual output; [`super::act::ActDeclaration::produced_variant`] predicts it from a
/// declaration, and a test asserts the two agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ProducedVariant {
    Resources,
    Regions,
}

impl StageOutput {
    /// Which variant this is — the same value `/api/query/validate` promised in advance.
    pub fn variant(&self) -> ProducedVariant {
        match self {
            StageOutput::Resources { .. } => ProducedVariant::Resources,
            StageOutput::Regions { .. } => ProducedVariant::Regions,
        }
    }

    /// The kind of thing this stage produced. Contract chaining compares kinds, so wrapping the
    /// rows must not cost that comparison.
    pub fn kind(&self) -> IdKind {
        match self {
            StageOutput::Resources { .. } => IdKind::Resource,
            StageOutput::Regions { .. } => IdKind::Region,
        }
    }

    /// Every score kind present in this output.
    ///
    /// A stage runs ONE act and an act produces ONE quantity, so this holds exactly one kind for
    /// any output the server built — which is what
    /// `a_stage_is_homogeneous_in_its_score_kind` asserts. It returns a set rather than an
    /// `Option` because the type no longer makes homogeneity impossible to violate, and a
    /// silent wrong answer would be worse than a visible plural.
    pub fn score_kinds(&self) -> std::collections::BTreeSet<&str> {
        match self {
            StageOutput::Resources { hits } => {
                hits.iter().map(|h| h.scoring.score_kind.as_str()).collect()
            }
            StageOutput::Regions { hits } => {
                hits.iter().map(|h| h.scoring.score_kind.as_str()).collect()
            }
        }
    }

    pub fn len(&self) -> usize {
        match self {
            StageOutput::Resources { hits } => hits.len(),
            StageOutput::Regions { hits } => hits.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::query::hits::{MatchLocation, ScoreKind, Scoring};
    use crate::types::query::id_set::{IdKind, IdSet};

    /// A hit whose only interesting property is its scoring.
    ///
    /// The `ResourceView` is built through the typed struct rather than from a JSON literal: it
    /// carries twenty fields and a literal missing one fails at RUNTIME with a serde error, which
    /// is a worse signal than a compile error and takes a test run to find. Learned the slow way.
    fn resource_hit(score_kind: ScoreKind, score: f32) -> ResourceHit {
        ResourceHit {
            resource: inert_resource(),
            scoring: Scoring { score_kind, score },
            located_at: None,
            via: vec![],
        }
    }

    fn inert_resource() -> crate::types::resource_view::ResourceView {
        use crate::types::ids::{ProfileId, ResourceId};
        crate::types::resource_view::ResourceView {
            id: ResourceId::new(),
            r#ref: "a-resource-019fe3d0".to_string(),
            title: "a resource".to_string(),
            origin_uri: String::new(),
            kb_context_id: None,
            context_name: None,
            context_slug: None,
            context_owner_ref: None,
            context_ref: None,
            cogmap_id: None,
            cogmap_name: None,
            doc_type_name: "session".to_string(),
            owner_handle: "j-cole-taylor".to_string(),
            owner_profile_id: ProfileId::new(),
            originator_profile_id: ProfileId::new(),
            is_active: true,
            created: chrono::Utc::now(),
            updated: chrono::Utc::now(),
            body_hash: None,
            ingest_state: None,
            body_storage: None,
            managed_meta: Default::default(),
            open_meta: None,
            content: None,
        }
    }

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
    fn the_envelope_tag_names_the_currency_and_nothing_about_scoring() {
        // `[decided — 2026-08-09, Pete]` What was PRODUCED is resources, or regions. How they were
        // scored is a different question, and putting it in this tag made the tag a misnomer —
        // `vec_hits` describes the scoring, not the thing.
        let o = StageOutput::Resources { hits: vec![] };
        let json = serde_json::to_value(&o).unwrap();
        assert_eq!(json["produced"], "resources");
        assert_eq!(o.variant(), ProducedVariant::Resources);
        assert_eq!(o.kind(), IdKind::Resource);
        assert!(o.is_empty());

        let r = StageOutput::Regions { hits: vec![] };
        assert_eq!(serde_json::to_value(&r).unwrap()["produced"], "regions");
        assert_eq!(r.kind(), IdKind::Region);
    }

    #[test]
    fn a_hit_says_what_kind_of_score_it_carries_without_reference_to_its_envelope() {
        // The property the four-variant draft gave up: a row had to be interpreted through the
        // envelope, because the quantity WAS the field name. Now the kind travels with the number,
        // so a row handed to a function on its own is still fully understood.
        let hit = resource_hit(ScoreKind::VecNorm, 0.86);
        let json = serde_json::to_value(&hit).unwrap();
        assert_eq!(json["scoring"]["score_kind"], "vec_norm");
        // Compared with a tolerance, not for equality: these scores are `f32` (the column type),
        // so 0.86 lands on the wire as 0.8600000143051147. That is pre-existing for `fts_norm` and
        // `vec_norm` on `/api/search` and is not something this shape introduced — noted here so
        // the next reader does not go looking for a rounding bug.
        let score = json["scoring"]["score"].as_f64().expect("a number");
        assert!((score - 0.86).abs() < 1e-6, "got: {score}");
        // And the row alone is enough — nothing here needs the envelope to be read.
        let alone: serde_json::Value = serde_json::from_str(&json.to_string()).unwrap();
        assert_eq!(alone["scoring"]["score_kind"], "vec_norm");
    }

    #[test]
    fn two_acts_over_the_same_currency_are_still_told_apart_by_their_score_kind() {
        // `no-cross-act-ranking`, now expressed as DATA rather than as two struct types.
        // `find-exact` and `follow-from` both produce RESOURCES and now share a row type — so what
        // stops their numbers being summed is that the kinds visibly differ, and a client can
        // CHECK that, which it could never do against a bare field name.
        let exact = StageOutput::Resources {
            hits: vec![resource_hit(ScoreKind::FtsNorm, 0.7)],
        };
        let walked = StageOutput::Resources {
            hits: vec![resource_hit(ScoreKind::GraphScore, 0.72)],
        };
        assert_eq!(exact.variant(), walked.variant(), "same currency");
        assert_ne!(
            exact.score_kinds(),
            walked.score_kinds(),
            "and still not the same quantity"
        );
    }

    #[test]
    fn a_stage_is_homogeneous_in_its_score_kind() {
        // A stage runs ONE act and an act produces ONE quantity, so every row of a real output
        // shares a kind. The type no longer makes a mixed list impossible — that is the trade this
        // shape accepts — so the invariant is asserted rather than structural, and `score_kinds`
        // returns a SET so a violation is visible instead of silently taking the first.
        let honest = StageOutput::Resources {
            hits: vec![
                resource_hit(ScoreKind::VecNorm, 0.9),
                resource_hit(ScoreKind::VecNorm, 0.8),
            ],
        };
        assert_eq!(honest.score_kinds().len(), 1);

        let mixed = StageOutput::Resources {
            hits: vec![
                resource_hit(ScoreKind::VecNorm, 0.9),
                resource_hit(ScoreKind::FtsNorm, 0.8),
            ],
        };
        assert_eq!(
            mixed.score_kinds().len(),
            2,
            "a mixed stage is constructible and must be VISIBLE, not silently read as its first row"
        );
    }

    #[test]
    fn an_unknown_score_kind_degrades_rather_than_failing_to_parse() {
        // OPEN, like `ActName`: a new act brings a new quantity, and a closed vocabulary would make
        // adding one breaking for every client. Safe because the operation on this value is
        // EQUALITY — are these two the same kind? — never exhaustive enumeration.
        let k: ScoreKind = serde_json::from_str("\"bm25_norm\"").expect("unknown kind parses");
        assert_eq!(k, ScoreKind::Other("bm25_norm".to_string()));
        assert!(!k.is_known());
        assert!(ScoreKind::VecNorm.is_known());
        // And it still compares unequal to a known kind, which is the whole job.
        assert_ne!(k, ScoreKind::VecNorm);
    }

    #[test]
    fn a_region_hit_has_no_match_location_field_at_all() {
        // Not "null for regions" — ABSENT. `located_at` sits on `ResourceHit` rather than on the
        // shared `Scoring` precisely so this shape cannot carry it: a region is not somewhere
        // inside a resource, so a field there could never be filled, and an always-null field says
        // "no match location" where the truth is "this can never tell you". That is the same
        // refusal that denied the old `FtsHit` an always-null one.
        //
        // On resource hits it IS absent-but-fillable, which is the ordinary case `discloses`
        // declares in advance — the distinction being kept is between an act that does not fill a
        // field and a shape that cannot.
        let region = serde_json::to_value(RegionHit {
            region: crate::types::cognitive_maps::CogmapRegionRow {
                region_id: crate::types::ids::RegionId::new(),
                lens_id: crate::types::ids::LensId::new(),
                salience: 0.61,
                content_cohesion: None,
                label: None,
                member_count: 4,
            },
            scoring: Scoring {
                score_kind: ScoreKind::RegionScore,
                score: 0.42,
            },
        })
        .unwrap();
        assert!(region.get("scoring").is_some());
        assert!(
            region.get("located_at").is_none() && region["scoring"].get("located_at").is_none(),
            "a region cannot have a position inside a resource: {region}"
        );

        // A resource hit with no location carries an EXPLICIT null (F4, ruled): the contract's
        // prose is load-bearing on the null meaning "not declared", so the key must be present —
        // omission would make this shape indistinguishable from the region one above, which is
        // the distinction under test.
        let unlocated = serde_json::to_value(resource_hit(ScoreKind::VecNorm, 0.9)).unwrap();
        assert!(
            unlocated.get("located_at").is_some() && unlocated["located_at"].is_null(),
            "a resource hit's absent location is an explicit null, never a missing key: {unlocated}"
        );

        // And a resource hit can carry one.
        let mut hit = resource_hit(ScoreKind::VecNorm, 0.9);
        hit.located_at = Some(MatchLocation {
            block_id: crate::types::ids::BlockId::new(),
            header_path: Some("## Findings".to_string()),
            snippet: None,
        });
        let json = serde_json::to_value(&hit).unwrap();
        assert!(json["located_at"]["block_id"].is_string(), "got: {json}");
    }

    #[test]
    fn there_is_no_returnable_bare_id_set() {
        // `[decided — 2026-08-09, Pete]` Ids stay the pipe's internal currency and are no longer
        // something a caller can ask to be handed back — subselection is `jq` today and a GraphQL
        // door later. Asserted over the whole union so a third variant added later faces this rule.
        for o in [
            StageOutput::Resources { hits: vec![] },
            StageOutput::Regions { hits: vec![] },
        ] {
            let json = serde_json::to_value(&o).unwrap();
            assert!(json.get("hits").is_some(), "must carry rows: {json}");
            assert!(json.get("set").is_none(), "never a bare id set: {json}");
        }
    }
}
