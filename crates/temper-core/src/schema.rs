//! Schema validation for temper vault frontmatter.
//!
//! Loads JSON Schema files embedded at compile time, validates YAML frontmatter
//! against them, detects legacy field names, and finds unknown temper-* fields.

use crate::error::{Result, TemperError};
use crate::frontmatter::fields::{KNOWN_TEMPER_FIELDS, LEGACY_FIELDS, SYSTEM_MANAGED_FIELDS};
use jsonschema::{Resource, Validator};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

// ---------------------------------------------------------------------------
// Embedded schemas
// ---------------------------------------------------------------------------

/// The base schema is referenced by every doc-type schema via
/// `{ "$ref": "base.schema.json" }`. The doc-type schema text itself is
/// owned by `DocType::schema_json()`.
const BASE_SCHEMA: &str = include_str!("../schemas/base.schema.json");

/// URI used as the `$id` in base.schema.json — must match what the doctype
/// schemas reference via `{ "$ref": "base.schema.json" }`.
const BASE_SCHEMA_URI: &str = "https://temperkb.io/schemas/base.schema.json";

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single validation finding about a frontmatter field.
#[derive(Debug, Clone, Serialize)]
pub struct ValidationIssue {
    /// JSON Pointer / field path, e.g. `"temper-stage"` or `"/properties/title"`.
    pub path: String,
    /// Human-readable description of the problem.
    pub message: String,
    /// Whether the issue can be automatically repaired without user input.
    pub auto_fixable: bool,
}

/// Aggregated validation result for a single file.
#[derive(Debug, Clone, Serialize)]
pub struct ValidationResult {
    /// Vault-relative or absolute path of the file.
    pub file_path: String,
    /// All issues found in this file.
    pub issues: Vec<ValidationIssue>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load and compile a JSON Schema validator for a named doctype.
///
/// The base schema is registered as an external resource so that `$ref`
/// references in doctype schemas resolve correctly.
///
/// # Errors
/// Returns [`TemperError::Config`] if `doc_type` is not one of the six known
/// values, or if schema compilation fails.
pub fn load_schema(doc_type: &str) -> Result<Validator> {
    let doc_schema_str = crate::frontmatter::DocType::from_str(doc_type)?.schema_json();

    let base_json: serde_json::Value = serde_json::from_str(BASE_SCHEMA)
        .map_err(|e| TemperError::Config(format!("base schema JSON parse error: {e}")))?;

    let doc_json: serde_json::Value = serde_json::from_str(doc_schema_str)
        .map_err(|e| TemperError::Config(format!("{doc_type} schema JSON parse error: {e}")))?;

    let base_resource = Resource::from_contents(base_json);

    let validator = jsonschema::options()
        .with_resource(BASE_SCHEMA_URI, base_resource)
        .build(&doc_json)
        .map_err(|e| {
            TemperError::Config(format!("schema compilation error for {doc_type}: {e}"))
        })?;

    Ok(validator)
}

/// Validate YAML frontmatter against the JSON Schema for the given doctype.
///
/// Returns a list of [`ValidationIssue`]s. An empty list means the frontmatter
/// is valid. Converts the YAML value to JSON before validation so that the
/// `jsonschema` crate can process it.
///
/// # Errors
/// Returns an error if the schema cannot be loaded or if YAML→JSON conversion
/// fails.
pub fn validate_frontmatter(
    doc_type: &str,
    frontmatter: &serde_yaml::Value,
) -> Result<Vec<ValidationIssue>> {
    let validator = load_schema(doc_type)?;

    // Convert YAML → JSON string → serde_json::Value
    let json_str = serde_json::to_string(
        &serde_json::to_value(frontmatter)
            .map_err(|e| TemperError::Config(format!("YAML→JSON conversion error: {e}")))?,
    )
    .map_err(|e| TemperError::Config(format!("JSON serialization error: {e}")))?;

    let json_value: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| TemperError::Config(format!("JSON parse error: {e}")))?;

