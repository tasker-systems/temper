//! The artifact asks a composition carries — bounded before any shape is loaded, checked for form
//! offline, and validated against the loaded shapes through one [`AskSession`] (spec
//! `2026-10-19-asking-artifacts-by-what-they-are` §3.4, §4.1, §4.3).
//!
//! **Three passes, one definition each.** The request caps are one function, read by the handler
//! that loads shapes ([`shapes_to_load`]), by the full validator and by the offline check, so a
//! request is refused for its size in the same words wherever it is measured. An ask's form is
//! `validate_ask_form`'s, which the offline check runs alone and [`AskSession::validate`] runs
//! first. Steps 1–4 of §3.5 are [`AskSession::validate`]'s alone; this module adds only what
//! needs more than one ask to decide — whether a stage repeats an ask, and whether its asks can
//! share one home.
//!
//! **Its refusals are both kinds, and say which.** The caps and the form need no shape and no
//! declaration, so the offline check raises them. Whether an ask lies inside its shape, and
//! whether a stage's asks share a home, need the loaded shapes, so only the full validator does.

use std::collections::{BTreeMap, BTreeSet};

use crate::types::artifact_ask::{
    validate_ask_form, ArtifactAsk, AskSession, RefusalTeaching, ShapeHome, ValidatedAsk,
};
use crate::types::ids::ShapeId;
use crate::types::query::composition::{Composition, StageNode};
use crate::types::query::disposition::RefusalReason;
use crate::types::query::envelope::ActInvocation;
use crate::types::query::stage::{StageInput, StageName};

use super::{refusal, PlanRefusal};

/// The most artifact asks one request may carry, across every stage's `artifact_filter`. Refused
/// as `too_many_filter_values` before any shape is loaded.
///
/// # 32, sized against what a request's session keeps and what its statement runs
///
/// One request's asks are validated through one session, which keeps every distinct ask it has
/// answered — keyed by the ask's JSON text, with its answer — so the gate's work answers the seal.
/// Each kept ask costs about its JSON twice (the key, and the validated predicates or the refusal
/// details) and at most three teachings, one per shape reason, each at most `MAX_TEACHING_BYTES`
/// (16 KiB).
///
/// **Two checks run before an ask is kept, and the bound is theirs.**
/// - The composition's budgets: [`super::validate`] reads no ask into the session over a
///   `text_budget_exceeded` or a `property_value_budget_exceeded`. So the caller text of every ask
///   kept — path segments, terms, values — is within `MAX_COMPOSITION_TEXT_BYTES` (768 KiB) between
///   them.
/// - The ask's own count and size caps: `AskSession::validate` keys no ask past them, answering it
///   from its form instead. So a kept ask has at most `MAX_ASK_PREDICATES` (32) predicates, each
///   path at most `MAX_ARTIFACT_PATH_DEPTH` (32) segments, at most `MAX_ASK_PROBES` (256) values,
///   and at most three intent and three state terms. Text that counts nothing — an empty segment or
///   term — is bounded by those counts, not by the budget, and the keys and punctuation around it
///   are under 8 KiB an ask.
///
/// The memo therefore holds at most about 2 × (768 KiB + 32 × 8 KiB) + 32 × 3 × 16 KiB =
/// 2 MiB + 1.5 MiB = 3.5 MiB, and a refusal list carrying every ask's teaching is at most 1.5 MiB
/// of them.
///
/// **The cap is also the statement's multiplier**: each ask is two calls of the counted set
/// (`__temper_ungated_artifact_ask_population`, `20261104000000`), each reading its shape's family
/// in its home — one in the selection's asks conjunct, one in the ask's population arm
/// (`__temper_ungated_artifact_ask_counts`, `20261105000000`) — plus one live-row lookup of its
/// shape, gated by the home check. So a request runs at most 64 counted-set reads and 32 lookups.
/// Within one filter a repeated ask is refused (`duplicate_set_member`), including one that differs
/// only in term order or in a default spelled out, since the validated bind is canonical in its
/// terms and defaults (not in predicate or value order, which the SQL also reads order-free); the
/// same ask on two stages is four reads, inside the cap. A real composition asks one or two shapes
/// from a few stages; 32 is an order of magnitude above it.
pub const MAX_ASKS_PER_REQUEST: usize = 32;

