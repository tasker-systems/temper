//! The composition envelope: an ordered list of stages plus the things that ride alongside them.
//!
//! The PRINCIPAL is deliberately absent. Visibility applies inside each act's execution — one
//! known application point per stage — and jaq reshapes what visibility admitted without ever
//! seeing the credential. There is no field here for it, by construction.

use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use super::disposition::RefusalDisposition;
use super::envelope::ActInvocation;
use super::scalars::{BoundTerm, MetaDetail};
use super::stage::StageName;

/// The question, computed once at composition start and threaded to every stage.
///
/// Its ABSENCE is meaningful: a `find-about-*` stage with no intention refuses, rather than the
/// server embedding on the caller's behalf. That is what makes "I chose not to embed" and
/// "I cannot embed" different states instead of one ambiguous one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct Intention {
    pub query: String,
    /// Whether an embedding was computed for it. Inspectable in the trace, which is what makes
    /// paraphrase-stability measurable from outside.
    pub embedded: bool,
}

/// One stage whose rows come back, and how much of each row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ReturnSpec {
    pub stage: StageName,
    /// Empty means the kind's default projection. Named fields subselect it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
}

/// A composition's pocket outcome register: what it is for, and which stages come back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct OutcomeDeclaration {
    /// What being served looks like. NOT optional.
    pub description: String,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
                Some(super::stage::StageInput::Upstream { stage }) => vec![stage],
                _ => vec![],
            },
            StageNode::Combine(c) => c.inputs.iter().collect(),
        }
    }
}

