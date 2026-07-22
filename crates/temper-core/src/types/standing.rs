//! Evidential-standing wire type (Set 3, spec 019f81e8) — the read-surface shape of a finding's
//! standing, as returned by SQL `resource_standing_shape` (`migrations/20260721000010`).
//!
//! **Standing is not truth, and the system cannot close the gap between them — only make its
//! shape visible** (spec §Bedrock preamble). This type does not carry a truth claim about the
//! finding; it carries a fact about the *structure of emitted evidence and its relations*:
//! independence-discounted breadth, adversarial survival, contradiction balance, and freshness.
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
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "standing.ts"))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct StandingShape {
    /// `kb_resources.id` of the finding this shape describes.
    pub finding_id: ResourceId,
    /// Independence-discounted breadth over the finding's evidentiary bases (spec §2.1). Silence
    /// default: an unasserted pair is assumed correlated, not independent.
    pub indep_breadth: f64,
    /// N challenges withstood (`resource_adversarial_survival`); 0 when there have been no
    /// challenges yet — distinct from a genuine zero-survival outcome (see `challenge_count`).
    pub adversarial_survival: f64,
    /// Count of adversarial challenges raised against the finding, so a consumer can distinguish
    /// "0 challenges" from "N challenges, 0 withstood."
    pub challenge_count: i32,
    /// Supports minus contradicts, as a vector-sum over declared edges (spec §1) — not a headcount.
    pub contradiction_balance: f64,
    /// Reversible time-decay off the finding's most recent uncorrected reinforcement; computed
    /// live at read (never from the memo) because it must reflect the current moment.
    pub freshness: f64,
    /// Reinforcement breadth: count of uncorrected provenance over the finding's live blocks.
    pub r_parent: f64,
    /// Lossy read-time summary band (`provisional` / `reinforced` / `near-canonical`) computed over
    /// the shape above. Carried WITH the shape, never presented instead of it (spec §1.1).
    pub band: String,
}
