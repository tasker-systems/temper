//! `validate_ask` — whether an ask lies inside its shape, decided before any SQL exists (spec §3.4,
//! §3.5 step 4), and the refusals that teach the shape's vocabulary when it does not.
//!
//! **Every refusal at once, one per reason.** A caller repairing an ask sees all of it in one round
//! trip, as `validate` promises for a composition. Each reason is reported once per ask, naming how
//! many predicates broke the rule and the first — the incumbent rule (`validate/shape.rs`, the
//! property-predicate checks), which keeps a body of bad predicates from becoming a refusal list
//! several times its size. A detail never echoes a caller's value or path: an index names it.
//!
//! **Order** (§3.5). The term vocabularies and each predicate's form need no shape. A predicate
//! whose form is refused is not walked, and an ask over the count caps walks nothing — refused once,
//! up front, so no check runs once per entry of an over-long list.
//!
//! The refusals carry `stage: None`. The composable arm attaches its stage; the batch read has none
//! (spec §8).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::reference::{is_integral, jsonb_eq};
use super::walk::{
    AskablePath, BoundKind, JsonType, PathFacts, PathResolution, Vocabulary, Walk, WalkBounds,
    MAX_SCHEMA_BYTES, MAX_WALK_DEPTH, MAX_WALK_PATHS,
};
use super::{
    ArtifactAsk, ArtifactPredicate, LoadedShape, ShapeLoad, ShapeSuccessor, ARTIFACT_INTENTS,
    ASKABLE_SHAPE_STATES, DEFAULT_ARTIFACT_INTENTS,
};
use crate::types::ids::ShapeId;
use crate::types::query::filter::{
    json_bytes, json_string_bytes, property_value_bytes, property_value_nodes, PropertyOp,
    MAX_FILTER_STRING_BYTES, MAX_PROPERTY_VALUE_BYTES, MAX_PROPERTY_VALUE_NODES,
};
use crate::types::query::validate::{MAX_PER_CANDIDATE_PREDICATES, MAX_PER_CANDIDATE_PROBES};
use crate::types::query::{PlanRefusal, RefusalReason};

/// The most predicates one ask may carry. Refused as `filter_not_applicable`, the incumbent
/// per-candidate caps' reason.
///
/// # Mirrors `MAX_PER_CANDIDATE_PREDICATES`, counted on its own population
///
/// Each predicate is one `get(content, path)` and one operator per candidate **artifact**, the same
/// `|candidates| × |predicates|` multiplier the incumbent bounds per candidate row. It is counted
/// per ask and never added to the resource filter's count, because its candidates are artifacts —
/// the rule the edge filter's count already follows (spec §3.4, *Caps*). Defined as the incumbent's
/// value so the two cannot drift apart unnoticed.
pub const MAX_ASK_PREDICATES: usize = MAX_PER_CANDIDATE_PREDICATES;

/// The most probes one ask may make per candidate artifact — one per `contains` value, one for
/// `has_key` and for `compare` ([`PropertyOp::probes`]). Mirrors `MAX_PER_CANDIDATE_PROBES` for the
/// same reason [`MAX_ASK_PREDICATES`] mirrors its sibling: a `contains` list is scanned in full for
/// every candidate that misses, so its length is the real second factor.
pub const MAX_ASK_PROBES: usize = MAX_PER_CANDIDATE_PROBES;

/// The most segments a path may have. Refused as `too_many_filter_values` (spec §3.1, the
/// `TooManyFilterValues` shape).
///
/// # 32
///
/// A form bound, decided without the shape, so it is twice the walk's own depth bound
/// ([`MAX_WALK_DEPTH`]): a deep path a legitimately recursive shape could declare meets the walk's
/// bound first, and is refused `path_outside_shape` with `teaches` naming that bound (spec §2.5).
/// This one bounds what a path costs to carry — at most 32 segments of
/// [`MAX_ARTIFACT_PATH_SEGMENT_BYTES`], 8 KiB.
pub const MAX_ARTIFACT_PATH_DEPTH: usize = 32;

/// The longest path segment, in JSON-escaped bytes. Refused as `filter_string_too_long`.
///
/// A segment is an object key, the counterpart of a property key, so it takes the property key's
/// cap — `MAX_FILTER_STRING_BYTES`, measured against the live keys.
pub const MAX_ARTIFACT_PATH_SEGMENT_BYTES: usize = MAX_FILTER_STRING_BYTES;

/// The most bytes of askable paths one refusal's `teaches` carries, as compact JSON. Paths past it
/// are elided, and [`TeachingElision`] says how many and where the whole vocabulary is read.
///
/// # 16 KiB
///
/// An ask raises at most one refusal per reason, so a refused ask carries a few teachings at most —
/// tens of kilobytes. 16 KiB holds dozens of paths with their declared types and enums, which is
/// the shape of a real vocabulary; a pathological one is cut, visibly.
pub const MAX_TEACHING_BYTES: usize = 16 * 1024;

/// What a refusal teaches, as data an agent reads without parsing English (spec §3.5).
///
/// **Open, by being a struct of optional fields** rather than a tagged enum: a later refusal that
/// teaches something new adds a field older clients skip, where a new variant would fail their
/// parse of the whole refusal list. Each refusal fills what is relevant to it:
///
/// | reason | teaches |
/// |---|---|
/// | `path_outside_shape` | `vocabulary` (the askable paths), `beyond_bound` when a walk bound stopped it |
/// | `operator_outside_shape` | `vocabulary` (each offending path's facts and feasible operators) |
/// | `value_outside_shape` | `vocabulary` (each offending path's declared types and values), `non_closure` |
/// | `unknown_artifact_intent`, `unknown_shape_state` | `terms` |
/// | `shape_superseded` | `successor` |
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct RefusalTeaching {
    /// The shape's askable paths — the same structure as a shape's `askable` — narrowed to what the
    /// refusal is about, and held to [`MAX_TEACHING_BYTES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vocabulary: Option<Vocabulary>,
    /// The walk bound an asked path lies beyond.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beyond_bound: Option<BoundKind>,
    /// A closed vocabulary of terms: the intents, or the askable shape states.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub terms: Vec<String>,
    /// The version now in force, for a folded shape that has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub successor: Option<ShapeSuccessor>,
    /// That stored content is not closed by its shape, and the route to values it does not
    /// declare (spec §3.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub non_closure: Option<String>,
    /// Paths cut from `vocabulary` to keep the teaching bounded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elided: Option<TeachingElision>,
}

/// How much of a teaching was cut, and where the whole of it is read: the shape's by-id read —
/// `temper data-artifact schema show`, `GET /api/shapes/{id}`, `get_data_artifact_shape`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct TeachingElision {
    pub omitted_paths: usize,
    pub shape: ShapeId,
}

