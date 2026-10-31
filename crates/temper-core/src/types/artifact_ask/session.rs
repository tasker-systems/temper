//! One request's asks, validated against one [`ShapeContext`] — the walk of each shape built once,
//! and one membership budget for every ask (spec §2.4, §3.3).
//!
//! A request can carry many asks: a composition's stages each carry an artifact filter, and its
//! gate and its seal both validate them (a batch read carries one). Validated one at a time, each ask
//! would rebuild its shape's walk, re-walk paths another ask already named, re-enumerate the
//! vocabulary it teaches, and spend a budget of its own — so the request's cost would grow with
//! how many asks it carries, which no single ask's bound holds. An [`AskSession`] holds what a
//! request has read of each shape, what it has answered, and what it may still spend, so every ask
//! after the first reuses the first's work, an ask asked again is answered as before, and every ask
//! draws on one budget.

use std::collections::{BTreeMap, HashMap};

use super::validate::{
    exceeds_form_caps, not_askable, refuse_unserved, validate_ask_form, validate_loaded,
    ValidatedAsk,
};
use super::walk::ShapeWalk;
use super::{ArtifactAsk, LoadedShape, ShapeContext, MAX_MEMBERSHIP_STEPS};
use crate::types::ids::ShapeId;
use crate::types::query::PlanRefusal;

/// The validation of one request's asks against the shapes its [`ShapeContext`] loaded.
///
/// Begun with [`ShapeContext::session`]; [`AskSession::validate`] is the only way an ask becomes a
/// [`ValidatedAsk`]. **One session per request, held for all of it:** a composition's gate and its
/// seal validate through the same session, and a batch read validates its one ask through one,
/// on every page. A second session
/// over the same context builds every walk again and starts a fresh budget, which is exactly the
/// multiplication a session exists to prevent.
///
/// **What it keeps, per shape:** the walk — the schema measured and read whole — built on the first
/// ask that needs it; every path any ask has resolved; and the shape's
/// [`PathTeaching`](super::PathTeaching), what every path refusal on it teaches beside its own
/// bound. Paths hold the walk's facts borrowed from the schema, so a session copies none of the
/// values the schema declares; the teaching holds only the paths it sends, at most
/// `MAX_TEACHING_BYTES` of them. A shape row is immutable per id (an amendment mints a new id,
/// §2.2), so none of it can go stale within a request.
///
/// **Where the teaching comes from.** The first path refusal on a shape enumerates its vocabulary,
/// makes the teaching from it and keeps it for every later refusal on that shape in the request.
/// Nothing of it outlives the session: each request that refuses a path on a shape enumerates that
/// shape once.
///
/// **What it answers again:** an ask equal to one the session has already validated gets that
/// answer back, byte for byte, and spends nothing. So a composition's seal, re-validating the asks
/// its gate admitted, cannot refuse them for work the gate already paid — whatever the gate spent.
///
/// **What it spends, in all:** one shared budget of [`MAX_MEMBERSHIP_STEPS`] membership steps,
/// drawn on by every ask's `contains` values; past it a predicate is refused `path_outside_shape`
/// with `teaches.beyond_bound: work`, never admitted unchecked. The walk itself needs no budget: a
/// field tree is read node by node, so resolving a path reads the schema along it, and a path
/// already resolved costs nothing to name again. **Plus one enumeration per distinct shape** whose
/// vocabulary a refusal teaches, linear in that shape's schema. So the request's walk work grows
/// with the number of distinct shapes it names, which the caller bounds — a composition by
/// `MAX_ASKED_SHAPES`, decided before any shape is loaded.
///
/// **Why it borrows the context rather than living in it.** A walk borrows the schema it reads, so
/// a cache of walks inside the struct that owns the schemas would be self-referential. The context
/// owns the loaded shapes and is plain data; the session borrows it for the request's length.
/// `Send`, so it can be held across the request's `.await`s.
#[derive(Debug)]
pub struct AskSession<'c> {
    context: &'c ShapeContext,
    shapes: BTreeMap<ShapeId, ShapeWalk<'c>>,
    budget: RequestBudget,
    /// Each ask validated, keyed by its JSON text, with its answer. serde_json writes an
    /// `ArtifactAsk` losslessly, so equal text is an equal ask; two equal asks written differently
    /// (object keys in another order) only miss, and are answered afresh.
    answered: HashMap<Vec<u8>, Result<ValidatedAsk, Vec<PlanRefusal>>>,
    /// How many asks were answered from `answered` rather than validated afresh.
    answered_again: usize,
}

