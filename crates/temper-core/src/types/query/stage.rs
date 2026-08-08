//! Stage identity, inputs, and outputs.
//!
//! One responsibility: what a node is called, what flows *into* it, and what it hands *out*. The
//! gap this whole phase exists to close lives here — [`StageInput`] lets an invocation finally
//! *declare* the upstream reference that [`super::trace::InputSource::Upstream`] could already
//! only *report*.

use serde::{Deserialize, Serialize};

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

/// What a stage produced. A tagged union with exactly ONE member today.
///
/// Tagged from the first line so that admitting a second currency later is additive rather than
/// breaking. It is NOT a claim that a second currency is coming — spec §10 refuses one for v0 and
/// states the reason (a derived intention cannot be embedded inside a single statement). The other
/// motivation is `substantiate`: an act that *annotates* rather than *selects* has no `IdSet` to
/// return, and the old required-field shape left `claims-carry-standing` nowhere to land.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", tag = "produced")]
pub enum StageOutput {
    Ids { set: IdSet },
}

impl StageOutput {
    /// The kind this stage produced. Contract chaining compares kinds, so wrapping the set must not
    /// cost that comparison.
    pub fn kind(&self) -> IdKind {
        match self {
            StageOutput::Ids { set } => set.kind.clone(),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            StageOutput::Ids { set } => set.ids.len(),
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
    fn a_stage_output_is_tagged_so_a_second_currency_would_be_additive() {
        // The one-variant union is the whole point: an untagged IdSet could not grow without a
        // breaking change, and `substantiate` — which annotates rather than selects — has no shape
        // to return at all under the old field type.
        let o = StageOutput::Ids {
            set: IdSet {
                kind: IdKind::Region,
                provenance: None,
                ids: vec![],
            },
        };
        let json = serde_json::to_string(&o).unwrap();
        assert!(
            json.contains("\"produced\""),
            "the discriminator is present from day one"
        );
        assert_eq!(serde_json::from_str::<StageOutput>(&json).unwrap(), o);
        assert_eq!(o.kind(), IdKind::Region);
        assert!(o.is_empty());
    }
}
