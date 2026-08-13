//! The composition envelope: an ordered list of stages plus the things that ride alongside them.
//!
//! The PRINCIPAL is deliberately absent. Visibility applies inside each act's execution — one
//! known application point per stage — and jaq reshapes what visibility admitted without ever
//! seeing the credential. There is no field here for it, by construction.

use serde::{Deserialize, Serialize};

use super::envelope::ActInvocation;
use super::stage::StageName;
use crate::types::resource_view::ResourceSection;

/// One find act's question: its text, and the caller's vector when there is one.
///
/// `[2026-08-12]` This line read *"computed once at composition start and threaded to every
/// stage"* — the envelope-placement claim spec ⟨7⟩ retired. It is corrected rather than left
/// standing because **this doc comment IS a published schema description**
/// (`tests/fixtures/query/intention.schema.json`), so a stale sentence here is a lie shipped to
/// every client that reads the contract.
///
/// **Its absence refuses, and that is about the QUESTION, not the vector.** A find stage with no
/// intention has no words to search for, so it comes back `MissingIntention`. That refusal is
/// forced rather than chosen: `find-exact` sources its query *text* from here — it becomes
/// `p_query` — and there is nowhere else to get it.
///
/// An absent EMBEDDING is a different absence and does **not** refuse. The CLI can embed; the ruby
/// gem, the TypeScript package and MCP structurally cannot, so refusing a vector search for want
/// of a precomputed vector would deny this surface to every non-CLI client. The server embeds when
/// none arrives, exactly as `/api/search` already does, and only a FAILED embed refuses — as
/// [`super::disposition::RefusalReason::EmbeddingUnavailable`], the one runtime refusal in the
/// contract. `[decided — 2026-08-08, Pete]`
/// **This is a per-STAGE field, carried by [`super::envelope::ActInvocation`].** `[decided —
/// 2026-08-12, Pete]`, spec ⟨7⟩. It sat on the composition envelope until then, which meant a
/// composition could ask exactly ONE question: every find stage in a DAG interrogated the same
/// string, and *"find A, find B, intersect them"* was inexpressible. That placement was never
/// ruled — it entered as a first-person commit paragraph and hardened into a test name.
///
/// **No `Eq`, only `PartialEq`** — [`Self::embedding`] holds `f32`. Same reason
/// [`super::envelope::StageResult`] derives neither, one derive milder: equality on a vector of
/// floats is well-defined enough for a test, total equality is not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct Intention {
    pub query: String,
    /// The query vector, when the caller computed one. Mirrors `SearchParams.embedding`: the CLI
    /// links temper-ingest and embeds locally, which is faster than making the server do it; the
    /// ruby gem, the TypeScript package and MCP structurally cannot, so the server embeds on their
    /// behalf and its absence is not a refusal.
    ///
    /// **It rides beside the text it was computed FROM, and that pairing is the point.** At
    /// composition level a vector and its query could drift apart; here they cannot.
    ///
    /// This never reaches a response: [`super::trace::CompositionTrace`] carries only `stages` and
    /// echoes no intention. Should a trace ever carry one, that stops being incidental and becomes
    /// a constraint — a 768-float array must not serialize back to the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
}

/// One stage whose rows come back, and how much of each row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ReturnSpec {
    pub stage: StageName,
    /// Which sections to hydrate onto each row, in the SAME vocabulary `temper resource show
    /// --with` uses. Empty means the kind's default projection.
    ///
    /// Replaces `fields: Vec<String>`, which promised field-level subselection over a projection,
    /// had nothing implementing it, and duplicated a vocabulary that already works.
    ///
    /// **This door admits a subset, and the rest are REFUSED rather than unsupported** — see
    /// [`Self::ADMITTED_SECTIONS`]. The refusal lands at validation rather than at deserialization
    /// on purpose: `/api/query` promises every refusal in one response, and a serde failure
    /// short-circuits before validation runs, so a caller with four problems would learn about one
    /// of them in a deserializer's vocabulary.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub with: Vec<ResourceSection>,
}

