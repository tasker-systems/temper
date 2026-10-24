//! Validating an ask — whether it lies inside its shape, decided before any SQL exists (spec §3.4,
//! §3.5 step 4), and the refusals that teach the shape's vocabulary when it does not.
//!
//! **Every refusal at once, one per reason.** A caller repairing an ask sees all of it in one round
//! trip, as `validate` promises for a composition. Each reason is reported once per ask, naming how
//! many predicates broke the rule and the first — the incumbent rule (`validate/shape.rs`, the
//! property-predicate checks), which keeps a body of bad predicates from becoming a refusal list
//! several times its size. A detail never echoes a caller's value or path: an index names it.
//!
//! **Order** (§3.5). The term vocabularies and each predicate's form need no shape. A predicate
//! whose form is refused is not walked, and an ask over the count or size caps walks nothing — refused
//! once, up front, so no check runs once per entry of an over-long list.
//!
//! The refusals carry `stage: None`. The composable arm attaches its stage; the batch read has none
//! (spec §8).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::reference::{is_integral, jsonb_contains_within, nested_contains_within};
use super::session::RequestBudget;
use super::walk::{
    json_bytes_up_to, AskablePath, AskableView, BoundKind, Enumerated, Facts, JsonType, Member,
    Resolved, ShapeWalk, Vocabulary, MAX_SCHEMA_BYTES, MAX_WALK_DEPTH, MAX_WALK_PATHS,
    MAX_WALK_VISITS,
};
use super::{
    ArtifactAsk, ArtifactPredicate, LoadedShape, ShapeHome, ShapeIdentity, ShapeInForce, ShapeLoad,
    ARTIFACT_INTENTS, ASKABLE_SHAPE_STATES, DEFAULT_ARTIFACT_INTENTS,
};
use crate::types::ids::ShapeId;
use crate::types::query::filter::{
    json_string_bytes, property_value_bytes, property_value_nodes, PropertyOp,
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

/// The most containment steps one request's asks may spend between them checking their `contains`
/// values against the enums their shapes declare — one step per node compared, and one more per 64
/// bytes of each object key looked up and of each string compared (`reference::BYTES_PER_STEP`), so
/// a long key or string costs what it carries. Past it the predicate is refused
/// `path_outside_shape` with `teaches.beyond_bound: work`, never admitted unchecked.
///
/// One budget for the request, not one per ask ([`AskSession`](super::AskSession)): a request that
/// carries many asks — a composition's stages, or a batch read's — would otherwise multiply it.
///
/// # 1,048,576
///
/// Membership is a containment test against every member, so its cost is the product of the
/// probes and the members. A real ask names a few values against an enum of tens or thousands; a
/// five-thousand-member enum checked against two hundred probes fits. The worst an ask can bring —
/// [`MAX_ASK_PROBES`] probes of [`MAX_PROPERTY_VALUE_NODES`] nodes against the most a walk can
/// carry — stops here after milliseconds rather than running for seconds.
pub const MAX_MEMBERSHIP_STEPS: usize = 1 << 20;

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
/// | `too_many_filter_values` (a term list longer than its vocabulary) | `terms` |
/// | `shape_superseded` | `successor` |
/// | `shape_schema_uncompilable` | `beyond_bound: schema_bytes` for a schema past the byte bound |
/// | `ask_scope_conflict` | `homes` |
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "artifact_ask.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct RefusalTeaching {
    /// The shape's askable paths — the same structure as a shape's `askable` — narrowed to what the
    /// refusal is about, and held to a bounded size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vocabulary: Option<Vocabulary>,
    /// The walk bound an asked path lies beyond, or — on `shape_schema_uncompilable` —
    /// `schema_bytes`, for a whole schema too large for the walk to read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beyond_bound: Option<BoundKind>,
    /// A closed vocabulary of terms: the intents, or the askable shape states.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub terms: Vec<String>,
    /// The version now in force, for a folded shape that has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub successor: Option<ShapeInForce>,
    /// That stored content is not closed by its shape, and the route to values it does not
    /// declare (spec §3.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub non_closure: Option<String>,
    /// Paths cut from `vocabulary` to keep the teaching bounded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elided: Option<TeachingElision>,
    /// The homes of the shapes a stage's asks name, when no one home can hold them all. Each is
    /// the home of a shape the asker can read — refusal step 1 admitted it — so naming it
    /// discloses nothing a by-id read of the shape would not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub homes: Vec<ShapeHome>,
}

/// What a `path_outside_shape` refusal teaches of one shape — its askable vocabulary held to
/// [`MAX_TEACHING_BYTES`], and how many paths were cut — without the bound the refused path met,
/// which each refusal sets beside it (`beyond_bound`). That is a function of the shape alone: the
/// vocabulary is enumerated under a budget of its own, whatever the request has spent. A shape row
/// is immutable per id (§2.2), so a caller may remember a teaching by shape id across requests and
/// hand it back in a later request's [`ShapeContext`](super::ShapeContext)
/// ([`ShapeContext::with_path_teaching`](super::ShapeContext::with_path_teaching)), whose session
/// then teaches from it byte for byte as it would have taught from an enumeration, and enumerates
/// nothing.
///
/// Made only by a session ([`AskSession::taught`](super::AskSession::taught) reports the ones it
/// made), never from a caller's data. Cheap to clone: the teaching is shared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathTeaching {
    shape: ShapeId,
    teaching: Arc<RefusalTeaching>,
}

impl PathTeaching {
    /// `teaching`, what `teaching` made of `shape`'s vocabulary, held with its paths' spare
    /// capacity released, so what it holds is what it sends.
    fn new(shape: ShapeId, mut teaching: RefusalTeaching) -> PathTeaching {
        if let Some(vocabulary) = teaching.vocabulary.as_mut() {
            vocabulary.paths.shrink_to_fit();
        }
        PathTeaching {
            shape,
            teaching: Arc::new(teaching),
        }
    }

    /// The shape this teaches.
    pub fn shape(&self) -> ShapeId {
        self.shape
    }

    /// The teaching itself, with no `beyond_bound`: what a holder weighs to remember it.
    pub fn teaching(&self) -> &RefusalTeaching {
        &self.teaching
    }
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
/// Parse-don't-validate: the fields are private and
/// [`AskSession::validate`](super::AskSession::validate) is the only constructor. It is
/// a validated **output**, not the ask rewritten in place: an array path's `contains` values arrive
/// wrapped (§3.4, *Why the array row wraps*), and the defaults of §2.1 are applied.
///
/// **It carries the identity of the shape it was validated against** — id, version, home and
/// family, taken from the [`LoadedShape`] and never from the ask — so the seal holds the loaded
/// shape rows for every ask it admitted (spec §4.3 item 3) and `compile` reads them from one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedAsk {
    shape: ShapeIdentity,
    predicates: Vec<ArtifactPredicate>,
    intent: Vec<&'static str>,
    include_folded: bool,
    shape_states: Vec<&'static str>,
}

impl ValidatedAsk {
    /// The shape this ask was validated against: the version, home and family the SQL uses.
    pub fn shape(&self) -> &ShapeIdentity {
        &self.shape
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

    /// The ask as the SQL binds it: one `jsonb` value carrying the shape's id and every list —
    /// `predicates`, `intent`, `shape_states` — written out, empty ones included.
    ///
    /// **Bound from this, never from the caller's [`ArtifactAsk`].** That type skips an empty list
    /// on the wire, so an ask with no predicates would bind no `predicates` key at all, and
    /// `_temper_artifact_ask_match` (`20261103000000`) reads a missing list as a match for nothing:
    /// the whole governed population asked for, and an empty answer given. Here the lists are the
    /// validated ones — the defaults applied, an array path's `contains` values wrapped — so the
    /// statement runs what was validated and nothing the caller wrote around it.
    pub fn bind(&self) -> Value {
        let ValidatedAsk {
            shape,
            predicates,
            intent,
            include_folded,
            shape_states,
        } = self;
        serde_json::to_value(AskBind {
            shape: shape.shape_id,
            predicates,
            intent,
            include_folded: *include_folded,
            shape_states,
        })
        .expect("an ask's bind is JSON with string keys, so it always serializes")
    }
}

/// [`ValidatedAsk::bind`]'s wire form. No field is skipped when empty: the SQL reads each one.
#[derive(Serialize)]
struct AskBind<'a> {
    shape: ShapeId,
    predicates: &'a [ArtifactPredicate],
    intent: &'a [&'static str],
    include_folded: bool,
    shape_states: &'a [&'static str],
}

