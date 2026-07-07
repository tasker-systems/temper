//! Pure shared actions — used by both DbBackend and CloudBackend before persisting.
//!
//! Each action is a pure function: takes inputs, returns transformed outputs.
//! No I/O, no DB, no file system. Side effects (persistence, network, file
//! writes) belong to the backend's command handler, not to actions.

use chrono::{DateTime, Utc};
use serde_json::Value;
use temper_core::types::ids::ResourceId;
use thiserror::Error;

use crate::defaults::apply_managed_defaults;
use crate::frontmatter::fields::{IDENTITY_FIELDS, TIER1_SYSTEM_FIELDS};
use crate::types::managed_meta::ManagedMeta;

use super::commands::{CreateResource, UpdateResource};

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
    #[error("invalid value: {0}")]
    InvalidValue(String),
}

/// Inject canonical identity keys (`temper-title`, `temper-slug`) into a
/// `managed_meta` JSONB value.
///
/// Called on both the send side (CLI / MCP build paths) before `compute_managed_hash`,
/// and on the receive side (server ingest / update services) before persisting and
/// hashing. Idempotent: running twice with the same inputs produces the same output.
///
/// `slug` is `Option` because `kb_resources.slug` is nullable — a resource born
/// without a slug should not have a `temper-slug` key in its managed_meta JSONB
/// (otherwise the column-NULL / JSONB-empty-string mismatch becomes a fresh drift).
/// When `slug` is `None`, any existing `temper-slug` key is removed.
///
/// If `meta` is not a JSON object, it is replaced with a fresh object containing
/// only the relevant identity keys. This handles the (unusual) case of a caller
/// passing `Value::Null` or a primitive; downstream validation will reject it on
/// shape grounds, but the helper does not silently drop the data.
pub fn ensure_managed_identity_keys(meta: &mut Value, title: &str, slug: Option<&str>) {
    if !meta.is_object() {
        *meta = Value::Object(serde_json::Map::new());
    }
    let obj = meta.as_object_mut().expect("just-coerced to object");
    obj.insert("temper-title".to_owned(), Value::String(title.to_owned()));
    match slug {
        Some(s) => {
            obj.insert("temper-slug".to_owned(), Value::String(s.to_owned()));
        }
        None => {
            obj.remove("temper-slug");
        }
    }
}

/// Apply managed-tier doctype-specific defaults to a `ManagedMeta` value,
/// in place.
///
/// Open-tier defaults (e.g. `date` for session/research) belong in `open_meta`
/// and are not handled here; callers route those through
/// [`crate::defaults::apply_open_defaults`] on the open-tier JSON.
pub fn apply_defaults(doctype: &str, meta: &mut ManagedMeta) {
    // ManagedMeta serializes round-trip-lossless through serde_json::Value;
    // round-trip into Value, apply defaults to the Value's object, deserialize back.
    let mut value = serde_json::to_value(&*meta).unwrap_or(serde_json::Value::Null);
    apply_managed_defaults(doctype, &mut value);
    if let Ok(updated) = serde_json::from_value::<ManagedMeta>(value) {
        *meta = updated;
    }
}

/// Apply managed-tier doctype defaults to a `serde_json::Value` in place.
///
/// Sibling to [`apply_defaults`] for callers that work with `Value` directly
/// (e.g. ingest_service's pre-validation pipeline). Both functions are thin
/// wrappers over the same underlying default-application — pick the variant
/// that matches your call site's natural type.
pub fn apply_defaults_value(doctype: &str, meta: &mut serde_json::Value) {
    crate::defaults::apply_managed_defaults(doctype, meta);
}

