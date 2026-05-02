//! Pure shared actions — used by both DbBackend and VaultBackend before persisting.
//!
//! Each action is a pure function: takes inputs, returns transformed outputs.
//! No I/O, no DB, no file system. Side effects (persistence, network, file
//! writes) belong to the backend's command handler, not to actions.

use thiserror::Error;

use crate::defaults::apply_doc_type_defaults;
use crate::types::managed_meta::ManagedMeta;

/// Errors that can arise during pure-action execution.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ActionError {
    #[error("invalid doctype: {0}")]
    InvalidDoctype(String),
    #[error("invalid slug: {0}")]
    InvalidSlug(String),
    #[error("missing required field: {0}")]
    MissingRequiredField(String),
    #[error("invalid managed_meta: {0}")]
    InvalidManagedMeta(String),
}

/// Apply doctype-specific defaults to a `ManagedMeta` value, in place.
///
/// Wraps the existing `temper_core::defaults::apply_doc_type_defaults` for
/// ergonomic use from operations callers and to keep all action logic
/// importable from one path.
pub fn apply_defaults(doctype: &str, meta: &mut ManagedMeta) {
    // ManagedMeta serializes round-trip-lossless through serde_json::Value;
    // round-trip into Value, apply defaults to the Value's object, deserialize back.
    let mut value = serde_json::to_value(&*meta).unwrap_or(serde_json::Value::Null);
    apply_doc_type_defaults(doctype, &mut value);
    if let Ok(updated) = serde_json::from_value::<ManagedMeta>(value) {
        *meta = updated;
    }
}

/// Validate that a slug conforms to the temper slug rules.
///
/// Rules: non-empty, lowercase alphanumeric + hyphens, must start and end with
/// alphanumeric, no consecutive hyphens. Slugs are scoped to (owner, context,
/// doctype); this validates lexical shape only.
pub fn validate_slug(slug: &str) -> Result<(), ActionError> {
    if slug.is_empty() {
        return Err(ActionError::InvalidSlug(
            "slug must not be empty".to_string(),
        ));
    }
    let bytes = slug.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() {
        return Err(ActionError::InvalidSlug(format!(
            "slug must start with alphanumeric, got: {slug}"
        )));
    }
    if !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
        return Err(ActionError::InvalidSlug(format!(
            "slug must end with alphanumeric, got: {slug}"
        )));
    }
    let mut prev_was_hyphen = false;
    for &b in bytes {
        let is_lower_alnum = b.is_ascii_lowercase() || b.is_ascii_digit();
        let is_hyphen = b == b'-';
        if !is_lower_alnum && !is_hyphen {
            return Err(ActionError::InvalidSlug(format!(
                "slug must be lowercase alphanumeric with hyphens, got: {slug}"
            )));
        }
        if is_hyphen && prev_was_hyphen {
            return Err(ActionError::InvalidSlug(format!(
                "slug must not contain consecutive hyphens, got: {slug}"
            )));
        }
        prev_was_hyphen = is_hyphen;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_defaults_task_sets_stage_when_missing() {
        let mut meta = ManagedMeta::default();
        apply_defaults("task", &mut meta);
        assert_eq!(meta.stage.as_deref(), Some("backlog"));
    }

    #[test]
    fn apply_defaults_task_does_not_overwrite_existing_stage() {
        let mut meta = ManagedMeta {
            stage: Some("in-progress".to_string()),
            ..ManagedMeta::default()
        };
        apply_defaults("task", &mut meta);
        assert_eq!(meta.stage.as_deref(), Some("in-progress"));
    }

    #[test]
    fn apply_defaults_unknown_doctype_is_noop() {
        let mut meta = ManagedMeta::default();
        apply_defaults("nonexistent", &mut meta);
        // No fields populated for unknown doctypes
        assert!(meta.stage.is_none());
        assert!(meta.status.is_none());
    }

    #[test]
    fn validate_slug_accepts_valid_slugs() {
        assert!(validate_slug("hello-world").is_ok());
        assert!(validate_slug("task-2026-04-29").is_ok());
        assert!(validate_slug("a").is_ok());
        assert!(validate_slug("a1b2").is_ok());
    }

    #[test]
    fn validate_slug_rejects_empty() {
        let err = validate_slug("").unwrap_err();
        assert!(matches!(err, ActionError::InvalidSlug(_)));
    }

    #[test]
    fn validate_slug_rejects_uppercase() {
        let err = validate_slug("Hello").unwrap_err();
        assert!(matches!(err, ActionError::InvalidSlug(_)));
    }

    #[test]
    fn validate_slug_rejects_leading_hyphen() {
        let err = validate_slug("-hello").unwrap_err();
        assert!(matches!(err, ActionError::InvalidSlug(_)));
    }

    #[test]
    fn validate_slug_rejects_trailing_hyphen() {
        let err = validate_slug("hello-").unwrap_err();
        assert!(matches!(err, ActionError::InvalidSlug(_)));
    }

    #[test]
    fn validate_slug_rejects_consecutive_hyphens() {
        let err = validate_slug("hello--world").unwrap_err();
        assert!(matches!(err, ActionError::InvalidSlug(_)));
    }
}