/// What one request may still spend reading shapes: the membership steps.
#[derive(Debug)]
pub(super) struct RequestBudget {
    pub(super) membership_steps: usize,
}

impl RequestBudget {
    fn new() -> Self {
        RequestBudget {
            membership_steps: MAX_MEMBERSHIP_STEPS,
        }
    }
}

impl<'c> AskSession<'c> {
    fn new(context: &'c ShapeContext) -> AskSession<'c> {
        AskSession {
            context,
            shapes: BTreeMap::new(),
            budget: RequestBudget::new(),
            answered: HashMap::new(),
            answered_again: 0,
        }
    }

    /// Refusal steps 1–4 of §3.5 for one ask: the load the context holds for the shape it names,
    /// then path, operator and value against that shape — every refusal at once.
    ///
    /// A load that refuses (steps 1–3) is reported beside the ask's shape-free refusals and
    /// nothing else: only refusals that never consulted the shape accompany it (§3.5), so an
    /// absent shape and an unreadable one answer identically, refusal for refusal. A shape the
    /// context was not given, or a load filed under another shape's id, answers what an absent
    /// shape does: an ask is never checked against a schema it did not name.
    ///
    /// An ask this session has validated before is answered as it was then, spending nothing.
    ///
    /// **An ask past its form's count or size caps is neither kept nor walked.** It is refused by
    /// its form — beside its load's refusal, if the load refuses — before it is keyed, so what the
    /// session keeps is an ask within the caps, and a caller cannot make the memo hold a list of any
    /// length by sending one. It is answered afresh each time it is asked, by its form (and its load's
    /// refusal, if the load refuses): its lists' lengths are compared before any is read, and nothing
    /// it builds is kept.
    pub fn validate(&mut self, ask: &ArtifactAsk) -> Result<ValidatedAsk, Vec<PlanRefusal>> {
        if exceeds_form_caps(ask) {
            return Err(match self.load(ask) {
                Ok(_) => {
                    let mut errs = validate_ask_form(ask);
                    refuse_unserved(ask, &mut errs);
                    errs
                }
                Err(refusal) => beside_the_form(refusal, ask),
            });
        }
        let key = serde_json::to_vec(ask).ok();
        if let Some(answer) = key.as_ref().and_then(|key| self.answered.get(key)).cloned() {
            self.answered_again += 1;
            return answer;
        }
        let answer = self.answer(ask);
        if let Some(key) = key {
            self.answered.insert(key, answer.clone());
        }
        answer
    }

    /// Steps 1–3 for an ask: the shape it names as the context loaded it, or the refusal its load
    /// decided. A shape the context was not given, or a load filed under another shape's id,
    /// answers what an absent shape does.
    fn load(&self, ask: &ArtifactAsk) -> Result<&'c LoadedShape, PlanRefusal> {
        let context: &'c ShapeContext = self.context;
        match context.get(&ask.shape).map(|load| load.loaded()) {
            Some(Ok(shape)) if shape.shape_id == ask.shape => Ok(shape),
            Some(Ok(_)) | None => Err(not_askable()),
            Some(Err(refusal)) => Err(refusal),
        }
    }

    /// Steps 1–4 for an ask the session has not answered before.
    fn answer(&mut self, ask: &ArtifactAsk) -> Result<ValidatedAsk, Vec<PlanRefusal>> {
        let shape = match self.load(ask) {
            Ok(shape) => shape,
            Err(refusal) => return Err(beside_the_form(refusal, ask)),
        };
        let walk = self
            .shapes
            .entry(shape.shape_id)
            .or_insert_with(|| ShapeWalk::new(shape.shape_id, &shape.schema));
        validate_loaded(ask, shape, walk, &mut self.budget)
    }