/// Refusal step 4 of §3.5 — path, operator and value — for one ask against the shape the loader
/// admitted, read through the request's walk of that shape and charged to the request's budget.
/// The shape-free checks are [`validate_ask_form`]'s, run first; steps 1–3 are
/// [`ShapeLoad::loaded`]; [`AskSession::validate`](super::AskSession::validate) runs all four.
///
/// `walk` must be the walk of `shape`, which the session keys by the same id.
pub(super) fn validate_loaded<'c>(
    ask: &ArtifactAsk,
    shape: &LoadedShape,
    walk: &mut ShapeWalk<'c>,
    budget: &mut RequestBudget<'c>,
) -> Result<ValidatedAsk, Vec<PlanRefusal>> {
    let mut errs = Vec::new();
    let form = check_form(ask, &mut errs);
    refuse_unserved(ask, &mut errs);
    let predicates = against_shape(&ask.predicates, &form.walkable, walk, budget, &mut errs);
    if !errs.is_empty() {
        return Err(errs);
    }
    Ok(ValidatedAsk {
        shape: shape.identity(),
        predicates,
        intent: form.intent,
        include_folded: ask.include_folded,
        shape_states: form.shape_states,
    })
}

/// The checks that need no shape: the term vocabularies, the per-ask caps, and each predicate's
/// path and value form. Every refusal they raise, at once.
///
/// One definition for both callers: [`AskSession::validate`](super::AskSession::validate) runs it
/// before walking the shape (and beside a steps 1–3 refusal, which consulted no schema), and an
/// offline check that cannot load shapes (`temper query --check`, spec §4.3 item 5) runs it alone.
///
/// **Form only, never what this server serves.** `include_folded: true` is well-formed and is
/// refused by the server alone (`refuse_unserved`): serving it is a capability a later server may
/// gain, and an offline check that refused it would refuse, on an older client, what a newer server
/// answers.
pub fn validate_ask_form(ask: &ArtifactAsk) -> Vec<PlanRefusal> {
    let mut errs = Vec::new();
    check_form(ask, &mut errs);
    errs
}

/// What an ask may say that this server does not serve: `include_folded: true`, declined by name
/// ("declined, never ignored") rather than answered over live artifacts alone. Needs no shape, so
/// it rides beside a steps 1–3 refusal as the form does; it is not part of
/// [`validate_ask_form`], whose offline caller cannot know what the server it will reach serves.
pub(super) fn refuse_unserved(ask: &ArtifactAsk, errs: &mut Vec<PlanRefusal>) {
    if ask.include_folded {
        errs.push(refusal(
            RefusalReason::FilterNotApplicable,
            INCLUDE_FOLDED.to_string(),
            None,
        ));
    }
}

/// Whether an ask is past a count or size cap of its form: more terms than a vocabulary holds, more
/// predicates or probes than an ask may make, a path deeper than `MAX_ARTIFACT_PATH_DEPTH` or a
/// segment longer than `MAX_ARTIFACT_PATH_SEGMENT_BYTES`, a value past `MAX_PROPERTY_VALUE_BYTES` or
/// `MAX_PROPERTY_VALUE_NODES`. Every one is a refusal [`validate_ask_form`] raises, so an ask this
/// answers `true` for is refused by its form.
///
/// **Checked before a session keys an ask** ([`AskSession::validate`](super::AskSession::validate)),
/// so what a session keeps is within these caps. Each list's length is compared before the list is
/// read, and a list is read only once its length is within its cap, so the check itself costs at
/// most what an ask within the caps carries; the form refusal that follows allocates in proportion
/// to the predicate count, transiently, never kept.
pub(super) fn exceeds_form_caps(ask: &ArtifactAsk) -> bool {
    if ask.intent.len() > INTENTS.known.len()
        || ask.shape_states.len() > SHAPE_STATES.known.len()
        || ask.predicates.len() > MAX_ASK_PREDICATES
    {
        return true;
    }
    if ask.predicates.iter().map(|p| p.op.probes()).sum::<usize>() > MAX_ASK_PROBES {
        return true;
    }
    ask.predicates.iter().any(|p| {
        p.path.len() > MAX_ARTIFACT_PATH_DEPTH
            || p.path
                .iter()
                .any(|s| json_string_bytes(s) > MAX_ARTIFACT_PATH_SEGMENT_BYTES)
            || p.op.values().iter().any(|v| {
                property_value_bytes(v) > MAX_PROPERTY_VALUE_BYTES
                    || property_value_nodes(v) > MAX_PROPERTY_VALUE_NODES
            })
    })
}

/// What the shape-free checks decided: the resolved terms, and which predicates may be walked.
struct Form {
    intent: Vec<&'static str>,
    shape_states: Vec<&'static str>,
    walkable: Vec<bool>,
}

fn check_form(ask: &ArtifactAsk, errs: &mut Vec<PlanRefusal>) -> Form {
    let intent = resolve_terms(&ask.intent, &INTENTS, errs);
    let shape_states = resolve_terms(&ask.shape_states, &SHAPE_STATES, errs);
    // Over a count cap nothing is walked: refused once, up front.
    let walkable = if within_caps(&ask.predicates, errs) {
        well_formed(&ask.predicates, errs)
    } else {
        vec![false; ask.predicates.len()]
    };
    Form {
        intent,
        shape_states,
        walkable,
    }
}

impl ShapeLoad {
    /// Refusal steps 1–3 of §3.5: the loaded shape, or the refusal its load decided.
    ///
    /// **Fixed sentences.** `shape_not_askable` reads the same for a shape that does not exist and
    /// one the asker cannot read, because they are one arm. `shape_schema_uncompilable` never
    /// echoes the compiler: its text is the author's material, not the asker's. A schema past the
    /// walk's byte bound is refused with the same reason, its own sentence naming the bound, and
    /// `teaches.beyond_bound: schema_bytes` — the bound as data, never a compiler's words.
    pub fn loaded(&self) -> Result<&LoadedShape, PlanRefusal> {
        let (reason, detail, teaches) = match self {
            ShapeLoad::Loaded(shape) => return Ok(shape),
            ShapeLoad::NotAskable => return Err(not_askable()),
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
            ShapeLoad::SchemaTooLarge => {
                return Err(refusal(
                    RefusalReason::ShapeSchemaUncompilable,
                    schema_too_large(),
                    Some(RefusalTeaching {
                        beyond_bound: Some(BoundKind::SchemaBytes),
                        ..RefusalTeaching::default()
                    }),
                ))
            }
        };
        Err(refusal(reason, detail.to_string(), teaches))
    }
}

const NOT_ASKABLE: &str =
    "the named shape does not exist or you cannot read it, so nothing can be asked of it";

/// The one `shape_not_askable` refusal: for a shape absent, unreadable, or not the one the ask
/// names.
pub(super) fn not_askable() -> PlanRefusal {
    refusal(
        RefusalReason::ShapeNotAskable,
        NOT_ASKABLE.to_string(),
        None,
    )
}

const SUPERSEDED: &str = "the named shape has been amended. An ask is never re-pointed at a \
     version it was not written against: if its predicates still mean the same thing, ask the \
     version now in force (`teaches.successor`)";