    let issues = validator
        .iter_errors(&json_value)
        .map(|err| ValidationIssue {
            path: err.instance_path().to_string(),
            message: err.to_string(),
            auto_fixable: false,
        })
        .collect();

    Ok(issues)
}

/// CLI-facing validator that accepts `temper-provisional-id` as a stand-in
/// for `temper-id`.
///
/// Provisional files exist only on the client before their first sync. The
/// server never sees `temper-provisional-id` and `base.schema.json` has no
/// knowledge of it — it remains server-authoritative. This wrapper clones
/// the frontmatter, substitutes `temper-id` with the provisional UUID when
/// needed, removes `temper-provisional-id` from the clone, and delegates to
/// [`validate_frontmatter`] with the clone. The original frontmatter is not
/// mutated.
///
/// Use this from CLI code paths (`temper doctor`, `temper sync` normalize).
/// Server code paths should continue to call [`validate_frontmatter`] directly.
///
/// # Errors
/// Propagates any error from [`validate_frontmatter`].
pub fn validate_allowing_provisional(
    doc_type: &str,
    frontmatter: &serde_yaml::Value,
) -> Result<Vec<ValidationIssue>> {
    let mut clone = frontmatter.clone();

    if let Some(mapping) = clone.as_mapping_mut() {
        let provisional_key = serde_yaml::Value::String("temper-provisional-id".to_string());
        let id_key = serde_yaml::Value::String("temper-id".to_string());

        let provisional_value = mapping.get(&provisional_key).cloned();

        if let Some(provisional) = provisional_value {
            if !mapping.contains_key(&id_key) {
                mapping.insert(id_key, provisional);
            }
        }
        mapping.remove(&provisional_key);
    }

    validate_frontmatter(doc_type, &clone)
}

/// Check for legacy field names that have been superseded by temper-* names.
///
/// All returned issues have `auto_fixable: true` because the fix is
/// deterministic (rename the field).
pub fn check_legacy_fields(frontmatter: &serde_yaml::Value) -> Vec<ValidationIssue> {
    let Some(mapping) = frontmatter.as_mapping() else {
        return vec![];
    };

    let mut issues = Vec::new();
    for (legacy, replacement) in LEGACY_FIELDS {
        if mapping.contains_key(*legacy) {
            issues.push(ValidationIssue {
                path: (*legacy).to_string(),
                message: format!("legacy field '{legacy}' should be renamed to '{replacement}'"),
                auto_fixable: true,
            });
        }
    }
    issues
}

/// Check for temper-* fields that are not in the known set (likely typos).
///
/// Issues are returned as non-auto-fixable because the correct field name
/// cannot be inferred with certainty.
pub fn check_unknown_temper_fields(frontmatter: &serde_yaml::Value) -> Vec<ValidationIssue> {
    let Some(mapping) = frontmatter.as_mapping() else {
        return vec![];
    };

    let known: HashSet<&str> = KNOWN_TEMPER_FIELDS.iter().copied().collect();
    let mut issues = Vec::new();

    for (key, _value) in mapping {
        let Some(key_str) = key.as_str() else {
            continue;
        };
        if key_str.starts_with("temper-") && !known.contains(key_str) {
            issues.push(ValidationIssue {
                path: key_str.to_string(),
                message: format!(
                    "unknown temper-* field '{key_str}' — possible typo or unsupported field"
                ),
                auto_fixable: false,
            });
        }
    }
    issues
}

// ---------------------------------------------------------------------------
// Updatable field helpers
// ---------------------------------------------------------------------------

