//! Hydrated rows — what a RETURNED stage hands back.
//!
//! **One shape per currency, and the quantity is data rather than schema.** A hit is
//! `{the thing} + {scoring}`, where [`Scoring`] names its own kind. So a row can be read without
//! knowing which envelope carried it, and a client has one type per currency instead of one per
//! act.
//!
//! # The line this walks, stated plainly
//!
//! `unified_search` is the worked failure the whole surface is shaped against: it renamed
//! `fts_norm` and `vec_norm` to `fts_score`/`vector_score` and **summed them** into
//! `combined_score`. The lesson is that incommensurable quantities must not be silently addable.
//!
//! An earlier draft enforced that by TYPE — a distinct hit struct per act, so two acts' rows could
//! not sit in one list. That guard was real but narrow, and it cost something specific: you could
//! not read a row without knowing its envelope, because the quantity was the field NAME. The
//! envelope's tag then had to say `vec_hits` rather than `resources`, which is a misnomer — what
//! was produced is resources; how they were scored is a different question.
//!
//! So the difference is expressed as DATA: every hit carries [`Scoring::score_kind`] beside its
//! number. That is strictly more legible than a bare field name — `fts_norm` requires the reader to
//! already know what it means, while `score_kind: "fts_norm"` is a machine-readable assertion that
//! kinds exist, differ, and must be compared before anything is combined.
//!
//! **What still prevents the conflation is structural and unchanged**: arms are keyed separately in
//! the response, and there is no merged ordered list anywhere for two acts' rows to fall into.
//! Combining two arms takes a deliberate act by the caller, and `score_kind` is what makes that act
//! obviously wrong when the kinds differ. `[decided — 2026-08-09, Pete]`

use serde::{Deserialize, Serialize};

use crate::types::cognitive_maps::CogmapRegionRow;
use crate::types::graph::{EdgeKind, Polarity};
use crate::types::ids::BlockId;
use crate::types::resource_view::ResourceView;

/// Which quantity a hit's score is, named so that combining two of different kinds reads as the
/// category error it is.
///
/// The values are the DEPLOYED column names the serving functions emit — not names invented here —
/// so a caller who greps the SQL for one finds it, and each equals the
/// [`super::act::ActQuantity::field`] of the act that produced the row.
///
/// **OPEN, like [`super::act::ActName`].** A new act brings a new quantity, and a closed vocabulary
/// would make adding one a breaking change for every client. Openness costs nothing here because
/// the operation a client performs on this value is EQUALITY — *are these two scores the same
/// kind?* — never exhaustive enumeration. Contrast
/// [`super::disposition::StageDisposition`], which stays closed precisely because consumers match
/// it exhaustively.
///
/// **The ranges differ and these names do not say so.** `vec_norm` rescales a cosine distance as
/// `1 - d/2` into `[0,1]`; `region_score` rescales the identical operator as `1 - d` and spans
/// `[-0.6, 1.0]`; `graph_score` is unbounded. The range rides once per stage on
/// [`super::envelope::StageResult::orders_by`] rather than per row — read it instead of assuming
/// `[0,1]`, which is the live mistake in this family, not a hypothetical one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub enum ScoreKind {
    /// `ts_rank` flag 33 — `rank/(rank+1)` plus log-length division — in `[0,1)`. `find-exact`.
    #[serde(rename = "fts_norm")]
    FtsNorm,
    /// pgvector cosine distance rescaled as `1 - d/2`, in `[0,1]`. Both `find-about-*` acts.
    #[serde(rename = "vec_norm")]
    VecNorm,
    /// Product of edge weights along the best path. UNBOUNDED. `follow-from`.
    #[serde(rename = "graph_score")]
    GraphScore,
    /// `0.4*sal_norm + 0.6*query_cos`, spanning `[-0.6, 1.0]`. `survey`.
    #[serde(rename = "region_score")]
    RegionScore,
    /// A kind this consumer does not recognize — only ever produced by deserializing a producer
    /// newer than this consumer.
    #[serde(untagged)]
    Other(String),
}

impl ScoreKind {
    /// Whether this is a kind the running binary knows how to interpret.
    pub fn is_known(&self) -> bool {
        !matches!(self, ScoreKind::Other(_))
    }