/// Tier-1 / tier-2 identity inputs for [`assemble_frontmatter_document`].
///
/// These values live as `kb_resources` columns (tier-1) or are derived from
/// the resolved request (tier-2), not in the managed_meta JSONB. They are
/// passed in from typed sources so the assembled document never relies on
/// placeholders.
///
/// `id` is always a real `ResourceId` UUID: create paths generate it up front
/// (before validation) rather than letting the database assign it, so there is
/// no "id not yet known" state to model.
#[derive(Debug)]
pub struct FrontmatterIdentity<'a> {
    /// Canonical resource id (`temper-id`).
    pub id: ResourceId,
    /// Creation timestamp (`temper-created`).
    pub created: DateTime<Utc>,
    /// Context / namespace (`temper-context`).
    pub context: &'a str,
    /// Document type (`temper-type`).
    pub doc_type: &'a str,
    /// Display title (`temper-title`).
    pub title: &'a str,
    /// Slug (`temper-slug`). `None` for slug-less resources — the key is then
    /// omitted entirely rather than written as an empty string.
    pub slug: Option<&'a str>,
}

/// Assemble the canonical complete frontmatter document for schema validation.
///
/// This is the single place that composes the "full temper frontmatter
/// document" from its parts: the managed-tier JSONB plus the tier-1/tier-2
/// identity and system keys that live as `kb_resources` columns. Both the
/// create path (`ingest_service`) and the update path
/// (`resource_service::update`) delegate here so identity injection is never
/// re-derived inline at a surface or service.
///
/// Steps:
/// 1. Coerce `managed_meta` to a JSON object (a non-object input yields a
///    fresh object — downstream schema validation rejects shape errors).
/// 2. Strip any identity / tier-1 system keys a caller may have smuggled into
///    the managed tier — those values are authoritative from `identity`.
/// 3. Apply doc-type managed-tier defaults so an absent optional field hashes
///    and validates identically to one where the default is explicit.
/// 4. Inject `temper-id` / `temper-created` / `temper-type` / `temper-context`
///    / `temper-title` / `temper-slug` from the typed `identity`.
///
/// The returned value is intended for [`crate::schema::validate_frontmatter`].
/// It is **not** the value to persist in the `managed_meta` JSONB column — the
/// tier-1 keys are owned by `kb_resources` columns; callers persist the
/// managed tier separately.
pub fn assemble_frontmatter_document(
    managed_meta: &Value,
    identity: &FrontmatterIdentity<'_>,
) -> Value {
    let mut doc = if managed_meta.is_object() {
        managed_meta.clone()
    } else {
        Value::Object(serde_json::Map::new())
    };

    // Strip authoritative system keys — only `identity` may set these.
    if let Some(obj) = doc.as_object_mut() {
        for field in IDENTITY_FIELDS.iter().chain(TIER1_SYSTEM_FIELDS.iter()) {
            obj.remove(*field);
        }
    }

    apply_managed_defaults(identity.doc_type, &mut doc);

    let obj = doc.as_object_mut().expect("coerced to object above");
    obj.insert(
        "temper-id".to_owned(),
        Value::String(identity.id.to_string()),
    );
    obj.insert(
        "temper-created".to_owned(),
        Value::String(identity.created.to_rfc3339()),
    );
    obj.insert(
        "temper-type".to_owned(),
        Value::String(identity.doc_type.to_owned()),
    );
    obj.insert(
        "temper-context".to_owned(),
        Value::String(identity.context.to_owned()),
    );
    obj.insert(
        "temper-title".to_owned(),
        Value::String(identity.title.to_owned()),
    );
    match identity.slug {
        Some(s) => {
            obj.insert("temper-slug".to_owned(), Value::String(s.to_owned()));
        }
        None => {
            obj.remove("temper-slug");
        }
    }

    doc
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

/// Validate a doctype's shape: non-empty.
///
/// Spec D3 ("closed-set-with-open-tail"): recognized labels (see
/// `crate::schema::DOC_TYPE_NAMES`) get frontmatter-schema enforcement via
/// `crate::schema::validate_frontmatter`; an unrecognized non-empty label is
/// still accepted here and stored verbatim as a free string — this gate only
/// rejects the shape-invalid case (empty/whitespace-only).
pub fn validate_doctype(doctype: &str) -> Result<(), ActionError> {
    if doctype.trim().is_empty() {
        Err(ActionError::InvalidDoctype(
            "doc_type must be non-empty".to_string(),
        ))
    } else {
        Ok(()) // recognized OR open-tail: the label is a free string at the kernel
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

/// Pre-flight validation for a `CreateResource` command.
///
/// Checks slug, doctype, context, and title shape, then applies per-doctype
/// pure invariants (mode/effort whitelists for `DocType::Task`) when the
/// doctype is recognized. Does not check uniqueness or authorization — those
/// are backend concerns.
pub fn validate_create(cmd: &CreateResource) -> Result<(), ActionError> {
    validate_slug(&cmd.slug)?;
    validate_doctype(&cmd.doctype)?;
    // home is a HomeAnchor (always set by construction); validation
    // of the ref string happens at the surface boundary before this.
    if cmd.title.is_empty() {
        return Err(ActionError::MissingRequiredField("title".to_string()));
    }

    // Open tail (spec D3 / Task A2): an unrecognized doctype has no
    // per-doctype invariants to enforce — `validate_doctype` above already
    // accepted it as a free string, so no additional rules apply here.
    if let Ok(doctype) = crate::frontmatter::DocType::from_str(&cmd.doctype) {
        match doctype {
            crate::frontmatter::DocType::Task => {
                if let Some(mode) = cmd.managed_meta.mode.as_deref() {
                    let valid = crate::schema::task_enum_values("temper-mode");
                    if !valid.iter().any(|v| v == mode) {
                        return Err(ActionError::InvalidValue(format!(
                            "mode '{mode}' not in {valid:?}"
                        )));
                    }
                }
                if let Some(effort) = cmd.managed_meta.effort.as_deref() {
                    let valid = crate::schema::task_enum_values("temper-effort");
                    if !valid.iter().any(|v| v == effort) {
                        return Err(ActionError::InvalidValue(format!(
                            "effort '{effort}' not in {valid:?}"
                        )));
                    }
                }
            }
            crate::frontmatter::DocType::Goal
            | crate::frontmatter::DocType::Session
            | crate::frontmatter::DocType::Research
            | crate::frontmatter::DocType::Concept
            | crate::frontmatter::DocType::Decision
            | crate::frontmatter::DocType::Fact
            | crate::frontmatter::DocType::Memory
            | crate::frontmatter::DocType::Question
            | crate::frontmatter::DocType::Theme
            | crate::frontmatter::DocType::Concern
            | crate::frontmatter::DocType::Principle
            | crate::frontmatter::DocType::Commitment
            | crate::frontmatter::DocType::Domain => {
                // No additional per-doctype pure invariants beyond the generic checks above.
            }
        }
    }

    Ok(())
}

/// Pre-flight validation for an `UpdateResource` command.
///
/// The resource is addressed by a `ResourceId`, which is always well-formed,
/// so there is nothing to validate pre-flight. Field-level validation of the
/// patch payload (managed_meta enums, etc.) is the backend's responsibility
/// after merging onto the resolved resource.
pub fn validate_update(_cmd: &UpdateResource) -> Result<(), ActionError> {
    Ok(())
}

/// Validate every top-level key in `open_meta` against the
/// `KNOWN_OPEN_FIELDS` registry. Accepts both canonical underscore form
/// (e.g. `relates_to`) and hyphen-form aliases (e.g. `relates-to`) via
/// `crate::frontmatter::registry::lookup`.
///
/// Returns the offending key on first miss so the caller can surface a
/// specific error. `Ok(())` if `open_meta` is not an object or is empty.
///
/// Server-side safety net for typo-d or unknown open-meta keys coming
/// from MCP / API clients that bypass the CLI's `Frontmatter::try_from`
/// alias normalization. The CLI's strict `Frontmatter` pipeline already
/// rejects unknown keys client-side, so well-formed CLI payloads pass
/// this check unchanged. Used by both DbBackend's update path and CloudBackend.
pub fn validate_open_meta_keys(open_meta: &serde_json::Value) -> Result<(), String> {
    let Some(obj) = open_meta.as_object() else {
        return Ok(());
    };
    for key in obj.keys() {
        if crate::frontmatter::registry::lookup(key.as_str()).is_none() {
            return Err(key.clone());
        }
    }
    Ok(())
}

/// Remove identity and tier-1 audit fields from input `managed_meta`.
///
/// Agents may echo these back from a `get_resource` call; they should not cause validation errors.
/// Uses `IDENTITY_FIELDS` and a subset of `TIER1_SYSTEM_FIELDS`. Intentionally does NOT strip
/// `temper-context` or `temper-type` — those remain so the update path can detect structural-move
/// attempts. (Moved here from `temper-api`'s `ingest_service` at the WS6 collapse — a pure helper over
/// temper-core's own field registry, used by the substrate create path.)
pub fn strip_system_managed_fields(mut meta: Value) -> Value {
    // temper-context and temper-type are kept for structural-move detection.
    const KEEP_FOR_MOVE_DETECTION: &[&str] = &["temper-context", "temper-type"];

    if let Some(obj) = meta.as_object_mut() {
        for field in IDENTITY_FIELDS
            .iter()
            .chain(TIER1_SYSTEM_FIELDS.iter())
            .filter(|f| !KEEP_FOR_MOVE_DETECTION.contains(f))
        {
            if obj.remove(*field).is_some() {
                tracing::warn!(
                    field = *field,
                    "stripped system field from input managed_meta"
                );
            }
        }
    }
    meta
}

/// Parameters for schema validation of `managed_meta` at a write boundary.
#[derive(Debug)]
pub struct ValidateManagedMetaParams<'a> {
    /// Canonical resource id — generated by the caller before validation.
    pub id: ResourceId,
    /// Creation timestamp.
    pub created: DateTime<Utc>,
    pub doc_type: &'a str,
    pub managed_meta: Option<&'a Value>,
    pub slug: &'a str,
    pub title: &'a str,
    pub context_name: &'a str,
}

/// Validate `managed_meta` against the doc-type schema, returning a typed [`temper_core::error::TemperError`] on failure
/// (always a `BadRequest` — these are caller-input faults, never system failures). Delegates document
/// assembly to [`assemble_frontmatter_document`] so identity injection is defined in exactly one place.
/// (Moved here from `temper-api`'s `ingest_service` at the WS6 collapse; the schema-validation
/// machinery already lives in temper-core.)
pub fn validate_managed_meta(
    params: &ValidateManagedMetaParams<'_>,
) -> Result<(), temper_core::error::TemperError> {
    use temper_core::error::TemperError;

    let managed: Value = params
        .managed_meta
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));
    if !managed.is_object() {
        return Err(TemperError::BadRequest(
            "invalid managed_meta shape: managed_meta must be a JSON object".to_owned(),
        ));
    }

    let identity = FrontmatterIdentity {
        id: params.id,
        created: params.created,
        context: params.context_name,
        doc_type: params.doc_type,
        title: params.title,
        slug: (!params.slug.is_empty()).then_some(params.slug),
    };
    let document = assemble_frontmatter_document(&managed, &identity);

    let yaml_value: serde_yaml::Value = serde_yaml::to_value(&document).map_err(|e| {
        TemperError::BadRequest(format!(
            "invalid managed_meta shape: JSON→YAML conversion: {e}"
        ))
    })?;

    let issues =
        crate::schema::validate_frontmatter(params.doc_type, &yaml_value).map_err(|e| {
            TemperError::BadRequest(format!("invalid managed_meta shape: schema load: {e}"))
        })?;

    if issues.is_empty() {
        Ok(())
    } else {
        let detail: Vec<String> = issues
            .iter()
            .map(|i| format!("{}: {}", i.path, i.message))
            .collect();
        Err(TemperError::BadRequest(format!(
            "managed_meta validation failed for doc_type={}: {}",
            params.doc_type,
            detail.join("; ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use temper_core::types::ids::ResourceId;
    use uuid::Uuid;

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
    fn validate_doctype_accepts_unknown_label_passthrough() {
        // memory is now recognized (A1); use a genuinely-unknown label for the tail.
        assert!(
            validate_doctype("anecdote").is_ok(),
            "unknown labels pass through (open tail)"
        );
        assert!(validate_doctype("").is_err(), "empty is still rejected");
    }

    #[test]
    fn validate_doctype_accepts_memory_as_cogmap_label() {
        // "memory" was previously reserved to avoid confusion with the
        // unrelated Claude-Code auto-memory feature (a file outside the
        // vault, not a resource doctype). Spec D3 (cognitive-map node
        // labels) now recognizes "memory" as a legitimate resource doctype
        // — the collision is name-only, and this test now asserts the
        // updated, intentional behavior.
        assert!(validate_doctype("memory").is_ok());
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

    #[test]
    fn validate_create_accepts_valid_command() {
        let cmd = CreateResource {
            slug: "valid-slug".to_string(),
            doctype: "task".to_string(),
            home: temper_core::types::home::HomeAnchor::Context(
                temper_core::types::ids::ContextId::new(),
            ),
            title: "Valid title".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            act: Default::default(),
            origin: super::super::Surface::CliCloud,
        };
        assert!(validate_create(&cmd).is_ok());
    }

    #[test]
    fn validate_create_rejects_invalid_slug() {
        let cmd = CreateResource {
            slug: "INVALID".to_string(),
            doctype: "task".to_string(),
            home: temper_core::types::home::HomeAnchor::Context(
                temper_core::types::ids::ContextId::new(),
            ),
            title: "X".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            act: Default::default(),
            origin: super::super::Surface::CliCloud,
        };
        assert!(matches!(
            validate_create(&cmd),
            Err(ActionError::InvalidSlug(_))
        ));
    }

    #[test]
    fn validate_create_accepts_unknown_doctype_passthrough() {
        // Open tail (Task A2): a genuinely-unknown doctype has no per-doctype
        // invariants to enforce and passes validate_create.
        let cmd = CreateResource {
            slug: "valid-slug".to_string(),
            doctype: "anecdote".to_string(),
            home: temper_core::types::home::HomeAnchor::Context(
                temper_core::types::ids::ContextId::new(),
            ),
            title: "X".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            act: Default::default(),
            origin: super::super::Surface::CliCloud,
        };
        assert!(
            validate_create(&cmd).is_ok(),
            "unknown doctype should pass through as an open-tail label"
        );
    }

    #[test]
    fn validate_create_rejects_empty_doctype() {
        let cmd = CreateResource {
            slug: "valid-slug".to_string(),
            doctype: String::new(),
            home: temper_core::types::home::HomeAnchor::Context(
                temper_core::types::ids::ContextId::new(),
            ),
            title: "X".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            act: Default::default(),
            origin: super::super::Surface::CliCloud,
        };
        assert!(matches!(
            validate_create(&cmd),
            Err(ActionError::InvalidDoctype(_))
        ));
    }

    #[test]
    fn validate_create_rejects_empty_title() {
        let cmd = CreateResource {
            slug: "valid".to_string(),
            doctype: "task".to_string(),
            home: temper_core::types::home::HomeAnchor::Context(
                temper_core::types::ids::ContextId::new(),
            ),
            title: "".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            act: Default::default(),
            origin: super::super::Surface::CliCloud,
        };
        assert!(matches!(
            validate_create(&cmd),
            Err(ActionError::MissingRequiredField(_))
        ));
    }

    #[test]
    fn validate_update_accepts_uuid_ref() {
        let cmd = UpdateResource {
            resource: ResourceId(Uuid::nil()),
            body: None,
            managed_meta: None,
            open_meta: None,
            move_to: None,
            context_ref: None,
            act: Default::default(),
            origin: super::super::Surface::CliCloud,
        };
        assert!(validate_update(&cmd).is_ok());
    }

    #[test]
    fn ensure_managed_identity_keys_inserts_when_absent() {
        let mut meta = serde_json::json!({"temper-stage": "backlog"});
        ensure_managed_identity_keys(&mut meta, "My Title", Some("my-slug"));
        assert_eq!(meta["temper-title"], "My Title");
        assert_eq!(meta["temper-slug"], "my-slug");
        assert_eq!(meta["temper-stage"], "backlog");
    }

    #[test]
    fn ensure_managed_identity_keys_overwrites_existing() {
        let mut meta = serde_json::json!({
            "temper-title": "Stale",
            "temper-slug": "stale-slug",
        });
        ensure_managed_identity_keys(&mut meta, "Fresh", Some("fresh-slug"));
        assert_eq!(meta["temper-title"], "Fresh");
        assert_eq!(meta["temper-slug"], "fresh-slug");
    }

    #[test]
    fn ensure_managed_identity_keys_is_idempotent() {
        let mut meta = serde_json::json!({});
        ensure_managed_identity_keys(&mut meta, "T", Some("s"));
        let after_first = meta.clone();
        ensure_managed_identity_keys(&mut meta, "T", Some("s"));
        assert_eq!(meta, after_first);
    }

    #[test]
    fn ensure_managed_identity_keys_replaces_non_object_with_object() {
        let mut meta = serde_json::Value::Null;
        ensure_managed_identity_keys(&mut meta, "T", Some("s"));
        assert!(meta.is_object());
        assert_eq!(meta["temper-title"], "T");
        assert_eq!(meta["temper-slug"], "s");
    }

    #[test]
    fn ensure_managed_identity_keys_omits_slug_when_none() {
        let mut meta = serde_json::json!({"temper-stage": "backlog"});
        ensure_managed_identity_keys(&mut meta, "T", None);
        assert_eq!(meta["temper-title"], "T");
        assert!(
            meta.get("temper-slug").is_none(),
            "temper-slug must be absent when slug is None; got: {meta}"
        );
        assert_eq!(meta["temper-stage"], "backlog");
    }

    #[test]
    fn ensure_managed_identity_keys_removes_existing_slug_when_none() {
        let mut meta = serde_json::json!({
            "temper-title": "T",
            "temper-slug": "stale-slug",
        });
        ensure_managed_identity_keys(&mut meta, "T", None);
        assert!(
            meta.get("temper-slug").is_none(),
            "existing temper-slug must be removed when slug is None; got: {meta}"
        );
    }

    #[test]
    fn apply_defaults_value_task_sets_stage_when_missing() {
        let mut meta = serde_json::json!({});
        apply_defaults_value("task", &mut meta);
        assert_eq!(meta["temper-stage"], "backlog");
    }

    #[test]
    fn apply_defaults_value_task_does_not_overwrite_existing_stage() {
        let mut meta = serde_json::json!({"temper-stage": "in-progress"});
        apply_defaults_value("task", &mut meta);
        assert_eq!(meta["temper-stage"], "in-progress");
    }

    #[test]
    fn apply_defaults_value_unknown_doctype_is_noop() {
        let mut meta = serde_json::json!({});
        apply_defaults_value("nonexistent", &mut meta);
        assert!(meta.as_object().unwrap().is_empty());
    }

    fn sample_identity<'a>(doc_type: &'a str, slug: Option<&'a str>) -> FrontmatterIdentity<'a> {
        FrontmatterIdentity {
            id: ResourceId::from(Uuid::parse_str("019d8110-8ff3-70c2-85ae-57e04ed62885").unwrap()),
            created: DateTime::parse_from_rfc3339("2026-05-21T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            context: "temper",
            doc_type,
            title: "My Title",
            slug,
        }
    }

    #[test]
    fn assemble_frontmatter_document_injects_all_identity_keys() {
        let managed = json!({"temper-stage": "in-progress"});
        let doc =
            assemble_frontmatter_document(&managed, &sample_identity("task", Some("my-task")));
        assert_eq!(doc["temper-id"], "019d8110-8ff3-70c2-85ae-57e04ed62885");
        assert_eq!(doc["temper-created"], "2026-05-21T12:00:00+00:00");
        assert_eq!(doc["temper-type"], "task");
        assert_eq!(doc["temper-context"], "temper");
        assert_eq!(doc["temper-title"], "My Title");
        assert_eq!(doc["temper-slug"], "my-task");
        // Caller-supplied managed-tier field is preserved.
        assert_eq!(doc["temper-stage"], "in-progress");
    }

    #[test]
    fn assemble_frontmatter_document_strips_smuggled_system_keys() {
        // A caller must not be able to override authoritative identity values
        // by stuffing them into the managed tier.
        let managed = json!({
            "temper-id": "00000000-0000-0000-0000-000000000000",
            "temper-created": "1999-01-01T00:00:00Z",
            "temper-type": "goal",
            "temper-context": "wrong-ctx",
            "temper-stage": "backlog",
        });
        let doc = assemble_frontmatter_document(&managed, &sample_identity("task", Some("s")));
        assert_eq!(doc["temper-id"], "019d8110-8ff3-70c2-85ae-57e04ed62885");
        assert_eq!(doc["temper-created"], "2026-05-21T12:00:00+00:00");
        assert_eq!(doc["temper-type"], "task");
        assert_eq!(doc["temper-context"], "temper");
        assert_eq!(doc["temper-stage"], "backlog");
    }

    #[test]
    fn assemble_frontmatter_document_omits_slug_when_none() {
        let managed = json!({"temper-slug": "stale"});
        let doc = assemble_frontmatter_document(&managed, &sample_identity("task", None));
        assert!(
            doc.get("temper-slug").is_none(),
            "temper-slug must be absent when slug is None; got: {doc}"
        );
    }

    #[test]
    fn assemble_frontmatter_document_applies_doc_type_defaults() {
        let doc = assemble_frontmatter_document(&json!({}), &sample_identity("task", Some("s")));
        assert_eq!(
            doc["temper-stage"], "backlog",
            "task doc-type default temper-stage should be filled in"
        );
    }

    #[test]
    fn assemble_frontmatter_document_coerces_non_object_input() {
        let doc = assemble_frontmatter_document(
            &serde_json::Value::Null,
            &sample_identity("session", Some("s")),
        );
        assert!(doc.is_object());
        assert_eq!(doc["temper-title"], "My Title");
    }

    #[test]
    fn validate_open_meta_accepts_canonical_keys() {
        let v = json!({
            "relates_to": ["foo"],
            "depends_on": ["bar"],
            "tags": ["auth"],
            "parent": "parent-slug",
        });
        assert!(validate_open_meta_keys(&v).is_ok());
    }

    #[test]
    fn validate_open_meta_accepts_hyphen_aliases() {
        let v = json!({
            "relates-to": ["foo"],
            "depends-on": ["bar"],
            "preceded-by": ["baz"],
            "derived-from": ["qux"],
        });
        assert!(validate_open_meta_keys(&v).is_ok());
    }

    #[test]
    fn validate_open_meta_accepts_mixed_canonical_and_alias() {
        let v = json!({
            "relates_to": ["foo"],
            "depends-on": ["bar"],
        });
        assert!(validate_open_meta_keys(&v).is_ok());
    }

    #[test]
    fn validate_open_meta_rejects_unknown_key() {
        let v = json!({
            "relates_to": ["foo"],
            "totally_made_up": "nope",
        });
        let err = validate_open_meta_keys(&v).unwrap_err();
        assert_eq!(err, "totally_made_up");
    }

    #[test]
    fn validate_open_meta_rejects_typo_of_known_key() {
        let v = json!({
            "relats_to": ["foo"],
        });
        let err = validate_open_meta_keys(&v).unwrap_err();
        assert_eq!(err, "relats_to");
    }

    #[test]
    fn validate_open_meta_empty_object_ok() {
        let v = json!({});
        assert!(validate_open_meta_keys(&v).is_ok());
    }

    #[test]
    fn validate_open_meta_non_object_ok() {
        // Non-object values are passed through — the caller's typed
        // MetaUpdatePayload wraps this in a Value that may be null or
        // some other shape during deserialization. Validation only
        // applies to well-formed object payloads.
        assert!(validate_open_meta_keys(&json!(null)).is_ok());
        assert!(validate_open_meta_keys(&json!([])).is_ok());
        assert!(validate_open_meta_keys(&json!("string")).is_ok());
    }

    #[test]
    fn validate_open_meta_reports_first_bad_key() {
        // BTreeMap key ordering in serde_json::Value::Object is insertion
        // order on recent versions, so this test documents the "first miss
        // wins" contract rather than asserting a specific order.
        let v = json!({
            "relates_to": ["a"],
            "bogus_one": "x",
            "bogus_two": "y",
        });
        let err = validate_open_meta_keys(&v).unwrap_err();
        assert!(
            err == "bogus_one" || err == "bogus_two",
            "expected first-bad-key to be one of the two unknowns, got: {err}"
        );
    }

    #[test]
    fn validate_create_rejects_task_with_unknown_mode() {
        let cmd = CreateResource {
            slug: "2026-05-14-test-task".to_string(),
            doctype: "task".to_string(),
            home: temper_core::types::home::HomeAnchor::Context(
                temper_core::types::ids::ContextId::new(),
            ),
            title: "Test task".to_string(),
            body: None,
            managed_meta: ManagedMeta {
                mode: Some("nonsense".to_string()),
                effort: Some("small".to_string()),
                goal: Some("temper-maintenance".to_string()),
                ..ManagedMeta::default()
            },
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            act: Default::default(),
            origin: super::super::Surface::CliCloud,
        };

        let err = validate_create(&cmd).unwrap_err();
        assert!(
            format!("{err:?}").contains("mode") || format!("{err:?}").contains("nonsense"),
            "expected error mentioning mode/nonsense, got: {err:?}"
        );
    }

    #[test]
    fn validate_create_rejects_task_with_unknown_effort() {
        let cmd = CreateResource {
            slug: "2026-05-14-test-task".to_string(),
            doctype: "task".to_string(),
            home: temper_core::types::home::HomeAnchor::Context(
                temper_core::types::ids::ContextId::new(),
            ),
            title: "Test task".to_string(),
            body: None,
            managed_meta: ManagedMeta {
                mode: Some("plan".to_string()),
                effort: Some("gigantic".to_string()),
                goal: Some("temper-maintenance".to_string()),
                ..ManagedMeta::default()
            },
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            act: Default::default(),
            origin: super::super::Surface::CliCloud,
        };

        let err = validate_create(&cmd).unwrap_err();
        assert!(
            format!("{err:?}").contains("effort") || format!("{err:?}").contains("gigantic"),
            "expected error mentioning effort/gigantic, got: {err:?}"
        );
    }

    #[test]
    fn validate_create_accepts_valid_task() {
        let cmd = CreateResource {
            slug: "2026-05-14-test-task".to_string(),
            doctype: "task".to_string(),
            home: temper_core::types::home::HomeAnchor::Context(
                temper_core::types::ids::ContextId::new(),
            ),
            title: "Test task".to_string(),
            body: None,
            managed_meta: ManagedMeta {
                mode: Some("plan".to_string()),
                effort: Some("medium".to_string()),
                goal: Some("temper-maintenance".to_string()),
                ..ManagedMeta::default()
            },
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            act: Default::default(),
            origin: super::super::Surface::CliCloud,
        };

        validate_create(&cmd).expect("valid task should pass validation");
    }

    #[test]
    fn validate_create_accepts_research_with_arbitrary_managed_meta() {
        let cmd = CreateResource {
            slug: "2026-05-14-test-research".to_string(),
            doctype: "research".to_string(),
            home: temper_core::types::home::HomeAnchor::Context(
                temper_core::types::ids::ContextId::new(),
            ),
            title: "Test research".to_string(),
            body: None,
            managed_meta: ManagedMeta {
                mode: Some("anything".to_string()),
                ..ManagedMeta::default()
            },
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            act: Default::default(),
            origin: super::super::Surface::CliCloud,
        };

        validate_create(&cmd).expect("non-task doctype should not be subject to task whitelist");
    }
}