/// Get the updatable field names for a doctype by reading the schema properties
/// and excluding system-managed fields.
pub fn updatable_fields(doc_type: &str) -> Result<Vec<(String, serde_json::Value)>> {
    let schema_str = crate::frontmatter::DocType::from_str(doc_type)?.schema_json();

    let schema: serde_json::Value = serde_json::from_str(schema_str)
        .map_err(|e| TemperError::Config(format!("schema parse error: {e}")))?;

    let mut fields = Vec::new();

    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        for (key, value) in props {
            if !SYSTEM_MANAGED_FIELDS.contains(&key.as_str()) {
                fields.push((key.clone(), value.clone()));
            }
        }
    }

    Ok(fields)
}

/// Field names to display in table output for a doctype, in order.
///
/// Unlike `updatable_fields`, this is a curated set used only for
/// human-readable table rendering in the CLI. JSON output is unaffected and
/// still contains the full frontmatter.
///
/// Universal columns come first (context, type, slug, updated), followed by
/// per-type extras. This is the single point of curation — adding a new
/// doctype means extending the match here.
pub fn display_fields(doc_type: &str) -> Result<Vec<String>> {
    const UNIVERSAL: &[&str] = &[
        "temper-context",
        "temper-type",
        "temper-slug",
        "temper-updated",
    ];

    let extras: &[&str] = match doc_type {
        "task" => &[
            "temper-stage",
            "temper-mode",
            "temper-effort",
            "temper-goal",
        ],
        "goal" => &["temper-status", "temper-seq"],
        "session" | "research" | "concept" | "decision" => &[],
        other => {
            return Err(TemperError::Config(format!(
                "unknown doctype '{other}' for display_fields"
            )));
        }
    };

    Ok(UNIVERSAL
        .iter()
        .chain(extras.iter())
        .map(|s| s.to_string())
        .collect())
}

