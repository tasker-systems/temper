//! Hydrated rows — what a RETURNED stage hands back.
//!
//! One type per act quantity, and that is the whole design. Every hit is
//! `{shared projection} + {one named act-specific quantity}`, a shape `/api/search` had already
//! arrived at before anything wrote it down: [`crate::types::resource_view::ResourceView`] is one
//! projection across `list`/`show`/`create`/`update`/`annotate` and both search arms, replacing six
//! near-identical ones, and the quantity's NAME is its identity.
//!
//! # There is no shared `score` field, anywhere, by construction
//!
//! `unified_search` is the worked failure this module is shaped against: it renamed `fts_norm` and
//! `vec_norm` to `fts_score`/`vector_score` and summed them into `combined_score`. Once two acts'
//! numbers share a field name, adding them is a keystroke and nothing objects.
//!
//! So the quantities keep their literal deployed column names — `fts_norm`, `vec_norm`,
//! `graph_score`, `region_score` — a caller who greps the SQL for one finds it, and no two of them
//! can land in the same field. Their RANGES differ and the names do not say so, which is why the
//! range rides once per stage on [`super::envelope::StageResult::orders_by`] rather than being left
//! to assumption: `vec_norm` rescales a cosine distance as `1 - d/2` into `[0,1]` while
//! `region_score` rescales the identical operator as `1 - d` and spans `[-0.6, 1.0]`.

use serde::{Deserialize, Serialize};

use crate::types::cognitive_maps::CogmapRegionRow;
use crate::types::ids::BlockId;
use crate::types::resource_view::ResourceView;

/// A resource whose exact words matched. Produced by `find-exact`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct FtsHit {
    pub resource: ResourceView,
    /// `ts_rank` flag 33 — `rank/(rank+1)` plus log-length division — in `[0,1)`.
    pub fts_norm: f32,
    // THERE IS NO `located_at` HERE, and none is coming without an index change. `find-exact`'s
    // index is ONE tsvector per RESOURCE, built by concatenating every chunk into a single blob, so
    // the block boundary is gone before the query is asked. A field that was always null would say
    // "no match location" where the truth is "this act can never tell you" — which is the exact
    // ambiguity `ActDeclaration::discloses` exists to remove.
    //
    // A SNIPPET is a named remainder rather than a gap. It is possible without a new index, via
    // `ts_headline` over re-fetched text, but that is a per-row fetch of prose the query did not
    // otherwise need, on a path that currently touches only the index. Nobody has measured it, and
    // the retired `SearchResultRow` carried a snippet from the `unified_search` era whose cost was
    // never isolated either. Filed, not guessed.
}

/// A resource that matched an idea rather than words. Produced by both `find-about-*` acts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct VecHit {
    pub resource: ResourceView,
    /// The pgvector cosine DISTANCE (span `[0,2]`) rescaled as `1 - d/2`, landing in `[0,1]`.
    ///
    /// NOT the same quantity as [`RegionHit::region_score`], which rescales the identical operator
    /// as `1 - d` and spans `[-1,1]`.
    pub vec_norm: f32,
    /// Where in the resource the match was — the closest chunk's block.
    ///
    /// **This field exists on this hit type and nowhere else**, and that is a statement about the
    /// two arms rather than an oversight: the wide arm matches at CHUNK grain and already computes
    /// which chunk was closest, then discards it collapsing to a per-resource score. Recovering it
    /// is an argmin beside an aggregate that already runs.
    ///
    /// Absent means the match was not localized, which for this act should not happen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub located_at: Option<MatchLocation>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

/// A neighbour, reached by walking. Produced by `follow-from`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct GraphHit {
    pub resource: ResourceView,
    /// **UNBOUNDED.** The walk multiplies `kb_edges.weight` at every hop, and that column is
    /// `DOUBLE PRECISION NOT NULL DEFAULT 1.0` with NO CHECK constraint. Today's values stay under
    /// 1 because nothing writes a larger weight — a property of the DATA, not of the quantity.
    pub graph_score: f32,
}

/// A region of a cognitive map. Produced by `survey`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "query.ts"))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct RegionHit {
    /// The region itself, in the same shape `cogmap_shape` answers in.
    ///
    /// **It carries its own `salience`, beside this hit's `region_score`.** Two numbers on one row,
    /// and they are not rivals: `salience` is an INPUT to `region_score` (`0.4·sal_norm +
    /// 0.6·query_cos`, where `sal_norm` is a rank over salience). Order by `region_score`; ordering
    /// by `salience` answers *"what does this map think is prominent"*, a different question.
    ///
    /// `[decided — 2026-08-08, Pete]` A narrower region projection without `salience` was
    /// considered and refused. [`ResourceView`] exists because six near-identical projections were
    /// collapsed into one; minting a divergent region shape immediately afterwards, to hide one
    /// field, would repeat the mistake that consolidation just paid to fix.
    pub region: CogmapRegionRow,
    /// Spans `[-0.6, 1.0]`, **not** `[0,1]`. `0.4*sal_norm + 0.6*query_cos` where `sal_norm` is a
    /// `percent_rank` in `[0,1]` but `query_cos = 1 - distance` over a cosine distance spanning
    /// `[0,2]`, so the similarity term spans `[-1,1]`.
    ///
    /// Read `orders_by.scale` on the stage rather than assuming.
    pub region_score: f32,
}
