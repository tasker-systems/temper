//! The search family's act declarations, as data.
//!
//! Every value here is a claim about the deployed system that T3's gates verify:
//! `build_state` against the live router, `accepts_*` against the SQL signatures, and
//! `scoring_revision` against a fingerprint of `served_by`'s body.
//!
//! The chainability matrix is not a separate structure: it is the relation induced by each
//! declaration's `produces` against every other's `accepts_bounds` / `accepts_seeds`. Encoding it
//! twice would be the `ADMIN_EVENT_TYPES` failure — a second copy that drifts from the first.

use std::collections::BTreeMap;

use super::act::{ActDeclaration, ActName, BuildState, VisibilityProfile};
use super::filter::FilterField;
use super::id_set::IdKind;
use super::scalars::BoundTerm;

fn fused() -> BuildState {
    BuildState::Fused {
        host: "unified_search".to_string(),
    }
}

/// The seven declarations. Order is stable — the generated contract renders them in this order.
pub fn search_family() -> Vec<ActDeclaration> {
    vec![
        ActDeclaration {
            name: ActName::FindExact,
            asker_holds: "I can quote the exact words".to_string(),
            served_by: Some("search_fts_candidates".to_string()),
            build_state: fused(),
            // Post-filter in unified_search's `corpus` CTE. Membership-equivalent to a pre-filter
            // because the FTS arm carries no top-k, so nothing can be crowded out of it.
            accepts_bounds: vec![IdKind::Resource],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Limit, BoundTerm::Offset],
            accepts_filters: vec![FilterField::Resource],
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            visibility_profile: VisibilityProfile::PrincipalRelative,
            scoring_revision: 2, // ts_rank flag 32 -> 33, migration 20260801000010
        },
        ActDeclaration {
            name: ActName::FindAboutAnywhere,
            asker_holds: "a concept, no exact words; search everything I can see".to_string(),
            served_by: Some("search_vector_candidates".to_string()),
            build_state: fused(),
            // A bound would make this find-about-within. Definitional exclusion, not a hole.
            accepts_bounds: vec![],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Limit, BoundTerm::Offset],
            accepts_filters: vec![FilterField::Resource],
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            visibility_profile: VisibilityProfile::PrincipalRelative,
            scoring_revision: 2, // best-of-N shrunk toward the chunk mean, 20260801000010
        },
        ActDeclaration {
            name: ActName::FindAboutWithin,
            asker_holds: "a concept, plus a set to search inside".to_string(),
            served_by: Some("search_vector_candidates".to_string()),
            build_state: fused(),
            accepts_bounds: vec![IdKind::Resource, IdKind::Context, IdKind::Cogmap],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Limit, BoundTerm::Offset],
            accepts_filters: vec![FilterField::Resource],
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            visibility_profile: VisibilityProfile::PrincipalRelative,
            scoring_revision: 2,
        },
        ActDeclaration {
            name: ActName::FollowFrom,
            asker_holds: "a found thing; I want its neighbours".to_string(),
            served_by: Some("search_graph_expand".to_string()),
            build_state: fused(),
            // Bounded follow-from is UNBUILT: search_graph_expand has no scope parameter, so
            // "walk from these seeds but stay inside this set" is unstatable. The one genuine
            // foreclosure — the act itself is fused, only its bounded form is missing.
            accepts_bounds: vec![],
            accepts_seeds: vec![IdKind::Resource],
            accepts_bound_terms: vec![BoundTerm::Limit],
            accepts_filters: vec![FilterField::Edge],
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            visibility_profile: VisibilityProfile::PrincipalRelative,
            scoring_revision: 1,
        },
        ActDeclaration {
            name: ActName::Survey,
            asker_holds: "a question about what a scope knows".to_string(),
            served_by: Some("wayfind_region_scores".to_string()),
            build_state: fused(),
            // Takes (p_anchor_table, p_anchor_id) — an anchor, which a typed IdSet can name.
            accepts_bounds: vec![IdKind::Cogmap, IdKind::Context],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Regions],
            accepts_filters: vec![],
            bound_ceilings: BTreeMap::from([(BoundTerm::Regions, 20)]),
            produces: Some(IdKind::Region),
            visibility_profile: VisibilityProfile::AgnosticInValueRelativeInDomain,
            scoring_revision: 1,
        },
        ActDeclaration {
            name: ActName::Substantiate,
            asker_holds: "a claim; I want its defensibility".to_string(),
            served_by: None,
            build_state: BuildState::Unbuilt,
            accepts_bounds: vec![],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![],
            accepts_filters: vec![],
            bound_ceilings: BTreeMap::new(),
            produces: None,
            visibility_profile: VisibilityProfile::PrincipalRelative,
            scoring_revision: 0,
        },
        ActDeclaration {
            name: ActName::Admit,
            asker_holds: "nothing — this is an anti-act, declared so that promoting \
                          cold-start admission to a real act must delete an explicit refusal"
                .to_string(),
            served_by: None,
            build_state: BuildState::Unbuilt,
            accepts_bounds: vec![],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![],
            accepts_filters: vec![],
            bound_ceilings: BTreeMap::new(),
            produces: None,
            visibility_profile: VisibilityProfile::PrincipalRelative,
            scoring_revision: 0,
        },
    ]
}

