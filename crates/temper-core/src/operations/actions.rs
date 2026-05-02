//! Pure shared actions — used by both DbBackend and VaultBackend before persisting.
//!
//! Each action is a pure function: takes inputs, returns transformed outputs.
//! No I/O, no DB, no file system. Side effects (persistence, network, file
//! writes) belong to the backend's command handler, not to actions.

use serde_json::Value;
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

/// Validate that a doctype is recognized.
///
/// Delegates to the canonical `crate::schema::DOC_TYPE_NAMES` slice — the
/// same set of doctypes that have embedded JSON schemas. Updates to the
/// recognized doctype set go in `schema.rs`, not here.
pub fn validate_doctype(doctype: &str) -> Result<(), ActionError> {
    if crate::schema::DOC_TYPE_NAMES.contains(&doctype) {
        Ok(())
    } else {
        Err(ActionError::InvalidDoctype(format!(
            "unknown doctype '{doctype}', expected one of: {}",
            crate::schema::DOC_TYPE_NAMES.join(", ")
        )))
    }
}

/// Partial-merge a `ManagedMeta` patch onto an existing `ManagedMeta`.
///
/// For each `Some(value)` in `patch`, overwrite the corresponding field in
/// `existing`. Fields that are `None` in the patch are left unchanged on
/// `existing`. The `extra` HashMap is merged key-by-key (patch keys
/// overwrite, keys absent from patch are preserved).
pub fn merge_managed_meta(existing: &mut ManagedMeta, patch: ManagedMeta) {
    if patch.doc_type.is_some() {
        existing.doc_type = patch.doc_type;
    }
    if patch.context.is_some() {
        existing.context = patch.context;
    }
    if patch.updated.is_some() {
        existing.updated = patch.updated;
    }
    if patch.source.is_some() {
        existing.source = patch.source;
    }
    if patch.stage.is_some() {
        existing.stage = patch.stage;
    }
    if patch.mode.is_some() {
        existing.mode = patch.mode;
    }
    if patch.effort.is_some() {
        existing.effort = patch.effort;
    }
    if patch.goal.is_some() {
        existing.goal = patch.goal;
    }
    if patch.seq.is_some() {
        existing.seq = patch.seq;
    }
    if patch.branch.is_some() {
        existing.branch = patch.branch;
    }
    if patch.pr.is_some() {
        existing.pr = patch.pr;
    }
    if patch.status.is_some() {
        existing.status = patch.status;
    }
    if patch.provenance.is_some() {
        existing.provenance = patch.provenance;
    }
    if patch.llm_model.is_some() {
        existing.llm_model = patch.llm_model;
    }
    if patch.llm_run.is_some() {
        existing.llm_run = patch.llm_run;
    }
    if patch.title.is_some() {
        existing.title = patch.title;
    }
    if patch.slug.is_some() {
        existing.slug = patch.slug;
    }

    // Merge extra HashMap key-by-key.
    for (k, v) in patch.extra {
        existing.extra.insert(k, v);
    }
}