    /// The deployed column name, which is also what [`super::act::ActQuantity::field`] carries.
    pub fn as_str(&self) -> &str {
        match self {
            ScoreKind::FtsNorm => "fts_norm",
            ScoreKind::VecNorm => "vec_norm",
            ScoreKind::GraphScore => "graph_score",
            ScoreKind::RegionScore => "region_score",
            ScoreKind::Other(s) => s.as_str(),
        }
    }
}

/// How one hit scored, and by what measure.
///
/// The kind travels WITH the number, which is what lets a row be understood on its own. Two hits
/// whose `score_kind` differs hold values that must never be added, averaged, or sorted into one
/// list — and unlike a bare field name, that is something a client can actually check.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct Scoring {
    pub score_kind: ScoreKind,
    /// Read [`super::envelope::StageResult::orders_by`] for this quantity's RANGE. It is not
    /// carried per row because it is a property of the act, identical for every row of a stage.
    pub score: f32,
    // `located_at` is NOT here, and the reason is the one this contract already applied once.
    //
    // `Scoring` is shared by both hit types, so a field here exists on region hits too — and a
    // region is not somewhere inside a resource, so it could never be filled. That is precisely
    // what was refused when `FtsHit` was denied an always-null `located_at`: a field that is
    // always null says "no match location" where the truth is "this can never tell you".
    //
    // It lives on `ResourceHit` instead, where it is null for `find-exact` and `follow-from` but
    // FILLABLE by the wide arm — which is the ordinary case `discloses` exists to declare in
    // advance. The distinction is between an act that does not fill a field and a shape that
    // cannot.
}

/// Where in a resource a chunk-grain match landed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct MatchLocation {
    /// The `kb_content_blocks` row the closest chunk belongs to.
    pub block_id: BlockId,
    /// The heading trail to that chunk, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_path: Option<String>,
    /// The chunk's own text, or the part of it that matched.
    ///
    /// A snippet for the EXACT arm is a named remainder rather than a gap: it is possible without a
    /// new index via `ts_headline` over re-fetched text, but that is a per-row fetch of prose the
    /// query did not otherwise need, on a path that currently touches only the index. Nobody has
    /// measured it, and the retired `SearchResultRow` carried a snippet from the `unified_search`
    /// era whose cost was never isolated either.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

/// One edge a walk reached a node by — **the edge as asserted**, and no quantity.
///
/// `[added — 2026-08-14]` with `follow-from`'s provenance-carrying mechanic
/// (`20260814000030`), design `internal/superpowers/specs/2026-08-14-follow-from-mechanic-design.md`
/// §4.
///
/// # Why source and target rather than a parent plus a direction
///
/// The walk is **undirected over a directed graph**: the fragment's `adj` unions both orientations,
/// so a traversal runs with or against the stored `source → target`. A `from_id` plus a
/// which-way-round flag is two fields that must agree, and two fields that must agree are two
/// fields that can drift. The edge as stored cannot contradict itself, and the parent is derivable
/// from it — so the derivable thing is the one left out.
///
/// # Polarity rides because the two arrows disagree in practice
///
/// [`Polarity`] exists because the structural arrow and the semantic one can differ — its own
/// example is a `depends_on` edge asserted source=dependant/target=dependency, whose causal arrow
/// runs the other way. `[measured on prod — 2026-08-14]` **1,099 of 4,454 resource-resource edges
/// are `inverse`, and 87% of `contains` edges are** — so an entry omitting it would report the
/// majority of containment relationships backwards.
///
/// # It carries no numbers, deliberately `[decided — 2026-08-14, Pete]`
///
/// A per-parent score is nearly free to compute at this grain and is **refused**: provenance is
/// structure rather than quantity, and a scored entry would be a second ranking axis inside a row
/// whose one quantity is [`Scoring::score`]. See the note on [`ResourceHit::via`] for the
/// non-correspondence that goes with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ViaEntry {
    /// Which of the caller's seeds this node descends from.
    ///
    /// The act's `orders_by.means` says the score is the best path **from any seed**, and the act
    /// takes a seed SET — so without this a multi-seed walk answers "here are the neighbours of the
    /// things you found" with a list that cannot say which. That is the reason `via` exists at all.
    #[cfg_attr(feature = "typescript", ts(type = "string"))]
    pub seed_id: uuid::Uuid,
    /// The edge's own `source_id`, as stored.
    #[cfg_attr(feature = "typescript", ts(type = "string"))]
    pub source_id: uuid::Uuid,
    /// The edge's own `target_id`, as stored.
    #[cfg_attr(feature = "typescript", ts(type = "string"))]
    pub target_id: uuid::Uuid,
    pub edge_kind: EdgeKind,
    /// The edge's label. **Nullable in the DDL and populated on every edge in prod today**
    /// (`kb_edges.label` is `TEXT` with no `NOT NULL`), so the `None` is real and unobserved rather
    /// than impossible — and an `edge_filter` naming labels silently excludes unlabelled edges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub polarity: Polarity,
}

