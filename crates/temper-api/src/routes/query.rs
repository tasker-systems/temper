//! `/api/query` alone, carrying the one bound on a composition that the schema cannot express.
//!
//! **Merged into `gated_routes` rather than mounted there, purely so the layer is scoped.** A
//! `DefaultBodyLimit` applies to every route in the router it is attached to, and no other gated
//! route has any reason to accept a body this size — the same argument that keeps
//! `webhook_intake_routes` separate, and the reason this is a merge rather than one more
//! `.routes(...)` line. It stays inside `gated_routes`' auth and system-access layers, which are
//! applied to the merged whole in `create_app`; nothing about the mounting changes who may knock.

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::handlers;
use temper_services::state::AppState;

pub(super) fn query_routes() -> OpenApiRouter<AppState> {
    use axum::extract::DefaultBodyLimit;

    OpenApiRouter::new()
        .routes(routes!(handlers::query::query))
        .layer(DefaultBodyLimit::max(QUERY_MAX_BODY_BYTES))
}

/// The largest composition `/api/query` will read.
///
/// # Declared because the inherited number is wrong, not merely because inheriting is untidy
///
/// Without its own limit this door inherits the 2 MiB default (`MAX_REQUEST_BODY_BYTES`), and that
/// is wrong in the direction that refuses legal plans: a composition the contract calls legal
/// serializes to up to **12,439,582 bytes** (below). The door would answer a plan its own contract
/// admits with a bare 413 — no refusal list, no vocabulary, in the door whose whole promise is that
/// every refusal arrives at once and in the caller's own terms.
/// `the_largest_legal_composition_fits_inside_the_declared_body_limit` holds the plan against this
/// number, and fails against the inherited one.
///
/// # Why 25 MB, and what it backstops
///
/// The network-door ruling (design §D4) sized this to the transport contract: `/api/query` is a
/// tool-carrying endpoint, and a second, smaller opinion about body size would be an invisible gate
/// the MCP edge's callers cannot see. The declaration caps are the real bound. This number catches
/// only the cost they cannot see.
///
/// **Every count, every predicate value, and every string's length the contract admits is
/// bounded.** The narrowing lists are capped by `MAX_FILTER_VALUES`, the closed vocabularies by
/// `DuplicateSetMember`, each narrowing string by `MAX_FILTER_STRING_BYTES` (256) or
/// `MAX_TITLE_CONTAINS_BYTES` (1024), and property-predicate values by `MAX_PROPERTY_VALUE_BYTES`
/// (4 KiB each) and `MAX_COMPOSITION_PROPERTY_VALUE_BYTES` (1 MiB across the composition).
/// `the_largest_legal_composition_fits_inside_the_declared_body_limit` builds the plan at every one
/// of those caps, with every string at its widest JSON encoding, and measures it at **12,439,582
/// bytes** `[measured — 2026-10-06]`, 2.1x under this number. So a plan the contract calls legal
/// never meets a bare 413.
///
/// **What this limit still catches is bytes that are not the plan**: whitespace, and fields serde
/// ignores. A caller can pad a legal plan past any limit that way, and a 413 is the right answer
/// to padding.
pub const QUERY_MAX_BODY_BYTES: usize = 25 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::QUERY_MAX_BODY_BYTES;
    use std::collections::BTreeMap;
    use temper_core::types::graph::EdgeKind;
    use temper_core::types::query::act::ActName;
    use temper_core::types::query::composition::{
        Composition, Intention, OutcomeDeclaration, ReturnSpec, StageNode,
        MAX_INTENTION_QUERY_BYTES, MAX_STAGES,
    };
    use temper_core::types::query::envelope::ActInvocation;
    use temper_core::types::query::filter::{
        json_string_bytes, property_value_bytes, worst_case_string_bytes, worst_case_value_bytes,
        EdgeFilter, FacetPredicate, PropertyOp, PropertyPredicate, ResourceFilter,
        MAX_COMPOSITION_PROPERTY_VALUE_BYTES, MAX_COMPOSITION_TEXT_BYTES, MAX_FILTER_STRING_BYTES,
        MAX_FILTER_VALUES, MAX_PROPERTY_VALUE_BYTES, MAX_TITLE_CONTAINS_BYTES,
    };
    use temper_core::types::query::id_set::{IdKind, IdSet, MAX_ID_SET_IDS};
    use temper_core::types::query::scalars::BoundTerm;
    use temper_core::types::query::stage::{StageInput, StageName, StageRelation};
    use temper_core::types::query::validate::validate;
    use temper_core::types::resource_view::ResourceSection;

    /// **The coherence condition that makes the caps one decision rather than several ifs.**
    ///
    /// Every bound on what a request may declare is published on the field it bounds and refused in
    /// the shape pass, with a typed reason and every sibling refusal beside it. The body limit is
    /// the one such bound the schema cannot carry, so it is declared here — and it is only
    /// coherent with the others if a caller never meets it while inside them. A composition at
    /// every published cap that does not FIT is a plan the contract calls legal and the transport
    /// answers with a bare 413: no refusal list, no vocabulary, in the door whose whole promise is
    /// that a plan is repaired in one round trip.
    ///
    /// **The plan is VALIDATED before it is measured, and that assertion is not ceremony**
    /// `[added — 2026-08-28, found in review]`. The first version of this fixture used
    /// `find-about-anywhere` and carried a seed and a bound — an act that declares
    /// `accepts_bounds: vec![]` and `accepts_seeds: vec![]` (`registry.rs:202-203`, *"a bound would
    /// make this find-about-within"*). So it measured a plan carrying **128 refusals** while its
    /// own doc called it *"a plan the contract calls legal"*, and nothing asked. The size claim
    /// happened to survive — `follow-from` is the one act declaring both, and swapping it moved the
    /// total by 512 bytes — but a size measured over an illegal plan proves nothing about what the
    /// door must accept, and the next edit to this fixture would have had no guard at all.
    ///
    /// The largest terms in the selection shape are the narrowing strings at their length caps
    /// (two 256-entry lists of 256-byte strings per stage), the caller id sets, and the 1 MiB of
    /// property-predicate values the fixture grows to exactly `MAX_COMPOSITION_PROPERTY_VALUE_BYTES`.
    ///
    /// **What it does NOT prove**, stated because a green here reads like completeness:
    ///
    /// - **It measures the plan, not the request.** Whitespace and fields serde ignores are bytes
    ///   a caller may add to any plan; they are padding, and the body limit answers padding.
    /// - **The maximum is over two pure shapes, not every mixture.** No single act admits every
    ///   bounded field: the walk carries id sets and no `ResourceFilter`, the selection the reverse.
    ///   Both are measured and the larger reported, which is the selection shape since string
    ///   lengths were capped.
    #[test]
    fn the_largest_legal_composition_fits_inside_the_declared_body_limit() {
        // **Two shapes, both measured, because no single act admits every bounded field and the
        // larger one is not obvious.** `follow-from` takes a seed, a bound and an `EdgeFilter`;
        // `find-resources-with` is the only act whose `ResourceFilter` is not refused
        // (`capability.rs`'s narrowings block), and it accepts no bounds and no page terms at all.
        // A first version mixed them and measured 1,756,196 — LESS than either pure shape, because
        // half its stages carried no id sets. Measuring both and taking the larger is what stops
        // this test from quietly reporting a maximum that is not one.
        let walk = at_the_text_budget(at_the_property_value_budget(plan_of(
            MAX_STAGES,
            ActName::FollowFrom,
        )));
        let select = at_the_text_budget(at_the_property_value_budget(plan_of(
            MAX_STAGES,
            ActName::FindResourcesWith,
        )));

        for (what, c) in [("walk", &walk), ("selection", &select)] {
            // Legal FIRST. A byte count over a plan the validator refuses is a measurement of
            // nothing — and the first version of this test measured one carrying 128 refusals
            // while its own doc called it legal `[found in review — 2026-08-28]`.
            assert!(
                validate(c).is_ok(),
                "the {what} fixture must be a composition this server would RUN, or its size says \
                 nothing about what the door has to accept: {:?}",
                validate(c).err()
            );
        }

        // Measured at the MOST EXPANSIVE standard encoder, not serde's: a client escaping every
        // non-ASCII character or `<>&` as `\uXXXX` sends the same legal plan in more bytes, and the
        // door must read it all the same. Serde's size is a lower bound beside it.
        let sizes: Vec<(usize, usize)> = [&walk, &select]
            .iter()
            .map(|c| {
                let value = serde_json::to_value(c).expect("a composition serializes");
                (
                    serde_json::to_vec(&value).expect("serializes").len(),
                    worst_case_value_bytes(&value),
                )
            })
            .collect();
        let bytes = sizes.iter().map(|s| s.1).max().expect("two shapes");
        assert!(
            bytes < QUERY_MAX_BODY_BYTES,
            "the largest composition at every published cap encodes to {bytes} bytes at worst \
             (walk {:?}, selection {:?}, as (serde, worst)), which the declared body limit of \
             {QUERY_MAX_BODY_BYTES} would refuse with a bare 413 — raise the limit, or lower the \
             caps, but do not let the contract admit a plan the door cannot read",
            sizes[0],
            sizes[1]
        );
    }

    /// `n` stages of one act, each maximal over every field that act admits and every cap the
    /// contract publishes.
    ///
    /// **A selection-shaped plan still ends in one walk stage**, because a selection orders nothing
    /// and is refused in `returns` (`StageNotReturnable`) while a composition that returns nothing
    /// is refused outright (`NoReturns`). So the pure shape is not legal at any size, and the
    /// largest selection-shaped plan is `n - 1` selections plus the walk that answers.
    fn plan_of(n: usize, act: ActName) -> Composition {
        let all_walk = act == ActName::FollowFrom;
        let stages: Vec<StageNode> = (0..n)
            .map(|i| {
                let walk = all_walk || i == n - 1;
                StageNode::Act(ActInvocation {
                    // Stage names at their own ceiling — 63 (`stage.rs:43`).
                    name: StageName::parse(&format!(
                        "s{i}{}",
                        "n".repeat(60 - i.to_string().len())
                    ))
                    .expect("legal stage name"),
                    act: if walk {
                        ActName::FollowFrom
                    } else {
                        act.clone()
                    },
                    intention: Some(Intention {
                        // Grown toward the text budget by `at_the_text_budget`.
                        query: "x".to_string(),
                        // A real normalized BGE component, so the serialized width is the one a
                        // caller actually sends rather than the two bytes `0.0` would cost.
                        //
                        // Every stage carries one, which is also what keeps this inside
                        // `MAX_COMPOSITION_INTENTION_BYTES`: that bound counts only what the SERVER
                        // must embed, and a caller who precomputed has already paid it. A fixture
                        // without embeddings is a DIFFERENT and SMALLER maximum, because the
                        // aggregate budget then caps its question text at 64 KB.
                        embedding: Some(vec![-0.041_899_003; 768]),
                    }),
                    inputs: if walk {
                        vec![
                            StageInput::Caller {
                                relation: StageRelation::Seed,
                                ids: full_id_set(),
                            },
                            StageInput::Caller {
                                relation: StageRelation::Bound,
                                ids: full_id_set(),
                            },
                        ]
                    } else {
                        // A selection accepts no bounds of any kind and no page terms — it declares
                        // a set. Both are `capability`'s refusals, and hitting them is how this
                        // fixture learned the shape rather than assuming it.
                        vec![]
                    },
                    terms: if walk {
                        BTreeMap::from([
                            (BoundTerm::Limit, i64::from(i32::MAX)),
                            (BoundTerm::Offset, i64::from(i32::MAX)),
                        ])
                    } else {
                        BTreeMap::new()
                    },
                    resource_filter: (!walk).then(full_resource_filter),
                    edge_filter: walk.then(full_edge_filter),
                    properties: vec![],
                })
            })
            .collect();

        Composition {
            outcome: OutcomeDeclaration {
                // A selection orders nothing and is refused in `returns` (`StageNotReturnable`), so
                // that shape returns its first stage only — which is what a caller would do.
                // Every walk stage, which for the walk shape is all of them and for the selection
                // shape is the one that answers.
                returns: stages
                    .iter()
                    .filter(|n| matches!(n, StageNode::Act(i) if i.act == ActName::FollowFrom))
                    .map(|node| ReturnSpec {
                        stage: node.name().clone(),
                        with: vec![ResourceSection::OpenMeta],
                    })
                    .collect(),
            },
            stages,
        }
    }

    /// Every narrowing list at [`MAX_FILTER_VALUES`], and both per-candidate containers at the caps
    /// `capability.rs` enforces — 32 predicates summing to 256 probes.
    fn full_resource_filter() -> ResourceFilter {
        ResourceFilter {
            // Every string starts short and is grown by `at_the_text_budget`.
            doc_type: vec!["d".to_string(); MAX_FILTER_VALUES],
            tags: vec!["t".to_string(); MAX_FILTER_VALUES],
            facets: (0..16)
                .map(|i| FacetPredicate {
                    key: format!("k{i}"),
                    // Grown toward the value budget by `at_the_property_value_budget`.
                    value: "v".to_string(),
                })
                .collect(),
            // 16 facets + 16 predicates = 32, the predicate cap; 16 facets + 240 probes = 256,
            // the probe cap. Facets count against BOTH, which is what the container's own doc
            // means by summing what walks the same candidate set.
            properties: capped_properties(16, 15),
            stage: Some("s".to_string()),
            status: Some("a".to_string()),
            owner: Some("o".to_string()),
            title_contains: Some("t".to_string()),
        }
    }

    fn full_edge_filter() -> EdgeFilter {
        EdgeFilter {
            // A closed vocabulary carried as a list, and repeats are refused — so its ceiling IS
            // the vocabulary, and naming every member is what makes this maximal. `[widened from
            // one — 2026-08-28, found in review]`
            edge_kinds: vec![
                EdgeKind::Express,
                EdgeKind::Contains,
                EdgeKind::LeadsTo,
                EdgeKind::Near,
            ],
            labels: vec!["l".to_string(); MAX_FILTER_VALUES],
            // No facets on an edge container, so all 32 predicates and all 256 probes are the
            // property list's.
            properties: capped_properties(32, 8),
        }
    }

    /// `preds` predicates each carrying `vals` values. The two caps a container must satisfy are
    /// `MAX_PER_CANDIDATE_PREDICATES` (32, summed with `facets` where the container has them) and
    /// `MAX_PER_CANDIDATE_PROBES` (256, likewise) — so the split differs between the two containers
    /// and is passed rather than assumed.
    /// Grow the plan's predicate values, none past the per-value cap, until together they total
    /// exactly the composition budget. The count caps admit 16,384 values, so at a few bytes each
    /// the fixture would leave out the ~1 MiB of value bytes a legal plan may carry.
    fn at_the_property_value_budget(mut c: Composition) -> Composition {
        // Facet values first: they are budgeted as raw string bytes, not JSON.
        let mut facet_values: Vec<&mut String> = Vec::new();
        for node in &mut c.stages {
            let StageNode::Act(inv) = node else { continue };
            facet_values.extend(
                inv.resource_filter
                    .iter_mut()
                    .flat_map(|f| &mut f.facets)
                    .map(|f| &mut f.value),
            );
        }
        let facet_total: usize = facet_values.iter().map(|v| json_string_bytes(v)).sum();
        let mut values: Vec<&mut serde_json::Value> = Vec::new();
        for node in &mut c.stages {
            let StageNode::Act(inv) = node else { continue };
            let resource = inv
                .resource_filter
                .iter_mut()
                .flat_map(|f| &mut f.properties);
            let edge = inv.edge_filter.iter_mut().flat_map(|f| &mut f.properties);
            for p in resource.chain(edge) {
                match &mut p.op {
                    PropertyOp::Contains { values: vs } => values.extend(vs.iter_mut()),
                    PropertyOp::Compare { value, .. } => values.push(value),
                    PropertyOp::HasKey => {}
                }
            }
        }
        let total: usize = values
            .iter()
            .map(|v| property_value_bytes(v))
            .sum::<usize>()
            + facet_total;
        let mut slack = MAX_COMPOSITION_PROPERTY_VALUE_BYTES
            .checked_sub(total)
            .expect("the fixture's values start inside the budget");
        for v in values {
            let current = property_value_bytes(v);
            let target = MAX_PROPERTY_VALUE_BYTES.min(current + slack);
            // A JSON string of n characters serializes to n + 2 bytes.
            *v = serde_json::json!("x".repeat(target - 2));
            slack -= target - current;
        }
        assert_eq!(
            slack, 0,
            "the fixture has too few values to reach the budget"
        );
        c
    }

    /// Grow every caller string — narrowing strings, keys, questions — toward its own cap, at its
    /// widest encoding, until the plan's caller text reaches `MAX_COMPOSITION_TEXT_BYTES` counted
    /// at `worst_case_string_bytes`, or every string is at its cap. The predicate values were
    /// already grown to their own budget and count toward this one. Control characters cost the
    /// same six bytes to every encoder, so the filler's worst case is also its serde size.
    fn at_the_text_budget(mut c: Composition) -> Composition {
        // (string, its cap in the unit its check counts, true when the cap counts escaped bytes
        // rather than decoded ones, the fixed prefix that keeps a key distinct).
        let mut strings: Vec<(&mut String, usize, bool, String)> = Vec::new();
        let mut value_text = 0usize;
        for node in &mut c.stages {
            let StageNode::Act(inv) = node else { continue };
            if let Some(i) = inv.intention.as_mut() {
                strings.push((
                    &mut i.query,
                    MAX_INTENTION_QUERY_BYTES,
                    false,
                    String::new(),
                ));
            }
            if let Some(f) = inv.resource_filter.as_mut() {
                for p in &f.properties {
                    value_text +=
                        p.op.values()
                            .iter()
                            .map(worst_case_value_bytes)
                            .sum::<usize>();
                }
                for x in f.doc_type.iter_mut().chain(f.tags.iter_mut()) {
                    strings.push((x, MAX_FILTER_STRING_BYTES, true, String::new()));
                }
                for facet in &mut f.facets {
                    value_text += worst_case_string_bytes(&facet.value);
                    let head = facet.key.clone();
                    strings.push((&mut facet.key, MAX_FILTER_STRING_BYTES, true, head));
                }
                for p in &mut f.properties {
                    let head = p.key.clone();
                    strings.push((&mut p.key, MAX_FILTER_STRING_BYTES, true, head));
                }
                for x in [&mut f.stage, &mut f.status, &mut f.owner]
                    .into_iter()
                    .flatten()
                {
                    strings.push((x, MAX_FILTER_STRING_BYTES, true, String::new()));
                }
                if let Some(x) = f.title_contains.as_mut() {
                    strings.push((x, MAX_TITLE_CONTAINS_BYTES, true, String::new()));
                }
            }
            if let Some(f) = inv.edge_filter.as_mut() {
                for p in &f.properties {
                    value_text +=
                        p.op.values()
                            .iter()
                            .map(worst_case_value_bytes)
                            .sum::<usize>();
                }
                for x in &mut f.labels {
                    strings.push((x, MAX_FILTER_STRING_BYTES, true, String::new()));
                }
                for p in &mut f.properties {
                    let head = p.key.clone();
                    strings.push((&mut p.key, MAX_FILTER_STRING_BYTES, true, head));
                }
            }
        }
        let text: usize = value_text
            + strings
                .iter()
                .map(|(s, ..)| worst_case_string_bytes(s))
                .sum::<usize>();
        let mut slack = MAX_COMPOSITION_TEXT_BYTES
            .checked_sub(text)
            .expect("the fixture's text starts inside the budget");
        for (s, cap, escaped, head) in strings {
            let current = worst_case_string_bytes(s);
            // A control character is six bytes escaped and at worst, one byte decoded.
            let widest_worst = if escaped { cap } else { cap * 6 };
            let target = widest_worst.min(current + slack);
            if target <= current {
                continue;
            }
            let room = target - head.len();
            *s = format!("{head}{}{}", "\u{1}".repeat(room / 6), "x".repeat(room % 6));
            slack -= worst_case_string_bytes(s) - current;
        }
        c
    }

    fn capped_properties(preds: usize, vals: usize) -> Vec<PropertyPredicate> {
        (0..preds)
            .map(|i| PropertyPredicate {
                key: format!("p{i}"),
                op: PropertyOp::Contains {
                    values: (0..vals)
                        .map(|v| serde_json::json!(format!("v{v}")))
                        .collect(),
                },
            })
            .collect()
    }

    fn full_id_set() -> IdSet {
        IdSet {
            kind: IdKind::Resource,
            provenance: None,
            ids: (0..MAX_ID_SET_IDS).map(|_| uuid::Uuid::now_v7()).collect(),
        }
    }
}