impl ReturnSpec {
    /// The hydration sections `/api/query` offers, and therefore the only ones it accepts.
    ///
    /// Per-door subsetting of one shared vocabulary, exactly as [`ResourceSection::LIST`] already
    /// does for `list` — not a second enum. Adding a section later is a change to this const.
    ///
    /// [`ResourceSection::Body`] is refused rather than merely absent. `fill_sections` records
    /// that the body arm "is an N+1 by construction" and that what keeps it honest is "the door,
    /// not the loop"; `/api/query` is a new door and is narrow from its first line. Ask `show`.
    ///
    /// [`ResourceSection::Edges`] is refused because edge listing has its own commands, and
    /// `follow-from` plus an `EdgeFilter` walk and filter on edges without returning them.
    pub const ADMITTED_SECTIONS: [ResourceSection; 1] = [ResourceSection::OpenMeta];
}

/// Which stages come back, and how much of each row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct OutcomeDeclaration {
    // There is deliberately no `description`. It was a required prose string — "a composition's
    // pocket outcome register" — which is goal-authoring discipline leaked into a wire contract.
    // Nobody should have to write a sentence about what being served looks like in order to run a
    // query. The register discipline belongs to goals, which are resources, not to request bodies.
    /// The stages whose rows are hydrated and returned. DECLARED, not inferred from graph shape:
    /// inferring from out-degree zero makes returning an intermediate impossible without a dummy
    /// consumer, and means adding a downstream stage silently stops returning what you used to get
    /// back. The composition's produced kind(s) are DERIVED from these, replacing the old single
    /// `produces` field which could only ever be right for a one-arm plan.
    pub returns: Vec<ReturnSpec>,
}

/// A set combinator's operation. `union` and `intersect` take two-or-more inputs; no act does,
/// which is why a combinator is its own node kind rather than an act invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum CombineOp {
    Union,
    Intersect,
}

/// A set combination over two-or-more upstream stages. Its own node kind because no act takes more
/// than one input, so modelling it as an act would lie about what an act is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct CombineNode {
    pub name: StageName,
    pub op: CombineOp,
    /// Two or more. One input is not a combination; validation refuses it (beat B).
    pub inputs: Vec<StageName>,
}

/// A node in the composition DAG: an act invocation, or a set combination over other nodes.
///
/// **No `Eq`, only `PartialEq`** — the act variant carries an intention, which carries the query
/// vector. See [`Intention`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
// Untagged so a plan's JSON reads naturally, with no synthetic node-kind discriminator. The two
// variants are unambiguous: a `CombineNode` carries `op`, an `ActInvocation` carries `act`.
#[serde(untagged)]
#[expect(
    clippy::large_enum_variant,
    reason = "Act is the overwhelmingly common node; boxing it to shave the rare Combine variant \
              would add a heap indirection on the hot path and force Box::new at every \
              construction and match site downstream, for no gain at a handful-of-nodes-per-\
              composition scale. The size asymmetry is inherent to ActInvocation carrying the \
              full per-act envelope, and the wire representation is identical either way."
)]
pub enum StageNode {
    Act(ActInvocation),
    Combine(CombineNode),
}

impl StageNode {
    pub fn name(&self) -> &StageName {
        match self {
            StageNode::Act(a) => &a.name,
            StageNode::Combine(c) => &c.name,
        }
    }

    /// Every upstream stage name this node reads. Empty for a caller-fed or root act.
    pub fn upstream_names(&self) -> Vec<&StageName> {
        match self {
            StageNode::Act(a) => match &a.input {
                Some(super::stage::StageInput::Upstream { stage, .. }) => vec![stage],
                _ => vec![],
            },
            StageNode::Combine(c) => c.inputs.iter().collect(),
        }
    }
}

