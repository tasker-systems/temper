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

/// The fate of one **managed** manifest key per the §7 table (G7). The 16 managed keys that exist in
/// production are all enumerated; an unrecognized managed key defaults to [`KeyFate::Property`] — the
/// conservative carry (workflow-meta verbatim, never a silent drop).
pub fn key_fate(key: &str) -> KeyFate {
    match key {
        "temper-title" | "temper-slug" | "temper-id" | "temper-context" => KeyFate::Die,
        "temper-stage" | "temper-mode" | "temper-effort" | "temper-status" | "temper-seq"
        | "temper-llm-run" | "temper-provenance" | "temper-branch" | "temper-pr" | "date" => {
            KeyFate::Property
        }
        "temper-goal" => KeyFate::Edge,
        "temper-type" => KeyFate::ReconcileToDocType,
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