/// The most distinct shapes one request's asks may name. Refused as `too_many_filter_values`
/// before any shape is loaded.
///
/// # 8, sized against what each shape costs
///
/// Every distinct shape is a gated read and a compile of up to `MAX_SCHEMA_BYTES` (256 KiB) before
/// the gate, a schema the request holds for its length, a walk built over it, and — when a
/// refusal teaches its vocabulary and no teaching of it is remembered — an enumeration budget of
/// its own, `MAX_WALK_VISITS`, beside the one budget every ask shares. The teaching a request makes
/// is remembered by shape id for later requests (temper-services' `shape_loader`), so each shape is
/// enumerated once per process while its teaching stays remembered. So the request's walk work is
/// at most nine budgets — one when every shape it names has its teaching remembered — and the
/// schemas it holds at most 2 MiB of compact JSON. A resource has one home, so a stage's asks name
/// one home's shapes; asking several homes means several stages combined by `union`, and eight
/// shapes is far above what one question names.
pub const MAX_ASKED_SHAPES: usize = 8;

/// The shapes a composition's asks name — the ids the handler loads before the gate — or the
/// refusals that decline the request before anything is loaded: more asks than
/// [`MAX_ASKS_PER_REQUEST`], or more distinct shapes than [`MAX_ASKED_SHAPES`].
///
/// Pure, and the same check [`super::validate`] makes, so a handler that loads nothing on a refusal
/// and validates against an empty context hands the caller these refusals beside every other, in
/// one body.
pub fn shapes_to_load(c: &Composition) -> Result<BTreeSet<ShapeId>, Vec<PlanRefusal>> {
    let mut errs = Vec::new();
    match within_request_caps(c, &mut errs) {
        Some(shapes) => Ok(shapes),
        None => Err(errs),
    }
}

/// Every distinct shape a composition's asks name, whatever the request caps say — what a caller
/// that cannot load shapes reports as not checked (`temper query --check`). [`shapes_to_load`] is
/// this set within the caps.
pub fn asked_shape_ids(c: &Composition) -> BTreeSet<ShapeId> {
    every_ask(c).map(|(_, ask)| ask.shape).collect()
}

/// Every ask on an act stage, with the stage it rides on.
fn every_ask(c: &Composition) -> impl Iterator<Item = (&StageName, &ArtifactAsk)> {
    c.stages.iter().flat_map(|node| {
        let asks: &[ArtifactAsk] = match node {
            StageNode::Act(inv) => &inv.artifact_filter,
            StageNode::Combine(_) => &[],
        };
        asks.iter().map(move |ask| (node.name(), ask))
    })
}

/// The request caps: the distinct shapes named, or `None` with the refusals pushed. Counted, never
/// clamped, and before anything is read — so an over-cap request loads nothing and walks nothing.
pub(super) fn within_request_caps(
    c: &Composition,
    errs: &mut Vec<PlanRefusal>,
) -> Option<BTreeSet<ShapeId>> {
    let asks = every_ask(c).count();
    let shapes = asked_shape_ids(c);
    if asks > MAX_ASKS_PER_REQUEST {
        errs.push(refusal(
            None,
            RefusalReason::TooManyFilterValues,
            format!(
                "a composition's artifact filters may carry at most {MAX_ASKS_PER_REQUEST} asks \
                 between them; this one carries {asks}. Each is validated against its shape \
                 before anything runs — ask fewer, or split the composition"
            ),
        ));
    }
    if shapes.len() > MAX_ASKED_SHAPES {
        errs.push(refusal(
            None,
            RefusalReason::TooManyFilterValues,
            format!(
                "a composition's asks may name at most {MAX_ASKED_SHAPES} distinct shapes; this \
                 one names {}. Each is loaded and read before anything runs — ask fewer shapes, or \
                 split the composition",
                shapes.len()
            ),
        ));
    }
    (asks <= MAX_ASKS_PER_REQUEST && shapes.len() <= MAX_ASKED_SHAPES).then_some(shapes)
}

