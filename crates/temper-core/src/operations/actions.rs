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
}
