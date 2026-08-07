//! Resource body-state enums — the two `snake_case` DB column vocabularies a read surfaces.
//!
//! These live here rather than in temper-workflow because both are fields of
//! [`ResourceView`], which temper-substrate — the layer *below* temper-workflow —
//! reads back directly. `temper_workflow::types::resource` re-exports both at their
//! incumbent paths; the row/request/response shapes around them stay there.
//!
//! [`ResourceView`]: crate::types::resource_view::ResourceView

use serde::{Deserialize, Serialize};

/// What guarantee a resource's body carries on read — a **surfaced projection** of coverage
/// (`kb_resources.body_storage`, recomputed by the block projectors), not an independently-set flag.
/// Orthogonal to [`IngestState`]: that asks *are all the bytes here?*, this asks *do the bytes I have
/// read back exactly, or only approximately?*
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum BodyStorage {
    /// Every live block carries its raw source bytes; the body reads back byte-for-byte.
    Verbatim,
    /// The body is reconstructed from chunks (lossy) — a pre-PR-3 resource, or one with only partial
    /// verbatim coverage.
    Derived,
}

impl BodyStorage {
    /// The canonical wire/DB string (matches the `ck_kb_resources_body_storage` CHECK values).
    pub fn as_str(self) -> &'static str {
        match self {
            BodyStorage::Verbatim => "verbatim",
            BodyStorage::Derived => "derived",
        }
    }

    /// Parse the DB/wire string. The `ck_kb_resources_body_storage` CHECK constrains the column to
    /// these two values, so an unrecognized string is a schema/version violation, not ordinary input —
    /// returned as `None` for the caller to handle rather than silently coerced.
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "verbatim" => Some(BodyStorage::Verbatim),
            "derived" => Some(BodyStorage::Derived),
            _ => None,
        }
    }
}

/// A resource's ingest-completion state — a **projection** of the append-only `kb_events` ledger
/// (`resource_created` → `block_created`… → `resource_finalized`), not an independently-mutated flag.
/// The ledger is the state machine; this is its materialized current-state view, kept as a column so
/// list/search can filter it with a cheap read instead of scanning events.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum IngestState {
    /// A segmented ingest has begun but not been finalized — the body is incomplete. Hidden from
    /// list/search, still resumable and readable via `show`.
    InProgress,
    /// The whole body is present: every atomic create, and every finalized segmented ingest.
    Complete,
}

impl IngestState {
    /// The canonical wire/DB string (matches the `ck_kb_resources_ingest_state` CHECK values).
    pub fn as_str(self) -> &'static str {
        match self {
            IngestState::InProgress => "in_progress",
            IngestState::Complete => "complete",
        }
    }

    /// Parse the DB/wire string. The `ck_kb_resources_ingest_state` CHECK constrains the column to
    /// these two values, so an unrecognized string is a schema/version violation, not ordinary input —
    /// returned as `None` for the caller to handle rather than silently coerced.
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "in_progress" => Some(IngestState::InProgress),
            "complete" => Some(IngestState::Complete),
            _ => None,
        }
    }
}
