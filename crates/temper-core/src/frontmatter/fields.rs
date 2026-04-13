//! Consolidated frontmatter field constants. Session 1 re-exports from
//! their existing locations; Session 3 moves them here properly.
//!
//! Downstream files in `crate::frontmatter` should import from this
//! module exclusively, so Session 3's move is a purely local edit.

pub use crate::hash::{IDENTITY_FIELDS, TIER1_SYSTEM_FIELDS};
pub use crate::schema::SYSTEM_MANAGED_FIELDS;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_fields_match_hash_module() {
        assert_eq!(IDENTITY_FIELDS, crate::hash::IDENTITY_FIELDS);
    }

    #[test]
    fn tier1_system_fields_match_hash_module() {
        assert_eq!(TIER1_SYSTEM_FIELDS, crate::hash::TIER1_SYSTEM_FIELDS);
    }

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
