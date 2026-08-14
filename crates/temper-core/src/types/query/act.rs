//! Act identity, build-state, and the declaration shape.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::filter::FilterField;
use super::hits::ScoreKind;
use super::id_set::IdKind;
use super::scalars::BoundTerm;
use super::stage::ProducedVariant;

/// The act vocabulary. Asker-shaped, not mechanism-shaped: an act names what the asker holds, and
/// the mechanic currently serving it is evidence rather than identity.
///
/// OPEN discriminator — adding an act is additive.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
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
    /// *I can say what these things ARE; I have no question about what they mean.*
    ///
    /// The only act with no intention — a pure selection. It narrows by what is known about a
    /// resource and orders nothing, so it SELECTS without SCORING, which is the distinction
    /// [`ActDeclaration::orders_by`] carries and which the registry's invariants had folded
    /// together under `produces.is_some()` until this act arrived.
    ///
    /// Its output is a set to pipe, never rows to read: a stage running it cannot appear in
    /// `returns`, for the same reason a combinator cannot — rows with no ordering quantity have
    /// nothing to score them, and the assembler would drop every one while reporting
    /// `disposition: answered`.
    #[serde(rename = "find-resources-with")]
    FindResourcesWith,
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

/// Whether a mechanic for this act exists, and whether the act has a door of its own or only its
/// host's. Every value is mechanically checkable by T3's gate — which is the whole point, because a
/// hand-maintained build-state is the `ADMIN_EVENT_TYPES` failure: a const beside a registry, with
/// a test holding its own second copy.
///
/// **This is the MECHANISM axis and nothing else.** *Which surfaces can reach the act* is
/// [`DoorReach`], a separate field, and the separation is load-bearing rather than tidy. A
/// `served`-vs-`served-on-some-doors` variant here would capture `substantiate` (served on API and
/// CLI, absent from MCP) and would miss the other half of the class outright: an act `Served`,
/// reachable from all three doors, and STILL door-partial on one of `DoorReach`'s shortfall axes —
/// a shape finer-grained than the act and orthogonal to whether a mechanic exists, so it cannot
/// ride on this enum no matter how many variants it grew.
///
/// `find-exact` and `find-about-within` were the live witness of that second half: `Served`
/// everywhere, yet no door's params carried a resource-id slot to supply the
/// [`super::id_set::IdKind::Resource`] bound they accept. That closed with this branch —
/// `SearchParams` gained `bound_ids`, and `temper search` gained `--within` — so **no act in the
/// registry occupies the cell today.** Recorded plainly rather than silently dropped: the
/// separation this paragraph argues for is structural, not contingent on a live example. Every
/// `DoorReach::Serves`'s three shortfall lists stay independently settable per door regardless of
/// `BuildState`, so the day any act's coverage goes non-empty again — on this axis or a new one —
/// this enum still could not have expressed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum BuildState {
    /// A door invokes this act alone, rather than only as part of a composite. Says nothing about
    /// HOW MANY doors, or which — that is [`ActDeclaration::door_coverage`].
    Served,
    /// The mechanic runs only inside a named composite; the act has no door, the host has one.
    Fused { host: String },
    /// No mechanic exists.
    Unbuilt,
}

impl BuildState {
    pub fn host(&self) -> Option<&str> {
        // Exhaustive on purpose — no `_` arm. Both matches on this enum used a wildcard, so a new
        // variant would have compiled silently and landed in whichever arm the wildcard caught,
        // which is the failure mode a widening is supposed to surface rather than absorb.
        match self {
            BuildState::Fused { host } => Some(host.as_str()),
            BuildState::Served | BuildState::Unbuilt => None,
        }
    }
}

/// One of Temper's three surfaces. Named as doors rather than as transports because the question
/// this vocabulary answers is *can a caller standing here ask this*, not *what protocol carries it*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Door {
    Cli,
    Api,
    Mcp,
}

impl Door {
    /// Every door. A declaration must account for all of them — see
    /// [`ActDeclaration::door_coverage`].
    pub const ALL: [Door; 3] = [Door::Cli, Door::Api, Door::Mcp];
}

