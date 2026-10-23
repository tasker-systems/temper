//! The artifact asks a composition carries — bounded before any shape is loaded, checked for form
//! offline, and validated against the loaded shapes through one [`AskSession`] (spec
//! `2026-10-19-asking-artifacts-by-what-they-are` §3.4, §4.1, §4.3).
//!
//! **Three passes, one definition each.** The request caps are one function, read by the handler
//! that loads shapes ([`shapes_to_load`]), by the full validator and by the offline check, so a
//! request is refused for its size in the same words wherever it is measured. An ask's form is
//! `validate_ask_form`'s, which the offline check runs alone and [`AskSession::validate`] runs
//! first. Steps 1–4 of §3.5 are [`AskSession::validate`]'s alone; this module adds only what
//! needs more than one ask to decide — whether a stage's asks can share one home.
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
use crate::types::query::id_set::IdKind;
use crate::types::query::stage::{StageInput, StageName};

use super::{refusal, PlanRefusal};

/// The most artifact asks one request may carry, across every stage's `artifact_filter`. Refused
/// as `too_many_filter_values` before any shape is loaded.
///
/// # 32, sized against what a request's session keeps
///
/// One request's asks are validated through one session, which keeps every distinct ask it has
/// answered — keyed by the ask's JSON text, with its answer — so the gate's work answers the seal.
/// Each kept ask costs about its text twice (the key, and the validated predicates or the refusal
/// details) and at most three teachings, one per shape reason, each at most `MAX_TEACHING_BYTES`
/// (16 KiB). The asks' text is inside the composition's caller-text budget
/// (`MAX_COMPOSITION_TEXT_BYTES`, 768 KiB), so the memo holds at most about
/// 2 × 768 KiB + 32 × 3 × 16 KiB = 1.5 MiB + 1.5 MiB = 3 MiB, and a refusal list carrying every
/// ask's teaching is at most 1.5 MiB of them. A real composition asks one or two shapes from a few
/// stages; 32 is an order of magnitude above it.
pub const MAX_ASKS_PER_REQUEST: usize = 32;

/// The most distinct shapes one request's asks may name. Refused as `too_many_filter_values`
/// before any shape is loaded.
///
/// # 8, sized against what each shape costs
///
/// Every distinct shape is a gated read and a compile of up to `MAX_SCHEMA_BYTES` (256 KiB) before
/// the gate, a schema the request holds for its length, a walk built over it, and — when a
/// refusal teaches its vocabulary — an enumeration budget of its own, `MAX_WALK_VISITS`, beside
/// the one budget every ask shares. So the request's walk work is at most nine budgets, and the
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
    let shapes: BTreeSet<ShapeId> = every_ask(c).map(|(_, ask)| ask.shape).collect();
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

/// The offline half (`validate_shape`): the request caps, and — within them, over a graph that
/// sorts — each ask's form at its stage. No shape is consulted, so a well-formed ask naming a shape
/// nobody loaded is not refused here: the offline check cannot say, and says so where it renders.
pub(super) fn check_forms(c: &Composition, acyclic: bool, errs: &mut Vec<PlanRefusal>) {
    if within_request_caps(c, errs).is_none() || !acyclic {
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

/// The full half (`validate`): the request caps, then — within them, over a graph that sorts —
/// every ask through the request's one session, and whether each stage's admitted asks can share
/// a home. Returns each stage's validated asks in filter order, for the seal to hold; a stage
/// with a refused ask seals nothing, and the composition is refused anyway.
pub(super) fn validate_asks(
    c: &Composition,
    acyclic: bool,
    session: &mut AskSession<'_>,
    errs: &mut Vec<PlanRefusal>,
) -> BTreeMap<StageName, Vec<ValidatedAsk>> {
    let mut sealed = BTreeMap::new();
    if within_request_caps(c, errs).is_none() || !acyclic {
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
        let mut refused = false;
        for ask in &inv.artifact_filter {
            match session.validate(ask) {
                Ok(validated) => admitted.push(validated),
                Err(rs) => {
                    refused = true;
                    errs.extend(rs.into_iter().map(|r| at_stage(r, stage)));
                }
            }
        }
        check_scope(inv, stage, &admitted, errs);
        if !refused {
            sealed.insert(stage.clone(), admitted);
        }
    }
    sealed
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
        let anchor_table = match ids.kind {
            IdKind::Context => "kb_contexts",
            IdKind::Cogmap => "kb_cogmaps",
            _ => return None,
        };
        match ids.ids.as_slice() {
            [anchor_id] => Some(ShapeHome {
                anchor_table: anchor_table.to_string(),
                anchor_id: *anchor_id,
            }),
            _ => None,
        }
    })
}
