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

use super::act::{
    ActDeclaration, ActName, ActQuantity, BuildState, Door, DoorReach, QuantityScale,
    VisibilityProfile,
};
use super::filter::FilterField;
use super::id_set::IdKind;
use super::scalars::BoundTerm;

/// `follow-from` and `survey` are in a state `BuildState` cannot currently express, and this helper
/// is where that is recorded rather than hidden.
///
/// Their mechanics are live — `search_graph_expand` and `wayfind_region_scores` are both still
/// deployed — but as of phase 1 steps 2-3 **no door reaches either one**. `/api/search` no longer
/// expands across edges and no longer runs the region funnel, and `unified_search`, the host named
/// below, no longer exists at all: it was dropped from the schema on 2026-08-06 with the rest of the
/// blended search mechanism. So `Fused` is now false in BOTH its clauses — there is no host, and it
/// has no door — while `Unbuilt` would still be false about a function that is right there, and
/// `Served` would still be false about a door that does not exist.
///
/// **A fourth variant is deliberately NOT added.** `/api/query` is the next phase and is being taken
/// up immediately; it gives both acts doors and expresses this remainder properly, so a variant
/// minted here would be born obsolete — churn on a shipped contract type, and a semver-breaking
/// widening at that. The declarations and their `served_by` mechanics are kept intact so the record
/// that these functions exist and are stranded is not lost in the meantime.
///
/// The cost of holding the line is that the emitted `host` now names nothing. That is tolerable
/// only because `build_state` has no runtime consumer — nothing branches on it; it is a wire and
/// codegen marker — so a dangling host misleads a reader of the contract and misroutes no call.
///
/// `[provisional — 2026-08-05; resolve in phase 4]`
fn provisionally_unexpressed() -> BuildState {
    BuildState::Fused {
        host: "unified_search".to_string(),
    }
}

/// The `/api/search` door shape: all three doors present and serving, differing only in which bound
/// terms a door cannot supply. Five declarations write it.
///
/// The name is historical — it was minted when those five acts were fused into one host. The host is
/// gone (retired 2026-08-06) and the doors did not move with it, which is why this helper is
/// unchanged: the three doors are `temper search`
/// `[verified — crates/temper-cli/src/cli.rs:286]`, `POST /api/search`
/// `[verified — crates/temper-api/src/routes.rs:164]`, and the MCP `search` tool
/// `[verified — crates/temper-mcp/src/service.rs:351-360]`.
///
/// Two of the five — `follow-from` and `survey` — carry this shape while
/// [`provisionally_unexpressed`] records that no door in fact reaches their mechanic. That tension is
/// stated there, and `/api/query` is what resolves it; it is not resolved by editing this map.
///
/// The MCP tool takes the whole [`crate::types::api::SearchParams`] as its `Parameters`, so every
/// wire field is reachable from it — worth stating because grepping the `temper-mcp` crate for a
/// param name finds nothing and reads as absence.
fn unified_doors(cli_unreachable: Vec<BoundTerm>) -> BTreeMap<Door, DoorReach> {
    BTreeMap::from([
        (
            Door::Cli,
            DoorReach::Serves {
                terms_unreachable: cli_unreachable,
            },
        ),
        (
            Door::Api,
            DoorReach::Serves {
                terms_unreachable: vec![],
            },
        ),
        (
            Door::Mcp,
            DoorReach::Serves {
                terms_unreachable: vec![],
            },
        ),
    ])
}

/// Both `find-about` acts are served by the same function and order by the same column, so the
/// quantity is written once. Two declarations sharing a quantity is not the thing
/// `no-cross-act-ranking` forbids — they are the same mechanic under two askers, and comparing
/// their values is meaningful in a way comparing `vec_norm` to `graph_score` is not.
fn vec_norm_quantity() -> ActQuantity {
    ActQuantity {
        field: "vec_norm".to_string(),
        means: "best-of-N cosine similarity over the resource's OWN current chunks, shrunk toward \
                that resource's chunk mean by the number of draws — framed per resource, so it \
                does not move with who is asking"
            .to_string(),
        // `1.0 - shrunk_distance / 2.0`, and `<=>` cosine distance spans [0,2]
        // `[verified — migrations/20260801000010:186-189, :211-214]`. Contrast `region_score`,
        // which rescales the same operator's output as `1 - d` and lands in [-1,1].
        scale: QuantityScale::UnitInterval,
    }
}