/// How much of an act one door can reach.
///
/// Two variants, not three: `Serves` with an empty shortfall IS full reach, so "full" needs no
/// variant of its own and cannot disagree with the lists beside it. An act absent from a door has
/// no shortfall to state, which is why `Absent` carries nothing.
///
/// **Absence is declared, never inferred from an omitted entry.** That is the whole point of the
/// field: goal `019fa618` (*surface parity — no door offers less than another without saying so*)
/// has no witnesses because no mechanical inventory of who-offers-what exists, and a declaration
/// that simply left a door out would reproduce exactly that hole in a new place.
///
/// **Three shortfall axes, because a door falls short in three different ways and only one of them
/// was expressible.** `terms_unreachable` shipped alone, so `Serves {}` was a promise nobody could
/// qualify: two real shortfalls had to be declared as full reach or not at all. A door can also be
/// unable to supply a whole BOUND KIND — the axis this exists for: `find-exact` and
/// `find-about-within`'s `Resource` bound was unsuppliable from every door until `SearchParams`
/// gained `bound_ids` and `temper search` gained `--within`, so as of this branch no act occupies
/// the cell (see `Serves`'s `bounds_unreachable` field doc below). And it can accept a FILTER SLOT
/// the act declares and then apply nothing, which is a silent substitution wearing a successful
/// narrowing's costume. Each entry is guarded against the act's own `accepts_*` list (see the
/// registry's tests): a door cannot fall short on something the act never admitted, in any axis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", tag = "reach")]
pub enum DoorReach {
    /// This door cannot invoke the act at all.
    Absent,
    /// This door invokes the act, minus whatever it lists as unreachable. Empty lists mean it
    /// reaches everything the act declares.
    Serves {
        /// Bound terms the act admits that a caller at this door has no way to supply. Every entry
        /// must be a term the act actually admits — an unreachable term the act never accepted is
        /// a contradiction, not a gap.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        terms_unreachable: Vec<BoundTerm>,
        /// Bound KINDS the act admits that this door has no slot for. Zero live instances anywhere
        /// in the registry as of this branch: the `find` acts' `Resource` bound was the one —
        /// every door hard-bound `NULL` for bound-ids and no door's params carried a resource-id
        /// list — until `SearchParams` gained `bound_ids` and `temper search` gained `--within`,
        /// closing it everywhere at once. The field stays because an act admitting a bound kind
        /// that not every door can supply is exactly this shape again; nothing currently occupies
        /// it. Same guard as `terms_unreachable` — every entry must appear in the act's
        /// `accepts_bounds`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        bounds_unreachable: Vec<IdKind>,
        /// Filter slots the act admits that this door accepts and then does not apply. Distinct
        /// from a slot the act never admitted, which is refused outright
        /// ([`super::disposition::RefusalReason::FilterNotApplicable`]) and needs no declaration:
        /// this axis is for the worse case, where the act says yes and the door narrows nothing.
        /// Every entry must appear in the act's `accepts_filters`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        filters_unapplied: Vec<FilterField>,
    },
}

/// The scale of an act's ordering quantity.
///
/// Carried because assuming `[0,1]` is the **live** mistake in this family, not a hypothetical one.
/// `search_wide` rescales a cosine distance as `1.0 - d/2.0` into `[0,1]`, while
/// `wayfind_region_scores` rescales *the same* `<=>` distance as `1 - d` into `[-1,1]`.
/// Neither column name says so, and one of the two feeds a weighted sum everyone reads as a
/// `[0,1]` score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", tag = "scale")]
pub enum QuantityScale {
    /// `[0,1]`, bounded by the expression's own arithmetic.
    UnitInterval,
    /// Bounded, but NOT to `[0,1]`. `bounds` states the actual range.
    OtherRange { bounds: String },
    /// Nothing in the expression or in the schema bounds it.
    Unbounded,
}

/// The quantity an act orders its answer by, named so that summing it with another act's reads as
/// the category error it is.
///
/// This is where `no-cross-act-ranking` becomes structural instead of a rule someone remembers.
/// Research [Asking Temper](./019fbd9b-2d28-7530-9da0-4515319d6688), delta 5: *"Act responses never
/// expose commensurable score fields — no bare `score: f64` shared across acts; each quantity
/// carries its act's name and shape."* Arithmetic follows names: two fields called `score` invite
/// `a.score + b.score` and no reviewer catches it. The retired `unified_search` is the worked
/// failure — it renamed `fts_norm` and `vec_norm` to `fts_score`/`vector_score` and then summed them
/// into `combined_score`, which is the exact expression the frame register forbids. It was dropped
/// on 2026-08-06; the body stays readable in the migration history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ActQuantity {
    /// The DEPLOYED column name the serving function emits. Not a name invented here — a
    /// declaration is a description, so a caller who greps the SQL for this string finds it.
    pub field: String,
    /// What the number measures, in this act's own terms.
    pub means: String,
    pub scale: QuantityScale,
}

