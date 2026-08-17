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
    ActDeclaration, ActName, ActQuantity, BuildState, Disclosure, Door, DoorReach, QuantityScale,
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
/// **The door half of the remainder is no longer held in a comment.** `[ruled — 2026-08-10, Pete]`
/// Both acts now declare [`DoorReach::Absent`] at all three doors, which is what the deployed system
/// is: a declaration describes the DEPLOYED system, and `unified_doors(vec![])` on an act nothing
/// reaches was the same defect that retracted `MatchLocation` and `input_contributed`. Only
/// `build_state` stays provisional here.
///
/// `[provisional — 2026-08-05; resolve in phase 4]`
/// `[unused — 2026-08-16]` No declaration carries this state today — `follow-from` and `survey`
/// both moved to `BuildState::Served` when their wrappers shipped. Kept rather than deleted because
/// a future act whose mechanic is live but door-stranded would need it again.
#[allow(dead_code)]
fn provisionally_unexpressed() -> BuildState {
    BuildState::Fused {
        host: "unified_search".to_string(),
    }
}

/// The `/api/search` door shape: all three doors present and serving. `cli_unreachable` is the
/// term-axis shortfall the CLI alone can carry — empty at every call site today, since `temper
/// search` now has a flag for every term the `find` acts admit — kept as a parameter rather than
/// dropped because it is the seam a later CLI-only shortfall would land in without a signature
/// change. **Three declarations write it** — the `find` acts, and only them.
///
/// The name is historical — it was minted when five acts were fused into one host. The host is gone
/// (retired 2026-08-06) and the doors did not move with it, which is why the shape survives it: the
/// three doors are `temper search`, `POST /api/search`, and the MCP `search` tool.
///
/// `follow-from` and `survey` used to carry this shape too, while [`provisionally_unexpressed`]
/// recorded in prose that no door in fact reaches their mechanic. That tension is **over**
/// `[ruled — 2026-08-10, Pete]`: both now declare their door reach directly — `follow-from`
/// restored to `Serves` on 2026-08-14 when its mechanic shipped, and `survey` restored on
/// 2026-08-16 when `query_survey` landed. Neither routes through here anymore.
///
/// `bounds_unreachable` is passed once for the same reason `cli_unreachable` is a parameter at all:
/// it is the seam a shortfall on this axis would land in without a signature change, kept even
/// though every call site now passes `vec![]`. `/api/search` gaining `bound_ids`, the MCP `search`
/// tool inheriting it for free through the shared `SearchParams`, and `temper search` gaining
/// repeatable `--within` closed the axis at all three doors at once — it is empty because every
/// door can supply what the two bounded acts accept, not because the axis is door-independent.
///
/// The MCP tool takes the whole [`crate::types::api::SearchParams`] as its `Parameters`, so every
/// wire field is reachable from it — worth stating because grepping the `temper-mcp` crate for a
/// param name finds nothing and reads as absence.
fn unified_doors(
    cli_unreachable: Vec<BoundTerm>,
    bounds_unreachable: Vec<IdKind>,
) -> BTreeMap<Door, DoorReach> {
    BTreeMap::from([
        (
            Door::Cli,
            DoorReach::Serves {
                terms_unreachable: cli_unreachable,
                bounds_unreachable: bounds_unreachable.clone(),
                filters_unapplied: vec![],
            },
        ),
        (
            Door::Api,
            DoorReach::Serves {
                terms_unreachable: vec![],
                bounds_unreachable: bounds_unreachable.clone(),
                filters_unapplied: vec![],
            },
        ),
        (
            Door::Mcp,
            DoorReach::Serves {
                terms_unreachable: vec![],
                bounds_unreachable,
                filters_unapplied: vec![],
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
        // `1.0 - shrunk_distance / 2.0`, and `<=>` cosine distance spans [0,2]. Contrast
        // `region_score`, which rescales the same operator's output as `1 - d` and lands in [-1,1].
        scale: QuantityScale::UnitInterval,
    }
}

/// The seven declarations. Order is stable — the generated contract renders them in this order.
pub fn search_family() -> Vec<ActDeclaration> {
    vec![
        ActDeclaration {
            name: ActName::FindExact,
            asker_holds: "I can quote the exact words".to_string(),
            served_by: Some("query_find_exact".to_string()),
            build_state: BuildState::Served,
            // Where the bound is applied cannot change WHICH resources come back — only how many
            // rows the scan touches.
            //
            // The reason is NOT the one this comment used to give. It read "the exact arm carries
            // no top-k, so nothing can be crowded out of it", which was true of `20260805000020`
            // and became FALSE the moment `20260806000020` put `LIMIT p_limit OFFSET p_offset`
            // inside the arm. What makes the position immaterial today is that
            // `query_find_exact` applies the bound BENEATH that ORDER BY/LIMIT
            // (`20260808000030`) — a property of where the conjunct sits, not an absence of
            // truncation. A stale rationale is worse than none: it invites the next reader to
            // conclude that adding a bound above the LIMIT would be safe.
            //
            // All three kinds are now true. `Resource` became honest with `p_bound_ids uuid[]`
            // (before the twins it named the one kind the fragment could NOT take); `Context` and
            // `Cogmap` were always accepted, through the anchor pair, and were simply omitted.
            accepts_bounds: vec![IdKind::Resource, IdKind::Context, IdKind::Cogmap],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Limit, BoundTerm::Offset],
            accepts_filters: vec![FilterField::Resource],
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            // Location NO, and not merely undeclared: the exact arm's index is ONE tsvector per
            // RESOURCE, built by concatenating every chunk into a single blob, so the block
            // boundary is gone before the query is asked — `located_at` is not a field it declines
            // to fill, it is a question its index cannot answer. (`InputContribution` was removed
            // from the vocabulary with its field — ratification ⟨6⟩/9d.)
            discloses: vec![],
            // The CLI's `search` command now has `--offset` beside `--context`, `--cogmap`,
            // `--doc-type`, `--limit`, `--text-only` and `--within` — so this act's term axis is
            // fully reachable from every door, and the BOUNDS axis below no longer holds it back
            // either: `Served`, reachable from every door on both axes. A `BuildState` variant
            // could never have carried the door-partial case this axis used to record —
            // door-partiality is orthogonal to build state — which is the whole reason the axis
            // exists, even now that it has closed.
            //
            // `bounds_unreachable: []` at ALL THREE doors is that closing. This act declares
            // `accepts_bounds: [Resource, Context, Cogmap]`; `Context` and `Cogmap` were always
            // reachable through `context_ref` / `cogmap_id` / `cogmap_ids`. `Resource` was the
            // gap: `SearchParams` gained `bound_ids`, the MCP tool inherits it because it takes
            // the whole `SearchParams` as its `Parameters`, and `temper search` gained repeatable
            // `--within` — so every door now reaches the fragment's `p_bound_ids uuid[]`, and the
            // act's acceptance is no longer aspirational anywhere.
            door_coverage: unified_doors(vec![], vec![]),
            orders_by: Some(ActQuantity {
                field: "fts_norm".to_string(),
                means: "postgres ts_rank of the query against the resource's own search vector — \
                        document-local, so it does not move with who is asking"
                    .to_string(),
                // Flag 33 = 1 | 32, and flag 32 is `rank / (rank + 1)`. The `_norm` in the column
                // name is earned, unlike `origin`'s claim to name the producing arm.
                scale: QuantityScale::UnitInterval,
            }),
            visibility_profile: Some(VisibilityProfile::PrincipalRelative),
            scoring_revision: 2, // ts_rank flag 32 -> 33, migration 20260801000010
        },
        ActDeclaration {
            name: ActName::FindAboutAnywhere,
            asker_holds: "a concept, no exact words; search everything I can see".to_string(),
            served_by: Some("query_find_wide".to_string()),
            build_state: BuildState::Served,
            // A bound would make this find-about-within. Definitional exclusion, not a hole.
            accepts_bounds: vec![],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Limit, BoundTerm::Offset],
            accepts_filters: vec![FilterField::Resource],
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            // `[ruled — 2026-08-10, Pete, ⟨4⟩/8e]` `MatchLocation` is undeclared until the wide
            // fragments carry the winning chunk's identity out — a declaration describes the
            // DEPLOYED system, and the executor hard-codes `located_at: None` today. The wide arm
            // matches at CHUNK grain and already computes which chunk was closest, then discards
            // it collapsing to a per-resource score; redeclaring is additive when the argmin ships.
            discloses: vec![],
            // No `bounds_unreachable`, and it is the empty list that carries the statement: this
            // act accepts NO bounds by definition, so there is no kind for a door to fall short on
            // — a different reason than the find acts either side of it, which now also declare an
            // empty list, but because every door CAN supply the bound they accept.
            door_coverage: unified_doors(vec![], vec![]),
            orders_by: Some(vec_norm_quantity()),
            visibility_profile: Some(VisibilityProfile::PrincipalRelative),
            scoring_revision: 2, // best-of-N shrunk toward the chunk mean, 20260801000010
        },
        ActDeclaration {
            name: ActName::FindAboutWithin,
            asker_holds: "a concept, plus a set to search inside".to_string(),
            served_by: Some("query_find_wide".to_string()),
            build_state: BuildState::Served,
            accepts_bounds: vec![IdKind::Resource, IdKind::Context, IdKind::Cogmap],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Limit, BoundTerm::Offset],
            accepts_filters: vec![FilterField::Resource],
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            // The same wide arm as `find-about-anywhere`, so the same ruling: `MatchLocation` is
            // undeclared until the fragments carry the winning chunk out
            // `[ruled — 2026-08-10, Pete, ⟨4⟩/8e]`.
            discloses: vec![],
            // The bound is the whole point of this act — "a concept, plus a set to search inside" —
            // and `Resource`, the one kind a caller most obviously holds a set of (40 hits from a
            // previous search), is now a kind every door can supply. `Context` and `Cogmap` reached
            // it all along through `context_ref` / `cogmap_id`. Same fix as `find-exact`, and the
            // same cause: `SearchParams.bound_ids` plus repeatable `temper search --within`.
            door_coverage: unified_doors(vec![], vec![]),
            orders_by: Some(vec_norm_quantity()),
            visibility_profile: Some(VisibilityProfile::PrincipalRelative),
            scoring_revision: 2,
        },
        ActDeclaration {
            name: ActName::FindResourcesWith,
            asker_holds: "I can say what these things ARE; I have no question about what they mean"
                .to_string(),
            served_by: Some("query_find_resources_with".to_string()),
            build_state: BuildState::Served,
            // **Anchors yes, resource sets no**, and the asymmetry is the point.
            //
            // A `Resource` bound would be a second spelling of `CombineOp::Intersect`: narrowing a
            // selection by an upstream set is set intersection, the combinator already does it, and
            // two selections piped together are just one selection carrying both predicates. This
            // act composes INTO a find act — its output is the bound a later stage consumes.
            //
            // An ANCHOR is not an id set and cannot be reached that way. Nothing produces "the
            // resources homed in this context" as a set, so without these two kinds *"every task in
            // @me/temper"* is inexpressible — which is the capability this act exists to add,
            // scoped. Served by the `(anchor_table, anchor_id)` pair exactly as the find acts are.
            accepts_bounds: vec![IdKind::Context, IdKind::Cogmap],
            accepts_seeds: vec![],
            // No `Limit`. A selection that truncates is not a selection — it is a sample, and a
            // sample piped into a find act would bound that act to an arbitrary subset while
            // looking like a narrowing. Whatever ceiling applies belongs on the act that RETURNS
            // rows, where the caller can see it disclosed.
            accepts_bound_terms: vec![],
            accepts_filters: vec![FilterField::Resource],
            bound_ceilings: BTreeMap::new(),
            produces: Some(IdKind::Resource),
            // Selects without scoring. `discloses` describes what a stage says about its own
            // output beyond the rows, and this stage has no rows — only ids.
            discloses: vec![],
            // **The first act whose doors are `/api/query`'s rather than `/api/search`'s**, which is
            // why this is written out instead of routing through `unified_doors` — that helper's
            // three doors are `temper search`, `POST /api/search` and the MCP `search` tool, and
            // none of them reaches this act. [`Door`] names a SURFACE, not an endpoint, so the same
            // three variants answer for a different endpoint here.
            //
            // CLI and API serve it in full. `temper query` takes a raw JSON plan
            // (`query_cmd.rs` → `resolve_plan`), so every wire field is reachable from it by
            // construction and there is no term, bound kind or filter it cannot express — which is
            // a stronger statement than the find acts' empty shortfall lists, and true for a
            // different reason: theirs is a flag surface that had to be brought level, this is a
            // document.
            //
            // **MCP is `Absent`, and that is the shortfall worth naming**: the MCP server exposes a
            // `search` tool and no `query` tool, so the door agents use cannot compose at all. Same
            // shape as `substantiate` being absent from exactly the door that most needs it. Not a
            // gap this act introduces, and not one it can close.
            door_coverage: BTreeMap::from([
                (
                    Door::Cli,
                    DoorReach::Serves {
                        terms_unreachable: vec![],
                        bounds_unreachable: vec![],
                        filters_unapplied: vec![],
                    },
                ),
                (
                    Door::Api,
                    DoorReach::Serves {
                        terms_unreachable: vec![],
                        bounds_unreachable: vec![],
                        filters_unapplied: vec![],
                    },
                ),
                (Door::Mcp, DoorReach::Absent),
            ]),
            // THE POINT OF THIS ACT, stated where the invariants read it. It orders nothing —
            // there is no quantity, because "has doc_type task" is not more or less true of one
            // resource than another. A selection returns a SET; ranking it would require inventing
            // a preference the caller never expressed.
            orders_by: None,
            // Follows `orders_by`, as `an_act_that_orders_nothing_classifies_no_ordering_fragment`
            // requires: the field describes the fragment that ORDERS this act's output, and there
            // is no such fragment.
            visibility_profile: None,
            scoring_revision: 0,
        },
        ActDeclaration {
            name: ActName::FollowFrom,
            // **A found SET, and the plural is the whole reason `via` exists**
            // `[rewritten — 2026-08-14, Pete]`. This said "a found thing", while the act declares
            // `accepts_seeds: vec![IdKind::Resource]` — an id set — and its own `means` already
            // says "from any seed". A found thing is always a set, even a set of one; and for a
            // multi-seed walk a bare `(node, score)` row cannot say which seed a neighbour belongs
            // to, which is the disclosure this act now carries.
            asker_holds: "found things; I want their neighbours, and which of them each came from"
                .to_string(),
            // `query_follow_from`, never `search_graph_expand`: the map this name is looked up in
            // is keyed GATED entry point -> ungated core, and the incumbent is now a third,
            // shape-preserving wrapper that delegates to this one (`20260814000030`).
            served_by: Some("query_follow_from".to_string()),
            // `[amended — 2026-08-16]` was `provisionally_unexpressed()` (`Fused { host:
            // "unified_search" }`), which was stale on two counts: `unified_search` was retired
            // (`20260806000020`) and `query_follow_from` is a standalone function. The
            // `door_coverage` was updated to `Serves` at CLI and API on 2026-08-14, but
            // `build_state` was left behind — the same known-false declaration this field exists
            // to stop. Now `Served`, matching the door coverage and `CALLABLE_FRAGMENTS`.
            build_state: BuildState::Served,
            // **The one genuine foreclosure, now closed** `[2026-08-14]`. This read: "Bounded
            // follow-from is UNBUILT: search_graph_expand has no scope parameter, so 'walk from
            // these seeds but stay inside this set' is unstatable."
            //
            // Two things had to land, and the second was found only at build time. The fragment
            // gained `p_bound_ids` (`20260814000030`), applied where visibility is applied so it
            // constrains INTERMEDIATE nodes and not merely the returned set — the output-only
            // reading is `CombineOp::Intersect` and would be a second spelling of a combinator.
            // Then the WIRE had to carry two sets at once: a bounded walk needs seeds to grow from
            // and a bound to stay within, and `ActInvocation.input` was one slot with one relation
            // until it became `inputs: Vec<StageInput>`. Declaring this before that widening would
            // have named a capability no caller could express.
            accepts_bounds: vec![IdKind::Resource],
            accepts_seeds: vec![IdKind::Resource],
            // **`Offset` joins `Limit`** `[amended — 2026-08-17]`. This read
            // `vec![BoundTerm::Limit]`: the only row-returning act in the family admitting a page
            // SIZE and no page NUMBER, which made its published ceiling of 50 an unpageable horizon
            // rather than merely a small page. The three find acts publish the same 50 and can walk
            // past it a page at a time; this act could not, so a node with more than 50 neighbours
            // of the asked kind could never be walked in full and `{"offset": 50}` was refused
            // outright as `BoundTermNotApplicable`. The damage is downstream and silent:
            // walk-then-narrow (`follow-from` -> `intersect` -> rank) computes the intersect against
            // an arbitrary 50-element subset and returns an answer that is plausible, well-formed
            // and wrong.
            //
            // **A stable total page order already exists, and none was invented for this.** The
            // fragment's `ranked` CTE orders `MAX(w.score) DESC, w.node` before it truncates
            // (`20260817000010_decompose_walk.sql:85`, the current body — `20260814000030` was
            // superseded when the walk was decomposed and no longer describes it), so node id is
            // the tiebreak and page 2 is exactly the rows page 1 did not take. The ordering field
            // is `graph_score`, which `orders_by` below already describes: paging here is defined
            // over a quantity this act publishes, not over an incidental scan order.
            accepts_bound_terms: vec![BoundTerm::Limit, BoundTerm::Offset],
            accepts_filters: vec![FilterField::Edge],
            // The `Limit` ceiling stays 50 — the ceiling was never the defect; a ceiling with no
            // page number was. And `Offset` gets NO ceiling, as on every other act that pages:
            // `applied_terms` reads absence here as *no ceiling*, and it is that same absence which
            // keeps an omitted offset from acquiring a default — *"`Offset` has no ceiling on any
            // act, so there is nothing for it to default to — and page 1 is the right answer to a
            // caller who named no page"* (`applied_terms`' doc). A ceiling on the page NUMBER would
            // reinstate the very horizon this amendment removes.
            bound_ceilings: BTreeMap::from([(BoundTerm::Limit, 50)]),
            produces: Some(IdKind::Resource),
            // No location: it returns nodes reached by walking, not chunks matched by a query, so
            // there is no match position for it to have.
            //
            // **`InputContribution` is here because the walk stopped discarding origin**
            // `[2026-08-14]`. This comment used to record the opposite — that the walk collapsed to
            // `SELECT node, MAX(score) ... GROUP BY node` and threw the path away, which was the
            // fact behind that disclosure's retirement. The sibling projects it instead: one row
            // per node carrying every edge it was reached by, as asserted.
            discloses: vec![Disclosure::InputContribution],
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
            // ABSENT AT EVERY DOOR `[ruled — 2026-08-10, Pete, ADJ-9a]`. This said
            // `unified_doors(vec![])` — full reach at CLI, API and MCP — and no door reaches this
            // act at all: `/api/search` calls only the two find fragments, and nothing outside
            // temper-substrate's tests calls `search_graph_expand`. The mechanic is live and
            // stranded, which `build_state` records; `door_coverage` answers a different question,
            // and the answer is none. A declaration describes the DEPLOYED system — the same
            // principle that retracted `MatchLocation` and `input_contributed` rather than holding
            // them until their fragments caught up. This restores to `Serves` when `/api/query`
            // gives the act a door, which is additive.
            //
            // Note what goes with it. The `Limit` term and the `Edge` filter this act declares are
            // now shortfalls with nowhere to be declared, because `Absent` carries no lists — which
            // is correct: a door that cannot invoke the act at all falls short on nothing in
            // particular. The `accepts_filters: [Edge]` that today's validator refuses outright is
            // recorded there, in `validate`'s FilterNotApplicable arm, not here.
            // **Absent -> Serves at CLI and API** `[2026-08-14]`, on the condition the previous
            // comment set: *"This restores to `Serves` when `/api/query` gives the act a door,
            // which is additive."* `query_follow_from` is in `CALLABLE_FRAGMENTS` and the compiler
            // emits its core, so both doors can now invoke it — `temper query` takes a raw JSON
            // plan, so every wire field is reachable from it by construction.
            //
            // **MCP stays `Absent`, and that is not this act's shortfall to close.** The MCP server
            // exposes a `search` tool and no `query` tool, so the door agents use cannot compose at
            // all. Flipping this to `Serves` because the mechanic exists would make the declaration
            // describe the code rather than the DEPLOYED system, which is the whole reason the
            // three were set to `Absent` in the first place.
            //
            // **`Offset` enters no `terms_unreachable` list** `[2026-08-17]`. The two serving doors
            // take a raw JSON plan (`temper query` -> `resolve_plan`, and `/api/query` takes the
            // `Composition` itself), so every wire field is reachable at them by construction — the
            // sentence two paragraphs up, now load-bearing for a second term rather than only for
            // `Limit`. A newly admitted term reaches those doors the moment it is admitted; there is
            // no flag or param to add first, which is what distinguishes these doors from the find
            // acts' `temper search`, where `--offset` had to ship before the axis could close. MCP
            // carries no list at all because `Absent` has none: a door that cannot invoke the act
            // falls short on nothing in particular.
            door_coverage: BTreeMap::from([
                (
                    Door::Cli,
                    DoorReach::Serves {
                        terms_unreachable: vec![],
                        bounds_unreachable: vec![],
                        filters_unapplied: vec![],
                    },
                ),
                (
                    Door::Api,
                    DoorReach::Serves {
                        terms_unreachable: vec![],
                        bounds_unreachable: vec![],
                        filters_unapplied: vec![],
                    },
                ),
                (Door::Mcp, DoorReach::Absent),
            ]),
            orders_by: Some(ActQuantity {
                field: "graph_score".to_string(),
                means: "the best decayed path from any seed to this node — \
                        MAX(gamma^hop * product of edge weights) over walks of at least one hop"
                    .to_string(),
                // NOT [0,1], and not merely un-normalized: `kb_edges.weight` is
                // `DOUBLE PRECISION NOT NULL DEFAULT 1.0` with NO CHECK constraint, and the walk
                // multiplies weights at every hop — so any edge written with a weight above 1 lifts
                // this above 1. Today's corpus stays under it because nothing writes such a weight,
                // which is a property of the DATA, not of the quantity; declaring `UnitInterval`
                // would claim the schema enforces something it does not.
                scale: QuantityScale::Unbounded,
            }),
            visibility_profile: Some(VisibilityProfile::AgnosticInValueRelativeInDomain),
            scoring_revision: 1,
        },
        ActDeclaration {
            name: ActName::Survey,
            asker_holds: "a question about what a scope knows".to_string(),
            // `[amended — 2026-08-16]` was `wayfind_region_scores`; the act now has its own
            // wrapper (`query_survey`, migration 20260816000020) that calls wayfind with
            // `p_lens = NULL` and joins matched regions to member resources. The `served_by`
            // points at the wrapper, not the underlying scorer, because the fingerprint and
            // reachability rules key on this name.
            served_by: Some("query_survey".to_string()),
            build_state: BuildState::Served,
            // Takes (p_anchor_table, p_anchor_id) — an anchor, which a typed IdSet can name.
            accepts_bounds: vec![IdKind::Cogmap, IdKind::Context],
            accepts_seeds: vec![],
            accepts_bound_terms: vec![BoundTerm::Regions],
            accepts_filters: vec![],
            bound_ceilings: BTreeMap::from([(BoundTerm::Regions, 20)]),
            // `[amended — 2026-08-16]` was `Region`; the ratified ⟨3⟩ redesign produces the
            // member RESOURCES of matched regions, not the regions. Regions move to `discloses`.
            produces: Some(IdKind::Resource),
            // The region each resource came from is trace disclosure, not the primary output.
            discloses: vec![Disclosure::Region],
            // `[amended — 2026-08-16]` was ABSENT AT EVERY DOOR. The act now has a door: the
            // `query_survey` wrapper is wired into `CALLABLE_FRAGMENTS` and the compiler emits
            // it. Survey is a knowledge subject (resources), so it reaches all three doors per
            // `subject-decides-the-door`. No term-axis shortfall at any door — `Regions` is the
            // only bound term and all three doors admit it.
            door_coverage: unified_doors(vec![], vec![]),
            orders_by: Some(ActQuantity {
                field: "region_score".to_string(),
                means: "alpha * sal_norm + beta * query_cos + kappa * prior, with alpha 0.4, beta \
                        0.6 and kappa 0.05 — the region's per-kind salience rank, blended with its \
                        centroid's similarity to the query, plus an anchor-kind prior of 1.0 for a \
                        region homed on a cogmap and 0.6 for one homed on a context. Resources \
                        within a matched region are ranked by the resource's own embedding \
                        similarity to the query (query_cos at a finer grain), not the region's \
                        centroid similarity."
                    .to_string(),
                // The surprise, and the reason this variant exists. `sal_norm` is a `percent_rank`
                // in [0,1], but `query_cos` is `1 - (centroid <=> p_emb)` and a cosine DISTANCE
                // spans [0,2], so the similarity spans [-1,1]. The composite CAN BE NEGATIVE, and
                // every discussion of this number in the arc's research treats it as a [0,1] score.
                //
                // `[corrected — 2026-08-10, ADJ-9e]` The declaration said `0.4*sal + 0.6*cos` over
                // `[-0.6, 1.0]` and the deployed function has a THIRD term: the `kappa * prior`
                // anchor-kind bonus. It is small and it is always present, so it moves both ends —
                // the span is 0.4*[0,1] + 0.6*[-1,1] + 0.05*{0.6, 1.0}, i.e. [-0.57, 1.05], which
                // exceeds 1. A declaration a caller normalises against must not be wrong about its
                // own range, and this one was wrong at both ends and in the wrong direction at the
                // top: it promised a value that could never exceed 1.0.
                //
                // Note what this is next to: `vec_norm` rescales the SAME `<=>` operator as
                // `1 - d/2` into [0,1]. Two rescales of one distance, in one search family, with
                // neither column name disclosing which it is.
                //
                // `[amended — 2026-08-16]` The within-region ranking reuses `query_cos` at a
                // finer grain (the resource's own embedding, not the region's centroid). This
                // does NOT change the `region_score` formula or its range — the blend is the
                // sal_norm open ruling, which stands. The within-resource `query_cos` spans
                // [-1,1] by the same reasoning, and is a disclosed quantity on each resource row.
                scale: QuantityScale::OtherRange {
                    bounds: "[-0.57, 1.05]".to_string(),
                },
            }),
            visibility_profile: Some(VisibilityProfile::AgnosticInValueRelativeInDomain),
            scoring_revision: 1,
        },
        ActDeclaration {
            name: ActName::Substantiate,
            asker_holds: "a claim; I want its defensibility".to_string(),
            // CORRECTED 2026-08-05 (was `None` / `Unbuilt`). This act SHIPS, and the declaration
            // said no mechanic exists: `GET /api/resources/{id}/evidence` calls
            // `evidential_standing_service::resource_evidence`, which reads SQL
            // `resource_standing_shape`. `temper resource evidence <ref>` is the CLI door.
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
            // Not composable, so it has no stage to disclose anything ABOUT. It annotates one
            // resource rather than selecting a set — `accepts_bounds: []` at the input end and
            // `produces: None` at the output end — and is served at its own door. An empty list
            // here is the same statement its other fields already make.
            discloses: vec![],
            // Every shortfall list is empty and every one of them is a real statement: this act
            // admits no bound terms, no bound kinds and no filters, so on all three axes there is
            // nothing for the two doors that serve it to fall short on.
            door_coverage: BTreeMap::from([
                (
                    Door::Cli,
                    DoorReach::Serves {
                        terms_unreachable: vec![],
                        bounds_unreachable: vec![],
                        filters_unapplied: vec![],
                    },
                ),
                (
                    Door::Api,
                    DoorReach::Serves {
                        terms_unreachable: vec![],
                        bounds_unreachable: vec![],
                        filters_unapplied: vec![],
                    },
                ),
                // Absent from MCP, which is the door agents use — so the substantiate act is
                // thinnest exactly where it is most needed. Nothing in `crates/temper-mcp` reads
                // standing; T1 columns 1-3 §6.2 recorded the same.
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
            // The anti-act. It runs nothing, so there is nothing it could disclose.
            discloses: vec![],
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
/// The value each admitted term will ACTUALLY run with — the caller's, clamped to the act's
/// published ceiling.
///
/// **The one definition, and it exists because there are two consumers who must not disagree.** The
/// compiler binds these into the statement; the assembler reports them as
/// [`super::envelope::StageResult::terms_applied`]. Computed twice, they would eventually differ,
/// and the difference would be a response claiming a page size that did not run — the quiet kind of
/// wrong this surface is built against.
///
/// There is deliberately **no "you were clamped" flag**. Ceilings are published per act, so the
/// applied value is the whole story; clamping to a ceiling nobody published would be the bug, not
/// the silence. A term the act does not admit is not clamped either — it is refused outright at
/// validation, and never reinterpreted to fit.
///
/// A term with no ceiling passes through unchanged: absence here means *no ceiling*, not *zero*.
///
/// # An omitted `limit` DEFAULTS to the ceiling, and is reported
///
/// `[ruled — 2026-08-10, Pete, ADJ-11]` This used to iterate only the terms the caller REQUESTED,
/// so an omitted `limit` never acquired a value at all: the compiler bound `NULL` and Postgres read
/// `LIMIT NULL` as *unbounded* — the whole visible match set, per stage, for a caller who simply did
/// not say how many they wanted. An act that publishes a ceiling of 50 and then returns everything
/// on the one request shape agents send most is not a permissive default, it is a missing one.
///
/// **A default and a clamp are different events and both are honestly reported as "what ran".** The
/// no-flag rationale above is about a clamp: a value the caller sent, reduced against a ceiling they
/// could have read. A default is a value the caller never sent, so there is nothing to compare it
/// against and nothing for them to have read wrong — but `terms_applied` means *the value actually
/// used*, which is exactly what a default is. A response that cannot account for its own row count
/// is the worse failure, so the default appears there beside the clamped values, indistinguishable
/// from them on the wire and identical in meaning: this is the number the statement ran with.
///
/// Only `Limit` defaults, and only from a published ceiling. `Regions` deliberately does not:
/// `wayfind_region_scores` has its own funnel default (3) beneath a ceiling of 20, and defaulting to
/// the ceiling here would widen every unbounded survey sevenfold while claiming to describe the
/// deployed system. `Offset` has no ceiling on any act, so there is nothing for it to default to —
/// and page 1 is the right answer to a caller who named no page.
pub fn applied_terms(
    requested: &std::collections::BTreeMap<super::scalars::BoundTerm, i64>,
    decl: &ActDeclaration,
) -> std::collections::BTreeMap<super::scalars::BoundTerm, i64> {
    let mut applied: std::collections::BTreeMap<super::scalars::BoundTerm, i64> = requested
        .iter()
        .filter(|(term, _)| decl.accepts_bound_terms.contains(term))
        .map(|(term, asked)| {
            let value = match decl.bound_ceilings.get(term) {
                Some(ceiling) => (*asked).min(*ceiling),
                None => *asked,
            };
            (*term, value)
        })
        .collect();

    // The default is written HERE, in the one definition the compiler and the assembler both read,
    // rather than in either of them. Computed twice it would eventually differ, and the difference
    // would be a response reporting a page size the statement did not run with.
    if decl.accepts_bound_terms.contains(&BoundTerm::Limit)
        && !applied.contains_key(&BoundTerm::Limit)
    {
        if let Some(ceiling) = decl.bound_ceilings.get(&BoundTerm::Limit) {
            applied.insert(BoundTerm::Limit, *ceiling);
        }
    }
    applied
}

pub fn declaration(name: &ActName) -> Option<ActDeclaration> {
    search_family().into_iter().find(|a| &a.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `[widened — 2026-08-14]` Eight, was seven. `find-resources-with` is the family's first act
    /// that asks the corpus nothing — every other member carries a question about meaning, or (in
    /// `follow-from`'s and `survey`'s case) a shape to walk. Worth naming rather than bumping the
    /// count past: a selection is a different KIND of member, not a seventh of the same kind.
    #[test]
    fn the_search_family_declares_eight_acts_including_the_anti_act() {
        let acts = search_family();
        assert_eq!(acts.len(), 8);
        let names: Vec<&ActName> = acts.iter().map(|a| &a.name).collect();
        for expected in [
            ActName::FindExact,
            ActName::FindAboutAnywhere,
            ActName::FindAboutWithin,
            ActName::FindResourcesWith,
            ActName::FollowFrom,
            ActName::Survey,
            ActName::Substantiate,
            ActName::Admit,
        ] {
            assert!(names.contains(&&expected), "missing {expected:?}");
        }
    }

    #[test]
    fn the_served_set_is_the_five_query_acts_plus_substantiate() {
        // WAS `substantiate_is_the_only_act_with_a_door_of_its_own`, and before that
        // `nothing_in_the_search_family_is_served`, whose comment read "every mechanic is reachable
        // only through unified_search". That sentence was FALSE about the deployed system while its
        // assertion passed — the same shape as `survey_is_the_only_act_relative_in_domain`, and the
        // same safe direction. It is recorded here because the set has now moved twice.
        //
        // Phase 1 steps 2-3 gave the three `find` acts doors of their own: `/api/search` invokes
        // their mechanic directly, neither fused into anything. `[re-cited — 2026-08-12]` That
        // mechanic is now `query_find_exact` / `query_find_wide`, not `search_exact` /
        // `search_wide` — the read path was repointed at the bound-accepting twins when
        // `/api/search` gained a resource bound, and `served_by` above followed it. The CLAIM is
        // unchanged (a door of its own, nothing fused); only the function it names moved.
        // `substantiate` keeps the door it has had since Set 5 (`GET /api/resources/{id}/evidence`).
        //
        // `[moved — 2026-08-14]` `find-resources-with` joins, and it is the first member whose door
        // is `/api/query` rather than one of its own — served by `query_find_resources_with`
        // (migration `20260814000010`) and emitted through `CALLABLE_FRAGMENTS`. It went `Unbuilt`
        // to `Served` within one branch, which is not a gap in the record: `Unbuilt` while only the
        // contract existed was a TRUE statement about the deployed system, and holding it there
        // once the fragment shipped would have been the same known-false declaration this field was
        // added to stop.
        //
        // `[moved — 2026-08-16]` `survey` joins, served by `query_survey` (migration
        // `20260816000020`). It went `Fused` to `Served` when the `p_lens` blocker was settled
        // (`NULL` is correct) and the wrapper shipped. Same day, `follow-from`'s `build_state` was
        // corrected from `Fused` to `Served` — its `door_coverage` was already `Serves` at CLI and
        // API since 2026-08-14, but `build_state` was left stale. Both are now `Served`.
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
                ActName::FindResourcesWith,
                ActName::FollowFrom,
                ActName::Survey,
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
        //
        // `[widened — 2026-08-10, ADJ-9b]` The same guard now covers all three shortfall axes.
        // Written as one loop over one destructuring rather than three tests, because the property
        // is one property — *a shortfall names something the act admits* — and splitting it would
        // let a fourth axis land guarded on two of three without anything saying so.
        for a in search_family() {
            for (door, reach) in &a.door_coverage {
                let DoorReach::Serves {
                    terms_unreachable,
                    bounds_unreachable,
                    filters_unapplied,
                } = reach
                else {
                    continue;
                };
                for term in terms_unreachable {
                    assert!(
                        a.accepts_bound_terms.contains(term),
                        "{:?} claims {door:?} cannot reach {term:?}, which it does not admit",
                        a.name
                    );
                }
                for kind in bounds_unreachable {
                    assert!(
                        a.accepts_bounds.contains(kind),
                        "{:?} claims {door:?} cannot supply a {kind:?} bound, which it does not \
                         admit",
                        a.name
                    );
                }
                for field in filters_unapplied {
                    assert!(
                        a.accepts_filters.contains(field),
                        "{:?} claims {door:?} leaves a {field:?} filter unapplied, which it does \
                         not admit",
                        a.name
                    );
                }
            }
        }
    }

    /// Every door can now supply the resource bound the find acts accept, and that is declared.
    ///
    /// The shortfall `terms_unreachable` alone could not express, and the reason the axis exists.
    /// `find-exact` and `find-about-within` both declare `accepts_bounds: [Resource, Context,
    /// Cogmap]`. `Context` and `Cogmap` were always reachable through `context_ref` / `cogmap_id` /
    /// `cogmap_ids`; `Resource` — the one kind a caller most obviously holds a set of — was not,
    /// until `/api/search` gained `bound_ids`, the MCP `search` tool inherited it for free through
    /// the shared `SearchParams`, and `temper search` gained repeatable `--within`. All three doors
    /// closed at once rather than door by door.
    ///
    /// `find-about-anywhere` still declares an empty list for the OTHER reason: it accepts no
    /// bounds at all. An empty list there is "nothing to fall short on", a different statement from
    /// an empty list here meaning "every door can supply what this act accepts" — which is why the
    /// two cases are asserted separately below rather than folded into one loop.
    ///
    /// Asserted per door rather than once, because the claim is per door: an act reachable
    /// everywhere and full in the same way everywhere is a different statement than one door
    /// lagging.
    #[test]
    fn every_door_can_now_supply_the_resource_bound_the_find_acts_accept() {
        // Counted rather than merely non-empty: this branch has twice shipped a loop that iterated
        // nothing and stayed green. Two acts times three doors, or the gate has silently emptied.
        let mut doors_checked = 0usize;
        for name in [ActName::FindExact, ActName::FindAboutWithin] {
            let a = declaration(&name).unwrap();
            assert!(a.accepts_bounds.contains(&IdKind::Resource));
            for door in Door::ALL {
                let Some(DoorReach::Serves {
                    bounds_unreachable, ..
                }) = a.door_coverage.get(&door)
                else {
                    panic!("{name:?} must serve {door:?}");
                };
                assert!(
                    bounds_unreachable.is_empty(),
                    "{name:?} at {door:?} still declares {bounds_unreachable:?} unreachable"
                );
                doors_checked += 1;
            }
        }
        assert_eq!(
            doors_checked,
            2 * Door::ALL.len(),
            "must check both bounded find acts at every door — a lower count means this loop is \
             iterating less than it claims to"
        );

        // And the act that accepts no bounds declares no shortfall — an empty list here is the
        // statement that there is nothing to fall short on, not a door that was forgotten.
        let anywhere = declaration(&ActName::FindAboutAnywhere).unwrap();
        assert!(anywhere.accepts_bounds.is_empty());
        for door in Door::ALL {
            let Some(DoorReach::Serves {
                bounds_unreachable, ..
            }) = anywhere.door_coverage.get(&door)
            else {
                panic!("find-about-anywhere must serve {door:?}");
            };
            assert!(
                bounds_unreachable.is_empty(),
                "an act that accepts no bounds cannot have a bound a door falls short on"
            );
        }
    }

    /// The act that was stranded now reaches every door.
    ///
    /// `[ruled — 2026-08-10, Pete, ADJ-9a]` `follow-from` and `survey` both declared
    /// `unified_doors(vec![])` — full reach at CLI, API and MCP — while `/api/search` calls only the
    /// two find fragments and nothing outside temper-substrate's tests calls `search_graph_expand`
    /// or `wayfind_region_scores`. Their mechanics were live and stranded, which is `build_state`'s
    /// business; `door_coverage` answers *can a caller standing here ask this*, and the answer was
    /// none the whole time.
    ///
    /// `[narrowed to one — 2026-08-14]` **`follow-from` is no longer stranded** — it reaches
    /// `/api/query` at CLI and API.
    ///
    /// `[closed — 2026-08-16]` **`survey` is no longer stranded either.** The `query_survey`
    /// wrapper (migration 20260816000020) gives it a door, and the `p_lens` blocker was settled
    /// (`NULL` is correct — the lens is a clustering-time parameter). Survey is a knowledge subject
    /// (resources), so `subject-decides-the-door` requires all three doors. This test was the
    /// stranded-mechanic assertion; it is now the served-at-every-door assertion, kept as an exact
    /// set so an act losing a door must be a deliberate edit here.
    #[test]
    fn the_survey_act_reaches_every_door() {
        let a = declaration(&ActName::Survey).unwrap();
        assert!(
            a.served_by.is_some(),
            "survey names a live mechanic — query_survey"
        );
        for door in Door::ALL {
            assert!(
                matches!(a.door_coverage.get(&door), Some(DoorReach::Serves { .. })),
                "survey must serve {door:?}"
            );
        }
    }

    /// `follow-from` reaches the two composing doors and **not** MCP, and the asymmetry is the
    /// point rather than an oversight `[2026-08-14]`.
    ///
    /// `temper query` takes a raw JSON plan, so every wire field is reachable from the CLI by
    /// construction — hence empty shortfall lists rather than a list nobody maintains. MCP exposes a
    /// `search` tool and no `query` tool, so the door agents actually use cannot compose at all;
    /// that is a gap this act cannot close and must not paper over by declaring reach it does not
    /// have.
    #[test]
    fn follow_from_reaches_the_composing_doors_and_not_mcp() {
        let a = declaration(&ActName::FollowFrom).unwrap();
        for door in [Door::Cli, Door::Api] {
            assert!(
                matches!(a.door_coverage.get(&door), Some(DoorReach::Serves { .. })),
                "{door:?} compiles a composition, so it reaches this act"
            );
        }
        assert_eq!(
            a.door_coverage.get(&Door::Mcp),
            Some(&DoorReach::Absent),
            "MCP exposes no `query` tool; the mechanic existing does not give that door reach"
        );
    }

    #[test]
    fn the_cli_can_now_page_the_find_acts_and_that_is_declared() {
        // The concrete parity gap that forced door coverage to be its own axis rather than a
        // `BuildState` variant — door-partiality is orthogonal to build state, so no
        // `BuildState` variant could ever have carried it. The gap is now CLOSED: `temper search`
        // gained `--offset`, so the axis records full term reach rather than a shortfall.
        //
        // Kept rather than deleted. The axis's value was never the non-empty entry; it is that
        // the declaration and the parser are held to each other, which
        // `the_cli_term_shortfall_is_what_clap_actually_lacks` now checks in the direction that
        // has content — every admitted term must have a flag.
        for name in [
            ActName::FindExact,
            ActName::FindAboutAnywhere,
            ActName::FindAboutWithin,
        ] {
            let a = declaration(&name).unwrap();
            assert_eq!(a.build_state, BuildState::Served);
            // Read the term axis alone: `..` keeps this from silently becoming a second
            // assertion about the bound axis, which
            // `every_door_can_now_supply_the_resource_bound_the_find_acts_accept` owns.
            let Some(DoorReach::Serves {
                terms_unreachable, ..
            }) = a.door_coverage.get(&Door::Cli)
            else {
                panic!("{name:?} must serve the CLI door");
            };
            assert!(
                terms_unreachable.is_empty(),
                "{name:?} declares {terms_unreachable:?} unreachable at the CLI, but \
                 `temper search` now accepts every term it admits"
            );
        }
    }

    /// Match location is declared by NO act — `[ruled — 2026-08-10, Pete, ⟨4⟩/8e]`.
    ///
    /// A declaration describes the DEPLOYED system, and the executor hard-codes
    /// `located_at: None` today: the wide fragments compute which chunk was closest and then
    /// discard it collapsing to a per-resource score, so nothing carries the winning chunk's
    /// identity out. When the argmin ships, redeclaring is additive — and this test is the edit
    /// that has to be made deliberately, alongside the executor actually filling the field.
    ///
    /// (`only_the_traversal_and_the_survey_decline_to_report_input_contribution` stood beside this
    /// test until ratification ⟨6⟩/9d removed the `InputContribution` disclosure with its field.)
    #[test]
    fn match_location_is_declared_by_no_act_until_the_wide_fragments_carry_the_chunk_out() {
        for d in search_family() {
            assert!(
                !d.discloses.contains(&Disclosure::MatchLocation),
                "{:?} declares a match location the executor never fills — ship the argmin first",
                d.name
            );
        }
    }

    /// No act declares filter counts, and that is measured rather than pending.
    ///
    /// No deployed fragment computes admitted/excluded per filter, and counting on demand costs a
    /// second query — the one `Extent` exists to avoid. The variant is kept because the fields it
    /// names exist; this test is what stops it being quietly declared by an act that does not in
    /// fact compute them.
    #[test]
    fn filter_counts_are_declared_by_nothing_because_no_fragment_computes_them() {
        assert!(
            search_family()
                .iter()
                .all(|d| !d.discloses.contains(&Disclosure::FilterCounts)),
            "declaring this would promise a number that costs a second query to produce"
        );
    }

    /// An act that produces nothing discloses nothing.
    ///
    /// The same invariant `visibility_profile` already holds against `orders_by`: a field about a
    /// stage's output is a claim with no referent on an act that has no output.
    #[test]
    fn an_act_that_selects_nothing_declares_no_disclosure() {
        for d in search_family() {
            if d.produces.is_none() {
                assert!(
                    d.discloses.is_empty(),
                    "{:?} produces nothing, so it has no stage whose disclosure could be described",
                    d.name
                );
            }
        }
    }

    /// Every act that SELECTS something predicts a response shape.
    ///
    /// Catches a new selecting act whose currency the response side has no hit type for — which
    /// would otherwise surface as `/api/query/validate` silently omitting a key it was asked about.
    #[test]
    fn every_selecting_act_predicts_a_response_shape() {
        for d in search_family() {
            if d.produces.is_some() {
                assert!(
                    d.produced_variant().is_some(),
                    "{:?} selects but predicts no response shape — it produces {:?}, which has \
                     no hit type",
                    d.name,
                    d.produces
                );
            }
        }
    }

    /// Every act that ORDERS declares a score kind the response side RECOGNIZES.
    ///
    /// `score_kind` derives from `orders_by.field` — the deployed column name — rather than from a
    /// table keyed on the act name, so this is what catches a scoring column renamed without its
    /// declaration following. The failure would otherwise be quiet and late: the promise and the
    /// rows would agree with each other (both read the same field) while both named a quantity no
    /// client has ever heard of.
    ///
    /// `[amended — 2026-08-14]` **The gate was `produces.is_some()` and is now `orders_by`.** That
    /// is a split, not a loosening: the old predicate was a PROXY for "this act orders", correct
    /// only while every act that selected also scored. `find-resources-with` breaks the proxy —
    /// it selects a set and ranks nothing — so under the old gate a selection-only act failed an
    /// invariant about scoring columns, which was never a statement about it.
    ///
    /// The same distinction was already drawn once, by
    /// `an_act_that_orders_nothing_classifies_no_ordering_fragment`, which reads `orders_by` to
    /// decide whether `visibility_profile` may be present. This is that cut applied to the field
    /// beside it, and the precedent is why it is an amendment rather than an exception.
    ///
    /// Nothing is lost in the direction that mattered. A renamed scoring column still moves
    /// `orders_by.field`, which is still what is checked; what stops being checked is an act with
    /// no ordering quantity, which had no column to rename.
    #[test]
    fn every_ordering_act_declares_a_known_score_kind() {
        for d in search_family() {
            if d.orders_by.is_some() {
                let kind = d.score_kind();
                assert!(
                    kind.as_ref()
                        .is_some_and(super::super::hits::ScoreKind::is_known),
                    "{:?} orders by {:?}, which the response side does not recognize as a score \
                     kind — rename the column here too, or teach `ScoreKind` about it",
                    d.name,
                    d.orders_by.as_ref().map(|q| q.field.as_str())
                );
            }
        }
    }

    /// The acts that SELECT without ORDERING are exactly these.
    ///
    /// The companion the amendment above owes. Widening a gate from `produces` to `orders_by`
    /// silently admits every future act into the cell the old gate refused, and a cell nothing
    /// names is a cell nobody reviews — so the population is pinned, and an act joining it has to
    /// be stated here rather than inherited from a gate that stopped looking.
    ///
    /// **What lands in this cell is not returnable**, and that is the reason the pin is worth its
    /// brittleness. A stage whose act orders nothing has no quantity to score its rows; asked for
    /// in `returns`, the assembler drops every row for want of a score kind and reports
    /// `disposition: answered` over an empty list. That is not hypothetical — it is verbatim the
    /// defect `CombinatorNotReturnable` was minted to close (`validate/shape.rs:202-216`), arriving
    /// a second time by a different route. An act added here without the matching returns-refusal
    /// reopens it.
    ///
    /// Failing this test is not a defect. It means the cell gained a member, and the question to
    /// answer is whether that member is unreturnable too.
    #[test]
    fn the_acts_that_select_without_ordering_are_exactly_these() {
        let selecting_unordered: Vec<ActName> = search_family()
            .into_iter()
            .filter(|d| d.produces.is_some() && d.orders_by.is_none())
            .map(|d| d.name)
            .collect();
        assert_eq!(
            selecting_unordered,
            vec![ActName::FindResourcesWith],
            "the set of acts that produce a set but rank nothing has moved; each member must also \
             be refused in `returns`, or its rows are dropped while the stage reports `answered`"
        );
    }

    /// And an act that selects NOTHING predicts nothing.
    ///
    /// The pair to the test above: without it, a `produced_variant` that returned some default
    /// would pass the first assertion everywhere and quietly give `substantiate` a result shape it
    /// does not have.
    #[test]
    fn an_act_that_selects_nothing_predicts_no_response_shape() {
        for d in search_family() {
            if d.produces.is_none() {
                assert!(
                    d.produced_variant().is_none(),
                    "{:?} produces nothing and must promise nothing",
                    d.name
                );
            }
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

    /// **The one genuine foreclosure, and it closed** `[2026-08-14]`.
    ///
    /// This asserted `accepts_bounds.is_empty()` — "follow-from bounded is unbuilt", because
    /// `search_graph_expand` had no scope parameter. Inverted rather than deleted: a foreclosure
    /// that opens should leave behind the assertion that it is open, or nothing records that the
    /// declaration is meant to carry both.
    ///
    /// **It takes BOTH, and both are `Resource`** — which is the shape that made this act the
    /// reason `ActInvocation.inputs` became a list. A stage hands `follow-from` a seed set to grow
    /// from and, optionally, a bound to stay within; with one input slot only one of those was
    /// sayable, so this declaration would have named a capability no caller could express.
    #[test]
    fn follow_from_takes_resource_seeds_and_a_resource_bound() {
        let a = declaration(&ActName::FollowFrom).unwrap();
        assert_eq!(a.accepts_seeds, vec![IdKind::Resource]);
        assert_eq!(
            a.accepts_bounds,
            vec![IdKind::Resource],
            "bounded follow-from is built: the fragment's p_bound_ids constrains the whole walk"
        );
        // The disclosure the enum predicted would "return when a walk carries origin".
        assert!(
            a.discloses.contains(&Disclosure::InputContribution),
            "the walk projects which seed each neighbour came from, and via which edge"
        );
    }

    #[test]
    fn survey_accepts_anchor_kinds_as_bounds() {
        // Dissolved by the typed currency: wayfind_region_scores takes (p_anchor_table,
        // p_anchor_id), so cogmap_list -> survey needs no SQL change.
        //
        // `[amended — 2026-08-16]` was `produces: Region`; the ratified ⟨3⟩ redesign produces
        // the member RESOURCES of matched regions. Regions move to `discloses`.
        let a = declaration(&ActName::Survey).unwrap();
        assert!(a.accepts_bounds.contains(&IdKind::Cogmap));
        assert!(a.accepts_bounds.contains(&IdKind::Context));
        assert_eq!(a.produces, Some(IdKind::Resource));
        assert!(a.discloses.contains(&Disclosure::Region));
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
        //   vec_norm  = 1.0 - d/2  -> [0,1]
        //   query_cos = 1   - d    -> [-1,1]
        // so `region_score` CAN BE NEGATIVE, while every discussion of it in this arc's research
        // treats it as a [0,1] score.
        //
        // `[corrected — 2026-08-10, ADJ-9e]` The span asserted here was `[-0.6, 1.0]`, computed
        // from a TWO-term expression the deployed function does not have. `wayfind_region_scores`
        // is `alpha*sal_norm + beta*query_cos + kappa*prior` with alpha 0.4, beta 0.6, kappa 0.05
        // and prior 1.0 (cogmap-homed) or 0.6 (context-homed), so:
        //   min = 0.4*0    + 0.6*(-1) + 0.05*0.6 = -0.57
        //   max = 0.4*1    + 0.6*( 1) + 0.05*1.0 =  1.05
        // The old figure was wrong at BOTH ends, and wrong in the dangerous direction at the top:
        // it promised a value that could never exceed 1.0, which is the promise a caller
        // normalising against the declared range would rely on.
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
                bounds: "[-0.57, 1.05]".to_string()
            }
        );
        assert_ne!(vec_scale, region_scale);
    }

    /// The declared range for `region_score` admits values above 1.0, and that is the point.
    ///
    /// A separate assertion because the string comparison above passes just as well against a
    /// figure someone re-derives wrongly. This one names the property a caller depends on: the
    /// quantity is not a similarity in disguise, and clamping or normalising it as though it were
    /// would silently discard the anchor-kind prior at the top of the range.
    #[test]
    fn the_survey_quantity_declares_a_range_that_exceeds_one() {
        let QuantityScale::OtherRange { bounds } = declaration(&ActName::Survey)
            .unwrap()
            .orders_by
            .unwrap()
            .scale
        else {
            panic!("region_score is not a unit interval and must not be declared as one");
        };
        let (lo, hi) = bounds
            .trim_matches(['[', ']'])
            .split_once(", ")
            .expect("bounds render as `[lo, hi]`");
        let lo: f64 = lo.parse().unwrap();
        let hi: f64 = hi.parse().unwrap();
        assert!(lo < 0.0, "the composite can be negative; got {lo}");
        assert!(
            hi > 1.0,
            "the kappa*prior term lifts the top above 1.0; got {hi}"
        );
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

    /// A caller who names no page size gets the act's published ceiling, not everything.
    ///
    /// `[ruled — 2026-08-10, Pete, ADJ-11]` The witness for the defect, not just the fix: before
    /// this, `applied_terms` iterated only what the caller REQUESTED, so an omitted `limit` never
    /// acquired a value, the compiler bound `NULL`, and `LIMIT NULL` returned the whole visible
    /// match set per stage. The assertion is on the reported map because that map is the same one
    /// the statement binds — a default that appeared in one and not the other would be a response
    /// unable to account for its own row count.
    #[test]
    fn an_omitted_limit_defaults_to_the_published_ceiling_and_is_reported() {
        for name in [
            ActName::FindExact,
            ActName::FindAboutAnywhere,
            ActName::FindAboutWithin,
        ] {
            let d = declaration(&name).unwrap();
            let applied = applied_terms(&BTreeMap::new(), &d);
            assert_eq!(
                applied.get(&BoundTerm::Limit),
                Some(&50),
                "{name:?} must default an omitted limit to its ceiling"
            );
            assert_eq!(
                applied.get(&BoundTerm::Limit),
                d.bound_ceilings.get(&BoundTerm::Limit),
                "{name:?}'s default is the PUBLISHED ceiling, never a second copy of the number"
            );
        }
    }

    /// The default does not disturb the clamp, and the clamp still runs.
    ///
    /// The two are different events over the same term — a value the caller sent, reduced; and a
    /// value the caller never sent, supplied — and both are reported identically as *what ran*.
    #[test]
    fn a_requested_limit_above_the_ceiling_still_clamps_to_it() {
        let d = declaration(&ActName::FindExact).unwrap();
        let over = applied_terms(&BTreeMap::from([(BoundTerm::Limit, 5_000)]), &d);
        assert_eq!(over.get(&BoundTerm::Limit), Some(&50));
        // And a value beneath the ceiling is honoured rather than raised to it — the default fires
        // only on ABSENCE, which is the distinction a `max(asked, default)` would destroy.
        let under = applied_terms(&BTreeMap::from([(BoundTerm::Limit, 3)]), &d);
        assert_eq!(under.get(&BoundTerm::Limit), Some(&3));
    }

    /// `offset` gains nothing from the default rule.
    ///
    /// It has no published ceiling on any act, and page 1 is the right answer to a caller who named
    /// no page. Asserted because the natural generalisation — *default every admitted term to its
    /// ceiling* — is wrong for both other terms and would have been the tidier code.
    #[test]
    fn an_omitted_offset_stays_absent() {
        let d = declaration(&ActName::FindExact).unwrap();
        assert!(d.accepts_bound_terms.contains(&BoundTerm::Offset));
        assert!(!d.bound_ceilings.contains_key(&BoundTerm::Offset));
        let applied = applied_terms(&BTreeMap::new(), &d);
        assert_eq!(applied.get(&BoundTerm::Offset), None);
        assert_eq!(applied.len(), 1, "only the limit default was added");
    }

    /// The walk pages, and its ceiling is a page size rather than a horizon.
    ///
    /// `[amended — 2026-08-17]` `follow-from` admitted `Limit` alone — the only row-returning act
    /// in the family with a page size and no page number — so the 50 it published was a hard stop:
    /// a node with more than 50 neighbours of the asked kind could never be walked in full, and
    /// `{"offset": 50}` was refused as `BoundTermNotApplicable`. The find acts share that ceiling
    /// and could always walk past it. All three properties are asserted together because either
    /// alternative — raising the ceiling, or dropping it — answers the symptom while leaving the
    /// act unpageable, so a test pinning only one of the three would pass against both.
    #[test]
    fn the_walk_pages_like_the_find_acts_and_keeps_their_ceiling() {
        let d = declaration(&ActName::FollowFrom).unwrap();
        assert!(
            d.accepts_bound_terms.contains(&BoundTerm::Offset),
            "the one edge-traversing act must be able to name a page"
        );
        assert!(
            !d.bound_ceilings.contains_key(&BoundTerm::Offset),
            "a ceiling on the page NUMBER would reinstate the horizon this removes"
        );
        assert_eq!(
            d.bound_ceilings.get(&BoundTerm::Limit),
            Some(&50),
            "the page SIZE ceiling is unchanged — it was never the defect"
        );
        // The term axis is the find acts' exactly, not a near-miss. An act that pages by its own
        // slightly different rules is what `the-same-bound-term-means-the-same-thing-on-every-read`
        // forbids, and an `accepts` list is where that divergence would first be legible.
        assert_eq!(
            d.accepts_bound_terms,
            declaration(&ActName::FindExact)
                .unwrap()
                .accepts_bound_terms
        );
        // And the consequence at the one definition the compiler and the assembler both read: a
        // requested page passes through unclamped, an omitted one stays absent, and the limit
        // default is untouched by either.
        let paged = applied_terms(&BTreeMap::from([(BoundTerm::Offset, 200)]), &d);
        assert_eq!(paged.get(&BoundTerm::Offset), Some(&200));
        assert_eq!(paged.get(&BoundTerm::Limit), Some(&50));
        assert_eq!(
            applied_terms(&BTreeMap::new(), &d).get(&BoundTerm::Offset),
            None
        );
    }

    /// An act that does not admit `Limit` gains nothing.
    ///
    /// `survey` is the case, and it is the reason the rule keys on `Limit` alone rather than on
    /// "every admitted term with a ceiling": its `Regions` bound publishes a ceiling of 20 while
    /// `wayfind_region_scores` defaults the funnel to 3, so defaulting to the ceiling would widen
    /// every unbounded survey nearly sevenfold and call it the deployed behaviour. `substantiate`
    /// and the anti-act admit no terms at all, so they can acquire nothing either.
    #[test]
    fn an_act_that_does_not_admit_limit_acquires_no_default() {
        for name in [ActName::Survey, ActName::Substantiate, ActName::Admit] {
            let d = declaration(&name).unwrap();
            assert!(!d.accepts_bound_terms.contains(&BoundTerm::Limit));
            let applied = applied_terms(&BTreeMap::new(), &d);
            assert!(
                applied.is_empty(),
                "{name:?} acquired {applied:?} from a rule that does not apply to it"
            );
        }
        // And `survey`'s own ceiling is not a default in disguise: asking for nothing leaves the
        // funnel width to the fragment, which is where its default lives.
        let survey = declaration(&ActName::Survey).unwrap();
        assert_eq!(survey.bound_ceilings.get(&BoundTerm::Regions), Some(&20));
        assert_eq!(
            applied_terms(&BTreeMap::new(), &survey).get(&BoundTerm::Regions),
            None
        );
    }
}
