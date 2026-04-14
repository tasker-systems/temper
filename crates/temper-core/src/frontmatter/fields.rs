//! Consolidated frontmatter field constants. Single source of truth for
//! every place in the codebase that needs to know "is X an identity field?"
//! / "is X a tier-1 system field?" / "is X a system-managed managed-tier
//! field?".
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

pub use crate::schema::SYSTEM_MANAGED_FIELDS; // moved in Task 3

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_managed_fields_match_schema_module() {
        assert_eq!(SYSTEM_MANAGED_FIELDS, crate::schema::SYSTEM_MANAGED_FIELDS);
    }

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
}