const SUPERSEDED_NO_SUCCESSOR: &str = "the named shape has been amended, and no version of it is \
     now in force in its home, so nothing can be asked of it";

const UNCOMPILABLE: &str =
    "the shape's schema does not compile, so nothing can be asked of it until the shape is amended";

/// The sentence for a schema past the walk's byte bound: never compiled, so never "does not
/// compile" — the reason is the size, and the sentence says which.
fn schema_too_large() -> String {
    format!(
        "the shape's schema is larger than the {MAX_SCHEMA_BYTES} bytes this server reads, so it \
         is not compiled and nothing can be asked of it until the shape is amended to a smaller \
         schema"
    )
}

/// `include_folded: true`, declined by name: an ask reads live artifacts and nothing else, and
/// admitting the flag while answering over the live ones would drop the folded ones without a
/// word.
const INCLUDE_FOLDED: &str = "`include_folded: true` is not served: an ask reads live artifacts \
     only, so the folded ones it names would be left out without a word. Ask without it — every \
     live artifact the shape governs is still in the population";

/// `unstorable_value`: a value no stored content can hold.
const UNSTORABLE: &str = "a predicate value carries the character U+0000, in a string or an \
     object key, which no stored content can hold — the database cannot represent it — so the \
     value could match nothing";

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

