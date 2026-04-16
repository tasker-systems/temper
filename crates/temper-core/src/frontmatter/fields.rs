//! Consolidated frontmatter field constants. Single source of truth for
//! every place in the codebase that needs to know "is X an identity field?"
//! / "is X a tier-1 system field?" / "is X a system-managed managed-tier
//! field?" / "is X a known temper-* field?" / "is X a legacy-form alias?".
//!
//! Owned here in Session 3 (Session 1 re-exported these from hash.rs and
//! schema.rs to keep the additive phase strictly non-breaking).

/// Identity fields are never included in any hash tier — they identify the
/// record but aren't content. Always rendered first in the canonical
/// display order.
pub const IDENTITY_FIELDS: &[&str] = &["temper-id", "temper-provisional-id"];

/// Tier-1 system fields are stripped from managed metadata before hashing.
/// The database owns authoritative values for these, so they must not
/// influence the content hash.
pub const TIER1_SYSTEM_FIELDS: &[&str] = &[
    "temper-context",
    "temper-type",
    "temper-created",
    "temper-updated",
    "temper-owner",
    "temper-source",
    "temper-legacy-id",
];

/// All temper-* field names that are explicitly defined across the schemas.
/// Used to detect possible typos in temper-* fields.
pub static KNOWN_TEMPER_FIELDS: &[&str] = &[
    "temper-id",
    "temper-provisional-id",
    "temper-type",
    "temper-context",
    "temper-created",
    "temper-updated",
    "temper-owner",
    "temper-source",
    // task
    "temper-stage",
    "temper-mode",
    "temper-effort",
    "temper-goal",
    "temper-seq",
    "temper-branch",
    "temper-pr",
    // goal
    "temper-status",
    // session, research, decision, concept have no extra temper-* beyond base
    // LLM-assist managed fields
    "temper-provenance",
    "temper-llm-model",
    "temper-llm-run",
];

/// Legacy field names that have been renamed to temper-* equivalents.
/// Maps old name → suggested new name.
pub static LEGACY_FIELDS: &[(&str, &str)] = &[
    ("id", "temper-id"),
    ("type", "temper-type"),
    ("doc_type", "temper-type"),
    ("context", "temper-context"),
    ("project", "temper-context"),
    ("created", "temper-created"),
    ("updated", "temper-updated"),
    ("source", "temper-source"),
    ("stage", "temper-stage"),
    ("status", "temper-status"),
    ("mode", "temper-mode"),
    ("effort", "temper-effort"),
    ("goal", "temper-goal"),
    ("branch", "temper-branch"),
    ("pr", "temper-pr"),
];

/// Fields that are system-managed and cannot be updated via CLI.
pub static SYSTEM_MANAGED_FIELDS: &[&str] = &[
    "temper-id",
    "temper-provisional-id",
    "temper-type",
    "temper-context",
    "temper-owner",
    "temper-created",
    "temper-updated",
    "temper-source",
    "temper-legacy-id",
    "slug",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_fields_contains_expected_keys() {
        assert!(IDENTITY_FIELDS.contains(&"temper-id"));
        assert!(IDENTITY_FIELDS.contains(&"temper-provisional-id"));
    }

    #[test]
    fn tier1_fields_contains_expected_keys() {
        for key in [
            "temper-context",
            "temper-type",
            "temper-created",
            "temper-updated",
            "temper-owner",
            "temper-source",
        ] {
            assert!(TIER1_SYSTEM_FIELDS.contains(&key), "missing key {key}");
        }
    }

    #[test]
    fn known_temper_fields_includes_lifecycle_keys() {
        for key in [
            "temper-stage",
            "temper-mode",
            "temper-effort",
            "temper-goal",
        ] {
            assert!(KNOWN_TEMPER_FIELDS.contains(&key), "missing key {key}");
        }
    }

    #[test]
    fn known_temper_fields_includes_llm_managed_fields() {
        for key in ["temper-provenance", "temper-llm-model", "temper-llm-run"] {
            assert!(KNOWN_TEMPER_FIELDS.contains(&key), "missing key {key}");
        }
    }

    #[test]
    fn legacy_fields_map_id_and_type() {
        assert!(LEGACY_FIELDS.contains(&("id", "temper-id")));
        assert!(LEGACY_FIELDS.contains(&("type", "temper-type")));
    }

    #[test]
    fn system_managed_fields_includes_temper_owner() {
        assert!(SYSTEM_MANAGED_FIELDS.contains(&"temper-owner"));
    }
}