/// Where the principal constraint applies to **the fragment that produces this act's ordering** —
/// not to its serving function as a whole.
///
/// The granularity is stated because it is not inferable and was once got wrong in both
/// directions. **Every** serving function in the family takes `p_principal` and joins a visibility
/// relation, so "does the mechanic read the principal" is not the question — answering *that* one
/// collapses all three variants into `PrincipalRelative` and deletes the distinction this type
/// exists to draw. The question is whether the quantity the act **orders by** would change if a
/// different set of rows were fed to it.
///
/// A consequence worth carrying: one function can emit fragments in different classes at once
/// (`cogmap_list_rows` returns principal-agnostic counts beside principal-relative-in-domain team
/// rollups), so this field is lossy by construction for any act whose output has more than one
/// ordering-bearing quantity. No act in the search family does today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum VisibilityProfile {
    PrincipalAgnostic,
    /// The ordering fragment's own expression and predicates are principal-free, but it is a
    /// window or aggregate whose **frame** is the principal's read-set — so the value is a
    /// function of *a row set*, and which row set is the principal's business. Extractable only
    /// with that domain made an explicit input.
    ///
    /// Two worked examples, because one taught the wrong lesson. `survey`'s `sal_norm` is a
    /// `percent_rank` over the visible anchor set. `follow-from`'s `graph_score` is a
    /// `MAX(score) GROUP BY node` over a walk whose adjacency requires **both** edge endpoints
    /// visible — arithmetic principal-free, path set principal-scoped.
    ///
    /// **The discriminator against `PrincipalAgnostic` is gate vs filter, and the two look
    /// identical to a grep.** A visibility *gate* (`WHERE subject IN (visible)`) is all-or-nothing
    /// on the subject and cannot change the answer — `resource_standing_shape` is gated and is
    /// agnostic, measured. A visibility *filter* (`JOIN vis ON vis.id = m.member_id`) removes rows
    /// from the aggregate's own input and does.
    AgnosticInValueRelativeInDomain,
    PrincipalRelative,
}

/// One optional piece of per-stage disclosure that some acts can produce and others cannot.
///
/// **One class with two members, not two special cases.** Each names a response field that is
/// filled for some acts and null for others, and in every case a null means *not declared*, never
/// zero — which is the whole reason the class exists. Note that `match_location` is currently
/// declared by NO act (see the registry): a declaration describes the DEPLOYED system, and the
/// executor hard-codes `located_at: None` today.
///
/// CLOSED, unlike [`super::disposition::RefusalReason`]. The two openness rules differ for the
/// reason they always do here: a consumer branches exhaustively on which disclosures an act offers,
/// whereas it must tolerate a refusal reason it has never seen. A third disclosure is a breaking
/// change, and should be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Disclosure {
    // `InputContribution` used to lead this enum. Removed with its `input_contributed` field
    // (ratification ⟨6⟩/9d `[2026-08-09, Pete]`); returns when a walk carries origin.
    /// Where in the resource the match was — `ResourceHit.located_at`.
    MatchLocation,
    /// How many rows each filter admitted and excluded — `NarrowedBy.admitted` / `.excluded`.
    ///
    /// Declared by NO act today, and that is a measured absence rather than an oversight: no
    /// deployed fragment computes these counts, and counting on demand costs a second query. The
    /// variant exists because the fields it names exist; a closed vocabulary may carry a member
    /// with no current declarer, where an open one may not.
    FilterCounts,
}

