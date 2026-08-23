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
//! The bounds are read from `temper_core`, not restated here. Depth goes further than a shared
//! constant: the service clamps through `clamp_traversal_depth`, so the range this refuses on and
//! the range the service enforces cannot come apart.

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

/// How one `--in` anchor must be resolved.
///
/// `--in` takes "a context or cogmap ref", and those two spell themselves differently. A cogmap ref
/// is the decorated `slug-<uuid>` form every resource ref uses, resolvable locally. A context ref is
/// `@me/slug` / `@handle/slug` / `+team-slug/slug`, which carries no uuid and needs the server.
///
/// Splitting the decision out keeps it pure and testable: the async resolution below is a thin
/// dispatch over this, and the classification itself never needs a client.
#[derive(Debug, PartialEq, Eq)]
pub enum AnchorRef {
    /// Already addressable — a bare UUID or a decorated `slug-<uuid>`.
    Id(Uuid),
    /// An owner-qualified context ref that only the server can resolve.
    ContextRef(String),
}

/// Classify one anchor without touching the network.
pub fn classify_anchor(raw: &str) -> AnchorRef {
    match temper_workflow::operations::parse_ref(raw) {
        Ok(parsed) => AnchorRef::Id(parsed.0),
        // Not locally addressable. Deferred rather than refused here, so the error a caller reads
        // comes from the resolver that actually looked.
        Err(_) => AnchorRef::ContextRef(raw.to_string()),
    }
}

/// Resolve `--in` anchors, reaching the server only for the refs that need it.
pub async fn resolve_anchors(
    client: &temper_client::TemperClient,
    raw: &[String],
) -> Result<Vec<Uuid>> {
    let mut ids = Vec::with_capacity(raw.len());
    for r in raw {
        match classify_anchor(r) {
            AnchorRef::Id(id) => ids.push(id),
            AnchorRef::ContextRef(ctx) => ids.push(
                crate::commands::context_cmd::resolve_context_id_for_read(client, &ctx).await?,
            ),
        }
    }
    Ok(ids)
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

    const COGMAP: &str = "temper-self-cognition-019f2391-e001-7933-b88a-28fb92e56ac1";

    #[test]
    fn a_decorated_ref_is_resolved_locally() {
        match classify_anchor(COGMAP) {
            AnchorRef::Id(id) => assert_eq!(
                id.to_string(),
                "019f2391-e001-7933-b88a-28fb92e56ac1",
                "the trailing uuid is the address"
            ),
            other => panic!("expected a local id, got {other:?}"),
        }
    }

    #[test]
    fn a_bare_uuid_is_resolved_locally() {
        let raw = "019f2391-e001-7933-b88a-28fb92e56ac1";
        assert_eq!(classify_anchor(raw), AnchorRef::Id(raw.parse().unwrap()));
    }

    /// The defect this cycle fixes. `--in @me/temper` is the form every other context-taking flag
    /// accepts, and it was refused locally with "not a resource ref", which does not hint that an
    /// `@me/` ref was ever the right thing to type.
    #[test]
    fn an_at_me_context_ref_is_deferred_to_the_server() {
        assert_eq!(
            classify_anchor("@me/temper"),
            AnchorRef::ContextRef("@me/temper".to_string())
        );
    }

    #[test]
    fn a_handle_qualified_context_ref_is_deferred_to_the_server() {
        assert_eq!(
            classify_anchor("@j-cole-taylor/temper"),
            AnchorRef::ContextRef("@j-cole-taylor/temper".to_string())
        );
    }

    #[test]
    fn a_team_qualified_context_ref_is_deferred_to_the_server() {
        assert_eq!(
            classify_anchor("+acme/roadmap"),
            AnchorRef::ContextRef("+acme/roadmap".to_string())
        );
    }

    /// Anything unrecognizable is deferred rather than refused here, so the error a caller reads
    /// comes from the resolver that actually looked — "not found among the contexts you can see"
    /// beats a local parse complaint that names the wrong grammar.
    #[test]
    fn an_unrecognizable_anchor_is_deferred_rather_than_refused_locally() {
        assert_eq!(
            classify_anchor("nonsense"),
            AnchorRef::ContextRef("nonsense".to_string())
        );
    }
}