/// The offline half (`validate_shape`): the request caps, and — within them, when `walk` says the
/// asks may be read — each ask's form at its stage. No shape is consulted, so a well-formed ask
/// naming a shape nobody loaded is not refused here: the offline check cannot say, and says so
/// where it renders.
///
/// `walk` is the condition [`validate_asks`] takes — a graph that sorts, inside the composition's
/// budgets — so over a budget the offline check and the server refuse the same plan with the same
/// ask refusals: the caps, and nothing per ask.
pub(super) fn check_forms(c: &Composition, walk: bool, errs: &mut Vec<PlanRefusal>) {
    if within_request_caps(c, errs).is_none() || !walk {
        return;
    }
    for (stage, ask) in every_ask(c) {
        errs.extend(
            validate_ask_form(ask)
                .into_iter()
                .map(|r| at_stage(r, stage)),
        );
    }
}

/// The full half (`validate`): the request caps, then — within them, and only when `walk` says the
/// asks may be walked — every ask through the request's one session, whether a stage repeats an
/// ask, and whether each stage's admitted asks can share a home. Returns each stage's validated
/// asks in filter order, for the seal to hold; a stage with a refused ask seals nothing, and the
/// composition is refused anyway.
///
/// `walk` is false over a graph that does not sort, and over a composition the shape pass refused
/// for a composition-wide budget: that request is refused whatever its asks say, and an ask
/// refusal is not actionable until the plan is shrunk, so no ask is read into the session.
pub(super) fn validate_asks(
    c: &Composition,
    walk: bool,
    session: &mut AskSession<'_>,
    errs: &mut Vec<PlanRefusal>,
) -> BTreeMap<StageName, Vec<ValidatedAsk>> {
    let mut sealed = BTreeMap::new();
    if within_request_caps(c, errs).is_none() || !walk {
        return sealed;
    }
    for node in &c.stages {
        let StageNode::Act(inv) = node else {
            continue;
        };
        if inv.artifact_filter.is_empty() {
            continue;
        }
        let stage = node.name();
        let mut admitted = Vec::with_capacity(inv.artifact_filter.len());
        // Each admitted ask's position in the stage's `artifact_filter`, for a refusal to name.
        let mut positions = Vec::with_capacity(inv.artifact_filter.len());
        let mut refused = false;
        for (position, ask) in inv.artifact_filter.iter().enumerate() {
            match session.validate(ask) {
                Ok(validated) => {
                    admitted.push(validated);
                    positions.push(position);
                }
                Err(rs) => {
                    refused = true;
                    errs.extend(rs.into_iter().map(|r| at_stage(r, stage)));
                }
            }
        }
        refused |= repeats_an_ask(stage, &admitted, &positions, errs);
        check_scope(inv, stage, &admitted, errs);
        if !refused {
            sealed.insert(stage.clone(), admitted);
        }
    }
    sealed
}

