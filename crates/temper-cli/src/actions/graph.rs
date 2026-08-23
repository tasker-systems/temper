//! `temper graph` business logic — thin wrappers over the graph client's two reads,
//! plus the bound checks the CLI makes **before** the wire.
//!
//! **Why the CLI refuses instead of letting the service clamp.** `AtlasEntry` declares its own
//! bounds, so an entry read can tell a caller what it did. `AtlasSubgraph` has no bounds field, so
//! a traversal that trips `TRAVERSAL_MAX_SEEDS` or walks past the depth ceiling is clamped
//! server-side and the response says nothing about it. A caller cannot distinguish "walked your
//! 400 seeds" from "walked 250 of them." Refusing locally is the only disclosure available without
//! changing the wire type: nothing out of bounds is ever sent, so nothing is silently clamped.
//!
//! The bounds are read from `temper_core`, not restated here — one copy, shared with the service.

use temper_core::error::TemperError;
use temper_core::types::graph_atlas::{
    AtlasEntry, AtlasSubgraph, TRAVERSAL_DEPTH_RANGE, TRAVERSAL_MAX_SEEDS,
};
use uuid::Uuid;

use crate::error::Result;

/// Check a traversal request against the bounds the service enforces, before sending it.
pub fn validate_traverse_bounds(seed_count: usize, depth: Option<i32>) -> Result<()> {
    if seed_count == 0 {
        return Err(TemperError::BadRequest(
            "traverse needs at least one seed (--from)".to_string(),
        ));
    }
    if seed_count > TRAVERSAL_MAX_SEEDS {
        return Err(TemperError::BadRequest(format!(
            "at most {TRAVERSAL_MAX_SEEDS} seeds (--from); got {seed_count}. \
             The service walks the first {TRAVERSAL_MAX_SEEDS} and the response cannot say it \
             dropped the rest, so this is refused rather than silently truncated"
        )));
    }
    if let Some(depth) = depth {
        if !TRAVERSAL_DEPTH_RANGE.contains(&depth) {
            return Err(TemperError::BadRequest(format!(
                "--depth must be {}..={}; got {depth}. The service clamps out-of-range depth and \
                 the response cannot say it did, so this is refused rather than silently walked \
                 at a depth you did not ask for",
                TRAVERSAL_DEPTH_RANGE.start(),
                TRAVERSAL_DEPTH_RANGE.end()
            )));
        }
    }
    Ok(())
}

/// Call the entry read for this principal, optionally confined to named anchors.
pub async fn entry_api(
    client: &temper_client::TemperClient,
    anchors: &[Uuid],
    k: Option<i32>,
) -> Result<AtlasEntry> {
    client
        .graph()
        .entry(anchors, k)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)
}

/// Call the traversal read from already-resolved seed ids.
pub async fn traverse_api(
    client: &temper_client::TemperClient,
    seeds: &[Uuid],
    depth: Option<i32>,
) -> Result<AtlasSubgraph> {
    client
        .graph()
        .traverse(seeds, depth)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(seed_count: usize, depth: Option<i32>) -> String {
        validate_traverse_bounds(seed_count, depth)
            .expect_err("expected a refusal")
            .to_string()
    }

    #[test]
    fn a_seed_set_at_the_cap_is_accepted() {
        assert!(validate_traverse_bounds(TRAVERSAL_MAX_SEEDS, None).is_ok());
    }

    #[test]
    fn one_seed_past_the_cap_is_refused_and_the_message_names_both_numbers() {
        let msg = err(TRAVERSAL_MAX_SEEDS + 1, None);
        assert!(msg.contains("250"), "message must name the bound: {msg}");
        assert!(
            msg.contains("251"),
            "message must name what was asked for: {msg}"
        );
    }

    /// The service returns `BadRequest("seeds must be non-empty")` for this. Refusing locally
    /// spends no round-trip on a request that cannot succeed.
    #[test]
    fn an_empty_seed_set_is_refused_locally() {
        let msg = err(0, None);
        assert!(
            msg.to_lowercase().contains("seed"),
            "message must say what is missing: {msg}"
        );
    }

    #[test]
    fn both_ends_of_the_depth_range_are_accepted() {
        assert!(validate_traverse_bounds(1, Some(1)).is_ok());
        assert!(validate_traverse_bounds(1, Some(3)).is_ok());
    }

    /// Depth 0 is the induced-subgraph read, not a degenerate walk — the service excludes it
    /// deliberately, so the CLI must not quietly turn it into 1.
    #[test]
    fn depth_below_the_range_is_refused() {
        let msg = err(1, Some(0));
        assert!(msg.contains("1..=3"), "message must name the range: {msg}");
    }

    #[test]
    fn depth_above_the_range_is_refused_rather_than_clamped() {
        let msg = err(1, Some(7));
        assert!(msg.contains("1..=3"), "message must name the range: {msg}");
        assert!(msg.contains('7'), "message must name what was asked: {msg}");
    }

    #[test]
    fn an_unnamed_depth_is_accepted_because_the_handler_defaults_it() {
        assert!(validate_traverse_bounds(1, None).is_ok());
    }

    /// The range constant is the one the service reads. If it ever widens, this test is how the
    /// CLI's refusal follows it rather than drifting.
    #[test]
    fn the_range_the_cli_enforces_is_the_shared_one() {
        assert_eq!(*TRAVERSAL_DEPTH_RANGE.start(), 1);
        assert_eq!(*TRAVERSAL_DEPTH_RANGE.end(), 3);
    }
}
