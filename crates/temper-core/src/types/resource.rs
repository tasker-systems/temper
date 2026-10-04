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

/// A resource's ingest state — a **projection** of the append-only `kb_events` ledger
/// (`resource_created` → `block_created`… → `resource_finalized`), not an independently-mutated flag.
/// The ledger is the state machine; this is its materialized current-state view, kept as a column so
/// list/search can filter it with a cheap read instead of scanning events.
///
/// Two wire values. `InProgress` is "the body is not whole": it covers an ingest still arriving and
/// an ingest that ended before it finalized; [`IngestEnded`] says which, and names the reason.
/// `Complete` is the whole body.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum IngestState {
    /// The body is not whole: a segmented ingest has begun and not been finalized, or the ingest
    /// ended before it finalized (see [`IngestEnded`]). Hidden from list/search, still readable via
    /// `show`.
    InProgress,
    /// The whole body is present: every atomic create, and every finalized segmented ingest.
    Complete,
}

impl IngestState {
    /// The canonical wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            IngestState::InProgress => "in_progress",
            IngestState::Complete => "complete",
        }
    }

    /// Parse a wire string. This type parses the two wire values; the `ck_kb_resources_ingest_state`
    /// column admits four (see [`IngestState::from_db`] for the projection of all of them). An
    /// unrecognized string is a schema/version violation, not ordinary input — returned as `None`
    /// for the caller to handle rather than silently coerced.
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "in_progress" => Some(IngestState::InProgress),
            "complete" => Some(IngestState::Complete),
            _ => None,
        }
    }

    /// Project a `kb_resources.ingest_state` column value onto the wire pair: the state, and the
    /// reason when the ingest ended before its body was whole. `cancelled` and `abandoned` are
    /// terminal DB states that read `in_progress` on the wire, with the reason in [`IngestEnded`].
    /// `None` for a string the column does not admit.
    pub fn from_db(s: &str) -> Option<(Self, Option<IngestEnded>)> {
        match s {
            "in_progress" => Some((IngestState::InProgress, None)),
            "complete" => Some((IngestState::Complete, None)),
            "cancelled" => Some((IngestState::InProgress, Some(IngestEnded::Cancelled))),
            "abandoned" => Some((IngestState::InProgress, Some(IngestEnded::Abandoned))),
            _ => None,
        }
    }
}

/// The terminal reason an ingest stopped before its body was whole. Present on a resource view only
/// beside `ingest_state = in_progress`; the body is incomplete and nothing more will arrive.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum IngestEnded {
    /// An operator act ended the ingest: the block history scrub, when its `cancelled_ingest` is
    /// true.
    Cancelled,
    /// Reserved for an abandoned-ingest reaper; nothing sets it yet.
    Abandoned,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_db_in_progress_is_in_progress_with_no_reason() {
        assert_eq!(
            IngestState::from_db("in_progress"),
            Some((IngestState::InProgress, None))
        );
    }

    #[test]
    fn from_db_complete_is_complete_with_no_reason() {
        assert_eq!(
            IngestState::from_db("complete"),
            Some((IngestState::Complete, None))
        );
    }

    #[test]
    fn from_db_cancelled_reads_in_progress_with_cancelled_reason() {
        assert_eq!(
            IngestState::from_db("cancelled"),
            Some((IngestState::InProgress, Some(IngestEnded::Cancelled)))
        );
    }

    #[test]
    fn from_db_abandoned_reads_in_progress_with_abandoned_reason() {
        assert_eq!(
            IngestState::from_db("abandoned"),
            Some((IngestState::InProgress, Some(IngestEnded::Abandoned)))
        );
    }

    #[test]
    fn from_db_rejects_a_string_the_column_does_not_admit() {
        assert_eq!(IngestState::from_db("finished"), None);
        assert_eq!(IngestState::from_db(""), None);
    }
}