    /// How many asks this session answered from what it had already validated, spending nothing —
    /// a composition's seal re-validating the asks its gate admitted. A caller reports it beside
    /// [`AskSession::shapes_walked`], and a test reads it to see that the gate and the seal shared
    /// this session: a seal through any other session leaves it at zero.
    pub fn answered_again(&self) -> usize {
        self.answered_again
    }

    /// How many shapes this session has built a walk of — at most one per distinct shape its asks
    /// named, whatever number of asks named each.
    pub fn shapes_walked(&self) -> usize {
        self.shapes.len()
    }

    /// How many shapes' vocabularies this session enumerated to teach a path refusal — at most one
    /// per distinct shape. A caller reports it beside [`AskSession::shapes_walked`].
    pub fn vocabularies_enumerated(&self) -> usize {
        self.shapes
            .values()
            .filter(|walk| walk.enumerated())
            .count()
    }

    /// The membership steps the request has left.
    #[cfg(test)]
    fn steps_left(&self) -> usize {
        self.budget.membership_steps
    }
}

/// A steps 1–3 refusal, followed by the ask's shape-free refusals: its form, and what it asks that
/// this server does not serve.
fn beside_the_form(refusal: PlanRefusal, ask: &ArtifactAsk) -> Vec<PlanRefusal> {
    let mut errs = vec![refusal];
    errs.extend(validate_ask_form(ask));
    refuse_unserved(ask, &mut errs);
    errs
}

impl ShapeContext {
    /// Begin validating one request's asks against this context (see [`AskSession`]).
    pub fn session(&self) -> AskSession<'_> {
        AskSession::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::artifact_ask::walk::{
        paths_reached, BoundKind, VOCABULARIES_ENUMERATED, WALKS_BUILT,
    };
    use crate::types::artifact_ask::{
        ArtifactPredicate, LoadedShape, ShapeLoad, ShapeSuccessor, ARTIFACT_INTENTS,
        ASKABLE_SHAPE_STATES, MAX_ASK_PREDICATES,
    };
    use crate::types::query::{PropertyOp, RefusalReason};
    use serde_json::{json, Value};
    use std::cell::Cell;
    use std::thread::LocalKey;
    use uuid::Uuid;

    const SHAPE: u128 = 7;

    fn shape_of(schema: Value) -> LoadedShape {
        LoadedShape {
            shape_id: ShapeId::from(Uuid::from_u128(SHAPE)),
            shape_version: 1,
            home_anchor_table: "kb_contexts".to_string(),
            home_anchor_id: Uuid::from_u128(1),
            kind_owner_table: "kb_teams".to_string(),
            kind_owner_id: Uuid::from_u128(2),
            artifact_kind: "measurement".to_string(),
            schema,
        }
    }

    fn context_of(shape: LoadedShape) -> ShapeContext {
        [(shape.shape_id, ShapeLoad::Loaded(shape))]
            .into_iter()
            .collect()
    }

    fn ask_of(paths: &[&str], op: PropertyOp) -> ArtifactAsk {
        ArtifactAsk {
            shape: ShapeId::from(Uuid::from_u128(SHAPE)),
            predicates: paths
                .iter()
                .map(|p| ArtifactPredicate {
                    path: vec![p.to_string()],
                    op: op.clone(),
                })
                .collect(),
            intent: vec![],
            include_folded: false,
            shape_states: vec![],
        }
    }

    fn reasons(rs: &[PlanRefusal]) -> Vec<RefusalReason> {
        rs.iter().map(|r| r.reason.clone()).collect()
    }

    fn beyond(rs: &[PlanRefusal]) -> Option<BoundKind> {
        rs.first()
            .and_then(|r| r.teaches.as_ref())
            .and_then(|t| t.beyond_bound)
    }

    /// How far a thread-local counter moved while `run` ran.
    fn counted<T>(counter: &'static LocalKey<Cell<usize>>, run: impl FnOnce() -> T) -> (usize, T) {
        let before = counter.with(Cell::get);
        let out = run();
        (counter.with(Cell::get) - before, out)
    }

