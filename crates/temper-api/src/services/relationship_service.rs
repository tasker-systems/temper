//! Relationship service — appends `relationship_*` events to the ledger and
//! projects their edge deltas into `kb_resource_edges` within one transaction.
//!
//! The ledger is truth; `kb_resource_edges` is a rebuildable projection.
//! `apply_relationship_event` does the incremental delta; `rebuild_edge_projection`
//! replays the whole stream. See the limb-1 design spec.

use temper_core::types::graph::EdgeKind;

/// Topic UUIDs seeded by migration 20260522100001.
pub const TOPIC_DECLARATION: &str = "019e3d6f-2300-7000-8000-000000000050";
pub const TOPIC_DEFORMATION: &str = "019e3d6f-2300-7000-8000-000000000051";
pub const TOPIC_JUDGMENT: &str = "019e3d6f-2300-7000-8000-000000000052";

/// Validation: a relationship label must be non-empty. The mandatory-label
/// rule stops `near` (and every kind) becoming a vague catch-all. An empty or
/// whitespace-only label is rejected for every kind.
pub fn validate_assertion_label(kind: EdgeKind, label: &str) -> Result<(), String> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return Err("relationship label must be non-empty".to_string());
    }
    let _ = kind; // kind-specific banned-generic-label checks may tighten later
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_label_is_rejected() {
        assert!(validate_assertion_label(EdgeKind::Near, "   ").is_err());
        assert!(validate_assertion_label(EdgeKind::Contains, "").is_err());
    }

    #[test]
    fn non_empty_label_is_accepted() {
        assert!(validate_assertion_label(EdgeKind::LeadsTo, "depends_on").is_ok());
    }
}