/// An ask that lies inside its shape, in the form the SQL binds.
///
/// Parse-don't-validate: the fields are private and [`validate_ask`] is the only constructor. It is
/// a validated **output**, not the ask rewritten in place: an array path's `contains` values arrive
/// wrapped (§3.4, *Why the array row wraps*), and the defaults of §2.1 are applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedAsk {
    shape: ShapeId,
    predicates: Vec<ArtifactPredicate>,
    intent: Vec<&'static str>,
    include_folded: bool,
    shape_states: Vec<&'static str>,
}

impl ValidatedAsk {
    pub fn shape(&self) -> ShapeId {
        self.shape
    }

    /// The predicates as bound: every `contains` value at an array path is the one-element array
    /// holding it.
    pub fn predicates(&self) -> &[ArtifactPredicate] {
        &self.predicates
    }

    pub fn intent(&self) -> &[&'static str] {
        &self.intent
    }

    pub fn include_folded(&self) -> bool {
        self.include_folded
    }

    pub fn shape_states(&self) -> &[&'static str] {
        &self.shape_states
    }
}

/// Refusal step 4 of §3.5: path, operator and value, against the shape the loader admitted.
///
/// `shape` is the load of `ask.shape`; steps 1–3 are [`ShapeLoad::loaded`].
pub fn validate_ask(
    ask: &ArtifactAsk,
    shape: &LoadedShape,
) -> Result<ValidatedAsk, Vec<PlanRefusal>> {
    let mut errs = Vec::new();
    let intent = resolve_terms(&ask.intent, &INTENTS, &mut errs);
    let shape_states = resolve_terms(&ask.shape_states, &SHAPE_STATES, &mut errs);
    let predicates = check_predicates(&ask.predicates, shape, &mut errs);
    if !errs.is_empty() {
        return Err(errs);
    }
    Ok(ValidatedAsk {
        shape: ask.shape,
        predicates,
        intent,
        include_folded: ask.include_folded,
        shape_states,
    })
}

impl ShapeLoad {
    /// Refusal steps 1–3 of §3.5: the loaded shape, or the refusal its load decided.
    ///
    /// **Fixed sentences.** `shape_not_askable` reads the same for a shape that does not exist and
    /// one the asker cannot read, because they are one arm. `shape_schema_uncompilable` never
    /// echoes the compiler: its text is the author's material, not the asker's.
    pub fn loaded(&self) -> Result<&LoadedShape, PlanRefusal> {
        let (reason, detail, teaches) = match self {
            ShapeLoad::Loaded(shape) => return Ok(shape),
            ShapeLoad::NotAskable => (RefusalReason::ShapeNotAskable, NOT_ASKABLE, None),
            ShapeLoad::Superseded {
                successor: Some(successor),
            } => (
                RefusalReason::ShapeSuperseded,
                SUPERSEDED,
                Some(RefusalTeaching {
                    successor: Some(*successor),
                    ..RefusalTeaching::default()
                }),
            ),
            ShapeLoad::Superseded { successor: None } => (
                RefusalReason::ShapeSuperseded,
                SUPERSEDED_NO_SUCCESSOR,
                None,
            ),
            ShapeLoad::Uncompilable => (RefusalReason::ShapeSchemaUncompilable, UNCOMPILABLE, None),
        };
        Err(refusal(reason, detail.to_string(), teaches))
    }
}

const NOT_ASKABLE: &str =
    "the named shape does not exist or you cannot read it, so nothing can be asked of it";

const SUPERSEDED: &str = "the named shape has been amended. An ask is never re-pointed at a \
     version it was not written against: if its predicates still mean the same thing, ask the \
     version now in force (`teaches.successor`)";

const SUPERSEDED_NO_SUCCESSOR: &str = "the named shape has been amended, and no version of it is \
     now in force in its home, so nothing can be asked of it";

const UNCOMPILABLE: &str =
    "the shape's schema does not compile, so nothing can be asked of it until the shape is amended";

/// The second half of every `value_outside_shape` (spec §3.4, ruled 2026-10-19): what the refusal
/// does not claim, and the route to the values it does not declare.
const NON_CLOSURE: &str = "stored content is not closed by its shape: an advisory shape records \
     non-conforming content, artifacts committed before the shape was declared stay \
     `declared_not_yet_checked`, and an amendment can narrow an enum over older data. So this \
     refusal says the shape does not declare the value, not that nothing matches. To read the \
     values it does not declare, ask `has_key` at the path with `shape_states: \
     [\"declared_not_satisfied\", \"declared_not_yet_checked\"]` in the batch read, which returns \
     the artifacts whole";

fn refusal(reason: RefusalReason, detail: String, teaches: Option<RefusalTeaching>) -> PlanRefusal {
    PlanRefusal {
        stage: None,
        reason,
        detail,
        teaches: teaches.map(Box::new),
    }
}

/// One refusal for a rule some predicates break: how many, and the first by index.
fn refuse_at(
    errs: &mut Vec<PlanRefusal>,
    reason: RefusalReason,
    at: &[usize],
    rule: &str,
    teaches: Option<RefusalTeaching>,
) {
    let Some(first) = at.first() else {
        return;
    };
    errs.push(refusal(
        reason,
        format!(
            "{rule}; {} of this ask's predicates do not hold to it, the first `predicates[{first}]`",
            at.len()
        ),
        teaches,
    ));
}

// ── The term vocabularies ───────────────────────────────────────────────────────────────────────

/// A closed vocabulary carried as strings, so an unknown term is a refusal that teaches rather
/// than a parse error (§2.1).
struct Terms {
    field: &'static str,
    known: &'static [&'static str],
    defaults: &'static [&'static str],
    unknown: RefusalReason,
    /// Terms of the wider vocabulary that are never right here, and why.
    unreachable: &'static [(&'static str, &'static str)],
}

const INTENTS: Terms = Terms {
    field: "intent",
    known: &ARTIFACT_INTENTS,
    defaults: &DEFAULT_ARTIFACT_INTENTS,
    unknown: RefusalReason::UnknownArtifactIntent,
    unreachable: &[],
};

const SHAPE_STATES: Terms = Terms {
    field: "shape_states",
    known: &ASKABLE_SHAPE_STATES,
    defaults: &ASKABLE_SHAPE_STATES,
    unknown: RefusalReason::UnknownShapeState,
    unreachable: &[(
        "never_declared",
        "an artifact under a live shape is never `never_declared`, so the state can match nothing",
    )],
};