    /// **An ask past its form's caps is refused by them, and neither walked nor kept.** Three asks
    /// whose lists carry no text — a long `intent` of empty terms, a long `shape_states` of empty
    /// terms, and one predicate whose path is a long run of empty segments — are each refused by the
    /// cap their list breaks (`too_many_filter_values`), build no walk, and are not answered from the
    /// memo when asked again: nothing was kept. A within-caps ask through the same session is kept,
    /// the control that the memo is reachable at all.
    ///
    /// Bites the cap check moved after the key (or removed from `validate`): each ask is keyed,
    /// and asked again is answered from the memo (`answered_again` above 0), and the shape enters
    /// the session's walks (`shapes_walked` 1).
    #[test]
    fn an_ask_past_its_form_caps_is_refused_without_being_walked_or_kept() {
        let context = context_of(shape_of(json!({ "properties": { "unit": {} } })));
        let mut session = context.session();
        const LONG: usize = 10_000;
        let empty_intents = ArtifactAsk {
            intent: vec![String::new(); LONG],
            ..ask_of(&["unit"], PropertyOp::HasKey)
        };
        let empty_states = ArtifactAsk {
            shape_states: vec![String::new(); LONG],
            ..ask_of(&["unit"], PropertyOp::HasKey)
        };
        let mut empty_segments = ask_of(&["unit"], PropertyOp::HasKey);
        empty_segments.predicates[0].path = vec![String::new(); LONG];
        for over in [empty_intents, empty_states, empty_segments] {
            for pass in 0..2 {
                let (built, rs) = counted(&WALKS_BUILT, || session.validate(&over));
                let rs = rs.expect_err("past a cap");
                assert!(
                    reasons(&rs).contains(&RefusalReason::TooManyFilterValues),
                    "pass {pass}: {rs:?}"
                );
                assert_eq!(built, 0, "pass {pass}: no walk");
            }
        }
        assert_eq!(session.answered_again(), 0, "nothing over a cap was kept");
        assert_eq!(session.shapes_walked(), 0);

        let within = ask_of(&["unit"], PropertyOp::HasKey);
        session.validate(&within).expect("within the caps");
        session.validate(&within).expect("answered again");
        assert_eq!(
            session.answered_again(),
            1,
            "the control: a within-caps ask is kept"
        );
    }

    /// Terms are bound in vocabulary order, whatever order the caller named them in, so two asks
    /// differing only in that order validate to one bind. Bites the caller's order kept.
    #[test]
    fn terms_bind_in_vocabulary_order() {
        let context = context_of(shape_of(json!({ "properties": { "unit": {} } })));
        let mut session = context.session();
        let named = |intent: [&str; 2], states: [&str; 2]| ArtifactAsk {
            intent: intent.iter().map(|t| t.to_string()).collect(),
            shape_states: states.iter().map(|t| t.to_string()).collect(),
            ..ask_of(&["unit"], PropertyOp::HasKey)
        };
        let forward = session
            .validate(&named(
                ["current", "pinned"],
                ["declared_satisfied", "declared_not_yet_checked"],
            ))
            .expect("known terms");
        let backward = session
            .validate(&named(
                ["pinned", "current"],
                ["declared_not_yet_checked", "declared_satisfied"],
            ))
            .expect("known terms");
        assert_eq!(forward.intent(), &["current", "pinned"]);
        assert_eq!(
            backward.shape_states(),
            &["declared_satisfied", "declared_not_yet_checked"]
        );
        assert_eq!(forward.bind(), backward.bind());
    }

    /// A shape of `MAX_ASK_PREDICATES` string properties, `p0` onwards.
    fn wide() -> LoadedShape {
        let props: serde_json::Map<String, Value> = (0..MAX_ASK_PREDICATES)
            .map(|i| (format!("p{i}"), json!({ "type": "string" })))
            .collect();
        shape_of(json!({ "properties": props }))
    }