/// The seven declarations. Order is stable — the generated contract renders them in this order.
pub fn search_family() -> Vec<ActDeclaration> {
    vec![
        ActDeclaration {
            name: ActName::FindExact,
            asker_holds: "I can quote the exact words".to_string(),
            served_by: Some("search_exact".to_string()),
            build_state: BuildState::Served,
            // The exact arm carries no top-k, so nothing can be crowded out of it and where the
            // bound is applied cannot change WHICH resources come back — only how many rows the
            // scan touches.
            accepts_bounds: vec![IdKind::Resource],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Limit, BoundTerm::Offset],
            accepts_filters: vec![FilterField::Resource],
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            // The CLI's `search` command has no `--offset` — the flag list runs query, context,
            // cogmap, wayfind, lens, regions, doc_type, limit, text_only, seed, edge-type, depth,
            // no_graph, seed-only and stops `[verified — crates/temper-cli/src/cli.rs:286-336]`.
            // So the CLI can only ever read page 1, and this act's declared `Offset` term is
            // unreachable from it. Fused, reachable from every door, and still door-partial — the
            // case a `BuildState` variant could not have carried.
            door_coverage: unified_doors(vec![BoundTerm::Offset]),
            orders_by: Some(ActQuantity {
                field: "fts_norm".to_string(),
                means: "postgres ts_rank of the query against the resource's own search vector — \
                        document-local, so it does not move with who is asking"
                    .to_string(),
                // Flag 33 = 1 | 32, and flag 32 is `rank / (rank + 1)`
                // `[verified — migrations/20260801000010:129]`. The `_norm` in the column name is
                // earned, unlike `origin`'s claim to name the producing arm.
                scale: QuantityScale::UnitInterval,
            }),
            visibility_profile: Some(VisibilityProfile::PrincipalRelative),
            scoring_revision: 2, // ts_rank flag 32 -> 33, migration 20260801000010
        },
        ActDeclaration {
            name: ActName::FindAboutAnywhere,
            asker_holds: "a concept, no exact words; search everything I can see".to_string(),
            served_by: Some("search_wide".to_string()),
            build_state: BuildState::Served,
            // A bound would make this find-about-within. Definitional exclusion, not a hole.
            accepts_bounds: vec![],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Limit, BoundTerm::Offset],
            accepts_filters: vec![FilterField::Resource],
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            door_coverage: unified_doors(vec![BoundTerm::Offset]),
            orders_by: Some(vec_norm_quantity()),
            visibility_profile: Some(VisibilityProfile::PrincipalRelative),
            scoring_revision: 2, // best-of-N shrunk toward the chunk mean, 20260801000010
        },
        ActDeclaration {
            name: ActName::FindAboutWithin,
            asker_holds: "a concept, plus a set to search inside".to_string(),
            served_by: Some("search_wide".to_string()),
            build_state: BuildState::Served,
            accepts_bounds: vec![IdKind::Resource, IdKind::Context, IdKind::Cogmap],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Limit, BoundTerm::Offset],
            accepts_filters: vec![FilterField::Resource],
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            door_coverage: unified_doors(vec![BoundTerm::Offset]),
            orders_by: Some(vec_norm_quantity()),
            visibility_profile: Some(VisibilityProfile::PrincipalRelative),
            scoring_revision: 2,
        },
        ActDeclaration {
            name: ActName::FollowFrom,
            asker_holds: "a found thing; I want its neighbours".to_string(),
            served_by: Some("search_graph_expand".to_string()),
            build_state: provisionally_unexpressed(),
            // Bounded follow-from is UNBUILT: search_graph_expand has no scope parameter, so
            // "walk from these seeds but stay inside this set" is unstatable. The one genuine
            // foreclosure — the act itself is fused, only its bounded form is missing.
            accepts_bounds: vec![],
            accepts_seeds: vec![IdKind::Resource],
            accepts_bound_terms: vec![BoundTerm::Limit],
            accepts_filters: vec![FilterField::Edge],
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            // CORRECTED 2026-08-05 (was PrincipalRelative). `graph_score` is `MAX(score) GROUP BY
            // node` over `walk`, whose `adj` admits an edge only when BOTH endpoints are visible —
            // so a severed intermediate node changes a surviving node's score. Bite-proven: a 25%
            // row-set cut with the seeds held fixed moves 42 of 415 shared nodes, max delta
            // 0.2016. The cross-principal differential missed it (0 differing) because this
            // corpus's two real principals are NESTED, not merely different.
            //
            // No `scoring_revision` bump: the body did not change, only what we correctly say
            // about it. A revision records a change in the scale or meaning of the quantity.
            //
            // `Limit` only, and the CLI has one — so no shortfall here.
            door_coverage: unified_doors(vec![]),
            orders_by: Some(ActQuantity {
                field: "graph_score".to_string(),
                means: "the best decayed path from any seed to this node — \
                        MAX(gamma^hop * product of edge weights) over walks of at least one hop"
                    .to_string(),
                // NOT [0,1], and not merely un-normalized: `kb_edges.weight` is
                // `DOUBLE PRECISION NOT NULL DEFAULT 1.0` with NO CHECK constraint
                // `[verified — migrations/20260624000001_canonical_schema.sql:637]`. The walk
                // multiplies weights `[verified — migrations/20260711000030:45]`, so any edge
                // written with a weight above 1 lifts this above 1. Today's corpus stays under it
                // because nothing writes such a weight — which is a property of the DATA, not of
                // the quantity, and declaring `UnitInterval` would claim the schema enforces
                // something it does not.
                scale: QuantityScale::Unbounded,
            }),
            visibility_profile: Some(VisibilityProfile::AgnosticInValueRelativeInDomain),
            scoring_revision: 1,
        },
        ActDeclaration {
            name: ActName::Survey,
            asker_holds: "a question about what a scope knows".to_string(),
            served_by: Some("wayfind_region_scores".to_string()),
            build_state: provisionally_unexpressed(),
            // Takes (p_anchor_table, p_anchor_id) — an anchor, which a typed IdSet can name.
            accepts_bounds: vec![IdKind::Cogmap, IdKind::Context],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Regions],
            accepts_filters: vec![],
            bound_ceilings: BTreeMap::from([(BoundTerm::Regions, 20)]),
            produces: Some(IdKind::Region),
            // `--wayfind` and `--regions` are both CLI flags
            // `[verified — crates/temper-cli/src/cli.rs:298, :305]`, and `survey` does not admit
            // `Offset`, so the CLI's missing `--offset` costs this act nothing.
            door_coverage: unified_doors(vec![]),
            orders_by: Some(ActQuantity {
                field: "region_score".to_string(),
                means: "0.4 * sal_norm + 0.6 * query_cos — the region's per-kind salience rank \
                        blended with its centroid's similarity to the query"
                    .to_string(),
                // The surprise, and the reason this variant exists. `sal_norm` is a `percent_rank`
                // in [0,1], but `query_cos` is `1 - (centroid <=> p_emb)` and a cosine DISTANCE
                // spans [0,2], so the similarity spans [-1,1]
                // `[verified — migrations/20260731000050:114-121]`. The composite therefore spans
                // [-0.6, 1.0] and CAN BE NEGATIVE. Every discussion of this number in the arc's
                // research treats it as a [0,1] score.
                //
                // Note what this is next to: `vec_norm` rescales the SAME `<=>` operator as
                // `1 - d/2` into [0,1]. Two rescales of one distance, in one search family, with
                // neither column name disclosing which it is.
                scale: QuantityScale::OtherRange {
                    bounds: "[-0.6, 1.0]".to_string(),
                },
            }),
            visibility_profile: Some(VisibilityProfile::AgnosticInValueRelativeInDomain),
            scoring_revision: 1,
        },
        ActDeclaration {
            name: ActName::Substantiate,
            asker_holds: "a claim; I want its defensibility".to_string(),
            // CORRECTED 2026-08-05 (was `None` / `Unbuilt`). This act SHIPS, and the declaration
            // said no mechanic exists: `GET /api/resources/{id}/evidence`
            // `[verified — crates/temper-api/src/routes.rs:63]` calls
            // `evidential_standing_service::resource_evidence`
            // `[verified — crates/temper-api/src/handlers/evidence.rs:24-31]`, which reads SQL
            // `resource_standing_shape`. `temper resource evidence <ref>` is the CLI door
            // `[verified — crates/temper-cli/src/cli.rs:615]`.
            //
            // The function was never hidden — `VisibilityProfile`'s own doc comment cites
            // `resource_standing_shape` BY NAME as the worked gated-and-therefore-agnostic example,
            // three declarations above the one claiming it did not exist.
            //
            // Same defect shape as the `follow-from` misclassification corrected the same day, and
            // again in the safe direction: `Unbuilt` under-claims, so nothing broke and nothing
            // caught it.
            served_by: Some("resource_standing_shape".to_string()),
            build_state: BuildState::Served,
            // Takes ONE resource id today, not a set. Declaring `accepts_bounds: [Resource]` would
            // claim a batch affordance the audit specifically recorded as absent — "a caller
            // holding 40 search hits issues 40 requests", T1 columns 1-3 §6.1. The act is served;
            // its COMPOSABLE form is what is unbuilt, and that distinction is exactly what
            // `build_state` and these slots are for.
            accepts_bounds: vec![],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![],
            accepts_filters: vec![],
            bound_ceilings: BTreeMap::new(),
            // Annotates rather than selects: it returns a standing shape over the id it was handed
            // and narrows nothing, so there is no produced set to hand onward. `None` here is a
            // real statement about the act and not a placeholder — and it is why the frame's
            // `claims-carry-standing` clause has nowhere to land yet, since `ActResult.produced`
            // is a required `IdSet` and an annotating act has no result shape in v0.
            produces: None,
            door_coverage: BTreeMap::from([
                (
                    Door::Cli,
                    DoorReach::Serves {
                        terms_unreachable: vec![],
                    },
                ),
                (
                    Door::Api,
                    DoorReach::Serves {
                        terms_unreachable: vec![],
                    },
                ),
                // Absent from MCP, which is the door agents use — so the substantiate act is
                // thinnest exactly where it is most needed. Nothing in `crates/temper-mcp` reads
                // standing `[verified — 2026-08-05]`; T1 columns 1-3 §6.2 recorded the same.
                (Door::Mcp, DoorReach::Absent),
            ]),
            // Standing is THREE axes and a band, never one number — `citation_magnitude`,
            // `audit_coverage`, `citation_quality`. There is no single ordering quantity here, and
            // inventing one would be the exact collapse the standing model forbids.
            orders_by: None,
            visibility_profile: None,
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
            // The anti-act is absent from every door BY DECLARATION, and that is the point: no door
            // offers cold-start admission AS an act. Its one mechanized home was the `thin_anchors`
            // arm of `wayfind_scope_reach`, retired 2026-08-06 with the wayfind scope funnel, so
            // today it is neither offered nor deployed. That changes nothing here: the declaration
            // was always about the DOOR, and the refusal is what a later phase must delete on
            // purpose. Promoting it means writing `Serves` here, deliberately.
            door_coverage: BTreeMap::from([
                (Door::Cli, DoorReach::Absent),
                (Door::Api, DoorReach::Absent),
                (Door::Mcp, DoorReach::Absent),
            ]),
            orders_by: None,
            // The anti-act orders nothing, so it has no ordering fragment to classify. v0 declared
            // `PrincipalRelative` here, which reads as a conservative default and is actually a
            // sentence about a function that does not exist.
            visibility_profile: None,
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
    fn the_served_set_is_the_three_find_acts_plus_substantiate() {
        // WAS `substantiate_is_the_only_act_with_a_door_of_its_own`, and before that
        // `nothing_in_the_search_family_is_served`, whose comment read "every mechanic is reachable
        // only through unified_search". That sentence was FALSE about the deployed system while its
        // assertion passed — the same shape as `survey_is_the_only_act_relative_in_domain`, and the
        // same safe direction. It is recorded here because the set has now moved twice.
        //
        // Phase 1 steps 2-3 gave the three `find` acts doors of their own: `/api/search` invokes
        // `search_exact` and `search_wide` directly, neither fused into anything. `substantiate`
        // keeps the door it has had since Set 5 (`GET /api/resources/{id}/evidence`).
        //
        // Kept as an EXACT set: an act acquiring or losing a door must be a deliberate edit here,
        // and `build_state` moving is BREAKING under the semver table (design §6.2). Order follows
        // `search_family()`.
        let served: Vec<ActName> = search_family()
            .into_iter()
            .filter(|a| a.build_state == BuildState::Served)
            .map(|a| a.name)
            .collect();
        assert_eq!(
            served,
            vec![
                ActName::FindExact,
                ActName::FindAboutAnywhere,
                ActName::FindAboutWithin,
                ActName::Substantiate,
            ]
        );
    }

    #[test]
    fn every_declaration_accounts_for_every_door() {
        // Absence is DECLARED, never inferred from an omitted entry. Goal `019fa618` (surface
        // parity) has no witnesses precisely because no inventory of who-offers-what exists; a
        // declaration allowed to stay silent about a door would rebuild that hole here.
        for a in search_family() {
            for door in Door::ALL {
                assert!(
                    a.door_coverage.contains_key(&door),
                    "{:?} says nothing about {door:?} — silence is not a reach claim",
                    a.name
                );
            }
            assert_eq!(a.door_coverage.len(), Door::ALL.len());
        }
    }

    #[test]
    fn an_unreachable_term_is_always_a_term_the_act_admits() {
        // Mirrors `every_ceiling_is_published_for_a_term_the_act_admits`. A door cannot fall short
        // on a term the act never accepted — that is a contradiction, not a parity gap, and it
        // would put a term in the contract twice with two different meanings.
        for a in search_family() {
            for (door, reach) in &a.door_coverage {
                let DoorReach::Serves { terms_unreachable } = reach else {
                    continue;
                };
                for term in terms_unreachable {
                    assert!(
                        a.accepts_bound_terms.contains(term),
                        "{:?} claims {door:?} cannot reach {term:?}, which it does not admit",
                        a.name
                    );
                }
            }
        }
    }

    #[test]
    fn the_cli_cannot_page_the_find_acts_and_that_is_declared() {
        // The concrete parity gap that forced door coverage to be its own axis rather than a
        // `BuildState` variant. The original form of this test noted the three acts were FUSED and
        // still door-partial; they are `Served` now, and the gap is UNCHANGED — which is the
        // stronger version of the same argument. Door-partiality is orthogonal to build state, so no
        // `BuildState` variant could ever have carried it.
        //
        // `temper search` still has no `--offset` and can only ever read page 1. Note that offset is
        // now applied PER ARM (`substrate_read::search_select`), so the CLI cannot page either arm.
        for name in [
            ActName::FindExact,
            ActName::FindAboutAnywhere,
            ActName::FindAboutWithin,
        ] {
            let a = declaration(&name).unwrap();
            assert_eq!(a.build_state, BuildState::Served);
            assert_eq!(
                a.door_coverage.get(&Door::Cli),
                Some(&DoorReach::Serves {
                    terms_unreachable: vec![BoundTerm::Offset]
                }),
                "{name:?} must declare the CLI's missing --offset"
            );
            assert_eq!(
                a.door_coverage.get(&Door::Api),
                Some(&DoorReach::Serves {
                    terms_unreachable: vec![]
                })
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
            // Exhaustive, no `_` arm: a future `BuildState` variant must be classified here
            // deliberately rather than inheriting whichever answer a wildcard happened to give.
            match a.build_state {
                BuildState::Unbuilt => assert!(
                    a.served_by.is_none(),
                    "{:?} is unbuilt but names a function",
                    a.name
                ),
                BuildState::Served | BuildState::Fused { .. } => assert!(
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
    fn exactly_survey_and_follow_from_are_relative_in_domain() {
        // WAS `survey_is_the_only_act_relative_in_domain`, and it was green for the wrong reason.
        //
        // `survey`'s sal_norm is a percent_rank framed on the asker's visible anchor set —
        // measured, 836 of 848 shared regions score differently across two real principals.
        // `follow-from`'s graph_score has the same property and was declared PrincipalRelative:
        // one class too strict, which is the SAFE direction, so nothing caught it. The old
        // assertion's sentence — "no other act in the family has that property" — was false about
        // the deployed system while its `assert_eq!` passed.
        //
        // Kept as an EXACT set rather than a `contains`, because the value of this test is that
        // adding or reclassifying an act must be a deliberate edit here. A `contains` would have
        // admitted the very drift that produced the correction.
        let relative: Vec<ActName> = search_family()
            .into_iter()
            .filter(|a| {
                a.visibility_profile == Some(VisibilityProfile::AgnosticInValueRelativeInDomain)
            })
            .map(|a| a.name)
            .collect();
        assert_eq!(relative, vec![ActName::FollowFrom, ActName::Survey]);
    }

    #[test]
    fn an_act_that_orders_nothing_classifies_no_ordering_fragment() {
        // `visibility_profile` is defined as a statement about "the fragment that produces this
        // act's ordering". An act that orders nothing has no such fragment, so any value there is
        // a claim with no referent — which is what v0 shipped for `substantiate` and `admit`,
        // both declaring `PrincipalRelative`. That reads as a conservative default and is instead
        // a sentence about a function that does not exist.
        //
        // The two fields move together by INVARIANT, so the vacuous claim is unrepresentable
        // rather than merely absent today.
        for a in search_family() {
            assert_eq!(
                a.orders_by.is_none(),
                a.visibility_profile.is_none(),
                "{:?} classifies an ordering fragment it does not have, or vice versa",
                a.name
            );
        }
        assert!(declaration(&ActName::Substantiate)
            .unwrap()
            .visibility_profile
            .is_none());
        assert!(declaration(&ActName::Admit)
            .unwrap()
            .visibility_profile
            .is_none());
    }

    #[test]
    fn no_two_acts_order_by_the_same_name_and_none_of_them_is_a_bare_score() {
        // This is `no-cross-act-ranking` with structural teeth rather than a rule someone
        // remembers. Research `019fbd9b` delta 5: "no bare `score: f64` shared across acts; each
        // quantity carries its act's name and shape."
        //
        // The worked failure is in the shipped host: `unified_search` renames `fts_norm` and
        // `vec_norm` to `fts_score`/`vector_score` and sums them into `combined_score`
        // (migrations/20260714000001_ingest_state.sql:294-299). Once both are called `*_score`,
        // adding them LOOKS like arithmetic instead of a category error.
        let mut seen: Vec<String> = Vec::new();
        for a in search_family() {
            let Some(q) = a.orders_by else { continue };
            assert_ne!(q.field, "score", "{:?} orders by a bare `score`", a.name);
            assert_ne!(
                q.field, "combined_score",
                "{:?} claims the cross-act sum as its own quantity",
                a.name
            );
            assert!(
                !q.means.trim().is_empty(),
                "{:?} names a quantity without saying what it measures",
                a.name
            );
            seen.push(q.field);
        }
        // `vec_norm` is deliberately shared by the two `find-about` acts — one mechanic, two
        // askers — so dedupe before asserting rather than forbidding repeats outright.
        let mut distinct = seen.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(
            distinct,
            vec![
                "fts_norm".to_string(),
                "graph_score".to_string(),
                "region_score".to_string(),
                "vec_norm".to_string()
            ]
        );
        assert_eq!(
            seen.len(),
            5,
            "five acts order by something; four names do it"
        );
    }

    #[test]
    fn the_two_quantities_built_from_one_cosine_distance_do_not_share_a_scale() {
        // The finding this field exists to carry. `vec_norm` and `region_score`'s `query_cos` are
        // both rescales of the SAME pgvector `<=>` cosine distance, and they disagree:
        //   vec_norm  = 1.0 - d/2  -> [0,1]   (migrations/20260801000010:186-189)
        //   query_cos = 1   - d    -> [-1,1]  (migrations/20260731000050:120)
        // so `region_score` = 0.4*sal_norm + 0.6*query_cos spans [-0.6, 1.0] and CAN BE NEGATIVE,
        // while every discussion of it in this arc's research treats it as a [0,1] score.
        //
        // Neither column name discloses which rescale it is. That is the incommensurability thesis
        // showing up one level below where the arc had been looking for it.
        let vec_scale = declaration(&ActName::FindAboutAnywhere)
            .unwrap()
            .orders_by
            .unwrap()
            .scale;
        let region_scale = declaration(&ActName::Survey)
            .unwrap()
            .orders_by
            .unwrap()
            .scale;
        assert_eq!(vec_scale, QuantityScale::UnitInterval);
        assert_eq!(
            region_scale,
            QuantityScale::OtherRange {
                bounds: "[-0.6, 1.0]".to_string()
            }
        );
        assert_ne!(vec_scale, region_scale);
    }

    #[test]
    fn graph_score_is_unbounded_because_the_schema_does_not_bound_edge_weight() {
        // Declared `Unbounded`, not `UnitInterval`, and the distinction is not pedantic: the walk
        // multiplies `kb_edges.weight` at every hop (migrations/20260711000030:45), and that
        // column is `DOUBLE PRECISION NOT NULL DEFAULT 1.0` with NO CHECK
        // (migrations/20260624000001_canonical_schema.sql:637). Today's values stay under 1
        // because nothing writes a larger one — a property of the DATA, not of the quantity.
        assert_eq!(
            declaration(&ActName::FollowFrom)
                .unwrap()
                .orders_by
                .unwrap()
                .scale,
            QuantityScale::Unbounded
        );
    }

    #[test]
    fn substantiate_is_absent_from_the_door_agents_use() {
        // Served on API and CLI, absent from MCP — so the one act about defensibility is missing
        // from the surface agents actually hold. Declared rather than discovered on a 404.
        let a = declaration(&ActName::Substantiate).unwrap();
        assert_eq!(a.door_coverage.get(&Door::Mcp), Some(&DoorReach::Absent));
        assert!(matches!(
            a.door_coverage.get(&Door::Api),
            Some(DoorReach::Serves { .. })
        ));
        assert_eq!(a.served_by.as_deref(), Some("resource_standing_shape"));
        // Annotates rather than selects: no produced set, which is why `claims-carry-standing`
        // has no result shape to land in while `ActResult.produced` is a required `IdSet`.
        assert_eq!(a.produces, None);
    }

    #[test]
    fn the_two_find_acts_order_by_a_principal_agnostic_quantity() {
        // The other half of the same correction, and the half with no code change: both `find`
        // acts' ORDERING quantities are principal-agnostic — `ts_rank` is document-local (0
        // differing over 185 shared resources) and the vector arm's shrunk order statistic is
        // framed `GROUP BY resource_id` over that resource's own chunks (0 over 93). Their
        // `PrincipalRelative` declarations describe their result SETS, which is true and is not
        // what this field asks.
        //
        // So this test does NOT assert those declarations are wrong. It pins the fact that makes
        // them a judgment rather than an oversight, so a later reader re-deriving the
        // classification finds the measurement instead of repeating it.
        for name in [
            ActName::FindExact,
            ActName::FindAboutAnywhere,
            ActName::FindAboutWithin,
        ] {
            let a = declaration(&name).unwrap();
            assert_ne!(
                a.visibility_profile,
                Some(VisibilityProfile::AgnosticInValueRelativeInDomain),
                "{name:?} orders by a document-local or resource-local quantity; \
                 declaring it relative-in-domain would claim a frame it does not have"
            );
        }
    }
}