/// The terms asked, or the defaults when none are. An unknown term and a repeated one are refused,
/// once each; a repeat because it changes nothing and is the only way to make an unbounded list
/// out of a bounded vocabulary (`DuplicateSetMember`'s incumbent rule).
fn resolve_terms(
    given: &[String],
    terms: &Terms,
    errs: &mut Vec<PlanRefusal>,
) -> Vec<&'static str> {
    if given.is_empty() {
        return terms.defaults.to_vec();
    }
    let mut out: Vec<&'static str> = Vec::new();
    let mut unknown = Vec::new();
    let mut unreachable = None;
    let mut repeated = None;
    for (i, term) in given.iter().enumerate() {
        match terms.known.iter().find(|k| **k == term.as_str()) {
            Some(known) if out.contains(known) => {
                repeated.get_or_insert(*known);
            }
            Some(known) => out.push(*known),
            None => {
                unknown.push(i);
                if let Some(entry) = terms.unreachable.iter().find(|(t, _)| *t == term.as_str()) {
                    unreachable.get_or_insert(*entry);
                }
            }
        }
    }
    let field = terms.field;
    if let Some(first) = unknown.first() {
        let why = unreachable
            .map(|(term, why)| format!(" — `{term}`: {why}"))
            .unwrap_or_default();
        errs.push(refusal(
            terms.unknown.clone(),
            format!(
                "`{field}` names {} terms outside its vocabulary, the first `{field}[{first}]`{why}",
                unknown.len()
            ),
            Some(RefusalTeaching {
                terms: terms.known.iter().map(|t| t.to_string()).collect(),
                ..RefusalTeaching::default()
            }),
        ));
    }
    if let Some(term) = repeated {
        errs.push(refusal(
            RefusalReason::DuplicateSetMember,
            format!("`{field}` names `{term}` more than once; a set names each member once"),
            None,
        ));
    }
    out
}

// ── The predicates ──────────────────────────────────────────────────────────────────────────────

/// The predicates as bound, or none when a refusal was raised (the caller discards them then).
fn check_predicates(
    predicates: &[ArtifactPredicate],
    shape: &LoadedShape,
    errs: &mut Vec<PlanRefusal>,
) -> Vec<ArtifactPredicate> {
    if !within_caps(predicates, errs) {
        return Vec::new();
    }
    let formed = well_formed(predicates, errs);
    against_shape(predicates, &formed, shape, errs)
}

/// The two per-ask cost caps (spec §3.4, *Caps*), each refused by name and never clamped.
fn within_caps(predicates: &[ArtifactPredicate], errs: &mut Vec<PlanRefusal>) -> bool {
    let count = predicates.len();
    let probes: usize = predicates.iter().map(|p| p.op.probes()).sum();
    if count > MAX_ASK_PREDICATES {
        errs.push(refusal(
            RefusalReason::FilterNotApplicable,
            format!(
                "an artifact ask admits at most {MAX_ASK_PREDICATES} predicates; this one has \
                 {count}. Each is evaluated against every candidate artifact, so the list \
                 multiplies the read — split the question into several asks"
            ),
            None,
        ));
    }
    if probes > MAX_ASK_PROBES {
        errs.push(refusal(
            RefusalReason::FilterNotApplicable,
            format!(
                "an artifact ask admits at most {MAX_ASK_PROBES} probes per candidate artifact; \
                 this one makes {probes}. A `contains` list is scanned in full for every artifact \
                 that carries the path, so its length multiplies the read the way the predicate \
                 count does"
            ),
            None,
        ));
    }
    count <= MAX_ASK_PREDICATES && probes <= MAX_ASK_PROBES
}

/// The form of each predicate, which needs no shape: whether each may be walked.
fn well_formed(predicates: &[ArtifactPredicate], errs: &mut Vec<PlanRefusal>) -> Vec<bool> {
    let indices = |broken: &dyn Fn(&ArtifactPredicate) -> bool| -> Vec<usize> {
        (0..predicates.len())
            .filter(|&i| broken(&predicates[i]))
            .collect()
    };
    let empty = indices(&|p: &ArtifactPredicate| p.path.is_empty());
    let deep = indices(&|p: &ArtifactPredicate| p.path.len() > MAX_ARTIFACT_PATH_DEPTH);
    let long = indices(&|p: &ArtifactPredicate| {
        p.path
            .iter()
            .any(|s| json_string_bytes(s) > MAX_ARTIFACT_PATH_SEGMENT_BYTES)
    });
    let no_values = indices(
        &|p: &ArtifactPredicate| matches!(&p.op, PropertyOp::Contains { values } if values.is_empty()),
    );
    let large = indices(&|p: &ArtifactPredicate| {
        p.op.values().iter().any(|v| {
            property_value_bytes(v) > MAX_PROPERTY_VALUE_BYTES
                || property_value_nodes(v) > MAX_PROPERTY_VALUE_NODES
        })
    });

    refuse_at(
        errs,
        RefusalReason::EmptyArtifactPath,
        &empty,
        "an artifact path needs at least one segment; an empty one would address the whole \
         document",
        None,
    );
    refuse_at(
        errs,
        RefusalReason::TooManyFilterValues,
        &deep,
        &format!("an artifact path may have at most {MAX_ARTIFACT_PATH_DEPTH} segments"),
        None,
    );
    refuse_at(
        errs,
        RefusalReason::FilterStringTooLong,
        &long,
        &format!("each path segment may be at most {MAX_ARTIFACT_PATH_SEGMENT_BYTES} bytes"),
        None,
    );
    refuse_at(
        errs,
        RefusalReason::EmptyContains,
        &no_values,
        "`contains` with no values narrows nothing",
        None,
    );
    refuse_at(
        errs,
        RefusalReason::PropertyValueTooLarge,
        &large,
        &format!(
            "a predicate value may be at most {MAX_PROPERTY_VALUE_BYTES} bytes and \
             {MAX_PROPERTY_VALUE_NODES} nested elements"
        ),
        None,
    );

    let faulted = [empty, deep, long, no_values, large];
    (0..predicates.len())
        .map(|i| !faulted.iter().any(|f| f.contains(&i)))
        .collect()
}

/// Why a declared path refuses a predicate (§3.4, the "Refused as" column).
enum Fault {
    Operator,
    Value,
}

/// The well-formed predicates checked against the shape, one walk per path: bound, or refused.
fn against_shape(
    predicates: &[ArtifactPredicate],
    formed: &[bool],
    shape: &LoadedShape,
    errs: &mut Vec<PlanRefusal>,
) -> Vec<ArtifactPredicate> {
    let walk = Walk::new(&shape.schema, &WalkBounds::BUILD);
    let sorted = sort(predicates, formed, &walk);
    refuse_paths(&sorted, &walk, shape.shape_id, errs);

    let (at, teaches) = offending(sorted.outside_operator, shape.shape_id);
    refuse_at(
        errs,
        RefusalReason::OperatorOutsideShape,
        &at,
        "an operator is not feasible at its path: `compare` needs a path declaring `number`, \
         `integer` or `string`, or no type, and a bound of a type declared there",
        Some(teaches),
    );
    let (at, mut teaches) = offending(sorted.outside_value, shape.shape_id);
    teaches.non_closure = Some(NON_CLOSURE.to_string());
    refuse_at(
        errs,
        RefusalReason::ValueOutsideShape,
        &at,
        "a value lies outside what the shape declares at its path — not of a declared type, not a \
         member of its enum, or not an admissible element of its array",
        Some(teaches),
    );
    sorted.bound
}