/// Partial-merge an open_meta patch onto an existing open_meta value.
///
/// open_meta is free-form JSON (an object). Patch semantics:
/// - For each key in `patch`, overwrite the corresponding key in `existing`.
/// - Keys in `existing` not present in `patch` are preserved.
/// - Non-object inputs (e.g., a top-level array or scalar) overwrite outright.
///
/// This is shallow merge — nested objects are not deep-merged. Callers that
/// need deep merge should compose this action with their own logic.
pub fn merge_open_meta(existing: &mut Value, patch: Value) {
    match (existing.as_object_mut(), patch) {
        (Some(existing_map), Value::Object(patch_map)) => {
            for (k, v) in patch_map {
                existing_map.insert(k, v);
            }
        }
        (_, patch) => {
            // Either existing isn't an object, or patch isn't — overwrite outright.
            *existing = patch;
        }
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

    #[test]
    fn validate_doctype_accepts_known() {
        assert!(validate_doctype("task").is_ok());
        assert!(validate_doctype("goal").is_ok());
        assert!(validate_doctype("session").is_ok());
        assert!(validate_doctype("research").is_ok());
        assert!(validate_doctype("concept").is_ok());
        assert!(validate_doctype("decision").is_ok());
    }

    #[test]
    fn validate_doctype_rejects_unknown() {
        let err = validate_doctype("widget").unwrap_err();
        assert!(matches!(err, ActionError::InvalidDoctype(_)));
    }

    #[test]
    fn validate_doctype_rejects_memory_not_a_real_doctype() {
        // "memory" is a temper memory-system concept (auto-memory), not a
        // resource doctype. Guard against accidental re-introduction.
        let err = validate_doctype("memory").unwrap_err();
        assert!(matches!(err, ActionError::InvalidDoctype(_)));
    }

    #[test]
    fn merge_managed_meta_overrides_present_fields() {
        let mut existing = ManagedMeta {
            stage: Some("backlog".to_string()),
            ..ManagedMeta::default()
        };
        let patch = ManagedMeta {
            stage: Some("done".to_string()),
            ..ManagedMeta::default()
        };
        merge_managed_meta(&mut existing, patch);
        assert_eq!(existing.stage.as_deref(), Some("done"));
    }

    #[test]
    fn merge_managed_meta_preserves_absent_fields() {
        let mut existing = ManagedMeta {
            stage: Some("backlog".to_string()),
            doc_type: Some("task".to_string()),
            ..ManagedMeta::default()
        };
        let patch = ManagedMeta {
            stage: Some("done".to_string()),
            ..ManagedMeta::default()
        };
        merge_managed_meta(&mut existing, patch);
        assert_eq!(existing.stage.as_deref(), Some("done"));
        assert_eq!(existing.doc_type.as_deref(), Some("task"));
    }

    #[test]
    fn merge_managed_meta_merges_extra_map() {
        use serde_json::json;
        let mut existing = ManagedMeta::default();
        existing.extra.insert("k1".to_string(), json!("v1"));
        existing.extra.insert("k2".to_string(), json!("v2"));

        let mut patch = ManagedMeta::default();
        patch.extra.insert("k2".to_string(), json!("patched"));
        patch.extra.insert("k3".to_string(), json!("v3"));

        merge_managed_meta(&mut existing, patch);
        assert_eq!(existing.extra.get("k1"), Some(&json!("v1")));
        assert_eq!(existing.extra.get("k2"), Some(&json!("patched")));
        assert_eq!(existing.extra.get("k3"), Some(&json!("v3")));
    }

    #[test]
    fn merge_managed_meta_covers_all_typed_fields() {
        let mut existing = ManagedMeta::default();
        let patch = ManagedMeta {
            doc_type: Some("task".to_string()),
            context: Some("temper".to_string()),
            updated: Some("2026-05-02".to_string()),
            source: Some("user".to_string()),
            stage: Some("done".to_string()),
            mode: Some("build".to_string()),
            effort: Some("medium".to_string()),
            goal: Some("g1".to_string()),
            seq: Some(7),
            branch: Some("jct/x".to_string()),
            pr: Some("123".to_string()),
            status: Some("active".to_string()),
            provenance: Some("user-created".to_string()),
            llm_model: Some("opus".to_string()),
            llm_run: Some("run-1".to_string()),
            title: Some("T".to_string()),
            slug: Some("s".to_string()),
            extra: Default::default(),
        };
        merge_managed_meta(&mut existing, patch);
        assert_eq!(existing.doc_type.as_deref(), Some("task"));
        assert_eq!(existing.context.as_deref(), Some("temper"));
        assert_eq!(existing.updated.as_deref(), Some("2026-05-02"));
        assert_eq!(existing.source.as_deref(), Some("user"));
        assert_eq!(existing.stage.as_deref(), Some("done"));
        assert_eq!(existing.mode.as_deref(), Some("build"));
        assert_eq!(existing.effort.as_deref(), Some("medium"));
        assert_eq!(existing.goal.as_deref(), Some("g1"));
        assert_eq!(existing.seq, Some(7));
        assert_eq!(existing.branch.as_deref(), Some("jct/x"));
        assert_eq!(existing.pr.as_deref(), Some("123"));
        assert_eq!(existing.status.as_deref(), Some("active"));
        assert_eq!(existing.provenance.as_deref(), Some("user-created"));
        assert_eq!(existing.llm_model.as_deref(), Some("opus"));
        assert_eq!(existing.llm_run.as_deref(), Some("run-1"));
        assert_eq!(existing.title.as_deref(), Some("T"));
        assert_eq!(existing.slug.as_deref(), Some("s"));
    }

    #[test]
    fn merge_open_meta_shallow_merges_objects() {
        use serde_json::json;
        let mut existing = json!({"a": 1, "b": 2});
        merge_open_meta(&mut existing, json!({"b": 99, "c": 3}));
        assert_eq!(existing, json!({"a": 1, "b": 99, "c": 3}));
    }

    #[test]
    fn merge_open_meta_overwrites_when_patch_is_not_object() {
        use serde_json::json;
        let mut existing = json!({"a": 1});
        merge_open_meta(&mut existing, json!([1, 2, 3]));
        assert_eq!(existing, json!([1, 2, 3]));
    }

    #[test]
    fn merge_open_meta_overwrites_when_existing_is_not_object() {
        use serde_json::json;
        let mut existing = json!("scalar");
        merge_open_meta(&mut existing, json!({"a": 1}));
        assert_eq!(existing, json!({"a": 1}));
    }
}
