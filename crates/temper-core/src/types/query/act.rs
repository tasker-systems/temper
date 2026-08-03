//! Act identity, build-state, and the declaration shape.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::filter::FilterField;
use super::id_set::IdKind;
use super::scalars::BoundTerm;

/// The act vocabulary. Asker-shaped, not mechanism-shaped: an act names what the asker holds, and
/// the mechanic currently serving it is evidence rather than identity.
///
/// OPEN discriminator — adding an act is additive.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
pub enum ActName {
    /// *I can quote the exact words.*
    #[serde(rename = "find-exact")]
    FindExact,
    /// *A concept, no exact words; search everything I can see.*
    #[serde(rename = "find-about-anywhere")]
    FindAboutAnywhere,
    /// *A concept, plus a set to search inside.*
    #[serde(rename = "find-about-within")]
    FindAboutWithin,
    /// *A found thing; I want its neighbours.*
    #[serde(rename = "follow-from")]
    FollowFrom,
    /// *A question about what a scope knows.*
    #[serde(rename = "survey")]
    Survey,
    /// *A claim; I want its defensibility.*
    #[serde(rename = "substantiate")]
    Substantiate,
    /// The anti-act: visibility-shaped admission wearing relevance's costume. Declared so that
    /// promoting it to a real act requires DELETING an explicit refusal.
    #[serde(rename = "admit")]
    Admit,
    #[serde(untagged)]
    Other(String),
}

/// Whether an act is reachable, and how. Every value is mechanically checkable by T3's gate —
/// which is the whole point, because a hand-maintained build-state is the `ADMIN_EVENT_TYPES`
/// failure: a const beside a registry, with a test holding its own second copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum BuildState {
    /// Exactly one door invokes this act alone.
    Served,
    /// The mechanic runs only inside a named composite; the act has no door, the host has one.
    Fused { host: String },
    /// No mechanic exists.
    Unbuilt,
}

impl BuildState {
    pub fn host(&self) -> Option<&str> {
        match self {
            BuildState::Fused { host } => Some(host.as_str()),
            _ => None,
        }
    }
}

/// Where the principal constraint applies to an act's mechanic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
#[serde(rename_all = "kebab-case")]
pub enum VisibilityProfile {
    PrincipalAgnostic,
    /// Every input and operation is principal-free, but the fragment is a window or aggregate
    /// whose frame is the principal's read-set. `survey`'s `sal_norm` is the worked example.
    AgnosticInValueRelativeInDomain,
    PrincipalRelative,
}

/// One act, declared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
pub struct ActDeclaration {
    pub name: ActName,
    /// What the asker holds. NOT optional — `every-act-is-situated` enforced by the signature.
    pub asker_holds: String,
    /// The SQL function serving this act; `None` when `build_state` is `unbuilt`. T3 fingerprints
    /// this function's body against `scoring_revision`.
    pub served_by: Option<String>,
    pub build_state: BuildState,
    pub accepts_bounds: Vec<IdKind>,
    pub accepts_seeds: Vec<IdKind>,
    /// Which bound terms this act admits. A term absent here is DECLINED, never reinterpreted:
    /// `survey` admits `[Regions]` and not `Limit`, because `wayfind_region_scores` takes a funnel
    /// width and has no rows to limit.
    pub accepts_bound_terms: Vec<BoundTerm>,
    /// Which filter slots this act admits. An unadmitted filter is declined
    /// (`RefusalReason::FilterNotApplicable`), never silently ignored.
    pub accepts_filters: Vec<FilterField>,
    /// Published ceilings, per admitted term. A ceiling the caller could have read is disclosed by
    /// `terms_effective` and owes no separate warning; an UNPUBLISHED ceiling is the defect, not
    /// the clamping. A term with no entry here has no ceiling.
    pub bound_ceilings: BTreeMap<BoundTerm, i64>,
    pub produces: Option<IdKind>,
    pub visibility_profile: VisibilityProfile,
    /// Bumped whenever the served-by body changes the scale or meaning of a quantity. T3 gate 4
    /// reds when the body hash moves and this does not.
    pub scoring_revision: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn act_names_are_asker_shaped_on_the_wire() {
        assert_eq!(
            serde_json::to_string(&ActName::FindExact).unwrap(),
            "\"find-exact\""
        );
        assert_eq!(
            serde_json::to_string(&ActName::FindAboutAnywhere).unwrap(),
            "\"find-about-anywhere\""
        );
        assert_eq!(
            serde_json::to_string(&ActName::FindAboutWithin).unwrap(),
            "\"find-about-within\""
        );
        assert_eq!(
            serde_json::to_string(&ActName::FollowFrom).unwrap(),
            "\"follow-from\""
        );
        assert_eq!(
            serde_json::to_string(&ActName::Survey).unwrap(),
            "\"survey\""
        );
        assert_eq!(
            serde_json::to_string(&ActName::Substantiate).unwrap(),
            "\"substantiate\""
        );
    }

    #[test]
    fn act_discriminator_is_open_unknown_acts_parse() {
        // Adding an act is ADDITIVE (design §6.1) — an older consumer must survive a newer one.
        let a: ActName = serde_json::from_str("\"enumerate\"").expect("unknown act must parse");
        assert_eq!(a, ActName::Other("enumerate".to_string()));
    }

    #[test]
    fn fused_build_state_names_its_host() {
        // `fused` is a fact, not a euphemism: the host is what T3's gate checks has a door while
        // the act itself does not.
        let b = BuildState::Fused {
            host: "unified_search".to_string(),
        };
        let back: BuildState = serde_json::from_str(&serde_json::to_string(&b).unwrap()).unwrap();
        assert_eq!(back, b);
        assert_eq!(back.host(), Some("unified_search"));
        assert_eq!(BuildState::Unbuilt.host(), None);
        assert_eq!(BuildState::Served.host(), None);
    }

    #[test]
    fn a_declaration_cannot_omit_what_the_asker_holds() {
        // `every-act-is-situated` enforced at the type layer: asker_holds is not Option.
        let d = ActDeclaration {
            name: ActName::FindExact,
            asker_holds: "I can quote the exact words".to_string(),
            served_by: Some("search_fts_candidates".to_string()),
            build_state: BuildState::Fused {
                host: "unified_search".to_string(),
            },
            accepts_bounds: vec![IdKind::Resource],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Limit, BoundTerm::Offset],
            accepts_filters: vec![FilterField::Resource],
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            visibility_profile: VisibilityProfile::PrincipalRelative,
            scoring_revision: 1,
        };
        assert!(!d.asker_holds.is_empty());
        assert_eq!(
            serde_json::from_str::<ActDeclaration>(&serde_json::to_string(&d).unwrap()).unwrap(),
            d
        );
    }
}