/// One act, declared.
///
/// See [`ActDeclaration::produced_variant`] for how a declaration predicts the response shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
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
    /// Which optional per-stage disclosures this act's mechanic **can** produce.
    ///
    /// The response fields [`Disclosure`] names are filled for some acts and null for others, and
    /// a null in any of them is otherwise ambiguous between *this act cannot* and *the answer is
    /// none* — which are opposite answers. This is what disambiguates them, and what
    /// `/api/query/validate` reads to tell a caller in advance rather than leaving them to
    /// discover it in a response.
    ///
    /// **Absence from this list is the declaration, not silence** — the same rule
    /// [`ActDeclaration::door_coverage`] follows. Today every act's list is empty: a declaration
    /// describes the DEPLOYED system, and no deployed fragment carries either remaining
    /// disclosure out (see the registry's per-site rulings).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub discloses: Vec<Disclosure>,
    /// Which surfaces reach this act, and how much of it. **Every [`Door`] carries an entry** —
    /// absence from a door is stated as [`DoorReach::Absent`], never by leaving the door out, so
    /// "this declaration says nothing about MCP" cannot be mistaken for "MCP serves it".
    pub door_coverage: BTreeMap<Door, DoorReach>,
    /// The quantity this act orders by. `None` for an act that orders nothing — an unbuilt act, the
    /// anti-act, or an act that annotates rather than selects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orders_by: Option<ActQuantity>,
    /// Where the principal constraint applies to the ordering fragment.
    ///
    /// `None` exactly when [`ActDeclaration::orders_by`] is `None`, and the two move together by
    /// invariant rather than by convention: this field's own definition is *"where the principal
    /// constraint applies to **the fragment that produces this act's ordering**"*, so an act with no
    /// ordering has no such fragment and any value here would be a claim with no referent. v0
    /// shipped `substantiate` and `admit` — both unbuilt, both ordering nothing — declaring
    /// `PrincipalRelative`, which is not a conservative default but a sentence about a function that
    /// does not exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility_profile: Option<VisibilityProfile>,
    /// Bumped whenever the served-by body changes the scale or meaning of a quantity. T3 gate 4
    /// reds when the body hash moves and this does not.
    pub scoring_revision: u32,
}

impl ActDeclaration {
    /// Which [`super::stage::StageOutput`] variant a stage running this act will carry.
    ///
    /// Now a straight read of `produces`, because the envelope tags CURRENCY and nothing else —
    /// how the rows were scored travels per row as [`super::hits::Scoring::score_kind`]. An earlier
    /// version had to consult `orders_by.field` here, which is what made `vec_hits` a possible tag
    /// and the envelope load-bearing for reading a row.
    ///
    /// `None` for an act that selects nothing (`substantiate`, `admit`) — those are not composable
    /// and never appear as a returned stage.
    pub fn produced_variant(&self) -> Option<ProducedVariant> {
        match self.produces.as_ref()? {
            IdKind::Resource => Some(ProducedVariant::Resources),
            IdKind::Region => Some(ProducedVariant::Regions),
            // `Cogmap` and `Context` are accepted as scopes and never produced; `Other` is a kind
            // this binary does not know. Either way there is no hit shape, and saying so is better
            // than picking the nearest variant — a wrong promise is worse than an absent one.
            IdKind::Cogmap | IdKind::Context | IdKind::Other(_) => None,
        }
    }

    /// The score kind every row of this act's output will carry.
    ///
    /// Derived from `orders_by.field`, which is the DEPLOYED column name the serving function
    /// emits — so this and [`super::hits::Scoring::score_kind`] are the same string by
    /// construction, and a caller who greps the SQL for it finds it. Keying on `ActName` instead
    /// would be a second copy of the relation, free to drift.
    ///
    /// `None` for an act that orders nothing. The consequence worth knowing: renaming a scoring
    /// column without updating `orders_by.field` makes this return an unrecognized kind, and
    /// `every_selecting_act_declares_a_known_score_kind` goes red. That is the intended direction —
    /// the declaration is supposed to describe the deployed system.
    pub fn score_kind(&self) -> Option<ScoreKind> {
        let field = &self.orders_by.as_ref()?.field;
        Some(match field.as_str() {
            "fts_norm" => ScoreKind::FtsNorm,
            "vec_norm" => ScoreKind::VecNorm,
            "graph_score" => ScoreKind::GraphScore,
            "region_score" => ScoreKind::RegionScore,
            other => ScoreKind::Other(other.to_string()),
        })
    }
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
            serde_json::to_string(&ActName::FindResourcesWith).unwrap(),
            "\"find-resources-with\""
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
            discloses: vec![],
            door_coverage: BTreeMap::from([
                (
                    Door::Cli,
                    DoorReach::Serves {
                        terms_unreachable: vec![BoundTerm::Offset],
                        bounds_unreachable: vec![IdKind::Resource],
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
                (
                    Door::Mcp,
                    DoorReach::Serves {
                        terms_unreachable: vec![],
                        bounds_unreachable: vec![],
                        filters_unapplied: vec![],
                    },
                ),
            ]),
            orders_by: Some(ActQuantity {
                field: "fts_norm".to_string(),
                means: "postgres ts_rank over the resource's own document".to_string(),
                scale: QuantityScale::UnitInterval,
            }),
            visibility_profile: Some(VisibilityProfile::PrincipalRelative),
            scoring_revision: 1,
        };
        assert!(!d.asker_holds.is_empty());
        assert_eq!(
            serde_json::from_str::<ActDeclaration>(&serde_json::to_string(&d).unwrap()).unwrap(),
            d
        );
    }

