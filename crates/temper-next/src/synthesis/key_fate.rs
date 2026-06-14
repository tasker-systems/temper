//! The §7 manifest-key fate table (WS6 convergence-delta spec §7; plan G7) — the single place that
//! decides what each production manifest key becomes in the destination shape. One enum, one match:
//! no stringly-typed scatter across the synthesis passes (the "no stringly-typed matches over bounded
//! sets" rule). The property pass consumes [`KeyFate::Property`]; the edge pass consumes
//! [`KeyFate::Edge`] (Task 8); `Die`/`ReconcileToDocType` keys are dropped.

/// What a production **managed** manifest key becomes during synthesis (spec §7 fate table). `open_meta`
/// keys are always properties verbatim and never reach this classifier — they carry unconditionally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyFate {
    /// A `kb_properties` row, key + value verbatim — the workflow fields (`temper-stage`/`-mode`/
    /// `-effort`/`-status`/`-seq`) and provenance fields (`temper-llm-run`/`-provenance`/`-branch`/
    /// `-pr`/`date`).
    Property,
    /// Dropped — authoritative state already carries it: `temper-title` is `kb_resources.title`,
    /// `temper-slug` is render-time decoration, `temper-id` is the row id, `temper-context` derives
    /// from the home row.
    Die,
    /// A `kb_edges` row, NOT a property — `temper-goal`, minted by the edge pass (Task 8) using the
    /// kind+label the frontmatter-edge projection emits.
    Edge,
    /// Reconciled against the authoritative doctype column — the column wins, the stray dies
    /// (`temper-type`); `doc_type` is already a property from the resource pass.
    ReconcileToDocType,
}

/// The managed manifest keys whose §7 fate is [`KeyFate::Property`] — the workflow fields
/// (`temper-stage`/`-mode`/`-effort`/`-status`/`-seq`) and the provenance fields
/// (`temper-llm-run`/`-provenance`/`-branch`/`-pr`/`date`). Single source of truth for both directions
/// of the fate: the forward [`key_fate`] classifier matches on it, and the read path
/// ([`crate::readback::meta`]) uses [`is_managed_property_key`] to tell a managed workflow/provenance
/// key apart from an open (user-defined) one — a distinction [`key_fate`] alone cannot make, because it
/// returns [`KeyFate::Property`] for UNKNOWN keys too (the conservative carry).
pub const MANAGED_PROPERTY_KEYS: &[&str] = &[
    "temper-stage",
    "temper-mode",
    "temper-effort",
    "temper-status",
    "temper-seq",
    "temper-llm-run",
    "temper-provenance",
    "temper-branch",
    "temper-pr",
    "date",
];

/// True iff `key` is one of the managed property keys ([`MANAGED_PROPERTY_KEYS`]) — i.e. a managed
/// workflow/provenance key that survives §7 as a `kb_properties` row. The read path uses this as the
/// inverse fate: a surviving property whose key is in this set is managed; anything else is open.
pub fn is_managed_property_key(key: &str) -> bool {
    MANAGED_PROPERTY_KEYS.contains(&key)
}

/// The fate of one **managed** manifest key per the §7 table (G7). The 16 managed keys that exist in
/// production are all enumerated; an unrecognized managed key defaults to [`KeyFate::Property`] — the
/// conservative carry (workflow-meta verbatim, never a silent drop).
pub fn key_fate(key: &str) -> KeyFate {
    match key {
        "temper-title" | "temper-slug" | "temper-id" | "temper-context" => KeyFate::Die,
        "temper-goal" => KeyFate::Edge,
        "temper-type" => KeyFate::ReconcileToDocType,
        // Two arms that intentionally share an outcome but NOT a meaning (kept distinct on purpose,
        // not dead code): the guard is the KNOWN workflow/provenance managed keys, carried verbatim and
        // single-sourced via `MANAGED_PROPERTY_KEYS` (this is the linkage `readback::meta`'s inverse
        // `is_managed_property_key` relies on); the wildcard is the UNKNOWN managed key — same
        // conservative `Property` carry (never a silent drop), but a different case the reader should see.
        k if is_managed_property_key(k) => KeyFate::Property,
        _ => KeyFate::Property,
    }
}

#[cfg(test)]
mod tests {
    use super::{key_fate, KeyFate};

    #[test]
    fn fate_table_encodes_section_7_exactly() {
        for k in ["temper-title", "temper-slug", "temper-id", "temper-context"] {
            assert_eq!(key_fate(k), KeyFate::Die, "{k} dies");
        }
        for k in [
            "temper-stage",
            "temper-mode",
            "temper-effort",
            "temper-status",
            "temper-seq",
            "temper-llm-run",
            "temper-provenance",
            "temper-branch",
            "temper-pr",
            "date",
        ] {
            assert_eq!(key_fate(k), KeyFate::Property, "{k} is a property");
        }
        assert_eq!(key_fate("temper-goal"), KeyFate::Edge);
        assert_eq!(key_fate("temper-type"), KeyFate::ReconcileToDocType);
        // An unknown managed key carries as a property (conservative — never a silent drop).
        assert_eq!(key_fate("temper-unheard-of"), KeyFate::Property);
    }
}