/// Validate a field value against a schema property definition.
pub fn validate_field_value(
    field_name: &str,
    value: &str,
    schema_prop: &serde_json::Value,
) -> Option<String> {
    // Check enum constraint
    if let Some(enum_values) = schema_prop.get("enum") {
        if let Some(arr) = enum_values.as_array() {
            let valid: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if !valid.contains(&value.to_string()) {
                return Some(format!(
                    "invalid value '{}' for --{}; expected one of: {}",
                    value,
                    field_name.strip_prefix("temper-").unwrap_or(field_name),
                    valid.join(", ")
                ));
            }
        }
    }
    // Check type constraint
    if let Some(type_val) = schema_prop.get("type") {
        if type_val == "integer" && value.parse::<i64>().is_err() {
            return Some(format!(
                "invalid value '{}' for --{}; expected integer",
                value,
                field_name.strip_prefix("temper-").unwrap_or(field_name),
            ));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Schema introspection helpers
// ---------------------------------------------------------------------------

/// Known doc type names (the set of schemas we embed).
pub static DOC_TYPE_NAMES: &[&str] =
    &["concept", "decision", "goal", "research", "session", "task"];

/// Return the raw JSON schema for a named doc type.
///
/// Unlike `load_schema` (which compiles a validator), this returns the
/// `serde_json::Value` for introspection — extracting required fields,
/// enum values, property descriptions, etc.
pub fn schema_value(doc_type: &str) -> Result<serde_json::Value> {
    let raw = crate::frontmatter::DocType::from_str(doc_type)?.schema_json();

    serde_json::from_str(raw)
        .map_err(|e| TemperError::Config(format!("{doc_type} schema JSON parse error: {e}")))
}

/// Return the `required` array from a doc type schema as `Vec<String>`.
///
/// This only returns the doc-type-level required fields (not the base
/// schema's required fields, which are merged via `allOf` at validation
/// time).
pub fn required_fields(doc_type: &str) -> Result<Vec<String>> {
    let schema = schema_value(doc_type)?;
    Ok(extract_required_fields(&schema))
}

/// Extract enum values from schema properties as a `BTreeMap<field_name, Vec<values>>`.
///
/// Only includes properties that have an `enum` constraint with string values.
pub fn enum_fields(doc_type: &str) -> Result<BTreeMap<String, Vec<String>>> {
    let schema = schema_value(doc_type)?;
    Ok(extract_enum_fields(&schema))
}

fn extract_required_fields(schema: &serde_json::Value) -> Vec<String> {
    schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn extract_enum_fields(schema: &serde_json::Value) -> BTreeMap<String, Vec<String>> {
    let mut result = BTreeMap::new();
    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        for (key, prop) in props {
            if let Some(enum_values) = prop.get("enum").and_then(|e| e.as_array()) {
                let values: Vec<String> = enum_values
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                if !values.is_empty() {
                    result.insert(key.clone(), values);
                }
            }
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // validate_field_value tests
    // -------------------------------------------------------------------------

    #[test]
    fn validate_field_value_valid_enum() {
        let schema_prop = serde_json::json!({
            "type": "string",
            "enum": ["backlog", "in-progress", "done", "cancelled"]
        });
        assert_eq!(
            validate_field_value("temper-stage", "in-progress", &schema_prop),
            None,
            "valid enum value should return None"
        );
    }

    #[test]
    fn validate_field_value_invalid_enum() {
        let schema_prop = serde_json::json!({
            "type": "string",
            "enum": ["backlog", "in-progress", "done", "cancelled"]
        });
        let result = validate_field_value("temper-stage", "invalid", &schema_prop);
        assert!(
            result.is_some(),
            "invalid enum value should return Some(error)"
        );
        let msg = result.unwrap();
        assert!(
            msg.contains("invalid"),
            "error message should include the bad value: {msg}"
        );
        assert!(
            msg.contains("stage"),
            "error message should include the field name (without temper- prefix): {msg}"
        );
    }

    #[test]
    fn validate_field_value_valid_integer() {
        let schema_prop = serde_json::json!({ "type": "integer", "minimum": 0 });
        assert_eq!(
            validate_field_value("temper-seq", "42", &schema_prop),
            None,
            "valid integer string should return None"
        );
    }

    #[test]
    fn validate_field_value_invalid_integer() {
        let schema_prop = serde_json::json!({ "type": "integer", "minimum": 0 });
        let result = validate_field_value("temper-seq", "abc", &schema_prop);
        assert!(
            result.is_some(),
            "non-integer string should return Some(error)"
        );
        let msg = result.unwrap();
        assert!(
            msg.contains("integer"),
            "error should mention 'integer': {msg}"
        );
    }

    #[test]
    fn validate_field_value_no_constraints_accepts_any() {
        let schema_prop = serde_json::json!({ "type": "string" });
        assert_eq!(
            validate_field_value("temper-goal", "anything-goes", &schema_prop),
            None,
            "field with no enum/integer constraint should always return None"
        );
    }

    #[test]
    fn validate_field_value_empty_schema_accepts_any() {
        let schema_prop = serde_json::json!({});
        assert_eq!(
            validate_field_value("title", "Some title", &schema_prop),
            None
        );
    }

    // -------------------------------------------------------------------------
    // updatable_fields tests
    // -------------------------------------------------------------------------

    #[test]
    fn updatable_fields_task_includes_expected_fields() {
        let fields = updatable_fields("task").expect("task schema should load");
        let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            names.contains(&"temper-stage"),
            "task should have temper-stage"
        );
        assert!(
            names.contains(&"temper-mode"),
            "task should have temper-mode"
        );
        assert!(
            names.contains(&"temper-effort"),
            "task should have temper-effort"
        );
    }

    #[test]
    fn updatable_fields_task_excludes_system_managed() {
        let fields = updatable_fields("task").expect("task schema should load");
        let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
        for system_field in SYSTEM_MANAGED_FIELDS {
            assert!(
                !names.contains(system_field),
                "system-managed field '{system_field}' should not be in updatable fields"
            );
        }
    }

    #[test]
    fn updatable_fields_goal_includes_status() {
        let fields = updatable_fields("goal").expect("goal schema should load");
        let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            names.contains(&"temper-status"),
            "goal should have temper-status"
        );
    }

    #[test]
    fn updatable_fields_session_has_fewer_than_task() {
        let task_fields = updatable_fields("task").expect("task schema should load");
        let session_fields = updatable_fields("session").expect("session schema should load");
        assert!(
            task_fields.len() > session_fields.len(),
            "task should have more updatable fields than session (got task={}, session={})",
            task_fields.len(),
            session_fields.len()
        );
    }

    #[test]
    fn updatable_fields_unknown_doctype_returns_error() {
        let result = updatable_fields("unknown-type");
        assert!(result.is_err(), "unknown doctype should return an error");
    }

    // -------------------------------------------------------------------------
    // display_fields tests
    // -------------------------------------------------------------------------

    #[test]
    fn display_fields_task_includes_extra_columns() {
        let fields = display_fields("task").expect("task display fields");
        let names: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
        // Universal first
        assert_eq!(names[0], "temper-context");
        assert_eq!(names[1], "temper-type");
        assert_eq!(names[2], "temper-slug");
        assert_eq!(names[3], "temper-updated");
        // Task extras
        assert!(names.contains(&"temper-stage"));
        assert!(names.contains(&"temper-mode"));
        assert!(names.contains(&"temper-effort"));
        assert!(names.contains(&"temper-goal"));
    }

    #[test]
    fn display_fields_goal_has_status_and_seq() {
        let fields = display_fields("goal").expect("goal display fields");
        assert!(fields.contains(&"temper-status".to_string()));
        assert!(fields.contains(&"temper-seq".to_string()));
    }

    #[test]
    fn display_fields_session_universal_only() {
        let fields = display_fields("session").expect("session display fields");
        assert_eq!(
            fields,
            vec![
                "temper-context".to_string(),
                "temper-type".to_string(),
                "temper-slug".to_string(),
                "temper-updated".to_string(),
            ]
        );
    }

    #[test]
    fn display_fields_unknown_type_errors() {
        assert!(display_fields("widget").is_err());
    }

    // -------------------------------------------------------------------------
    // validate_allowing_provisional tests
    // -------------------------------------------------------------------------

    #[test]
    fn validate_allowing_provisional_accepts_provisional_only_task() {
        let yaml_str = r#"
temper-provisional-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: "task"
temper-context: "temper"
temper-created: "2026-04-12T00:00:00Z"
temper-stage: "backlog"
temper-title: "Test task"
temper-slug: "test-task"
"#;
        let fm: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap();
        let issues = validate_allowing_provisional("task", &fm).unwrap();
        assert!(
            issues.is_empty(),
            "provisional-only task should validate clean, got: {issues:?}"
        );
    }

    #[test]
    fn validate_allowing_provisional_accepts_both_ids() {
        let yaml_str = r#"
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-provisional-id: "019d8110-8ff3-70c2-85ae-57e04ed62886"
temper-type: "task"
temper-context: "temper"
temper-created: "2026-04-12T00:00:00Z"
temper-stage: "backlog"
temper-title: "Test task"
temper-slug: "test-task"
"#;
        let fm: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap();
        let issues = validate_allowing_provisional("task", &fm).unwrap();
        assert!(
            issues.is_empty(),
            "task with both ids should validate clean (temper-id wins, provisional is stripped from clone), got: {issues:?}"
        );
    }

    #[test]
    fn validate_allowing_provisional_ignores_invalid_provisional_when_id_present() {
        // When temper-id is present and valid, the wrapper must NOT overwrite
        // it with a (potentially invalid) temper-provisional-id. Proves the
        // swap only fires when temper-id is absent.
        let yaml_str = r#"
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-provisional-id: "not-a-uuid-at-all"
temper-type: "task"
temper-context: "temper"
temper-created: "2026-04-12T00:00:00Z"
temper-stage: "backlog"
temper-title: "Test task"
temper-slug: "test-task"
"#;
        let fm: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap();
        let issues = validate_allowing_provisional("task", &fm).unwrap();
        assert!(
            issues.is_empty(),
            "when temper-id is present and valid, an invalid temper-provisional-id should be silently stripped (not copied over temper-id), got: {issues:?}"
        );
    }

    #[test]
    fn validate_allowing_provisional_copies_provisional_when_id_absent() {
        // When temper-id is absent, the provisional value is copied into the
        // temper-id slot on the clone. If the provisional fails the UUID
        // pattern, the delegated validator sees that failure — proving the
        // copy happened (not the strip).
        let yaml_str = r#"
temper-provisional-id: "not-a-uuid-at-all"
temper-type: "task"
temper-context: "temper"
temper-created: "2026-04-12T00:00:00Z"
temper-stage: "backlog"
temper-title: "Test task"
temper-slug: "test-task"
"#;
        let fm: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap();
        let issues = validate_allowing_provisional("task", &fm).unwrap();
        assert!(
            !issues.is_empty(),
            "invalid provisional-id copied into temper-id slot must surface as a pattern validation error"
        );
        // The validator emits the instance path (e.g., "/temper-id") and a
        // message like `"not-a-uuid-at-all" does not match "^[0-9a-f]..."`.
        // Join both into one string to assert the failure landed on the
        // right field for the right reason.
        let joined = issues
            .iter()
            .map(|i| format!("{} {}", i.path, i.message))
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(
            joined.contains("temper-id") && joined.contains("not-a-uuid-at-all"),
            "error should mention temper-id (the field) and the invalid value (proving the provisional value was copied over, not stripped), got: {joined}"
        );
    }

    #[test]
    fn validate_allowing_provisional_rejects_neither_id() {
        let yaml_str = r#"
temper-type: "task"
temper-context: "temper"
temper-created: "2026-04-12T00:00:00Z"
temper-stage: "backlog"
title: "Test task"
temper-slug: "test-task"
"#;
        let fm: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap();
        let issues = validate_allowing_provisional("task", &fm).unwrap();
        assert!(
            !issues.is_empty(),
            "task with neither id must report missing temper-id"
        );
        let joined = issues
            .iter()
            .map(|i| i.message.clone())
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(
            joined.contains("temper-id"),
            "error should mention temper-id, got: {joined}"
        );
    }

    #[test]
    fn validate_allowing_provisional_does_not_mutate_input() {
        let yaml_str = r#"
temper-provisional-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: "task"
temper-context: "temper"
temper-created: "2026-04-12T00:00:00Z"
temper-stage: "backlog"
title: "Test task"
temper-slug: "test-task"
"#;
        let fm: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap();
        let _ = validate_allowing_provisional("task", &fm).unwrap();
        let mapping = fm.as_mapping().expect("original should still be mapping");
        assert!(
            mapping.contains_key(serde_yaml::Value::String(
                "temper-provisional-id".to_string()
            )),
            "original frontmatter must still contain temper-provisional-id after validation"
        );
    }

    #[test]
    fn validate_allowing_provisional_preserves_other_errors() {
        let yaml_str = r#"
temper-provisional-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: "task"
temper-context: "temper"
temper-created: "2026-04-12T00:00:00Z"
temper-stage: "nonsense"
title: "Test task"
temper-slug: "test-task"
"#;
        let fm: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap();
        let issues = validate_allowing_provisional("task", &fm).unwrap();
        assert!(
            !issues.is_empty(),
            "invalid enum value must still be reported"
        );
    }

    #[test]
    fn check_unknown_temper_fields_accepts_provisional_id() {
        let yaml_str = r#"
temper-provisional-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: "task"
title: "Test task"
"#;
        let fm: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap();
        let issues = check_unknown_temper_fields(&fm);
        assert!(
            issues.is_empty(),
            "temper-provisional-id should be a known field, got: {issues:?}"
        );
    }
}
