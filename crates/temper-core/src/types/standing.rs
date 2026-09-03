//! Evidential-standing wire type (Set 5 model, spec 019f81e8) — the read-surface shape of a
//! finding's standing, as returned by SQL `resource_standing_shape`
//! (`migrations/20260724000120_standing_citation_components.sql`).
//!
//! **Standing is not truth, and the system cannot close the gap between them — only make its
//! shape visible** (spec §Bedrock preamble). This type does not carry a truth claim about the
//! finding; it carries a fact about the *structure of emitted evidence and its relations*: the
//! six components of `StandingShape` plus the lossy `band` summary computed over them.
//!
//! Standing is shape-primary (spec §1.1): the vector of components IS the standing. `band` is a
//! lossy read-time summary computed OVER that shape — always carried WITH the shape, on this same
//! struct, and never returned in place of it. There is no canonical view / no view from nowhere;
//! every consumer handles the full vector, not a single scalar verdict.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::types::ids::ResourceId;

/// A finding's evidential-standing shape (SQL `resource_standing_shape`). All fields are
/// non-nullable: the access gate is INSIDE the SQL (a `gated` CTE over `resources_readable_by`),
/// so an unreadable finding yields zero rows — never a partial/nullable row — and the caller-side
/// read returns `Option<StandingShape>`, `None` for "not readable."
///
/// Reader-independent by decision: one value for every caller who can read the finding, with
/// per-contributor attribution on the citation-audit trail, never on the shape (stored-aggregates
/// exception: `internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md`).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "standing.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct StandingShape {
    /// `kb_resources.id` of the finding this shape describes. Reader-independent by decision: one
    /// value for every caller who can read the finding, with per-contributor attribution on the
    /// citation-audit trail, never on the shape (stored-aggregates exception:
    /// `internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md`).
    pub finding_id: ResourceId,
    /// Count of **distinct live** sources the finding cites (Set 5, spec §3.1). Monotone — citing
    /// more evidence never lowers it. The *findability* axis: deliberately NOT `r_parent`, which
    /// counts provenance rows including duplicates. Ten citations of one source is
    /// `r_parent = 10, citation_magnitude = 1`, and collapsing the two reintroduces the
    /// actor-count fallacy.
    ///
    /// Reader-independent by decision: one value for every caller who can read the finding, with
    /// per-contributor attribution on the citation-audit trail, never on the shape
    /// (stored-aggregates exception:
    /// `internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md`).
    pub citation_magnitude: i32,
    /// How many of those distinct sources carry at least one audit. Monotone under the append-only
    /// audit trail (spec §4.1) — once a source is audited it stays covered. The *evaluated-ness*
    /// axis, and the thing that tells "nobody tried" apart from "N sources were evaluated."
    ///
    /// Reader-independent by decision: one value for every caller who can read the finding, with
    /// per-contributor attribution on the citation-audit trail, never on the shape
    /// (stored-aggregates exception:
    /// `internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md`).
    pub audit_coverage: i32,
    /// Mean, over the **audited subset only**, of each audited source's decay-weighted audit value,
    /// in `[-1.0, 1.0]`. Reads as a neutral `0.0` when `audit_coverage = 0`: an unaudited finding
    /// makes no quality claim, and its low standing comes from the band gate, never from a poisoned
    /// mean (spec §3.2).
    ///
    /// Reader-independent by decision: one value for every caller who can read the finding, with
    /// per-contributor attribution on the citation-audit trail, never on the shape
    /// (stored-aggregates exception:
    /// `internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md`).
    pub citation_quality: f64,
    /// Supports minus contradicts, as a vector-sum over declared edges (spec §1) — not a headcount.
    ///
    /// Reader-independent by decision: one value for every caller who can read the finding, with
    /// per-contributor attribution on the citation-audit trail, never on the shape
    /// (stored-aggregates exception:
    /// `internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md`).
    pub contradiction_balance: f64,
    /// Reversible time-decay off the finding's most recent uncorrected reinforcement; computed
    /// live at read (never from the memo) because it must reflect the current moment.
    ///
    /// Reader-independent by decision: one value for every caller who can read the finding, with
    /// per-contributor attribution on the citation-audit trail, never on the shape
    /// (stored-aggregates exception:
    /// `internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md`).
    pub freshness: f64,
    /// Reinforcement breadth: count of uncorrected provenance over the finding's live blocks.
    ///
    /// Reader-independent by decision: one value for every caller who can read the finding, with
    /// per-contributor attribution on the citation-audit trail, never on the shape
    /// (stored-aggregates exception:
    /// `internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md`).
    pub r_parent: f64,
    /// Lossy read-time summary band (`provisional` / `reinforced` / `disputed` / `near-canonical`)
    /// computed over the shape above. Carried WITH the shape, never presented instead of it
    /// (spec §1.1).
    ///
    /// Reader-independent by decision: one value for every caller who can read the finding, with
    /// per-contributor attribution on the citation-audit trail, never on the shape
    /// (stored-aggregates exception:
    /// `internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md`).
    pub band: String,
}
