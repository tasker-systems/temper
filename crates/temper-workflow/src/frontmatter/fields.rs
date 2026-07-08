//! Consolidated frontmatter field constants. Single source of truth for
//! every place in the codebase that needs to know "is X an identity field?"
//! / "is X a tier-1 system field?" / "is X a system-managed managed-tier
//! field?" / "is X a known temper-* field?".

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
    // managed-tier identity (post-rename)
    "temper-title",
    "temper-slug",
    // task
    "temper-stage",
    "temper-mode",
    "temper-effort",
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
    "temper-slug",
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
        for key in ["temper-stage", "temper-mode", "temper-effort"] {
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
    fn system_managed_fields_includes_temper_owner() {
        assert!(SYSTEM_MANAGED_FIELDS.contains(&"temper-owner"));
    }

    #[test]
    fn known_temper_fields_includes_temper_title_and_temper_slug() {
        assert!(
            KNOWN_TEMPER_FIELDS.contains(&"temper-title"),
            "missing temper-title"
        );
        assert!(
            KNOWN_TEMPER_FIELDS.contains(&"temper-slug"),
            "missing temper-slug"
        );
    }

    #[test]
    fn system_managed_fields_uses_temper_slug_not_bare_slug() {
        assert!(
            SYSTEM_MANAGED_FIELDS.contains(&"temper-slug"),
            "expected temper-slug in SYSTEM_MANAGED_FIELDS"
        );
        assert!(
            !SYSTEM_MANAGED_FIELDS.contains(&"slug"),
            "bare `slug` should have been renamed to `temper-slug`"
        );
    }
}