    #[test]
    fn a_door_that_serves_nothing_is_distinguishable_from_one_that_serves_everything() {
        // The failure this shape exists to prevent is a reader treating an omitted door as an
        // unremarkable one. `Absent` and `Serves {}` must not render alike.
        let absent = serde_json::to_string(&DoorReach::Absent).unwrap();
        let full = serde_json::to_string(&DoorReach::Serves {
            terms_unreachable: vec![],
            bounds_unreachable: vec![],
            filters_unapplied: vec![],
        })
        .unwrap();
        assert_ne!(absent, full);
        assert_eq!(
            serde_json::from_str::<DoorReach>(&absent).unwrap(),
            DoorReach::Absent
        );
    }

    #[test]
    fn a_door_can_fall_short_on_a_kind_or_a_filter_and_not_only_on_a_term() {
        // The axis `terms_unreachable` alone could not express. A door that supplies every term and
        // still cannot express the act's `Resource` bound is NOT full reach, and before the
        // widening it had no way to say so — `Serves { terms_unreachable: vec![] }` was the only
        // honest-looking value available, and it claimed the opposite.
        let partial = DoorReach::Serves {
            terms_unreachable: vec![],
            bounds_unreachable: vec![IdKind::Resource],
            filters_unapplied: vec![FilterField::Edge],
        };
        let full = DoorReach::Serves {
            terms_unreachable: vec![],
            bounds_unreachable: vec![],
            filters_unapplied: vec![],
        };
        assert_ne!(partial, full);
        assert_eq!(
            serde_json::from_str::<DoorReach>(&serde_json::to_string(&partial).unwrap()).unwrap(),
            partial
        );
        // Empty shortfall lists stay off the wire, so full reach renders as the bare tag it always
        // did — the widening is additive for a reader of the emitted contract.
        assert_eq!(
            serde_json::to_string(&full).unwrap(),
            "{\"reach\":\"serves\"}"
        );
    }

    #[test]
    fn build_state_host_is_exhaustive_over_the_enum() {
        // Both matches on `BuildState` used a `_` arm before door coverage landed, so a new variant
        // would have compiled silently into a wildcard. This pins the answer for every variant that
        // exists, so removing the wildcard cannot be quietly undone.
        assert_eq!(
            BuildState::Fused {
                host: "unified_search".to_string()
            }
            .host(),
            Some("unified_search")
        );
        assert_eq!(BuildState::Served.host(), None);
        assert_eq!(BuildState::Unbuilt.host(), None);
    }

    #[test]
    fn a_quantity_states_a_scale_that_is_not_assumed_to_be_zero_to_one() {
        // `OtherRange` exists because `region_score` really is `[-0.57, 1.05]` — 0.4·sal_norm (a
        // percent_rank in [0,1]) + 0.6·query_cos + 0.05·prior, where query_cos is
        // `1 - (centroid <=> emb)` and a cosine DISTANCE spans [0,2], so the similarity spans
        // [-1,1], and the anchor-kind prior is 1.0 or 0.6. Collapsing that into `UnitInterval`
        // would be the assumption this variant exists to refuse — and note that the range does not
        // merely dip below 0, it also exceeds 1.
        let r = QuantityScale::OtherRange {
            bounds: "[-0.57, 1.05]".to_string(),
        };
        assert_ne!(
            serde_json::to_string(&r).unwrap(),
            serde_json::to_string(&QuantityScale::UnitInterval).unwrap()
        );
        assert_eq!(
            serde_json::from_str::<QuantityScale>(&serde_json::to_string(&r).unwrap()).unwrap(),
            r
        );
    }
}