/// Each walked predicate, sorted by what the shape says of it.
#[derive(Default)]
struct Sorted {
    bound: Vec<ArtifactPredicate>,
    outside_path: Vec<usize>,
    /// The first walk bound an outside path met.
    beyond: Option<BoundKind>,
    outside_operator: Vec<(usize, AskablePath)>,
    outside_value: Vec<(usize, AskablePath)>,
}

fn sort(
    predicates: &[ArtifactPredicate],
    formed: &[bool],
    walk: &Result<Walk<'_>, BoundKind>,
) -> Sorted {
    let mut sorted = Sorted::default();
    for (i, predicate) in predicates.iter().enumerate() {
        if !formed[i] {
            continue;
        }
        let resolution = match walk {
            Ok(walk) => walk.resolve(&predicate.path),
            Err(at) => PathResolution::BeyondBound(*at),
        };
        let facts = match resolution {
            PathResolution::Declared(facts) => facts,
            PathResolution::NotDeclared => {
                sorted.outside_path.push(i);
                continue;
            }
            PathResolution::BeyondBound(at) => {
                sorted.outside_path.push(i);
                sorted.beyond.get_or_insert(at);
                continue;
            }
        };
        let described = || AskablePath::new(predicate.path.clone(), facts.clone());
        match bind(&predicate.op, &facts) {
            Ok(op) => sorted.bound.push(ArtifactPredicate {
                path: predicate.path.clone(),
                op,
            }),
            Err(Fault::Operator) => sorted.outside_operator.push((i, described())),
            Err(Fault::Value) => sorted.outside_value.push((i, described())),
        }
    }
    sorted
}

/// `path_outside_shape`, teaching the askable vocabulary and the bound an asked path met.
fn refuse_paths(
    sorted: &Sorted,
    walk: &Result<Walk<'_>, BoundKind>,
    shape: ShapeId,
    errs: &mut Vec<PlanRefusal>,
) {
    if sorted.outside_path.is_empty() {
        return;
    }
    let vocabulary = match walk {
        Ok(walk) => walk.enumerate(),
        Err(at) => Vocabulary {
            paths: Vec::new(),
            truncated: Some(*at),
        },
    };
    let mut rule = "a path is not one the shape declares — only `properties` chains, local \
                    `$ref`s, `allOf`, `anyOf` and `oneOf` are walked"
        .to_string();
    if let Some(at) = sorted.beyond {
        rule.push_str(&format!(", and {}", bound_said(at)));
    }
    let mut teaches = teaching(vocabulary, shape);
    teaches.beyond_bound = sorted.beyond;
    refuse_at(
        errs,
        RefusalReason::PathOutsideShape,
        &sorted.outside_path,
        &rule,
        Some(teaches),
    );
}

/// The offending predicates' indices, and a teaching that describes each offending path once.
fn offending(found: Vec<(usize, AskablePath)>, shape: ShapeId) -> (Vec<usize>, RefusalTeaching) {
    let at = found.iter().map(|(i, _)| *i).collect();
    let mut paths: Vec<AskablePath> = Vec::new();
    for (_, path) in found {
        if !paths.iter().any(|p| p.path == path.path) {
            paths.push(path);
        }
    }
    let teaches = teaching(
        Vocabulary {
            paths,
            truncated: None,
        },
        shape,
    );
    (at, teaches)
}

fn bound_said(at: BoundKind) -> String {
    match at {
        BoundKind::Depth => format!("the walk descends at most {MAX_WALK_DEPTH} segments"),
        BoundKind::Paths => format!("the vocabulary lists at most {MAX_WALK_PATHS} paths"),
        BoundKind::SchemaBytes => {
            format!("the walk reads a schema of at most {MAX_SCHEMA_BYTES} bytes")
        }
    }
}

/// A vocabulary held to [`MAX_TEACHING_BYTES`]: its paths in order until the next would not fit,
/// and an elision saying how many were cut.
fn teaching(vocabulary: Vocabulary, shape: ShapeId) -> RefusalTeaching {
    let Vocabulary { paths, truncated } = vocabulary;
    let total = paths.len();
    let mut kept = Vec::new();
    let mut used = 0;
    for path in paths {
        let bytes = json_bytes(&path);
        if used + bytes > MAX_TEACHING_BYTES {
            break;
        }
        used += bytes;
        kept.push(path);
    }
    let omitted_paths = total - kept.len();
    RefusalTeaching {
        vocabulary: Some(Vocabulary {
            paths: kept,
            truncated,
        }),
        elided: (omitted_paths > 0).then_some(TeachingElision {
            omitted_paths,
            shape,
        }),
        ..RefusalTeaching::default()
    }
}

// ── §3.4, the feasibility table ─────────────────────────────────────────────────────────────────

/// The operator as the SQL binds it, or why the declared path refuses it.
fn bind(op: &PropertyOp, facts: &PathFacts) -> Result<PropertyOp, Fault> {
    match op {
        PropertyOp::HasKey => Ok(PropertyOp::HasKey),
        PropertyOp::Contains { .. } if !facts.admits_contains() => Err(Fault::Operator),
        PropertyOp::Contains { values } => values
            .iter()
            .map(|v| bind_contains(facts, v).ok_or(Fault::Value))
            .collect::<Result<Vec<_>, _>>()
            .map(|values| PropertyOp::Contains { values }),
        PropertyOp::Compare { value, .. } if compare_admits(facts, value) => Ok(op.clone()),
        PropertyOp::Compare { .. } => Err(Fault::Operator),
    }
}

/// The `contains` rows. A value whose own type the path declares is bound as it is — the scalar
/// row (type, then enum membership) or the object row (an object; containment is partial, so an
/// object is never an enum-membership question). Otherwise, at a path declaring `array`, the value
/// is an **element**: admissible under `items` and bound wrapped as `[v]`. An untyped path binds
/// every value unwrapped, with the incumbent `PropertyOp` semantics.
fn bind_contains(facts: &PathFacts, value: &Value) -> Option<Value> {
    let Some(types) = &facts.types else {
        return scalar_member(facts.values.as_deref(), value).then(|| value.clone());
    };
    if !value.is_array() && admits_type(types, value) {
        return scalar_member(facts.values.as_deref(), value).then(|| value.clone());
    }
    if types.contains(&JsonType::Array) && element_admitted(facts.items.as_deref(), value) {
        return Some(Value::Array(vec![value.clone()]));
    }
    None
}