/// `duplicate_set_member` when a stage's filter asks one question twice, over the asks it admitted.
///
/// **Equality is the validated bind** ([`ValidatedAsk::bind`]), not the caller's text: the bind is
/// what the statement runs, so two asks with one bind are one ask's counted-set calls made
/// twice. The bind is canonical in its terms and defaults — terms in vocabulary order, defaults applied — so it
/// also catches asks that differ only in term order or in a default spelled out, which the caller's
/// text would call different. Predicates and `contains` values keep the caller's order, so asks
/// differing only there are not caught; the request cap bounds them. AND-ing an ask with itself narrows nothing, so the repeat is refused rather than
/// collapsed: a caller who sent it has a plan that says something other than what they meant.
/// Refused once per stage, naming the first repeat by its positions in `artifact_filter`
/// (`positions[i]` is admitted ask `i`'s).
fn repeats_an_ask(
    stage: &StageName,
    admitted: &[ValidatedAsk],
    positions: &[usize],
    errs: &mut Vec<PlanRefusal>,
) -> bool {
    let binds: Vec<(usize, serde_json::Value)> = positions
        .iter()
        .copied()
        .zip(admitted.iter().map(ValidatedAsk::bind))
        .collect();
    let repeat = (1..binds.len()).find_map(|later| {
        let (at, bind) = &binds[later];
        binds[..later]
            .iter()
            .find(|(_, earlier)| earlier == bind)
            .map(|(first, _)| (*first, *at))
    });
    let Some((first, later)) = repeat else {
        return false;
    };
    errs.push(refusal(
        Some(stage),
        RefusalReason::DuplicateSetMember,
        format!(
            "this stage's `artifact_filter` asks the same question twice (`artifact_filter[{first}]` \
             and `artifact_filter[{later}]` once validated); asks AND together, so a repeat narrows \
             nothing and runs the same read again — ask it once"
        ),
    ));
    true
}

/// A refusal from the ask's own validation, attached to the stage carrying it.
fn at_stage(mut r: PlanRefusal, stage: &StageName) -> PlanRefusal {
    r.stage = Some(stage.clone());
    r
}

/// `ask_scope_conflict` (§4.1, *Scope*), over the asks a stage admitted: every shape named must be
/// homed in one home, and that home must be the stage's anchor when it has one. A resource has
/// exactly one home, so either conflict is an empty stage decided before it runs. An unanchored
/// stage takes its shapes' home; a stage bounded by a resource set is not decided here, since
/// where those resources live is the database's to say.
///
/// `teaches.homes` names the shapes' homes, every one readable to the asker — each shape passed
/// refusal step 1 — and never the stage's anchor, which the caller wrote.
fn check_scope(
    inv: &ActInvocation,
    stage: &StageName,
    admitted: &[ValidatedAsk],
    errs: &mut Vec<PlanRefusal>,
) {
    let mut homes: Vec<ShapeHome> = Vec::new();
    for ask in admitted {
        let home = ask.shape().home();
        if !homes.contains(&home) {
            homes.push(home);
        }
    }
    let conflict = if homes.len() > 1 {
        Some(format!(
            "this stage's asks name shapes homed in {} different homes, and they all hold of one \
             resource, which has one home — so no resource can satisfy them. Ask each home in a \
             stage of its own and combine the stages with `union`",
            homes.len()
        ))
    } else {
        anchor_of(inv)
            .filter(|anchor| homes.first().is_some_and(|home| home != anchor))
            .map(|_| {
                "this stage is anchored to a home that is not the asked shape's, and an ask ranges \
                 over its shape's home only — so no resource can satisfy it. Anchor the stage to \
                 the shape's home, or leave it unanchored and it takes that home"
                    .to_string()
            })
    };
    if let Some(detail) = conflict {
        let mut r = refusal(Some(stage), RefusalReason::AskScopeConflict, detail);
        r.teaches = Some(Box::new(RefusalTeaching {
            homes,
            ..RefusalTeaching::default()
        }));
        errs.push(r);
    }
}

/// The home a stage is anchored to: a caller-supplied context or cogmap set of one id, served by
/// the fragments' `(anchor_table, anchor_id)` pair whichever relation carries it. A set of another
/// size is refused `anchor_takes_one_id` elsewhere and anchors nothing here.
fn anchor_of(inv: &ActInvocation) -> Option<ShapeHome> {
    inv.inputs.iter().find_map(|input| {
        let StageInput::Caller { ids, .. } = input else {
            return None;
        };
        let anchor_table = ids.kind.anchor_table()?;
        match ids.ids.as_slice() {
            [anchor_id] => Some(ShapeHome {
                anchor_table: anchor_table.to_string(),
                anchor_id: *anchor_id,
            }),
            _ => None,
        }
    })
}