/// A composition, declared before execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct Composition {
    pub outcome: OutcomeDeclaration,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intention: Option<Intention>,
    /// What happens when a stage refuses. Declared, never improvised.
    pub on_stage_refusal: RefusalDisposition,
    #[serde(default)]
    pub meta_detail: MetaDetail,
    /// The SECOND bound layer: over the composition's own output, distinct from the act-level
    /// terms on each stage. A composition never carries a total — with each stage's output the
    /// next stage's domain, a full-composition total is not well-defined.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bounds: BTreeMap<BoundTerm, i64>,
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
    use crate::types::query::disposition::RefusalDisposition;
    use crate::types::query::envelope::ActInvocation;
    use crate::types::query::scalars::BoundsMode;
    use crate::types::query::stage::StageInput;
    use std::collections::BTreeMap;

    /// A minimal root act node named `s`.
    fn stage(act: ActName) -> StageNode {
        StageNode::Act(ActInvocation {
            name: StageName::parse("s").unwrap(),
            act,
            input: None,
            bounds_mode: None,
            terms: BTreeMap::new(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
        })
    }

    #[test]
    fn a_composition_declares_its_refusal_disposition_up_front() {
        // Declared BEFORE execution; the executor never improvises it.
        let c = Composition {
            outcome: OutcomeDeclaration {
                description: "bias-review over a curated corpus".to_string(),
                returns: vec![],
            },
            intention: None,
            on_stage_refusal: RefusalDisposition::Halt,
            meta_detail: Default::default(),
            bounds: BTreeMap::new(),
            stages: vec![stage(ActName::FindExact)],
        };
        assert_eq!(c.on_stage_refusal, RefusalDisposition::Halt);
        assert_eq!(
            serde_json::from_str::<Composition>(&serde_json::to_string(&c).unwrap()).unwrap(),
            c
        );
    }

    #[test]
    fn the_intention_is_a_composition_level_field_not_a_per_stage_one() {
        // Computed ONCE at composition start and threaded, so every find-about-* stage provably
        // interrogates the same intention rather than re-embedding a mutated string.
        let c = Composition {
            outcome: OutcomeDeclaration {
                description: "x".to_string(),
                returns: vec![],
            },
            intention: Some(Intention {
                query: "wayfind salience".to_string(),
                embedded: true,
            }),
            on_stage_refusal: RefusalDisposition::DegradeAndDisclose,
            meta_detail: Default::default(),
            bounds: BTreeMap::new(),
            stages: vec![stage(ActName::FindAboutAnywhere)],
        };
        let json = serde_json::to_string(&c).unwrap();
        // One intention on the envelope; the stage carries none.
        assert_eq!(json.matches("\"intention\"").count(), 1);
        assert_eq!(serde_json::from_str::<Composition>(&json).unwrap(), c);
    }

    #[test]
    fn an_absent_intention_is_representable_so_a_stage_can_refuse_rather_than_substitute() {
        // "I chose not to embed" and "I cannot embed" become distinguishable: with no intention
        // on the envelope, a find-about-* stage refuses (RefusalReason::MissingIntention) rather
        // than the server quietly embedding on the caller's behalf.
        let c = Composition {
            outcome: OutcomeDeclaration {
                description: "lexical only".to_string(),
                returns: vec![],
            },
            intention: None,
            on_stage_refusal: RefusalDisposition::Halt,
            meta_detail: Default::default(),
            bounds: BTreeMap::new(),
            stages: vec![stage(ActName::FindExact)],
        };
        assert!(c.intention.is_none());
        assert!(!serde_json::to_string(&c).unwrap().contains("intention"));
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
            input: Some(StageInput::Upstream {
                stage: StageName::parse("hits").unwrap(),
            }),
            bounds_mode: Some(BoundsMode::Seed),
            terms: BTreeMap::new(),
            resource_filter: None,
            edge_filter: None,
            properties: vec![],
            act: ActName::FollowFrom,
        });
        assert_eq!(seeded.upstream_names().len(), 1);

        let rooted = StageNode::Act(ActInvocation {
            name: StageName::parse("hits").unwrap(),
            input: None,
            bounds_mode: None,
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
        let c = Composition {
            outcome: OutcomeDeclaration {
                description: "exact hits and their neighbours".to_string(),
                returns: vec![],
            },
            intention: None,
            on_stage_refusal: RefusalDisposition::Halt,
            meta_detail: Default::default(),
            bounds: BTreeMap::new(),
            stages: vec![],
        };
        assert!(c.stages.is_empty());
    }

    #[test]
    fn an_outcome_declares_which_stages_come_back() {
        let o = OutcomeDeclaration {
            description: "neighbours of my exact hits".to_string(),
            returns: vec![ReturnSpec {
                stage: StageName::parse("near").unwrap(),
                fields: vec!["title".to_string(), "home".to_string()],
            }],
        };
        assert_eq!(o.returns.len(), 1);
        assert_eq!(
            serde_json::from_str::<OutcomeDeclaration>(&serde_json::to_string(&o).unwrap())
                .unwrap(),
            o
        );
    }

    #[test]
    fn an_empty_field_list_means_the_default_projection_and_serializes_to_nothing() {
        let r = ReturnSpec {
            stage: StageName::parse("near").unwrap(),
            fields: vec![],
        };
        assert!(!serde_json::to_string(&r).unwrap().contains("fields"));
    }

    #[test]
    fn a_composition_no_longer_declares_one_produced_kind() {
        // A resource arm beside a region arm has no single answer. `produces` was a field that
        // could only ever be right for a single-arm plan — it is derived from `returns` now, not
        // declared.
        let json = serde_json::to_string(&OutcomeDeclaration {
            description: "x".to_string(),
            returns: vec![],
        })
        .unwrap();
        assert!(!json.contains("produces"));
    }

    #[test]
    fn an_outcome_declaration_cannot_omit_its_description() {
        // The pocket outcome register: a named plan states its served-by. Not Option.
        let o = OutcomeDeclaration {
            description: "what being served looks like".to_string(),
            returns: vec![],
        };
        assert!(!o.description.is_empty());
        assert_eq!(
            serde_json::from_str::<OutcomeDeclaration>(&serde_json::to_string(&o).unwrap())
                .unwrap(),
            o
        );
    }
}