/// A composition, declared before execution.
///
/// **No `Eq`, only `PartialEq`** — it transitively holds the query vector. See [`Intention`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct Composition {
    pub outcome: OutcomeDeclaration,
    // There is deliberately no `intention` here — it moved onto each `ActInvocation`
    // `[decided — 2026-08-12, Pete]`, spec ⟨7⟩. On the envelope it was computed once and threaded,
    // which made "every find stage asks the same question" structural — and made asking TWO
    // questions in one DAG impossible. That trade is now taken the other way: two stages may ask
    // different things, and each stage's question is declared where the stage is.
    //
    // Removed under the same remove-and-tolerate precedent as `on_stage_refusal` and `bounds`: an
    // envelope-level `intention` key on an old payload is ignored like any unknown field, pinned by
    // the legacy-payload test below. It must NOT come back as a DEFAULT a stage inherits when it
    // declares none — that is tasker-grammar's prev-else-context fallback under another name, and
    // this contract exists partly to refuse it.
    // There is deliberately no `on_stage_refusal`. It was a required
    // `RefusalDisposition {halt | degrade_and_disclose}` describing a case that cannot occur:
    // every `RefusalReason` bar one is decidable statically, so a composition that could refuse
    // never runs at all — it comes back 400 with all of its refusals.
    //
    // The single runtime refusal, `EmbeddingUnavailable`, does not want a caller-declared
    // disposition either. Its two settings are observationally near-identical (both answer 200
    // with a full trace; the only difference is whether downstream stages run against empty sets),
    // and it is not per-stage — the intention is computed ONCE and threaded, so an embedding
    // failure fails every find-about-* stage at the same instant. A per-composition disposition
    // over a per-stage refusal is machinery for a shape that does not exist.
    //
    // ONE BEHAVIOUR: the refused stage reports `refused` with no rows, every other stage runs, and
    // a stage downstream of a refusal receives an EMPTY set — never an absent one. Empty is
    // bounded-to-nothing; absent is unbounded, and collapsing them turns a failed stage into a
    // global search wearing a full page of plausible results. That distinction needs no new
    // mechanism: the fragments already read `p_bound_ids = '{}'` as zero rows and
    // `p_bound_ids IS NULL` as unbounded.
    //
    // If a later runtime refusal genuinely wants a choice, the field returns as an OPTIONAL
    // addition, which is additive rather than breaking. `[decided — 2026-08-08, Pete]`
    //
    // There is also no `meta_detail`. It selected how much per-resource meta the trace would
    // retain — a metadata-budget concept whose job nobody could state (YAGNI); nothing ever
    // honoured it. Removed by ADJ-4 `[2026-08-10, Pete]`.
    //
    // And no `bounds`. A composition cannot meaningfully have bounds: its output is nothing but
    // the returned stages' outputs, each already bounded by its own `terms`, and a cross-stage
    // budget would be a different, undesigned feature. Removed by ADJ-4 `[2026-08-10, Pete]`, per
    // the `on_stage_refusal` remove-and-tolerate precedent: an unknown `bounds` key is ignored
    // like any unknown field — pinned by the legacy-payload test below.
    /// The DAG's nodes. Each references its inputs explicitly by stage name — there is no
    /// prev-else-fallback, and no single execution order (a DAG has none). Beat B's topological
    /// sort derives the order; there is deliberately no `act_sequence` method, which would be a
    /// false claim that a DAG has one sequence.
    pub stages: Vec<StageNode>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::query::act::ActName;
    use crate::types::query::envelope::ActInvocation;
    use crate::types::query::stage::{StageInput, StageRelation};
    use std::collections::BTreeMap;

    /// A minimal root act node named `s`, carrying no question.
    fn stage(act: ActName) -> StageNode {
        StageNode::Act(ActInvocation {
            name: StageName::parse("s").unwrap(),
            act,
            intention: None,
            input: None,
            terms: BTreeMap::new(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })
    }

    /// [`stage`] carrying a question. These are serialization tests, so they never run `validate`
    /// and a questionless find act is representable here — which is exactly what
    /// `an_absent_intention_is_representable_so_a_stage_can_refuse_rather_than_substitute` asserts.
    fn stage_asking(act: ActName, query: &str) -> StageNode {
        match stage(act) {
            StageNode::Act(mut inv) => {
                inv.intention = Some(Intention {
                    query: query.to_string(),
                    embedding: None,
                });
                StageNode::Act(inv)
            }
            other => other,
        }
    }

    fn outcome(returns: Vec<ReturnSpec>) -> OutcomeDeclaration {
        OutcomeDeclaration { returns }
    }

    fn composition(stages: Vec<StageNode>) -> Composition {
        Composition {
            outcome: outcome(vec![]),
            stages,
        }
    }

    #[test]
    fn a_composition_no_longer_declares_what_to_do_when_a_stage_refuses() {
        // `on_stage_refusal` described a case that cannot occur: every refusal but one is static,
        // so a composition that could refuse comes back 400 and never runs. The one runtime
        // refusal is composition-wide (the intention is embedded ONCE), so a per-stage disposition
        // would be machinery for a shape that does not exist.
        //
        // Asserting on the SERIALIZED form rather than the absence of a field: the field being
        // gone is a compile-time fact this test cannot restate, but the wire not carrying it is
        // what a client actually observes.
        let c = composition(vec![stage(ActName::FindExact)]);
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.contains("on_stage_refusal"), "got: {json}");
        assert!(!json.contains("halt"));
        assert_eq!(serde_json::from_str::<Composition>(&json).unwrap(), c);
    }

    #[test]
    fn a_composition_that_still_carries_on_stage_refusal_is_accepted_and_the_field_ignored() {
        // Removing a required field is a breaking change for anyone who was sending it. Nothing
        // ships against this contract yet — no route exists — so the removal is free, and serde's
        // default is to ignore unknown fields rather than reject them. Pinned so that if someone
        // later adds `deny_unknown_fields` for good reasons, they see this decision rather than
        // silently turning an ignored legacy field into a hard 400.
        let legacy = r#"{"outcome":{"returns":[]},"on_stage_refusal":"halt","stages":[]}"#;
        assert!(serde_json::from_str::<Composition>(legacy).is_ok());
    }

    #[test]
    fn a_composition_that_still_carries_bounds_or_meta_detail_is_accepted_and_both_ignored() {
        // Same precedent as `on_stage_refusal` above: `bounds` and `meta_detail` were removed by
        // ADJ-4 `[2026-08-10, Pete]`, nothing ships against this contract yet, and serde's default
        // is to ignore unknown fields rather than reject them. Pinned so that a later
        // `deny_unknown_fields` sees this decision rather than silently turning an ignored legacy
        // field into a hard 400 — and so the removal is visibly remove-and-tolerate, not a parse
        // hazard.
        let legacy = r#"{"outcome":{"returns":[]},"meta_detail":"surviving","bounds":{"limit":10},"stages":[]}"#;
        let parsed = serde_json::from_str::<Composition>(legacy).expect("legacy fields parse");
        assert_eq!(
            parsed,
            composition(vec![]),
            "and neither influences anything"
        );
    }

    #[test]
    fn the_intention_is_a_per_stage_field_and_two_stages_may_ask_different_questions() {
        // `[inverted — 2026-08-12]` This asserted the OPPOSITE, under the name
        // `the_intention_is_a_composition_level_field_not_a_per_stage_one`, on the rationale
        // "computed ONCE at composition start and threaded, so every find-about-* stage provably
        // interrogates the same intention rather than re-embedding a mutated string."
        //
        // That property is GIVEN UP deliberately — spec ⟨7⟩, `[decided — 2026-08-12, Pete]` — and
        // not lost. It made "every find stage asks the same question" structural, and made asking
        // two questions in one DAG impossible, so `find A, find B, intersect them` could not be
        // expressed at all. What replaces it is per-stage declaration: each stage's question sits
        // where the stage is.
        //
        // Kept as an inversion rather than deleted because the old name is what hardened: it was
        // greppable, it read as a settled invariant, and it steered three sessions of planning. A
        // test asserting the opposite is what stops it being re-derived.
        let c = composition(vec![
            stage_asking(ActName::FindAboutAnywhere, "wayfind salience"),
            stage_asking(ActName::FindAboutAnywhere, "region scoring"),
        ]);
        let json = serde_json::to_string(&c).unwrap();
        // TWO intentions, one per stage — and none on the envelope.
        assert_eq!(json.matches("\"intention\"").count(), 2);
        assert!(json.contains("wayfind salience") && json.contains("region scoring"));
        assert_eq!(serde_json::from_str::<Composition>(&json).unwrap(), c);
    }

    #[test]
    fn an_absent_intention_is_representable_so_a_stage_can_refuse_rather_than_substitute() {
        // The absence that refuses is the QUESTION's, not the vector's. With no intention there is
        // no query text, and `find-exact` has nowhere else to get its `p_query` — so the stage
        // refuses `MissingIntention`. An absent EMBEDDING is a different absence entirely: the
        // server computes one, because API callers cannot.
        //
        // Now asserted PER STAGE: the refusal follows the stage that omitted its question, not the
        // composition that failed to thread one.
        let c = composition(vec![stage(ActName::FindExact)]);
        let StageNode::Act(inv) = &c.stages[0] else {
            panic!("act node");
        };
        assert!(inv.intention.is_none());
        assert!(!serde_json::to_string(&c).unwrap().contains("intention"));
    }

    #[test]
    fn an_intention_carries_the_query_and_the_callers_vector_and_asserts_nothing_about_it() {
        // `[rewritten — 2026-08-12]` Was `an_intention_carries_the_fact_of_embedding_and_never_the
        // _vector`, pinning a byte-exact `{"query":…,"embedded":false}`. Both halves are retired.
        //
        // The VECTOR is here now (spec ⟨7⟩): it rides beside the text it was computed from, so the
        // two cannot drift apart. The old rationale — "putting it in the envelope would be a wire
        // contract nobody asked for" — was about the ENVELOPE and about response bloat, and
        // `CompositionTrace` carries only `stages` and echoes no intention, so nothing here reaches
        // a response.
        //
        // And `embedded: bool` is GONE `[decided — 2026-08-13, Pete]`. It claimed to make
        // paraphrase-stability "measurable from outside" while never appearing in any trace; its
        // only live distinction — your vector versus the server's — is already covered on the
        // failing side by `EmbeddingUnavailable`; and the real hazard it looked like it addressed
        // (a CLI embedding with a different model than the corpus) is guarded by build.rs's model
        // sha256 pin, which a boolean could not express anyway.
        let bare = Intention {
            query: "composable search fragments".to_string(),
            embedding: None,
        };
        // An absent vector serializes to nothing at all — a caller that cannot embed sends a
        // question and no key, exactly as the ruby gem and MCP do.
        assert_eq!(
            serde_json::to_string(&bare).unwrap(),
            r#"{"query":"composable search fragments"}"#
        );

        let carried = Intention {
            query: "composable search fragments".to_string(),
            embedding: Some(vec![0.25, -0.5]),
        };
        assert_eq!(
            serde_json::to_string(&carried).unwrap(),
            r#"{"query":"composable search fragments","embedding":[0.25,-0.5]}"#
        );
        assert_eq!(
            serde_json::from_str::<Intention>(&serde_json::to_string(&carried).unwrap()).unwrap(),
            carried
        );
    }

    #[test]
    fn a_combinator_is_its_own_node_kind_because_no_act_takes_two_inputs() {
        let c = CombineNode {
            name: StageName::parse("both").unwrap(),
            op: CombineOp::Union,
            inputs: vec![
                StageName::parse("quoted").unwrap(),
                StageName::parse("wide").unwrap(),
            ],
        };
        let node = StageNode::Combine(c.clone());
        assert_eq!(node.name(), &c.name);
        assert_eq!(node.upstream_names().len(), 2);
        assert_eq!(
            serde_json::from_str::<StageNode>(&serde_json::to_string(&node).unwrap()).unwrap(),
            node
        );
    }

    #[test]
    fn an_act_node_reports_its_single_upstream_and_a_caller_fed_one_reports_none() {
        let seeded = StageNode::Act(ActInvocation {
            name: StageName::parse("near").unwrap(),
            intention: None,
            input: Some(StageInput::Upstream {
                relation: StageRelation::Seed,
                stage: StageName::parse("hits").unwrap(),
            }),
            terms: BTreeMap::new(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
            act: ActName::FollowFrom,
        });
        assert_eq!(seeded.upstream_names().len(), 1);

        let rooted = StageNode::Act(ActInvocation {
            name: StageName::parse("hits").unwrap(),
            intention: None,
            input: None,
            terms: BTreeMap::new(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
            act: ActName::FindExact,
        });
        assert!(rooted.upstream_names().is_empty());
    }

    #[test]
    fn a_composition_carries_nodes_and_no_longer_claims_a_single_sequence() {
        // `act_sequence()` is gone on purpose: a DAG has no one order, and a method returning one
        // would be a false claim that reads as true. Beat B's topological order replaces it.
        assert!(composition(vec![]).stages.is_empty());
    }

    #[test]
    fn an_outcome_declares_which_stages_come_back_and_nothing_about_why() {
        // `description` is gone. It was a required prose string, which is goal-authoring
        // discipline leaked into a wire contract — nobody should write a sentence about what
        // being served looks like in order to run a query.
        let o = outcome(vec![ReturnSpec {
            stage: StageName::parse("near").unwrap(),
            with: vec![],
        }]);
        let json = serde_json::to_string(&o).unwrap();
        assert_eq!(o.returns.len(), 1);
        assert!(!json.contains("description"), "got: {json}");
        assert_eq!(
            serde_json::from_str::<OutcomeDeclaration>(&json).unwrap(),
            o
        );
    }

    #[test]
    fn an_empty_section_list_means_the_default_projection_and_serializes_to_nothing() {
        let r = ReturnSpec {
            stage: StageName::parse("near").unwrap(),
            with: vec![],
        };
        assert!(!serde_json::to_string(&r).unwrap().contains("with"));
    }

    #[test]
    fn hydration_sections_ride_the_wire_in_the_same_words_show_and_list_use() {
        // ONE vocabulary, not a query-local copy. `open-meta` is kebab here because it is kebab
        // everywhere a human or agent types it; a second spelling would be a second vocabulary
        // wearing the first one's name.
        let r = ReturnSpec {
            stage: StageName::parse("near").unwrap(),
            with: vec![ResourceSection::OpenMeta],
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""with":["open-meta"]"#), "got: {json}");
        assert_eq!(serde_json::from_str::<ReturnSpec>(&json).unwrap(), r);
    }

    #[test]
    fn a_section_this_door_refuses_still_deserializes_so_validation_can_refuse_it() {
        // The load-bearing half of choosing ONE vocabulary over a narrow query-local enum. Were
        // `with` a single-member enum, `body` would fail to DESERIALIZE — and serde
        // short-circuits before validation runs, so a caller with four problems would learn about
        // one, phrased by a deserializer rather than by this contract.
        //
        // So `body` parses here and is refused at validation, which is what lets it come back
        // alongside every other refusal with an explanation that names `show`.
        let parsed: ReturnSpec =
            serde_json::from_str(r#"{"stage":"near","with":["body","edges"]}"#).unwrap();
        assert_eq!(
            parsed.with,
            vec![ResourceSection::Body, ResourceSection::Edges]
        );
        assert!(!ReturnSpec::ADMITTED_SECTIONS.contains(&ResourceSection::Body));
        assert!(!ReturnSpec::ADMITTED_SECTIONS.contains(&ResourceSection::Edges));
    }

    #[test]
    fn the_admitted_sections_are_a_subset_of_the_shared_vocabulary_never_a_parallel_one() {
        // The guard against this door quietly growing a word `show` and `list` do not know. If a
        // section is ever added here, it has to be added to `ResourceSection` first.
        for section in ReturnSpec::ADMITTED_SECTIONS {
            assert!(
                ResourceSection::ALL.contains(&section),
                "{section:?} is not part of the shared section vocabulary"
            );
        }
    }

    #[test]
    fn a_composition_no_longer_declares_one_produced_kind() {
        // A resource arm beside a region arm has no single answer. `produces` was a field that
        // could only ever be right for a single-arm plan — it is derived from `returns` now, not
        // declared.
        assert!(!serde_json::to_string(&outcome(vec![]))
            .unwrap()
            .contains("produces"));
    }
}