    /// An ask naming the wide shape's paths `p{from}` .. `p{from + count - 1}`.
    fn wide_paths(from: usize, count: usize) -> ArtifactAsk {
        let names: Vec<String> = (from..from + count).map(|i| format!("p{i}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        ask_of(&names, PropertyOp::HasKey)
    }

    /// The `n`th variation of an ask (`1 <= n < 64`), naming its own subset of the intents and of
    /// the shape states: asks that differ from each other and from `ask` (variation 0), and walk
    /// exactly what `ask` walks — so a session answers none of them from its memo of earlier asks.
    fn varied(mut ask: ArtifactAsk, n: usize) -> ArtifactAsk {
        let pick = |terms: &[&str], bits: usize| -> Vec<String> {
            terms
                .iter()
                .enumerate()
                .filter(|(i, _)| ((bits >> i) & 1) == 1)
                .map(|(_, t)| t.to_string())
                .collect()
        };
        ask.intent = pick(&ARTIFACT_INTENTS, n & 7);
        ask.shape_states = pick(&ASKABLE_SHAPE_STATES, (n >> 3) & 7);
        ask
    }

    /// A shape whose one path `n` carries a 5,000-member enum of the integers `0..5,000`.
    fn late_enum() -> LoadedShape {
        let members: Vec<Value> = (0..5_000).map(|i| json!(i)).collect();
        shape_of(json!({ "properties": { "n": { "enum": members } } }))
    }

    /// An ask checking 64 late members of [`late_enum`]: window 0 is `4,936..5,000`, window 1
    /// `4,872..4,936`, and so on. A probe `v` costs `v + 1` membership steps, so the windows cost
    /// 317,984, 313,888, 309,792 and 305,696 steps.
    fn late_window(w: usize) -> ArtifactAsk {
        let end = 5_000 - 64 * w;
        ask_of(
            &["n"],
            PropertyOp::Contains {
                values: (end - 64..end).map(|i| json!(i)).collect(),
            },
        )
    }

    // ── One walk per shape per request ──────────────────────────────────────────────────────

    /// Many asks against one shape, through one session, cost about one walk: the first ask
    /// builds the walk, resolves its paths and enumerates the vocabulary its refusal teaches; the
    /// 62 after it — each a different ask, so none is answered from the session's memo of asks —
    /// build nothing, enumerate nothing and reach no path again, because every path they name was
    /// resolved by the first. Bites the session's caches: build the shape's walk per ask (a fresh
    /// `ShapeWalk` in `AskSession::answer`) and the walk is built once per ask, the
    /// vocabulary enumerated once per refused ask, and every ask reaches its paths again.
    #[test]
    fn many_asks_against_one_shape_cost_about_one_walk() {
        const ASKS: usize = 62;
        let context = context_of(wide());
        let admitted = wide_paths(0, 2);
        let refused = {
            let mut a = wide_paths(0, 2);
            a.predicates.push(ArtifactPredicate {
                path: vec!["undeclared".to_string()],
                op: PropertyOp::HasKey,
            });
            a
        };
        let mut session = context.session();
        let (built, (enumerated, first)) = counted(&WALKS_BUILT, || {
            counted(&VOCABULARIES_ENUMERATED, || session.validate(&refused))
        });
        let first = first.expect_err("the undeclared path is refused");
        assert_eq!(reasons(&first), vec![RefusalReason::PathOutsideShape]);
        assert_eq!(
            (built, enumerated),
            (1, 1),
            "the first ask builds and enumerates"
        );
        let after_first = paths_reached();

        let (built, (enumerated, outcomes)) = counted(&WALKS_BUILT, || {
            counted(&VOCABULARIES_ENUMERATED, || {
                (0..ASKS)
                    .map(|i| {
                        let a = if i % 2 == 0 { &refused } else { &admitted };
                        session.validate(&varied(a.clone(), i + 1))
                    })
                    .collect::<Vec<_>>()
            })
        });

        assert_eq!(
            built, 0,
            "the walk was built by the first ask and never again"
        );
        assert_eq!(
            enumerated, 0,
            "the vocabulary was enumerated by the first ask"
        );
        assert_eq!(
            paths_reached(),
            after_first,
            "every path the later asks name was resolved by the first"
        );
        for (i, outcome) in outcomes.iter().enumerate() {
            if i % 2 == 0 {
                let rs = outcome
                    .as_ref()
                    .expect_err("the undeclared path is refused");
                assert_eq!(
                    reasons(rs),
                    vec![RefusalReason::PathOutsideShape],
                    "ask {i}"
                );
                assert_eq!(beyond(rs), None, "ask {i} met no bound");
            } else {
                assert!(outcome.is_ok(), "ask {i}: {outcome:?}");
            }
        }
    }

    /// The first ask of a session builds the walk it needs, and nothing is built for an ask refused
    /// before its shape is consulted. Pins that the per-shape walk is lazy, so the caps refusal
    /// stays as cheap as D made it.
    #[test]
    fn a_walk_is_built_once_and_only_when_an_ask_needs_it() {
        let context = context_of(shape_of(json!({ "properties": { "p": {} } })));
        let mut session = context.session();
        let over_the_caps = ask_of(&["p"; MAX_ASK_PREDICATES + 1], PropertyOp::HasKey);
        let (built, rs) = counted(&WALKS_BUILT, || session.validate(&over_the_caps));
        assert_eq!(
            reasons(&rs.unwrap_err()),
            vec![RefusalReason::FilterNotApplicable]
        );
        assert_eq!(built, 0, "an ask over the caps builds no walk");
        let (built, ok) = counted(&WALKS_BUILT, || {
            session.validate(&ask_of(&["p"], PropertyOp::HasKey))
        });
        assert!(ok.is_ok());
        assert_eq!(built, 1);
    }

    // ── One budget per request ──────────────────────────────────────────────────────────────

    /// The membership steps are the request's too. Four distinct asks, each checking a window of
    /// 64 late members of a 5,000-member enum: each fits alone, three fit together (941,664 of
    /// 1,048,576 steps), and the fourth passes the budget part-way and is refused. Bites a per-ask
    /// membership budget (`RequestBudget::new()` per call in `AskSession::answer`, or a fresh
    /// `MAX_MEMBERSHIP_STEPS` in `sort`): every ask then fits.
    #[test]
    fn membership_steps_are_one_budget_for_the_request() {
        let context = context_of(late_enum());
        let asks: Vec<ArtifactAsk> = (0..4).map(late_window).collect();
        for (k, a) in asks.iter().enumerate() {
            assert!(
                context.session().validate(a).is_ok(),
                "ask {k} fits a request of its own"
            );
        }
        let mut session = context.session();
        for (k, a) in asks.iter().take(3).enumerate() {
            assert!(
                session.validate(a).is_ok(),
                "ask {k} fits beside the earlier ones"
            );
        }
        let rs = session
            .validate(&asks[3])
            .expect_err("the fourth passes the request's membership steps");
        assert_eq!(reasons(&rs), vec![RefusalReason::PathOutsideShape]);
        assert_eq!(beyond(&rs), Some(BoundKind::Work));
    }

    // ── Asked again, answered the same ─────────────────────────────────────────────────────

    /// Validating asks a session has already validated answers what it answered then, byte for
    /// byte, and spends nothing — so a composition's seal, re-validating its gate's asks, cannot
    /// refuse what the gate admitted. Two admitted asks spend 631,872 membership steps, past half
    /// the request's 1,048,576, so a second pass that spent them again would meet `work` at the
    /// second; a third ask is refused, so a refusal is pinned to repeat too. Bites the session's
    /// answer memo (in `AskSession::validate`, drop the `answered` lookup and call `self.answer`
    /// directly): the second pass refuses the second ask for work, and the steps fall.
    #[test]
    fn validating_the_same_asks_again_spends_nothing_and_answers_the_same() {
        let context = context_of(late_enum());
        let asks = [
            late_window(0),
            late_window(1),
            ask_of(&["undeclared"], PropertyOp::HasKey),
        ];
        let mut session = context.session();
        let first: Vec<_> = asks.iter().map(|a| session.validate(a)).collect();
        assert!(
            first[0].is_ok() && first[1].is_ok(),
            "both windows fit once"
        );
        assert!(first[2].is_err(), "the undeclared path is refused");
        let (reached, steps) = (paths_reached(), session.steps_left());

        let second: Vec<_> = asks.iter().map(|a| session.validate(a)).collect();
        assert!(second == first, "the second pass answers as the first did");
        let wire = |answers: &[Result<ValidatedAsk, Vec<PlanRefusal>>]| {
            answers
                .iter()
                .map(|a| a.as_ref().err().map(|rs| serde_json::to_vec(rs).unwrap()))
                .collect::<Vec<_>>()
        };
        assert_eq!(wire(&second), wire(&first));
        assert_eq!(
            (paths_reached(), session.steps_left()),
            (reached, steps),
            "the second pass spends nothing"
        );
    }

    // ── Steps 1–3, through the session ──────────────────────────────────────────────────────

    /// An absent shape (one the context was not given), an unreadable one (the loader's
    /// `NotAskable`), and a load filed under another shape's id answer the same refusals, byte for
    /// byte: the step 1 refusal, then the ask's shape-free refusals and nothing that consulted a
    /// shape. Bites a session that tells them apart, or walks a shape for an ask that did not name
    /// it.
    #[test]
    fn absent_unreadable_and_misfiled_shapes_answer_identically() {
        let mut a = ask_of(&["anything"], PropertyOp::HasKey);
        a.intent = vec!["favourite".to_string()];
        let absent = ShapeContext::default().session().validate(&a).unwrap_err();
        let unreadable: ShapeContext = [(a.shape, ShapeLoad::NotAskable)].into_iter().collect();
        let unreadable = unreadable.session().validate(&a).unwrap_err();
        let mut elsewhere = shape_of(json!({ "properties": { "anything": {} } }));
        elsewhere.shape_id = ShapeId::from(Uuid::from_u128(SHAPE + 1));
        let misfiled: ShapeContext = [(a.shape, ShapeLoad::Loaded(elsewhere))]
            .into_iter()
            .collect();
        let misfiled = misfiled.session().validate(&a).unwrap_err();

        assert_eq!(
            reasons(&absent),
            vec![
                RefusalReason::ShapeNotAskable,
                RefusalReason::UnknownArtifactIntent
            ]
        );
        let wire = |rs: &[PlanRefusal]| serde_json::to_vec(rs).unwrap();
        assert_eq!(wire(&absent), wire(&unreadable));
        assert_eq!(wire(&absent), wire(&misfiled));
    }

    /// A folded shape, an uncompilable one and one past the byte bound refuse through the session
    /// with their load's refusal, beside the ask's shape-free refusals, and walk nothing.
    #[test]
    fn a_refusing_load_is_reported_beside_the_form_and_walks_nothing() {
        let a = {
            let mut a = ask_of(&[], PropertyOp::HasKey);
            a.predicates.push(ArtifactPredicate {
                path: vec![],
                op: PropertyOp::HasKey,
            });
            a
        };
        let successor = ShapeSuccessor {
            shape_id: ShapeId::from(Uuid::from_u128(SHAPE + 1)),
            shape_version: 2,
        };
        for (load, reason) in [
            (
                ShapeLoad::Superseded {
                    successor: Some(successor),
                },
                RefusalReason::ShapeSuperseded,
            ),
            (
                ShapeLoad::Uncompilable,
                RefusalReason::ShapeSchemaUncompilable,
            ),
            (
                ShapeLoad::SchemaTooLarge,
                RefusalReason::ShapeSchemaUncompilable,
            ),
        ] {
            let expected = load.loaded().unwrap_err();
            let context: ShapeContext = [(a.shape, load)].into_iter().collect();
            let (built, rs) = counted(&WALKS_BUILT, || context.session().validate(&a));
            let rs = rs.unwrap_err();
            assert_eq!(reasons(&rs), vec![reason, RefusalReason::EmptyArtifactPath]);
            assert_eq!(rs[0], expected);
            assert_eq!(built, 0);
        }
    }

    /// `prepare` holds the session across the embed's `.await`; a `!Send` session would make every
    /// handler's future `!Send`. Bites a value set held in an `Rc` rather than an `Arc` (this does
    /// not compile).
    #[test]
    fn an_ask_session_can_be_held_across_an_await() {
        fn assert_send<T: Send>() {}
        assert_send::<AskSession<'static>>();
    }
}