/// One resource that matched, however it matched.
///
/// [`ResourceView`] is the same projection `list`/`show`/`create`/`update`/`annotate` and both
/// search arms answer in — six near-identical projections collapsed into one. A hit adds scoring
/// and nothing else.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceHit {
    pub resource: ResourceView,
    pub scoring: Scoring,
    /// Where in the resource the match was — the closest chunk's block.
    ///
    /// **Filled only by the wide arm**, and its absence is declared in advance rather than
    /// discovered here: [`super::act::ActDeclaration::discloses`] says which acts can report it.
    /// The wide arm matches at CHUNK grain and already computes which chunk was closest, then
    /// discards it collapsing to a per-resource score; recovering it is an argmin beside an
    /// aggregate that already runs.
    ///
    /// `find-exact` structurally cannot: its index is one tsvector per RESOURCE, built by
    /// concatenating every chunk into a single blob, so the block boundary is gone before the query
    /// is asked. `follow-from` has no match position at all — it returns nodes reached by walking.
    ///
    /// It sits on this type rather than on [`Scoring`] because `Scoring` is shared with
    /// [`RegionHit`], and a region is not somewhere inside a resource.
    ///
    /// **`None` serializes as an explicit `"located_at": null`** (F4, ruled): the contract's prose
    /// is load-bearing on the null — every `located_at` is null today and the null unambiguously
    /// means *not declared* — and omitting the key made the resource-hit null indistinguishable
    /// from a shape with no such field, which is exactly the distinction [`RegionHit`] exists to
    /// draw.
    #[serde(default)]
    // **The published contract has to say PRESENT-null, and the derive says the opposite.** utoipa
    // reads `Option<T>` as "may be omitted" and leaves the key out of `required` — but no
    // `skip_serializing_if` sits here, and its absence is the F4 ruling above rather than an
    // oversight: `[ruled — 2026-08-10, Pete]`, the absent-versus-null rule, of which this field is
    // the PRESENT-null half. So the key is ALWAYS written, and a client generated from the raw
    // derive treats an always-present key as optional — unable to rely on the very distinction F4
    // ruled the null exists to draw: `located_at: null` on a resource hit means *no location
    // declared*, where [`RegionHit`] has no such key at all.
    //
    // Verified by serializing, not by reading the derive.
    #[cfg_attr(feature = "web-api", schema(required = true))]
    pub located_at: Option<MatchLocation>,
    /// How a walk reached this resource — one entry per edge it was reached by.
    ///
    /// **Filled only by `follow-from`**, and declared in advance rather than discovered: the act's
    /// [`super::act::ActDeclaration::discloses`] carries
    /// [`super::act::Disclosure::InputContribution`], which the enum predicted would *"return when
    /// a walk carries origin"*.
    ///
    /// # EVERY parent, and the score corresponds to only one of them
    ///
    /// `graph_score` is a `MAX` over paths; this is a union over **all** of them. They disagree by
    /// construction — a node reached both by a strong one-hop edge and a weak three-hop chain names
    /// both and scores from one. **The non-correspondence is declared here rather than repaired**,
    /// exactly as [`RegionHit::region`] carries `salience` beside `region_score` and says outright
    /// that the two are not rivals. Naming every parent is the more useful and the clearer answer;
    /// naming only the winning path's would make this field a second, silent statement about rank.
    ///
    /// # ABSENT rather than null, which is the opposite of [`Self::located_at`]
    ///
    /// The two look like the same question and are not, and F4's rule is what separates them.
    /// `located_at` is PRESENT-null because its null carries a *claim* — *no location declared* —
    /// that a missing key could not make. A collection has no such claim to carry: `[]` and absent
    /// would both mean "no provenance", so a null adds a third spelling of one fact. The key is
    /// therefore dropped for every act that is not a walk, and its presence means the same thing as
    /// the act's `discloses` already promised.
    ///
    /// # No cap, and the reason is structural rather than "the corpus is small"
    ///
    /// `[measured on prod — 2026-08-14]` at the deliberate worst case (25 highest-degree seeds,
    /// depth 3) the whole walk holds 9,434 entries with 124 on one node, while **the page that
    /// ships carries 125 in total and at most 9 on any row**. Nodes with enormous parent sets are
    /// reached by many long weak paths, so the score ranks them out and the limit sheds them before
    /// they are ever described. Capping would silently truncate provenance, which is the one thing
    /// this field exists to be trusted about.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub via: Vec<ViaEntry>,
}

