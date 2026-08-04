//! The composition envelope: an ordered list of stages plus the things that ride alongside them.
//!
//! The PRINCIPAL is deliberately absent. Visibility applies inside each act's execution — one
//! known application point per stage — and jaq reshapes what visibility admitted without ever
//! seeing the credential. There is no field here for it, by construction.

use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use super::act::ActName;
use super::disposition::RefusalDisposition;
use super::envelope::ActInvocation;
use super::id_set::IdKind;
use super::scalars::{BoundTerm, MetaDetail};

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

/// A composition's pocket outcome register: what it is for, in the act schemas' own terms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct OutcomeDeclaration {
    /// What being served looks like. NOT optional.
    pub description: String,
    /// The kind the whole composition yields, when it is fixed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub produces: Option<IdKind>,
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
    /// Ordered. Stages reference their inputs explicitly — there is no prev-else-fallback.
    pub stages: Vec<ActInvocation>,
}

impl Composition {
    /// The acts this composition names, in order. Used by the contract-chaining check.
    pub fn act_sequence(&self) -> Vec<&ActName> {
        self.stages.iter().map(|s| &s.act).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::query::act::ActName;
    use crate::types::query::disposition::RefusalDisposition;
    use crate::types::query::envelope::ActInvocation;

    fn stage(act: ActName) -> ActInvocation {
        ActInvocation {
            act,
            bounds: None,
            bounds_mode: None,
            terms: std::collections::BTreeMap::new(),
            resource_filter: None,
            edge_filter: None,
        }
    }

    #[test]
    fn a_composition_declares_its_refusal_disposition_up_front() {
        // Declared BEFORE execution; the executor never improvises it.
        let c = Composition {
            outcome: OutcomeDeclaration {
                description: "bias-review over a curated corpus".to_string(),
                produces: None,
            },
            intention: None,
            on_stage_refusal: RefusalDisposition::Halt,
            meta_detail: Default::default(),
            bounds: std::collections::BTreeMap::new(),
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
                produces: None,
            },
            intention: Some(Intention {
                query: "wayfind salience".to_string(),
                embedded: true,
            }),
            on_stage_refusal: RefusalDisposition::DegradeAndDisclose,
            meta_detail: Default::default(),
            bounds: std::collections::BTreeMap::new(),
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
                produces: None,
            },
            intention: None,
            on_stage_refusal: RefusalDisposition::Halt,
            meta_detail: Default::default(),
            bounds: std::collections::BTreeMap::new(),
            stages: vec![stage(ActName::FindExact)],
        };
        assert!(c.intention.is_none());
        assert!(!serde_json::to_string(&c).unwrap().contains("intention"));
    }

    #[test]
    fn an_outcome_declaration_cannot_omit_its_description() {
        // The pocket outcome register: a named plan states its served-by. Not Option.
        let o = OutcomeDeclaration {
            description: "what being served looks like".to_string(),
            produces: None,
        };
        assert!(!o.description.is_empty());
        assert_eq!(
            serde_json::from_str::<OutcomeDeclaration>(&serde_json::to_string(&o).unwrap())
                .unwrap(),
            o
        );
    }
}