/// Look up one declaration by name.
pub fn declaration(name: &ActName) -> Option<ActDeclaration> {
    search_family().into_iter().find(|a| &a.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_family_declares_seven_acts_including_the_anti_act() {
        let acts = search_family();
        assert_eq!(acts.len(), 7);
        let names: Vec<&ActName> = acts.iter().map(|a| &a.name).collect();
        for expected in [
            ActName::FindExact,
            ActName::FindAboutAnywhere,
            ActName::FindAboutWithin,
            ActName::FollowFrom,
            ActName::Survey,
            ActName::Substantiate,
            ActName::Admit,
        ] {
            assert!(names.contains(&&expected), "missing {expected:?}");
        }
    }

    #[test]
    fn nothing_in_the_search_family_is_served() {
        // This is the FINDING, not an omission: every mechanic is reachable only through
        // unified_search. When a real door lands, this test changes deliberately.
        for a in search_family() {
            assert_ne!(
                a.build_state,
                BuildState::Served,
                "{:?} claims served",
                a.name
            );
        }
    }

    #[test]
    fn every_declaration_states_what_the_asker_holds() {
        for a in search_family() {
            assert!(
                !a.asker_holds.trim().is_empty(),
                "{:?} omits asker_holds",
                a.name
            );
        }
    }

    #[test]
    fn unbuilt_acts_name_no_serving_function_and_built_ones_do() {
        for a in search_family() {
            match a.build_state {
                BuildState::Unbuilt => assert!(
                    a.served_by.is_none(),
                    "{:?} is unbuilt but names a function",
                    a.name
                ),
                _ => assert!(
                    a.served_by.is_some(),
                    "{:?} is built but names no function",
                    a.name
                ),
            }
        }
    }

    #[test]
    fn follow_from_takes_resource_seeds_and_cannot_be_bounded() {
        // The one genuine foreclosure: search_graph_expand has no scope parameter.
        let a = declaration(&ActName::FollowFrom).unwrap();
        assert_eq!(a.accepts_seeds, vec![IdKind::Resource]);
        assert!(
            a.accepts_bounds.is_empty(),
            "follow-from bounded is unbuilt"
        );
    }

    #[test]
    fn survey_accepts_anchor_kinds_as_bounds() {
        // Dissolved by the typed currency: wayfind_region_scores takes (p_anchor_table,
        // p_anchor_id), so cogmap_list -> survey needs no SQL change.
        let a = declaration(&ActName::Survey).unwrap();
        assert!(a.accepts_bounds.contains(&IdKind::Cogmap));
        assert!(a.accepts_bounds.contains(&IdKind::Context));
        assert_eq!(a.produces, Some(IdKind::Region));
    }

    #[test]
    fn find_about_anywhere_accepts_no_bounds_by_definition() {
        // A bound would make it find-about-within. This is a definitional exclusion, not a hole.
        let a = declaration(&ActName::FindAboutAnywhere).unwrap();
        assert!(a.accepts_bounds.is_empty());
        let w = declaration(&ActName::FindAboutWithin).unwrap();
        assert!(!w.accepts_bounds.is_empty());
    }

    #[test]
    fn survey_admits_regions_and_refuses_limit() {
        // `limit` means ROWS on every read. wayfind_region_scores takes p_regions_n — a funnel
        // width — and has no rows to limit, so survey DECLINES `limit` rather than quietly
        // reinterpreting it as a region count. That is the-same-bound-term-means-the-same-thing
        // holding by construction.
        let a = declaration(&ActName::Survey).unwrap();
        assert!(a.accepts_bound_terms.contains(&BoundTerm::Regions));
        assert!(!a.accepts_bound_terms.contains(&BoundTerm::Limit));

        let e = declaration(&ActName::FindExact).unwrap();
        assert!(e.accepts_bound_terms.contains(&BoundTerm::Limit));
        assert!(!e.accepts_bound_terms.contains(&BoundTerm::Regions));
    }

    #[test]
    fn every_ceiling_is_published_for_a_term_the_act_admits() {
        // An unpublished ceiling is the defect, not the clamping: a caller who could have read the
        // ceiling is owed no separate warning, but one who could not is owed the refusal.
        for a in search_family() {
            for term in a.bound_ceilings.keys() {
                assert!(
                    a.accepts_bound_terms.contains(term),
                    "{:?} publishes a ceiling for a term it does not admit: {term:?}",
                    a.name
                );
            }
        }
    }

    #[test]
    fn follow_from_filters_edges_and_the_find_acts_filter_resources() {
        // The two slots never cross: an edge-walking act has no resource predicate to apply, and
        // a lexical/vector act has no edge to filter.
        assert_eq!(
            declaration(&ActName::FollowFrom).unwrap().accepts_filters,
            vec![FilterField::Edge]
        );
        assert_eq!(
            declaration(&ActName::FindExact).unwrap().accepts_filters,
            vec![FilterField::Resource]
        );
    }

    #[test]
    fn the_anti_act_is_declared_unbuilt_and_produces_nothing() {
        let a = declaration(&ActName::Admit).unwrap();
        assert_eq!(a.build_state, BuildState::Unbuilt);
        assert_eq!(a.produces, None);
        assert!(a.accepts_bounds.is_empty() && a.accepts_seeds.is_empty());
    }

    #[test]
    fn survey_is_the_only_act_relative_in_domain() {
        // sal_norm is a percent_rank whose window frame is the asker's visible set — measured,
        // 382 of 385 regions score differently across two visible-anchor sets. No other act in
        // the family has that property, and claiming one did would be a false declaration.
        let relative: Vec<ActName> = search_family()
            .into_iter()
            .filter(|a| a.visibility_profile == VisibilityProfile::AgnosticInValueRelativeInDomain)
            .map(|a| a.name)
            .collect();
        assert_eq!(relative, vec![ActName::Survey]);
    }
}