/// The terms asked, in vocabulary order, or the defaults when none are. An unknown term and a
/// repeated one are refused, once each; a repeat because it changes nothing and is the only way to
/// make an unbounded list out of a bounded vocabulary (`DuplicateSetMember`'s incumbent rule).
///
/// **A list longer than the vocabulary is refused by its length**, `too_many_filter_values`, before
/// any term is read: it must repeat a term or name one outside the set, and its length — not its
/// terms — is what a caller controls, so it costs one comparison to refuse whatever it holds.
///
/// **Vocabulary order, never the caller's**, so a validated ask's terms are canonical: two asks naming
/// the same terms in different orders bind the same JSON, and the SQL reads the list order-free.
fn resolve_terms(
    given: &[String],
    terms: &Terms,
    errs: &mut Vec<PlanRefusal>,
) -> Vec<&'static str> {
    if given.is_empty() {
        return terms.defaults.to_vec();
    }
    let field = terms.field;
    if given.len() > terms.known.len() {
        errs.push(refusal(
            RefusalReason::TooManyFilterValues,
            format!(
                "`{field}` names {} terms and its vocabulary has {}, so the list repeats a term or \
                 names one outside it; a set names each member once",
                given.len(),
                terms.known.len()
            ),
            Some(RefusalTeaching {
                terms: terms.known.iter().map(|t| t.to_string()).collect(),
                ..RefusalTeaching::default()
            }),
        ));
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
    out.sort_by_key(|term| terms.known.iter().position(|known| known == term));
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

/// The two per-ask cost caps (spec §3.4, *Caps*), each refused by name and never clamped.
fn within_caps(predicates: &[ArtifactPredicate], errs: &mut Vec<PlanRefusal>) -> bool {
    let count = predicates.len();
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
        // Past the count, the probes are not summed: the list is refused for its length, and
        // reading it whole would cost what that length costs.
        return false;
    }
    let probes: usize = predicates.iter().map(|p| p.op.probes()).sum();
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
    probes <= MAX_ASK_PROBES
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
    // A path past the depth cap is refused for its depth, and its segments are not read.
    let long = indices(&|p: &ArtifactPredicate| {
        p.path.len() <= MAX_ARTIFACT_PATH_DEPTH
            && p.path
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
    let unstorable = indices(&|p: &ArtifactPredicate| p.op.values().iter().any(holds_nul));

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
    refuse_at(
        errs,
        RefusalReason::UnstorableValue,
        &unstorable,
        UNSTORABLE,
        None,
    );

    let faulted = [empty, deep, long, no_values, large, unstorable];
    (0..predicates.len())
        .map(|i| !faulted.iter().any(|f| f.contains(&i)))
        .collect()
}

/// Whether a value carries U+0000 anywhere — in a string, or in an object key, at any depth. jsonb
/// cannot store it, so no artifact holds such a value, and binding one is a database error rather
/// than a refusal. Depth is the parsed value's, which the JSON parser has already bounded.
fn holds_nul(value: &Value) -> bool {
    match value {
        Value::String(s) => s.contains('\0'),
        Value::Array(elements) => elements.iter().any(holds_nul),
        Value::Object(members) => members
            .iter()
            .any(|(key, member)| key.contains('\0') || holds_nul(member)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

/// Why a declared path refuses a predicate (§3.4, the "Refused as" column), or that checking it
/// ran out of membership steps first.
enum Fault {
    Operator,
    Value,
    Work,
}

/// The well-formed predicates checked against the shape, one walk per path: bound, or refused.
fn against_shape<'c>(
    predicates: &[ArtifactPredicate],
    formed: &[bool],
    walk: &mut ShapeWalk<'c>,
    budget: &mut RequestBudget<'c>,
    errs: &mut Vec<PlanRefusal>,
) -> Vec<ArtifactPredicate> {
    if !formed.contains(&true) {
        return Vec::new();
    }
    let shape = walk.shape();
    let sorted = sort(predicates, formed, walk, budget);
    refuse_paths(&sorted, walk, errs);

    let (at, teaches) = offending(&sorted.outside_operator, &sorted.resolved, shape);
    refuse_at(
        errs,
        RefusalReason::OperatorOutsideShape,
        &at,
        "an operator is not feasible at its path: `compare` needs a path declaring `number`, \
         `integer` or `string`, or no type, and a bound of a type declared there",
        Some(teaches),
    );
    let (at, mut teaches) = offending(&sorted.outside_value, &sorted.resolved, shape);
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
struct Sorted<'p, 's> {
    bound: Vec<ArtifactPredicate>,
    outside_path: Vec<usize>,
    /// The first walk bound an outside path met.
    beyond: Option<BoundKind>,
    /// Each predicate refused for its operator: its index, and its path's slot in `resolved`.
    outside_operator: Vec<(usize, usize)>,
    /// Each predicate refused for a value: its index, and its path's slot in `resolved`.
    outside_value: Vec<(usize, usize)>,
    /// Each distinct path the ask walks, once, with what the shape says of it — the walk's own
    /// facts, borrowed from the schema and never copied.
    resolved: Vec<(&'p [String], Resolved<'s>)>,
}

/// Every well-formed predicate against the shape.
///
/// **One walk per distinct path, all under the request's budget** (§3.3: the walk "never stalls or
/// exhausts the server"). A per-predicate budget let one ask spend [`MAX_ASK_PREDICATES`] budgets;
/// a per-ask budget let one request spend one per ask; memoising by path alone would still let it
/// spend one per distinct path. Sharing the budget holds what every path of every ask in the
/// request costs to [`MAX_WALK_VISITS`] however many they name, and the walk's memo makes a path
/// any ask of the request already named free. A predicate past the spent budget is refused
/// `path_outside_shape` naming `work` — visibly, as every bound is. What a path refusal teaches is
/// the shape's [`PathTeaching`]: taken from the request's context when its caller remembered one,
/// and otherwise made once per shape for the request from a vocabulary enumerated under a budget of
/// its own, so a spent request still teaches what it may ask.
///
/// No path's values are copied to sort it: the facts stay the walk's, borrowed from the schema, so
/// a value set several paths share is read once and costs nothing per path after. A refused path's
/// facts are copied only into a teaching, once per distinct path and only as far as the teaching
/// keeps them ([`offending`], [`teaching`]).
fn sort<'p, 'c>(
    predicates: &'p [ArtifactPredicate],
    formed: &[bool],
    walk: &mut ShapeWalk<'c>,
    budget: &mut RequestBudget<'c>,
) -> Sorted<'p, 'c> {
    let mut slots: HashMap<&'p [String], usize> = HashMap::new();
    let mut distinct: Vec<&'p [String]> = Vec::new();
    let slot_of: Vec<Option<usize>> = predicates
        .iter()
        .zip(formed)
        .map(|(predicate, &walkable)| {
            walkable.then(|| {
                *slots.entry(predicate.path.as_slice()).or_insert_with(|| {
                    distinct.push(predicate.path.as_slice());
                    distinct.len() - 1
                })
            })
        })
        .collect();
    let resolutions: Vec<Resolved<'c>> = distinct
        .iter()
        .map(|path| walk.resolve(path, &mut budget.walk))
        .collect();
    let mut sorted = Sorted {
        resolved: distinct.into_iter().zip(resolutions).collect(),
        ..Sorted::default()
    };
    let steps = &mut budget.membership_steps;
    for (i, (predicate, slot)) in predicates.iter().zip(slot_of).enumerate() {
        let Some(slot) = slot else {
            continue;
        };
        let facts = match &sorted.resolved[slot].1 {
            Resolved::Declared(facts) => facts,
            Resolved::NotDeclared => {
                sorted.outside_path.push(i);
                continue;
            }
            Resolved::BeyondBound(at) => {
                sorted.outside_path.push(i);
                sorted.beyond.get_or_insert(*at);
                continue;
            }
        };
        match bind(&predicate.op, facts, steps) {
            Ok(op) => sorted.bound.push(ArtifactPredicate {
                path: predicate.path.clone(),
                op,
            }),
            Err(Fault::Operator) => sorted.outside_operator.push((i, slot)),
            Err(Fault::Value) => sorted.outside_value.push((i, slot)),
            Err(Fault::Work) => {
                sorted.outside_path.push(i);
                sorted.beyond.get_or_insert(BoundKind::Work);
            }
        }
    }
    sorted
}

/// `path_outside_shape`, teaching the askable vocabulary and the bound an asked path met.
///
/// The vocabulary half is the shape's [`PathTeaching`], the same for every refusal on that shape:
/// the one the request's context remembered, or else made once for the request by
/// [`teach_paths`] and kept on the walk. Only `beyond_bound` is this refusal's own.
fn refuse_paths(sorted: &Sorted<'_, '_>, walk: &mut ShapeWalk<'_>, errs: &mut Vec<PlanRefusal>) {
    if sorted.outside_path.is_empty() {
        return;
    }
    let mut rule = "a path is not one the shape declares — only `properties` chains, local \
                    `$ref`s, `allOf`, `anyOf` and `oneOf` are walked"
        .to_string();
    if let Some(at) = sorted.beyond {
        rule.push_str(&format!(", and {}", bound_said(at)));
    }
    let mut teaches = RefusalTeaching::clone(walk.path_teaching(teach_paths).teaching());
    teaches.beyond_bound = sorted.beyond;
    refuse_at(
        errs,
        RefusalReason::PathOutsideShape,
        &sorted.outside_path,
        &rule,
        Some(teaches),
    );
}

/// A shape's [`PathTeaching`], made from its vocabulary — enumerated over borrowed facts, each path
/// offered to [`teaching`] as an [`AskableView`], so the teaching copies only the paths it has
/// first measured to fit, and `omitted_paths` still counts the rest.
fn teach_paths(vocabulary: &Enumerated<Facts<'_>>, shape: ShapeId) -> PathTeaching {
    PathTeaching::new(
        shape,
        teaching(
            vocabulary
                .paths
                .iter()
                .map(|(path, facts)| AskableView::new(path, facts))
                .collect(),
            AskableView::into_askable,
            vocabulary.truncated.clone(),
            shape,
        ),
    )
}

/// The offending predicates' indices, and a teaching that describes each offending path once:
/// deduplicated by slot first, then each path offered to the teaching as an [`AskableView`] over
/// the walk's borrowed facts — so a path is measured before it is copied, and one the teaching
/// cannot hold is never copied at all.
fn offending(
    found: &[(usize, usize)],
    resolved: &[(&[String], Resolved<'_>)],
    shape: ShapeId,
) -> (Vec<usize>, RefusalTeaching) {
    let at = found.iter().map(|(i, _)| *i).collect();
    let mut slots: Vec<usize> = Vec::new();
    for &(_, slot) in found {
        if !slots.contains(&slot) {
            slots.push(slot);
        }
    }
    let declared: Vec<(&[String], &Facts<'_>)> = slots
        .into_iter()
        .filter_map(|slot| match &resolved[slot] {
            (path, Resolved::Declared(facts)) => Some((*path, facts)),
            _ => None,
        })
        .collect();
    let views = declared
        .into_iter()
        .map(|(path, facts)| AskableView::new(path, facts))
        .collect();
    (
        at,
        teaching(views, AskableView::into_askable, BTreeSet::new(), shape),
    )
}

fn bound_said(at: BoundKind) -> String {
    match at {
        BoundKind::Depth => format!("the walk descends at most {MAX_WALK_DEPTH} segments"),
        BoundKind::Paths => format!("the vocabulary lists at most {MAX_WALK_PATHS} paths"),
        BoundKind::SchemaBytes => {
            format!("the walk reads a schema of at most {MAX_SCHEMA_BYTES} bytes")
        }
        BoundKind::Work => format!(
            "the shapes a request names are read within a bounded amount of work — at most \
             {MAX_WALK_VISITS} visits and {MAX_MEMBERSHIP_STEPS} membership steps across every \
             ask in the request"
        ),
    }
}

/// A vocabulary held to [`MAX_TEACHING_BYTES`]: its paths in order until the next would not fit,
/// and an elision saying how many were cut — counted over every candidate, so `omitted_paths`
/// says how much of the whole vocabulary the teaching left out.
///
/// **No copy it has not first bounded.** Both callers offer each candidate as an [`AskableView`]
/// over the walk's borrowed facts — the offending paths an ask resolved, and the shape's
/// vocabulary, enumerated without copying a value ([`teach_paths`], whose teaching is then kept for
/// the shape rather than made again). Each is measured as it serializes, and only as
/// far as the room left, then turned into an owned path by `own` only when it fits. So the values
/// a teaching copies are the values it sends, at most [`MAX_TEACHING_BYTES`] on the wire; a path
/// that does not fit is never copied, and measuring it stops at the first write past the room
/// left. That write may be one long string, which serde_json scans whole before writing: at most
/// one string per teaching, allocating nothing.
fn teaching<T: Serialize>(
    candidates: Vec<T>,
    own: impl Fn(T) -> AskablePath,
    truncated: BTreeSet<BoundKind>,
    shape: ShapeId,
) -> RefusalTeaching {
    let total = candidates.len();
    let mut kept = Vec::new();
    let mut used = 0;
    for candidate in candidates {
        let room = MAX_TEACHING_BYTES - used;
        let bytes = json_bytes_up_to(&candidate, room);
        if bytes > room {
            break;
        }
        used += bytes;
        kept.push(own(candidate));
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
fn bind(op: &PropertyOp, facts: &Facts<'_>, steps: &mut usize) -> Result<PropertyOp, Fault> {
    match op {
        PropertyOp::HasKey => Ok(PropertyOp::HasKey),
        // Every `contains` refusal is the value's (§3.4: the contains rows refuse
        // `value_outside_shape`), including at a path whose branches declare no common type.
        PropertyOp::Contains { values } => values
            .iter()
            .map(|v| bind_contains(facts, v, steps))
            .collect::<Result<Vec<_>, _>>()
            .map(|values| PropertyOp::Contains { values }),
        PropertyOp::Compare { value, .. } if compare_admits(facts, value) => Ok(op.clone()),
        PropertyOp::Compare { .. } => Err(Fault::Operator),
    }
}

/// The `contains` rows. A value whose own type the path declares is bound as it is — the scalar
/// row or the object row. Otherwise, at a path declaring `array`, the value is an **element**:
/// admissible under `items` and bound wrapped as `[v]`. An untyped path binds every value
/// unwrapped, with the incumbent `PropertyOp` semantics. Whatever is bound is then held to the
/// path's enum.
fn bind_contains(facts: &Facts<'_>, value: &Value, steps: &mut usize) -> Result<Value, Fault> {
    let bound = match &facts.types {
        None => value.clone(),
        Some(types) if !value.is_array() && admits_type(types, value) => value.clone(),
        Some(types) if types.contains(&JsonType::Array) => {
            if !element_admitted(facts.items.as_deref(), value, steps)? {
                return Err(Fault::Value);
            }
            Value::Array(vec![value.clone()])
        }
        Some(_) => return Err(Fault::Value),
    };
    if member(facts.values.as_deref().map(Vec::as_slice), &bound, steps)? {
        Ok(bound)
    } else {
        Err(Fault::Value)
    }
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
/// `items` enum, contained by a member as an element is — below the top level, so an array member
/// never holds a primitive.
fn element_admitted(
    items: Option<&Facts<'_>>,
    value: &Value,
    steps: &mut usize,
) -> Result<bool, Fault> {
    let Some(items) = items else {
        return Ok(true);
    };
    if !items.types.as_ref().is_none_or(|t| admits_type(t, value)) {
        return Ok(false);
    }
    within_enum(items.values.as_deref().map(Vec::as_slice), |m| {
        nested_contains_within(m, value, steps)
    })
}

/// Whether a bound value can match conforming content under the path's enum: some member, as stored
/// content, contains it — jsonb's `@>` at the top level, so a scalar must equal a member or be an
/// element of an array member, and a container must be held by a member of its own kind.
fn member(values: Option<&[Member<'_>]>, bound: &Value, steps: &mut usize) -> Result<bool, Fault> {
    within_enum(values, |m| jsonb_contains_within(m, bound, steps))
}

/// Whether some member passes a containment test, or `Ok(true)` with no enum. The tests spend
/// membership steps; running out is [`Fault::Work`].
fn within_enum(
    values: Option<&[Member<'_>]>,
    mut holds: impl FnMut(&Value) -> Option<bool>,
) -> Result<bool, Fault> {
    let Some(members) = values else {
        return Ok(true);
    };
    for m in members {
        if holds(m.value).ok_or(Fault::Work)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The `compare` row: a number where `number` or `integer` is declared (a non-integral bound at an
/// `integer` path is admitted — ordering, unlike equality, is meaningful against it), a string
/// where `string` is; any bound at an untyped path, whose runtime type guard makes a mismatch an
/// honest empty. A path declaring none of the three refuses every bound, which is
/// `PathFacts::admits_compare` read with the bound in hand.
fn compare_admits(facts: &Facts<'_>, bound: &Value) -> bool {
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
    use crate::types::artifact_ask::{enumerate, AskOperator, PathFacts, ShapeContext, WalkBounds};
    use crate::types::query::filter::json_bytes;
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

    /// One ask, validated in a request of its own: a one-shape context and a fresh session, so the
    /// walk it reads and the budget it spends are its alone — what every test below measures.
    fn validate_ask(
        ask: &ArtifactAsk,
        shape: &LoadedShape,
    ) -> Result<ValidatedAsk, Vec<PlanRefusal>> {
        let context: ShapeContext = [(shape.shape_id, ShapeLoad::Loaded(shape.clone()))]
            .into_iter()
            .collect();
        context.session().validate(ask)
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
            // object — each value must be an object; under an enum, one some member contains
            Row { declared: json!({ "type": "object" }), has_key: A,
                  contains: [V, V, V, V, V, A, V], compare: [O, O, O, O, O, O, O] },
            Row { declared: json!({ "type": "object", "enum": [{ "z": 1 }] }), has_key: A,
                  contains: [V, V, V, V, V, V, V], compare: [O, O, O, O, O, O, O] },
            Row { declared: json!({ "type": "object", "enum": [{ "k": 1, "z": 2 }] }), has_key: A,
                  contains: [V, V, V, V, V, A, V], compare: [O, O, O, O, O, O, O] },
            // array of strings — each value is an ELEMENT, wrapped; `["a"]` is not a string element
            Row { declared: json!({ "type": "array", "items": { "type": "string" } }), has_key: A,
                  contains: [W, V, V, V, V, V, V], compare: [O, O, O, O, O, O, O] },
            Row { declared: json!({ "type": "array", "items": { "type": "string", "enum": ["z"] } }),
                  has_key: A,
                  contains: [V, V, V, V, V, V, V], compare: [O, O, O, O, O, O, O] },
            // no type — every operator; an enum holds scalars to membership and containers to
            // containment by a member of their own kind
            Row { declared: json!({}), has_key: A,
                  contains: [A, A, A, A, A, A, A], compare: [A, A, A, A, A, A, A] },
            Row { declared: json!({ "enum": ["z", 7] }), has_key: A,
                  contains: [V, V, V, V, V, V, V], compare: [A, A, A, A, A, A, A] },
            // `"a"` matches stored `["a","b"]` by jsonb's top-level exception, so it is admitted
            Row { declared: json!({ "enum": [{ "k": 1 }, ["a", "b"]] }), has_key: A,
                  contains: [A, V, V, V, V, A, A], compare: [A, A, A, A, A, A, A] },
            // a string-or-array path whose one member is an array: a string the member holds is
            // admitted as written; anything else becomes an element `[v]` the member must hold
            Row { declared: json!({ "type": ["string", "array"], "enum": [["a", "b"]] }), has_key: A,
                  contains: [A, V, V, V, V, V, V], compare: [A, O, O, O, O, O, O] },
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

    /// A validator under test: `validate_ask`, or a stand-in for one.
    type Validator = dyn Fn(&ArtifactAsk, &LoadedShape) -> Result<ValidatedAsk, Vec<PlanRefusal>>;

    /// What `validate_ask` says about one predicate at `["p"]`.
    fn verdict(schema: &Value, op: PropertyOp) -> Verdict {
        verdict_by(&validate_ask, schema, op)
    }

    /// What a validator says about one predicate at `["p"]`.
    fn verdict_by(validate: &Validator, schema: &Value, op: PropertyOp) -> Verdict {
        let shape = shape_of(schema.clone());
        let original = op.clone();
        match validate(&ask(vec![at(&["p"], op)]), &shape) {
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

    /// Every cell of the expectation table and the undeclared paths, as (schema, operator, the
    /// hand-written verdict).
    fn cells() -> Vec<(Value, PropertyOp, Verdict)> {
        let mut out = Vec::new();
        for row in table() {
            let schema = wrapped(&row.declared);
            out.push((schema.clone(), PropertyOp::HasKey, row.has_key));
            for (i, probe) in probes().into_iter().enumerate() {
                out.push((
                    schema.clone(),
                    contains(vec![probe.clone()]),
                    row.contains[i],
                ));
                out.push((schema.clone(), compare(probe), row.compare[i]));
            }
        }
        for schema in undeclared() {
            out.push((schema.clone(), PropertyOp::HasKey, P));
            out.push((schema.clone(), contains(vec![json!("a")]), P));
            out.push((schema, compare(json!(1)), P));
        }
        out
    }

    #[test]
    fn every_operator_declared_type_enum_and_value_type_has_its_hand_written_verdict() {
        for (schema, op, expected) in cells() {
            let said = format!("{op:?} at {schema}");
            assert_eq!(verdict(&schema, op), expected, "{said}");
        }
    }

    /// A `validate_ask` that refuses nothing: every predicate admitted as written.
    fn refuses_nothing(
        ask: &ArtifactAsk,
        shape: &LoadedShape,
    ) -> Result<ValidatedAsk, Vec<PlanRefusal>> {
        Ok(ValidatedAsk {
            shape: shape.identity(),
            predicates: ask.predicates.clone(),
            intent: vec![],
            include_folded: false,
            shape_states: vec![],
        })
    }

    #[test]
    fn a_validator_that_refuses_nothing_fails_the_expectation_table() {
        // The bite: run the same table against a stand-in that refuses nothing, and collect the
        // verdicts it gets wrong. It must fail rows of all three shape reasons.
        let failed: Vec<Verdict> = cells()
            .into_iter()
            .filter_map(|(schema, op, expected)| {
                (verdict_by(&refuses_nothing, &schema, op) != expected).then_some(expected)
            })
            .collect();
        for reason in [P, O, V] {
            assert!(
                failed.contains(&reason),
                "no {reason:?} row fails the stand-in"
            );
        }
    }

    #[test]
    fn membership_against_a_large_enum_stops_at_its_step_budget() {
        // Bites the step budget: with it removed, every probe here is admitted and the ask
        // validates. Catches the regression where enum membership ran every probe against every
        // member unbounded (round-3 review N1).
        //
        // The probes are members, matched late. A probe outside the enum refuses the whole
        // `contains` at the first miss, so misses cost one probe's scan; the expensive ask is one
        // whose every probe is admissible. Each probe `v` here costs `v + 1` steps (one per member
        // compared, in order), so the 256 probes 4,744..=4,999 cost 1,247,360 — past the budget.
        let members: Vec<Value> = (0..5_000).map(|i| json!(i)).collect();
        let shape = shape_of(json!({ "properties": { "n": { "enum": members } } }));
        let late: Vec<Value> = (5_000 - MAX_ASK_PROBES..5_000).map(|i| json!(i)).collect();
        let rs = refusals(&ask(vec![at(&["n"], contains(late))]), &shape);
        assert_eq!(reasons(&rs), vec![RefusalReason::PathOutsideShape]);
        assert_eq!(
            rs[0].teaches.as_ref().and_then(|t| t.beyond_bound),
            Some(BoundKind::Work)
        );
        // A handful of probes against the same enum is checked in full.
        let few = ask(vec![at(&["n"], contains(vec![json!(4_999), json!(7)]))]);
        assert!(validate_ask(&few, &shape).is_ok());
    }

    /// A shape each of whose `MAX_ASK_PREDICATES` properties references one `allOf` of 2,500
    /// branches: resolving one property costs about 2,504 visits, so one fits the walk budget and
    /// all of them — about 80,000 — do not.
    fn heavy_properties() -> LoadedShape {
        let branches: Vec<Value> = (0..2_500).map(|_| json!({})).collect();
        let props: serde_json::Map<String, Value> = (0..MAX_ASK_PREDICATES)
            .map(|i| (format!("p{i}"), json!({ "$ref": "#/$defs/w" })))
            .collect();
        shape_of(json!({ "$defs": { "w": { "allOf": branches } }, "properties": props }))
    }

    /// An ask's paths are resolved under one budget, so naming more of them does not buy
    /// more work. Bites the shared budget in `sort`: with a budget per predicate (or per distinct
    /// path) each of these fits and the ask validates.
    #[test]
    fn an_asks_paths_share_one_walk_budget() {
        let shape = heavy_properties();
        let one = ask(vec![at(&["p0"], PropertyOp::HasKey)]);
        assert!(validate_ask(&one, &shape).is_ok(), "one path fits");
        let every = ask((0..MAX_ASK_PREDICATES)
            .map(|i| at(&[format!("p{i}").as_str()], PropertyOp::HasKey))
            .collect());
        let rs = refusals(&every, &shape);
        assert_eq!(reasons(&rs), vec![RefusalReason::PathOutsideShape]);
        assert_eq!(
            rs[0].teaches.as_ref().and_then(|t| t.beyond_bound),
            Some(BoundKind::Work)
        );
    }

    /// A path named by many predicates is resolved once. Two memos give it — `sort`'s slots within
    /// the ask and `ShapeWalk::resolve`'s cache across the request — so the bite removes both:
    /// resolving the same path once per predicate under the shared budget spends it, and the ask is
    /// refused.
    #[test]
    fn a_path_named_by_many_predicates_is_walked_once() {
        let shape = heavy_properties();
        let same = ask((0..MAX_ASK_PREDICATES)
            .map(|_| at(&["p0"], PropertyOp::HasKey))
            .collect());
        let v = validate_ask(&same, &shape).expect("one walk of `p0` fits");
        assert_eq!(v.predicates().len(), MAX_ASK_PREDICATES);
    }

    /// Every path an ask may name, each over one shared 9,000-member enum, fits
    /// one ask's budget — the enum is read once, about 18,000 visits, and no path's values are
    /// copied. Before, each path paid for the set again and for a wire copy, about 18,000 visits a
    /// path, so the fourth met `work`. Bites the per-budget set memo in `Budget::members`, and
    /// `ShapeWalk::resolve` copying facts onto the wire.
    #[test]
    fn an_ask_may_name_every_path_over_one_large_shared_enum() {
        let members: Vec<Value> = (0..9_000).map(|i| json!(i)).collect();
        let props: serde_json::Map<String, Value> = (0..MAX_ASK_PREDICATES)
            .map(|i| (format!("p{i}"), json!({ "$ref": "#/$defs/e" })))
            .collect();
        let shape = shape_of(json!({ "$defs": { "e": { "enum": members } }, "properties": props }));
        let every = ask((0..MAX_ASK_PREDICATES)
            .map(|i| at(&[format!("p{i}").as_str()], contains(vec![json!(i)])))
            .collect());
        let v = validate_ask(&every, &shape).expect("one read of the enum fits");
        assert_eq!(v.predicates().len(), MAX_ASK_PREDICATES);
    }

    /// The values `Facts::to_wire` copies while one ask is validated, on this thread.
    fn values_copied_validating(a: &ArtifactAsk, shape: &LoadedShape) -> (usize, Vec<PlanRefusal>) {
        use crate::types::artifact_ask::walk::VALUES_COPIED;
        let before = VALUES_COPIED.with(std::cell::Cell::get);
        let rs = refusals(a, shape);
        (VALUES_COPIED.with(std::cell::Cell::get) - before, rs)
    }

    /// `$defs.a` is an array whose `enum` holds one
    /// 200,000-byte string and whose `items` is `a` again, so the facts at `x` carry that one set
    /// at the path's end and at every one of the 16 items levels below it. `compare` there is not
    /// feasible, so the refusal teaches `x` — which cannot fit 16 KiB, is measured through the
    /// borrowed view, elided, and never copied. Bites copying a path before measuring it: that
    /// clones the string at all 17 levels, 3.4 MB, to throw it away.
    #[test]
    fn a_teaching_copies_no_path_it_cannot_hold() {
        let shape = shape_of(json!({
            "$defs": { "a": {
                "type": "array",
                "enum": ["s".repeat(200_000)],
                "items": { "$ref": "#/$defs/a" }
            } },
            "properties": { "x": { "$ref": "#/$defs/a" } }
        }));
        let (copied, rs) =
            values_copied_validating(&ask(vec![at(&["x"], compare(json!(1)))]), &shape);
        assert_eq!(reasons(&rs), vec![RefusalReason::OperatorOutsideShape]);
        let teaches = rs[0].teaches.clone().expect("an operator refusal teaches");
        assert_eq!(teaches.vocabulary.map(|v| v.paths), Some(vec![]));
        assert_eq!(teaches.elided.map(|e| e.omitted_paths), Some(1));
        assert_eq!(copied, 0);
        // A path that fits is copied once: its two values, and nothing else.
        let small = shape_of(json!({ "properties": {
            "unit": { "type": "string", "enum": ["ms", "s"] }
        } }));
        let (copied, _) = values_copied_validating(
            &ask(vec![at(&["unit"], contains(vec![json!("fortnights")]))]),
            &small,
        );
        assert_eq!(copied, 2);
    }

    #[test]
    fn contains_where_the_branches_share_no_type_refuses_the_value() {
        let schema = wrapped(&json!({ "allOf": [{ "type": "string" }, { "type": "number" }] }));
        assert_eq!(verdict(&schema, contains(vec![json!("a")])), V);
        assert_eq!(verdict(&schema, compare(json!("a"))), O);
    }

    #[test]
    fn a_container_value_is_held_to_containment_by_an_enum_member() {
        let schema = wrapped(&json!({ "enum": [{ "k": 1, "z": 2 }, ["a", "b"]] }));
        assert_eq!(verdict(&schema, contains(vec![json!({ "z": 2 })])), A);
        assert_eq!(verdict(&schema, contains(vec![json!(["b"])])), A);
        assert_eq!(verdict(&schema, contains(vec![json!({ "k": 2 })])), V);
        assert_eq!(verdict(&schema, contains(vec![json!(["c"])])), V);
        // `{}` is held by the object member, and by no member of the other kind.
        assert_eq!(verdict(&schema, contains(vec![json!({})])), A);
        assert_eq!(
            verdict(
                &wrapped(&json!({ "enum": [["a"]] })),
                contains(vec![json!({})])
            ),
            V
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

    // ── The shape a validated ask carries ───────────────────────────────────────────────────

    #[test]
    fn a_validated_ask_carries_the_loaded_shapes_identity() {
        let v = validate_ask(&ask(vec![]), &shape_of(json!({}))).unwrap();
        assert_eq!(
            v.shape(),
            &ShapeIdentity {
                shape_id: ShapeId::from(Uuid::from_u128(7)),
                shape_version: 1,
                home_anchor_table: "kb_contexts".to_string(),
                home_anchor_id: Uuid::from_u128(1),
                kind_owner_table: "kb_teams".to_string(),
                kind_owner_id: Uuid::from_u128(2),
                artifact_kind: "measurement".to_string(),
            }
        );
    }

    /// The refusal is `not_askable`'s exactly — reason, sentence, nothing taught (§3.5: unreadable
    /// and absent look identical). Bites a mismatch-specific refusal: a sentence of its own would
    /// tell a loader defect from an absent shape.
    #[test]
    fn a_shape_other_than_the_one_the_ask_names_is_refused_not_checked() {
        let mut a = ask(vec![at(&["undeclared"], PropertyOp::HasKey)]);
        a.shape = ShapeId::from(Uuid::from_u128(8));
        let rs = refusals(&a, &shape_of(json!({})));
        assert_eq!(rs, vec![ShapeLoad::NotAskable.loaded().unwrap_err()]);
    }

    #[test]
    fn the_form_checks_run_alone_and_need_no_shape() {
        let mut a = ask(vec![
            at(&[], PropertyOp::HasKey),
            at(&["anything"], PropertyOp::HasKey),
        ]);
        a.intent = vec!["favourite".to_string()];
        // The undeclared-anywhere path is not judged: there is no shape to judge it by.
        assert_eq!(
            reasons(&validate_ask_form(&a)),
            vec![
                RefusalReason::UnknownArtifactIntent,
                RefusalReason::EmptyArtifactPath
            ]
        );
        assert!(validate_ask_form(&ask(vec![at(&["x"], PropertyOp::HasKey)])).is_empty());
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
                truncated: BTreeSet::new(),
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
                truncated: BTreeSet::from([BoundKind::SchemaBytes])
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

    /// A schema past the walk's byte bound is refused at the load with the compile step's reason,
    /// a sentence of its own naming the bound, and `teaches.beyond_bound: schema_bytes`. Bites a
    /// `SchemaTooLarge` arm that answers `Uncompilable`'s refusal, which would say the schema does
    /// not compile when it was never compiled.
    #[test]
    fn a_schema_past_the_byte_bound_is_refused_at_the_load_naming_the_bound() {
        let r = ShapeLoad::SchemaTooLarge.loaded().unwrap_err();
        assert_eq!(r.reason, RefusalReason::ShapeSchemaUncompilable);
        assert_ne!(r.detail, UNCOMPILABLE);
        assert!(
            r.detail.contains(&MAX_SCHEMA_BYTES.to_string()),
            "{}",
            r.detail
        );
        assert_eq!(
            r.teaches.and_then(|t| t.beyond_bound),
            Some(BoundKind::SchemaBytes)
        );
    }

    #[test]
    fn a_folded_shape_teaches_its_successor() {
        let successor = ShapeInForce {
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

    // ── The form checks `validate_ask_form` adds for the composable arm ───────────────────────

    /// `include_folded: true` is refused by name by the server — through the session, whatever the
    /// shape's load — and never by the form check an offline caller runs, because whether it is
    /// served is a capability of the server and not a fact about the ask's form. Beside an
    /// unaskable shape it accompanies `shape_not_askable`, as every shape-free refusal does, so an
    /// absent shape and an unreadable one still answer alike.
    ///
    /// Bites the server's check removed (`refuse_unserved` in `validate_loaded`): the ask validates,
    /// and the statement would answer over live artifacts while the asker named folded ones too.
    /// Bites the check moved back into `check_form`: the form check refuses it.
    #[test]
    fn include_folded_is_refused_by_the_server_and_never_by_the_form_check() {
        let mut a = ask(vec![at(&["p"], PropertyOp::HasKey)]);
        a.include_folded = true;
        assert!(
            validate_ask_form(&a).is_empty(),
            "a well-formed ask the server may one day serve"
        );
        let through_the_session = refusals(&a, &shape_of(json!({ "properties": { "p": {} } })));
        assert_eq!(
            reasons(&through_the_session),
            vec![RefusalReason::FilterNotApplicable]
        );
        assert_eq!(through_the_session[0].detail, INCLUDE_FOLDED);

        let unaskable = ShapeContext::empty()
            .session()
            .validate(&a)
            .expect_err("no shape is loaded");
        assert_eq!(
            reasons(&unaskable),
            vec![
                RefusalReason::ShapeNotAskable,
                RefusalReason::FilterNotApplicable
            ]
        );
    }

    /// U+0000 anywhere in a `contains` or `compare` value — a string, a string nested in an object
    /// or an array, an object key — is refused `unstorable_value` before the predicate is walked;
    /// the same value without it is not. Bites the NUL check removed: each validates, and the
    /// database refuses the bound value instead.
    #[test]
    fn a_nul_anywhere_in_a_value_is_refused_before_the_predicate_is_walked() {
        let shape = shape_of(json!({ "properties": { "p": {} } }));
        for value in [
            json!("ms\u{0}"),
            json!({ "unit": "m\u{0}s" }),
            json!([["a", "b\u{0}"]]),
            json!({ "k\u{0}": 1 }),
        ] {
            for op in [
                contains(vec![json!("fine"), value.clone()]),
                compare(value.clone()),
            ] {
                let a = ask(vec![at(&["p"], op)]);
                assert_eq!(
                    reasons(&validate_ask_form(&a)),
                    vec![RefusalReason::UnstorableValue],
                    "{value}"
                );
                assert_eq!(
                    reasons(&refusals(&a, &shape)),
                    vec![RefusalReason::UnstorableValue],
                    "{value}: refused, not walked"
                );
            }
        }
        let fine = ask(vec![at(&["p"], contains(vec![json!({ "unit": "ms" })]))]);
        assert!(validate_ask_form(&fine).is_empty());
        assert!(validate_ask(&fine, &shape).is_ok());
    }

    // ── What the SQL binds ───────────────────────────────────────────────────────────────────

    /// An ask with no predicates binds `"predicates": []` — every list written out, the defaults
    /// applied — where the caller's ask, serialized, carries no `predicates` key at all, which the
    /// SQL matcher reads as a match for nothing. Bites a bind serialized from the raw ask.
    #[test]
    fn an_ask_binds_every_list_explicitly_even_when_empty() {
        let raw = ask(vec![]);
        assert!(
            serde_json::to_value(&raw)
                .unwrap()
                .get("predicates")
                .is_none(),
            "the raw ask skips the empty list, which is why it is never what binds"
        );
        let bound = validate_ask(&raw, &shape_of(json!({}))).unwrap().bind();
        assert_eq!(
            bound,
            json!({
                "shape": Uuid::from_u128(7),
                "predicates": [],
                "intent": ["current", "member"],
                "include_folded": false,
                "shape_states": [
                    "declared_satisfied",
                    "declared_not_satisfied",
                    "declared_not_yet_checked"
                ]
            })
        );
    }

    /// What binds is the validated predicate: an array path's `contains` value arrives wrapped.
    #[test]
    fn an_ask_binds_its_validated_predicates_not_the_callers() {
        let shape = shape_of(json!({ "properties": {
            "rows": { "type": "array", "items": { "type": "object" } }
        } }));
        let a = ask(vec![at(&["rows"], contains(vec![json!({ "a": 1 })]))]);
        let bound = validate_ask(&a, &shape).unwrap().bind();
        assert_eq!(
            bound["predicates"],
            json!([{ "path": ["rows"], "op": { "op": "contains", "values": [[{ "a": 1 }]] } }])
        );
    }

    // ── A teaching enumerates borrowed, and copies only what it sends ────────────────────────

    /// The values a list of paths carries, at every items level.
    fn values_in(paths: &[AskablePath]) -> usize {
        fn of(facts: &PathFacts) -> usize {
            facts.values.as_ref().map_or(0, Vec::len) + facts.items.as_deref().map_or(0, of)
        }
        paths.iter().map(|p| of(&p.facts)).sum()
    }

    /// 200 paths, each over one shared `enum` of 40 small nested objects — the shape whose owned
    /// vocabulary copies every member into every path (8,000 values) to teach the few that fit.
    fn nested_object_enum() -> LoadedShape {
        let members: Vec<Value> = (0..40).map(|i| json!({ "k": { "v": i } })).collect();
        let props: serde_json::Map<String, Value> = (0..200)
            .map(|i| (format!("p{i:03}"), json!({ "$ref": "#/$defs/e" })))
            .collect();
        shape_of(json!({ "$defs": { "e": { "enum": members } }, "properties": props }))
    }

    /// A `path_outside_shape` refusal copies no more values than its teaching sends, however many
    /// the whole vocabulary declares; `omitted_paths` still counts the whole vocabulary. Bites the
    /// session's vocabulary taken from the owned enumeration: every path's values are then copied
    /// — the whole vocabulary, ten times what fits — and only the first few sent.
    #[test]
    fn a_path_refusal_copies_only_the_values_its_teaching_sends() {
        let shape = nested_object_enum();
        let whole = values_in(&enumerate(&shape.schema, &WalkBounds::BUILD).paths);
        assert_eq!(whole, 200 * 40);

        let (copied, rs) =
            values_copied_validating(&ask(vec![at(&["absent"], PropertyOp::HasKey)]), &shape);
        assert_eq!(reasons(&rs), vec![RefusalReason::PathOutsideShape]);
        let teaches = rs[0].teaches.clone().expect("a path refusal teaches");
        let kept = teaches.vocabulary.expect("the vocabulary").paths;
        let sent = values_in(&kept);
        assert!(sent > 0, "some paths fit 16 KiB");
        assert!(copied <= sent, "{copied} values copied, {sent} sent");
        assert!(
            copied < whole,
            "{copied} copied of the {whole} the shape declares"
        );
        assert_eq!(
            teaches.elided.map(|e| e.omitted_paths + kept.len()),
            Some(200),
            "the elision counts the whole vocabulary"
        );
    }

    /// On ordinary schemas — wherever the owned enumeration is not cut by what its copies cost — a
    /// path refusal teaches byte for byte what teaching from the owned vocabulary taught, which
    /// stays reachable here as the oracle. Bites any drift between the borrowed enumeration and the
    /// owned one: a path, a fact, an order or a cut that differs.
    #[test]
    fn a_path_refusal_teaches_byte_for_byte_what_the_owned_vocabulary_teaches() {
        let mut wide = serde_json::Map::new();
        for i in 0..200 {
            wide.insert(
                format!("{i:03}{}", "k".repeat(200)),
                json!({ "type": "string" }),
            );
        }
        let schemas = [
            json!({ "properties": {
                "unit": { "type": "string", "enum": ["ms", "s"] },
                "count": { "type": "integer" }
            } }),
            json!({ "properties": {
                "rows": { "type": "array", "items": {
                    "type": "object", "properties": { "x": { "type": "number" } }
                } },
                "meta": { "type": "object", "properties": {
                    "source": { "const": "sensor" },
                    "tags": { "type": "array", "items": { "enum": ["a", "b"] } }
                } }
            } }),
            json!({
                "$defs": { "code": { "enum": ["ms", 7, null, { "k": [1, 2.5] }] } },
                "allOf": [
                    { "properties": { "a": { "$ref": "#/$defs/code" } } },
                    { "properties": { "b": { "anyOf": [{ "type": "string" }, { "type": "null" }] } } }
                ]
            }),
            json!({ "properties": wide }),
            json!({
                "$defs": { "n": { "type": "object", "properties": {
                    "child": { "$ref": "#/$defs/n" },
                    "name": { "type": "string" }
                } } },
                "$ref": "#/$defs/n"
            }),
            nested_object_enum().schema,
            json!({ "properties": { "p": { "description": "x".repeat(MAX_SCHEMA_BYTES) } } }),
        ];
        for (i, schema) in schemas.into_iter().enumerate() {
            let shape = shape_of(schema);
            let rs = refusals(&ask(vec![at(&["absent"], PropertyOp::HasKey)]), &shape);
            assert_eq!(reasons(&rs), vec![RefusalReason::PathOutsideShape]);
            let taught = rs[0].teaches.as_deref().expect("a path refusal teaches");

            let owned = enumerate(&shape.schema, &WalkBounds::BUILD);
            let mut oracle = teaching(
                owned.paths.iter().collect(),
                AskablePath::clone,
                owned.truncated.clone(),
                shape.shape_id,
            );
            oracle.beyond_bound = taught.beyond_bound;
            assert_eq!(
                serde_json::to_vec(taught).unwrap(),
                serde_json::to_vec(&oracle).unwrap(),
                "schema {i}"
            );
        }
    }

    /// The one place the two differ, as ruled: where the owned vocabulary's copy charge is what
    /// cuts it at `work` — twenty paths over one 5,000-member enum, of which it affords about
    /// eleven — the borrowed one charges no copy, enumerates all twenty, and is not cut. Neither
    /// sends a path (one carries 5,000 values, past 16 KiB), so the teaching differs only in what
    /// it counts: `omitted_paths` is 20, and `truncated` is empty where the owned one said `work`.
    #[test]
    fn where_its_copies_cut_the_owned_vocabulary_a_teaching_counts_every_path() {
        let members: Vec<Value> = (0..5_000).map(|i| json!(i)).collect();
        let props: serde_json::Map<String, Value> = (0..20)
            .map(|i| (format!("p{i:02}"), json!({ "$ref": "#/$defs/e" })))
            .collect();
        let shape = shape_of(json!({ "$defs": { "e": { "enum": members } }, "properties": props }));

        let owned = enumerate(&shape.schema, &WalkBounds::BUILD);
        assert!(owned.paths.len() < 20, "{}", owned.paths.len());
        assert_eq!(owned.truncated, BTreeSet::from([BoundKind::Work]));

        let (copied, rs) =
            values_copied_validating(&ask(vec![at(&["absent"], PropertyOp::HasKey)]), &shape);
        let taught = rs[0].teaches.clone().expect("a path refusal teaches");
        assert_eq!(
            taught.vocabulary,
            Some(Vocabulary {
                paths: vec![],
                truncated: BTreeSet::new()
            })
        );
        assert_eq!(taught.elided.map(|e| e.omitted_paths), Some(20));
        assert_eq!(copied, 0, "no path fits, so none is copied");
    }
}