/// One region of a cognitive map. Produced by `survey`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct RegionHit {
    /// The region itself, in the same shape `cogmap_shape` answers in.
    ///
    /// **It carries its own `salience`, beside this hit's score.** Two numbers on one row, and they
    /// are not rivals: `salience` is an INPUT to `region_score` (`0.4·sal_norm + 0.6·query_cos`,
    /// where `sal_norm` is a rank over salience). Order by the score; ordering by `salience`
    /// answers *"what does this map think is prominent"*, a different question.
    ///
    /// `[decided — 2026-08-08, Pete]` A narrower region projection without `salience` was
    /// considered and refused. [`ResourceView`] exists because six near-identical projections were
    /// collapsed into one; minting a divergent region shape immediately afterwards, to hide one
    /// field, would repeat the mistake that consolidation just paid to fix.
    pub region: CogmapRegionRow,
    pub scoring: Scoring,
}

/// One region a `survey` stage matched, and the score it matched at.
///
/// **Trace disclosure, never a row.** `survey` produces RESOURCES — the ⟨3⟩ redesign
/// (`20260816000020_survey_act.sql`) moved regions out of the output precisely so a caller could not
/// draw them as though the reader had authored them. This says which groupings answered, for a
/// caller that wants to explain *why these*, and it deliberately carries **no per-resource
/// mapping**: that would put a region back on the row shape the redesign just cleared it from.
///
/// Contrast [`RegionHit`], which is a region as a RESULT — a row a caller asked for and may rank.
/// This is a region as an EXPLANATION of resource rows, and the two must not be conflated: one is
/// the answer, the other is a note about how the answer was reached.
///
/// # `region_score` is raw, and is NOT in `[0,1]`
///
/// It is `0.4·sal_norm + 0.6·query_cos + 0.05·prior`, measured spanning `[-0.57, 1.05]` — so it goes
/// negative at one end and past 1 at the other. Whether the `sal_norm` term belongs in it at all is
/// an **OPEN ruling** `[2026-08-14, Pete]`.
///
/// Transporting it unchanged is what keeps that ruling open. A carrier that normalized it into
/// `[0,1]` would silently settle a question it is not entitled to settle, and would do it invisibly,
/// since the normalized number would look exactly like a well-behaved score. Do not clamp, rescale,
/// or attach a [`super::act::QuantityScale`] claim this quantity does not have.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct RegionDisclosure {
    /// The region's id, as stored.
    ///
    /// **Not durable, and the instability is the caller's to handle.** `assert_region`
    /// (`temper-substrate/src/write.rs:673`) reuses this row when a region's member set is unchanged
    /// and **mints a new id otherwise**, soft-folding the displaced row. So an id disclosed here may
    /// name nothing by the time a caller acts on it — which a caller must render as *re-derived*,
    /// never as an error and never as the reader's mistake.
    pub region_id: uuid::Uuid,
    /// The blend this region matched at. See the type's own note: raw, unbounded, open ruling.
    pub region_score: f64,
}