/// Whether a value's JSON type is among the declared ones. A number is an `integer` only when it is
/// integral, since equality with an integer can hold for nothing else.
fn admits_type(types: &BTreeSet<JsonType>, value: &Value) -> bool {
    match value {
        Value::Number(n) => {
            types.contains(&JsonType::Number)
                || (types.contains(&JsonType::Integer) && is_integral(n))
        }
        other => types.contains(&JsonType::of(other)),
    }
}

/// Whether a value may be an element of the array: its type admissible under `items` and, under an
/// `items` enum, a member.
fn element_admitted(items: Option<&PathFacts>, value: &Value) -> bool {
    let Some(items) = items else {
        return true;
    };
    items.types.as_ref().is_none_or(|t| admits_type(t, value))
        && scalar_member(items.values.as_deref(), value)
}

/// Enum membership, by jsonb equality, for a scalar. A container's containment is partial, so it is
/// not a membership question and is admitted here.
fn scalar_member(values: Option<&[Value]>, value: &Value) -> bool {
    if value.is_array() || value.is_object() {
        return true;
    }
    values.is_none_or(|members| members.iter().any(|m| jsonb_eq(m, value)))
}

/// The `compare` row: a number where `number` or `integer` is declared (a non-integral bound at an
/// `integer` path is admitted — ordering, unlike equality, is meaningful against it), a string
/// where `string` is; any bound at an untyped path, whose runtime type guard makes a mismatch an
/// honest empty. A path declaring none of the three refuses every bound, which is
/// [`PathFacts::admits_compare`] read with the bound in hand.
fn compare_admits(facts: &PathFacts, bound: &Value) -> bool {
    let Some(types) = &facts.types else {
        return true;
    };
    match bound {
        Value::Number(_) => types.contains(&JsonType::Number) || types.contains(&JsonType::Integer),
        Value::String(_) => types.contains(&JsonType::String),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::artifact_ask::AskOperator;
    use crate::types::query::filter::OrdOp;
    use serde_json::json;
    use uuid::Uuid;

    fn shape_of(schema: Value) -> LoadedShape {
        LoadedShape {
            shape_id: ShapeId::from(Uuid::from_u128(7)),
            shape_version: 1,
            home_anchor_table: "kb_contexts".to_string(),
            home_anchor_id: Uuid::from_u128(1),
            kind_owner_table: "kb_teams".to_string(),
            kind_owner_id: Uuid::from_u128(2),
            artifact_kind: "measurement".to_string(),
            schema,
        }
    }

    fn ask(predicates: Vec<ArtifactPredicate>) -> ArtifactAsk {
        ArtifactAsk {
            shape: ShapeId::from(Uuid::from_u128(7)),
            predicates,
            intent: vec![],
            include_folded: false,
            shape_states: vec![],
        }
    }

    fn at(path: &[&str], op: PropertyOp) -> ArtifactPredicate {
        ArtifactPredicate {
            path: path.iter().map(|s| s.to_string()).collect(),
            op,
        }
    }

    fn contains(values: Vec<Value>) -> PropertyOp {
        PropertyOp::Contains { values }
    }

    fn compare(value: Value) -> PropertyOp {
        PropertyOp::Compare {
            direction: OrdOp::Gte,
            value,
        }
    }

    fn refusals(a: &ArtifactAsk, shape: &LoadedShape) -> Vec<PlanRefusal> {
        validate_ask(a, shape).expect_err("expected a refusal")
    }

    fn reasons(rs: &[PlanRefusal]) -> Vec<RefusalReason> {
        rs.iter().map(|r| r.reason.clone()).collect()
    }

    // ── The D3 expectation table: operator × declared type × enum × value type ──────────────
    //
    // Every verdict below is written by hand from spec §3.4, never computed by the code under
    // test. The probe at each row's path is one of seven values, in this order:
    //
    //     "a"   1   1.5   true   null   {"k":1}   ["a"]
    //
    // A = admitted as written; W = admitted, wrapped as `[v]`; V = value_outside_shape;
    // O = operator_outside_shape; P = path_outside_shape.

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Verdict {
        A,
        W,
        V,
        O,
        P,
    }
    use Verdict::{A, O, P, V, W};

    fn probes() -> [Value; 7] {
        [
            json!("a"),
            json!(1),
            json!(1.5),
            json!(true),
            json!(null),
            json!({ "k": 1 }),
            json!(["a"]),
        ]
    }

    struct Row {
        declared: Value,
        has_key: Verdict,
        contains: [Verdict; 7],
        compare: [Verdict; 7],
    }

    #[rustfmt::skip]
    fn table() -> Vec<Row> {
        vec![
            // string, no enum / enum
            Row { declared: json!({ "type": "string" }), has_key: A,
                  contains: [A, V, V, V, V, V, V], compare: [A, O, O, O, O, O, O] },
            Row { declared: json!({ "type": "string", "enum": ["z"] }), has_key: A,
                  contains: [V, V, V, V, V, V, V], compare: [A, O, O, O, O, O, O] },
            // number
            Row { declared: json!({ "type": "number" }), has_key: A,
                  contains: [V, A, A, V, V, V, V], compare: [O, A, A, O, O, O, O] },
            Row { declared: json!({ "type": "number", "enum": [7] }), has_key: A,
                  contains: [V, V, V, V, V, V, V], compare: [O, A, A, O, O, O, O] },
            // integer — a non-integral value is outside for `contains`, admitted as a bound
            Row { declared: json!({ "type": "integer" }), has_key: A,
                  contains: [V, A, V, V, V, V, V], compare: [O, A, A, O, O, O, O] },
            Row { declared: json!({ "type": "integer", "enum": [7] }), has_key: A,
                  contains: [V, V, V, V, V, V, V], compare: [O, A, A, O, O, O, O] },
            // boolean — not orderable
            Row { declared: json!({ "type": "boolean" }), has_key: A,
                  contains: [V, V, V, A, V, V, V], compare: [O, O, O, O, O, O, O] },
            Row { declared: json!({ "type": "boolean", "enum": [false] }), has_key: A,
                  contains: [V, V, V, V, V, V, V], compare: [O, O, O, O, O, O, O] },
            // null — its enum can only hold null, so null is a member
            Row { declared: json!({ "type": "null" }), has_key: A,
                  contains: [V, V, V, V, A, V, V], compare: [O, O, O, O, O, O, O] },
            Row { declared: json!({ "type": "null", "enum": [null] }), has_key: A,
                  contains: [V, V, V, V, A, V, V], compare: [O, O, O, O, O, O, O] },
            // object — each value must be an object; containment is partial, not membership
            Row { declared: json!({ "type": "object" }), has_key: A,
                  contains: [V, V, V, V, V, A, V], compare: [O, O, O, O, O, O, O] },
            Row { declared: json!({ "type": "object", "enum": [{ "z": 1 }] }), has_key: A,
                  contains: [V, V, V, V, V, A, V], compare: [O, O, O, O, O, O, O] },
            // array of strings — each value is an ELEMENT, wrapped; `["a"]` is not a string element
            Row { declared: json!({ "type": "array", "items": { "type": "string" } }), has_key: A,
                  contains: [W, V, V, V, V, V, V], compare: [O, O, O, O, O, O, O] },
            Row { declared: json!({ "type": "array", "items": { "type": "string", "enum": ["z"] } }),
                  has_key: A,
                  contains: [V, V, V, V, V, V, V], compare: [O, O, O, O, O, O, O] },
            // no type — every operator; an enum constrains scalars only
            Row { declared: json!({}), has_key: A,
                  contains: [A, A, A, A, A, A, A], compare: [A, A, A, A, A, A, A] },
            Row { declared: json!({ "enum": ["z", 7] }), has_key: A,
                  contains: [V, V, V, V, V, A, A], compare: [A, A, A, A, A, A, A] },
        ]
    }

    /// Paths the shape does not declare, under every operator.
    fn undeclared() -> Vec<Value> {
        vec![
            json!({ "type": "object", "properties": { "q": { "type": "string" } } }),
            json!({ "type": "object", "additionalProperties": { "type": "string" } }),
            json!({ "type": "object", "properties": { "p": false } }),
        ]
    }

    /// What `validate_ask` says about one predicate at `["p"]`.
    fn verdict(schema: &Value, op: PropertyOp) -> Verdict {
        let shape = shape_of(schema.clone());
        let original = op.clone();
        match validate_ask(&ask(vec![at(&["p"], op)]), &shape) {
            Ok(v) => match (&original, &v.predicates()[0].op) {
                (
                    PropertyOp::Contains { values: asked },
                    PropertyOp::Contains { values: bound },
                ) if bound != asked => {
                    assert_eq!(bound, &vec![Value::Array(asked.clone())]);
                    W
                }
                _ => A,
            },
            Err(rs) => match reasons(&rs).as_slice() {
                [RefusalReason::ValueOutsideShape] => V,
                [RefusalReason::OperatorOutsideShape] => O,
                [RefusalReason::PathOutsideShape] => P,
                other => panic!("unexpected refusals {other:?}"),
            },
        }
    }

    fn wrapped(declared: &Value) -> Value {
        json!({ "type": "object", "properties": { "p": declared } })
    }

    #[test]
    fn every_operator_declared_type_enum_and_value_type_has_its_hand_written_verdict() {
        for row in table() {
            let schema = wrapped(&row.declared);
            assert_eq!(
                verdict(&schema, PropertyOp::HasKey),
                row.has_key,
                "has_key at {}",
                row.declared
            );
            for (i, probe) in probes().into_iter().enumerate() {
                assert_eq!(
                    verdict(&schema, contains(vec![probe.clone()])),
                    row.contains[i],
                    "contains {probe} at {}",
                    row.declared
                );
                assert_eq!(
                    verdict(&schema, compare(probe.clone())),
                    row.compare[i],
                    "compare {probe} at {}",
                    row.declared
                );
            }
        }
    }

    #[test]
    fn an_undeclared_path_is_refused_under_every_operator() {
        for schema in undeclared() {
            assert_eq!(verdict(&schema, PropertyOp::HasKey), P, "{schema}");
            assert_eq!(verdict(&schema, contains(vec![json!("a")])), P, "{schema}");
            assert_eq!(verdict(&schema, compare(json!(1))), P, "{schema}");
        }
    }

    #[test]
    fn a_validator_that_refuses_nothing_fails_the_expectation_table() {
        // The bite, held by the table's contents: a `validate_ask` returning no refusals answers
        // A or W for every row, so each of the three shape reasons must be expected somewhere.
        let rows = table();
        let expected: Vec<Verdict> = rows
            .iter()
            .flat_map(|r| {
                std::iter::once(r.has_key)
                    .chain(r.contains)
                    .chain(r.compare)
            })
            .collect();
        assert!(
            expected.contains(&V),
            "value_outside_shape is expected somewhere"
        );
        assert!(
            expected.contains(&O),
            "operator_outside_shape is expected somewhere"
        );
        assert!(
            !undeclared().is_empty(),
            "path_outside_shape is expected somewhere"
        );
    }

    #[test]
    fn an_enum_member_is_admitted_and_numbers_are_members_by_value() {
        let at_p = |declared: Value| wrapped(&declared);
        assert_eq!(
            verdict(
                &at_p(json!({ "type": "string", "enum": ["a"] })),
                contains(vec![json!("a")])
            ),
            A
        );
        assert_eq!(
            verdict(
                &at_p(json!({ "type": "number", "enum": [5] })),
                contains(vec![json!(5.0)])
            ),
            A
        );
        assert_eq!(
            verdict(
                &at_p(json!({ "type": "integer" })),
                contains(vec![json!(5.0)])
            ),
            A
        );
        assert_eq!(
            verdict(
                &at_p(json!({ "type": "array", "items": { "enum": ["a"] } })),
                contains(vec![json!("a")])
            ),
            W
        );
    }

    #[test]
    fn an_array_value_at_an_array_path_is_an_element_and_is_wrapped() {
        let schema = wrapped(&json!({ "type": "array", "items": { "type": "array" } }));
        assert_eq!(verdict(&schema, contains(vec![json!([1, 3])])), W);
    }

    // ── The rewrite is in the validated output, not in place ────────────────────────────────

    #[test]
    fn an_array_contains_binds_wrapped_values_and_leaves_the_ask_as_written() {
        let shape = shape_of(json!({ "properties": {
            "rows": { "type": "array", "items": { "type": "object" } }
        } }));
        let a = ask(vec![at(&["rows"], contains(vec![json!({ "a": 1 })]))]);
        let v = validate_ask(&a, &shape).expect("an object element is admissible");
        assert_eq!(
            v.predicates(),
            &[at(&["rows"], contains(vec![json!([{ "a": 1 }])]))]
        );
        assert_eq!(a.predicates[0].op, contains(vec![json!({ "a": 1 })]));
    }

    // ── The term vocabularies ───────────────────────────────────────────────────────────────

    #[test]
    fn the_defaults_are_current_and_member_and_every_askable_state() {
        let v = validate_ask(&ask(vec![]), &shape_of(json!({}))).unwrap();
        assert_eq!(v.intent(), &["current", "member"]);
        assert_eq!(
            v.shape_states(),
            &[
                "declared_satisfied",
                "declared_not_satisfied",
                "declared_not_yet_checked"
            ]
        );
        assert!(v.predicates().is_empty());
        assert!(!v.include_folded());
    }

    #[test]
    fn an_unknown_intent_is_refused_and_teaches_the_vocabulary() {
        let mut a = ask(vec![]);
        a.intent = vec!["current".to_string(), "favourite".to_string()];
        let rs = refusals(&a, &shape_of(json!({})));
        assert_eq!(reasons(&rs), vec![RefusalReason::UnknownArtifactIntent]);
        assert_eq!(
            rs[0].teaches.as_ref().map(|t| t.terms.clone()),
            Some(vec![
                "current".to_string(),
                "member".to_string(),
                "pinned".to_string()
            ])
        );
        assert!(rs[0].detail.contains("`intent[1]`"), "{}", rs[0].detail);
        assert!(
            !rs[0].detail.contains("favourite"),
            "the term is not echoed"
        );
    }

    #[test]
    fn never_declared_is_refused_as_a_state_no_live_shape_can_have() {
        let mut a = ask(vec![]);
        a.shape_states = vec!["never_declared".to_string()];
        let rs = refusals(&a, &shape_of(json!({})));
        assert_eq!(reasons(&rs), vec![RefusalReason::UnknownShapeState]);
        assert!(rs[0].detail.contains("never_declared"), "{}", rs[0].detail);
        assert_eq!(
            rs[0].teaches.as_ref().map(|t| t.terms.clone()),
            Some(ASKABLE_SHAPE_STATES.iter().map(|s| s.to_string()).collect())
        );
    }

    #[test]
    fn a_repeated_term_is_refused_rather_than_deduplicated() {
        let mut a = ask(vec![]);
        a.intent = vec!["pinned".to_string(), "pinned".to_string()];
        assert_eq!(
            reasons(&refusals(&a, &shape_of(json!({})))),
            vec![RefusalReason::DuplicateSetMember]
        );
    }

    #[test]
    fn named_terms_replace_the_defaults() {
        let mut a = ask(vec![]);
        a.intent = vec!["pinned".to_string()];
        a.shape_states = vec!["declared_not_yet_checked".to_string()];
        let v = validate_ask(&a, &shape_of(json!({}))).unwrap();
        assert_eq!(v.intent(), &["pinned"]);
        assert_eq!(v.shape_states(), &["declared_not_yet_checked"]);
    }

    // ── Form ────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn an_empty_path_is_refused_as_empty_artifact_path() {
        let rs = refusals(
            &ask(vec![at(&[], PropertyOp::HasKey)]),
            &shape_of(json!({})),
        );
        assert_eq!(reasons(&rs), vec![RefusalReason::EmptyArtifactPath]);
    }

    #[test]
    fn a_path_past_the_form_cap_and_a_segment_past_its_length_are_refused_by_name() {
        let deep: Vec<String> = (0..=MAX_ARTIFACT_PATH_DEPTH)
            .map(|i| format!("k{i}"))
            .collect();
        let long = "k".repeat(MAX_ARTIFACT_PATH_SEGMENT_BYTES + 1);
        let a = ask(vec![
            ArtifactPredicate {
                path: deep,
                op: PropertyOp::HasKey,
            },
            ArtifactPredicate {
                path: vec![long],
                op: PropertyOp::HasKey,
            },
        ]);
        assert_eq!(
            reasons(&refusals(&a, &shape_of(json!({})))),
            vec![
                RefusalReason::TooManyFilterValues,
                RefusalReason::FilterStringTooLong
            ]
        );
    }

    #[test]
    fn an_empty_contains_and_an_oversized_value_are_refused_by_name() {
        let shape = shape_of(json!({ "properties": { "p": {} } }));
        let big = json!("x".repeat(MAX_PROPERTY_VALUE_BYTES));
        let a = ask(vec![
            at(&["p"], contains(vec![])),
            at(&["p"], contains(vec![big])),
        ]);
        assert_eq!(
            reasons(&refusals(&a, &shape)),
            vec![
                RefusalReason::EmptyContains,
                RefusalReason::PropertyValueTooLarge
            ]
        );
    }

    #[test]
    fn the_per_ask_caps_are_refused_and_nothing_is_walked() {
        let shape = shape_of(json!({ "properties": { "p": {} } }));
        let too_many = ask((0..=MAX_ASK_PREDICATES)
            .map(|_| at(&["undeclared"], PropertyOp::HasKey))
            .collect());
        // Only the cap: none of the undeclared paths was walked.
        assert_eq!(
            reasons(&refusals(&too_many, &shape)),
            vec![RefusalReason::FilterNotApplicable]
        );
        let values: Vec<Value> = (0..=MAX_ASK_PROBES).map(|i| json!(i)).collect();
        let too_wide = ask(vec![at(&["p"], contains(values))]);
        assert_eq!(
            reasons(&refusals(&too_wide, &shape)),
            vec![RefusalReason::FilterNotApplicable]
        );
    }

    // ── Shape: one refusal per reason, teaching ─────────────────────────────────────────────

    #[test]
    fn a_path_outside_the_shape_teaches_the_askable_vocabulary_once_for_the_ask() {
        let shape = shape_of(json!({ "properties": { "unit": { "type": "string" } } }));
        let a = ask(vec![
            at(&["units"], PropertyOp::HasKey),
            at(&["count"], PropertyOp::HasKey),
        ]);
        let rs = refusals(&a, &shape);
        assert_eq!(reasons(&rs), vec![RefusalReason::PathOutsideShape]);
        assert!(
            rs[0].detail.contains("2 of this ask's predicates"),
            "{}",
            rs[0].detail
        );
        assert!(rs[0].detail.contains("`predicates[0]`"), "{}", rs[0].detail);
        let teaches = rs[0].teaches.clone().expect("a path refusal teaches");
        assert_eq!(
            teaches.vocabulary,
            Some(Vocabulary {
                paths: vec![AskablePath::new(
                    vec!["unit".to_string()],
                    PathFacts {
                        types: Some([JsonType::String].into_iter().collect()),
                        ..PathFacts::default()
                    }
                )],
                truncated: None,
            })
        );
        assert_eq!(teaches.beyond_bound, None);
        assert_eq!(teaches.elided, None);
    }

    #[test]
    fn a_path_deeper_than_the_walk_is_refused_naming_the_bound() {
        let shape = shape_of(json!({ "properties": { "a": {} } }));
        let deep: Vec<String> = (0..=MAX_WALK_DEPTH).map(|i| format!("k{i}")).collect();
        let rs = refusals(
            &ask(vec![ArtifactPredicate {
                path: deep,
                op: PropertyOp::HasKey,
            }]),
            &shape,
        );
        assert_eq!(reasons(&rs), vec![RefusalReason::PathOutsideShape]);
        assert_eq!(
            rs[0].teaches.as_ref().and_then(|t| t.beyond_bound),
            Some(BoundKind::Depth)
        );
    }

    #[test]
    fn a_schema_past_the_byte_bound_refuses_every_path_and_says_why() {
        let huge =
            json!({ "properties": { "p": { "description": "x".repeat(MAX_SCHEMA_BYTES) } } });
        let rs = refusals(&ask(vec![at(&["p"], PropertyOp::HasKey)]), &shape_of(huge));
        assert_eq!(reasons(&rs), vec![RefusalReason::PathOutsideShape]);
        let teaches = rs[0].teaches.clone().unwrap();
        assert_eq!(teaches.beyond_bound, Some(BoundKind::SchemaBytes));
        assert_eq!(
            teaches.vocabulary,
            Some(Vocabulary {
                paths: vec![],
                truncated: Some(BoundKind::SchemaBytes)
            })
        );
    }

    #[test]
    fn a_value_outside_an_enum_teaches_the_declared_values_and_the_route_to_the_rest() {
        let shape = shape_of(json!({ "properties": {
            "unit": { "type": "string", "enum": ["ms", "s"] }
        } }));
        let rs = refusals(
            &ask(vec![at(&["unit"], contains(vec![json!("fortnights")]))]),
            &shape,
        );
        assert_eq!(reasons(&rs), vec![RefusalReason::ValueOutsideShape]);
        let teaches = rs[0].teaches.clone().unwrap();
        assert_eq!(
            teaches.vocabulary.map(|v| v.paths),
            Some(vec![AskablePath::new(
                vec!["unit".to_string()],
                PathFacts {
                    types: Some([JsonType::String].into_iter().collect()),
                    values: Some(vec![json!("ms"), json!("s")]),
                    items: None,
                }
            )])
        );
        let route = teaches
            .non_closure
            .expect("the refusal says what it does not claim");
        for needle in [
            "not closed",
            "has_key",
            "declared_not_satisfied",
            "declared_not_yet_checked",
            "batch",
        ] {
            assert!(route.contains(needle), "{needle}: {route}");
        }
        assert!(
            !rs[0].detail.contains("fortnights"),
            "the value is not echoed"
        );
    }

    #[test]
    fn an_operator_outside_the_shape_teaches_the_paths_facts_and_feasible_operators() {
        let shape = shape_of(json!({ "properties": { "flag": { "type": "boolean" } } }));
        let rs = refusals(&ask(vec![at(&["flag"], compare(json!(true)))]), &shape);
        assert_eq!(reasons(&rs), vec![RefusalReason::OperatorOutsideShape]);
        let paths = rs[0]
            .teaches
            .clone()
            .and_then(|t| t.vocabulary)
            .unwrap()
            .paths;
        assert_eq!(
            paths[0].operators,
            vec![AskOperator::HasKey, AskOperator::Contains]
        );
    }

    #[test]
    fn a_teaching_is_bounded_and_says_how_much_it_cut() {
        let mut props = serde_json::Map::new();
        for i in 0..200 {
            props.insert(
                format!("{i:03}{}", "k".repeat(200)),
                json!({ "type": "string" }),
            );
        }
        let shape = shape_of(json!({ "properties": props }));
        let rs = refusals(&ask(vec![at(&["absent"], PropertyOp::HasKey)]), &shape);
        let teaches = rs[0].teaches.clone().unwrap();
        let kept = teaches.vocabulary.clone().unwrap().paths;
        let elided = teaches.elided.expect("200 long paths do not fit");
        assert_eq!(kept.len() + elided.omitted_paths, 200);
        assert!(!kept.is_empty());
        assert_eq!(elided.shape, shape.shape_id);
        let bytes: usize = kept.iter().map(json_bytes).sum();
        assert!(bytes <= MAX_TEACHING_BYTES, "{bytes}");
    }

    #[test]
    fn a_refused_form_is_not_walked_but_its_well_formed_siblings_are() {
        let shape = shape_of(json!({ "properties": { "p": {} } }));
        let a = ask(vec![
            at(&[], PropertyOp::HasKey),
            at(&["undeclared"], PropertyOp::HasKey),
        ]);
        assert_eq!(
            reasons(&refusals(&a, &shape)),
            vec![
                RefusalReason::EmptyArtifactPath,
                RefusalReason::PathOutsideShape
            ]
        );
    }

    // ── Refusal steps 1–3, from the load ────────────────────────────────────────────────────

    #[test]
    fn a_loaded_shape_passes_through() {
        let shape = shape_of(json!({}));
        let load = ShapeLoad::Loaded(shape.clone());
        assert_eq!(load.loaded(), Ok(&shape));
    }

    #[test]
    fn not_askable_and_uncompilable_refuse_with_fixed_sentences() {
        let not_askable = ShapeLoad::NotAskable.loaded().unwrap_err();
        assert_eq!(not_askable.reason, RefusalReason::ShapeNotAskable);
        assert_eq!(not_askable.detail, NOT_ASKABLE);
        assert_eq!(not_askable.teaches, None);
        let uncompilable = ShapeLoad::Uncompilable.loaded().unwrap_err();
        assert_eq!(uncompilable.reason, RefusalReason::ShapeSchemaUncompilable);
        assert_eq!(uncompilable.detail, UNCOMPILABLE);
    }

    #[test]
    fn a_folded_shape_teaches_its_successor() {
        let successor = ShapeSuccessor {
            shape_id: ShapeId::from(Uuid::from_u128(8)),
            shape_version: 2,
        };
        let r = ShapeLoad::Superseded {
            successor: Some(successor),
        }
        .loaded()
        .unwrap_err();
        assert_eq!(r.reason, RefusalReason::ShapeSuperseded);
        assert_eq!(r.teaches.and_then(|t| t.successor), Some(successor));
    }

    #[test]
    fn a_folded_shape_with_no_live_successor_refuses_without_one_and_does_not_panic() {
        // Decision P16: unreachable today, and its absence is a refusal that omits the successor.
        let r = ShapeLoad::Superseded { successor: None }
            .loaded()
            .unwrap_err();
        assert_eq!(r.reason, RefusalReason::ShapeSuperseded);
        assert_eq!(r.teaches, None);
        assert_eq!(r.detail, SUPERSEDED_NO_SUCCESSOR);
    }

    #[test]
    fn teaches_is_absent_on_the_wire_when_there_is_nothing_to_teach() {
        let r = ShapeLoad::NotAskable.loaded().unwrap_err();
        let wire = serde_json::to_value(&r).unwrap();
        assert!(wire.get("teaches").is_none(), "{wire}");
        let back: PlanRefusal = serde_json::from_value(json!({
            "reason": "shape_not_askable", "detail": NOT_ASKABLE
        }))
        .expect("an older producer's refusal, with no teaches, still parses");
        assert_eq!(back, r);
    }
}
