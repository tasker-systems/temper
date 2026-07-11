//! Schema validation for temper vault frontmatter.
//!
//! Loads JSON Schema files embedded at compile time, validates YAML frontmatter
//! against them, and finds unknown temper-* fields.

use crate::frontmatter::fields::{KNOWN_TEMPER_FIELDS, SYSTEM_MANAGED_FIELDS};
use jsonschema::{Resource, Validator};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;
use temper_core::error::{Result, TemperError};

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

/// The open_meta recognized-conventions schema. Constrains the shape of
/// recognized open (caller-defined) keys — the FTS-indexed pair
/// (`keywords`/`tags`@C, `descriptor`@D) plus shape-only conventions
/// (`date`, relationship refs) — while leaving the tier open
/// (`additionalProperties: true`). The indexed set is versioned by migration;
/// see `docs/search-open-meta-indexing.md`.
const OPEN_META_SCHEMA: &str = include_str!("../schemas/open_meta.schema.json");

/// Discouraged open_meta keys → the managed field that supersedes each.
/// A bare `slug`/`title` in the open tier is almost always a mistake: the
/// canonical identity lives in `temper-slug`/`temper-title` (managed), and a
/// bare copy drifts silently. Surfaced as a warning, never a hard error.
const DISCOURAGED_OPEN_META_KEYS: &[(&str, &str)] =
    &[("slug", "temper-slug"), ("title", "temper-title")];

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
/// Open tail (spec D3 / Task A2): an unrecognized `doc_type` carries no
/// embedded JSON schema to enforce, so it short-circuits to `Ok(vec![])`
/// rather than erroring — recognized doctypes are unaffected.
///
/// # Errors
/// Returns an error if the schema cannot be loaded or if YAML→JSON conversion
/// fails.
pub fn validate_frontmatter(
    doc_type: &str,
    frontmatter: &serde_yaml::Value,
) -> Result<Vec<ValidationIssue>> {
    if crate::frontmatter::DocType::from_str(doc_type).is_err() {
        return Ok(Vec::new()); // open tail: unrecognized doctype has no frontmatter schema to enforce
    }
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
/// Use this from CLI code paths that may inspect a resource holding a
/// provisional id before its first publish. Server code paths should call
/// [`validate_frontmatter`] directly.
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
// open_meta recognized-conventions validation
// ---------------------------------------------------------------------------

/// Compile a validator for the open_meta recognized-conventions schema.
///
/// The schema is self-contained (no `$ref`), so no external resource needs
/// registering — unlike [`load_schema`].
///
/// # Errors
/// Returns [`TemperError::Config`] if the embedded schema fails to parse or
/// compile (a programming error — the schema ships with the binary).
pub fn load_open_meta_schema() -> Result<Validator> {
    let schema_json: serde_json::Value = serde_json::from_str(OPEN_META_SCHEMA)
        .map_err(|e| TemperError::Config(format!("open_meta schema JSON parse error: {e}")))?;

    jsonschema::options()
        .build(&schema_json)
        .map_err(|e| TemperError::Config(format!("open_meta schema compilation error: {e}")))
}

/// Validate an `open_meta` object against the recognized-conventions schema.
///
/// Returns the **shape** issues only: a recognized key carrying a wrong shape
/// (e.g. `descriptor: 42`, `keywords: "a,b"` where a list was meant, a
/// malformed `date`). Because the schema is `additionalProperties: true`, an
/// unrecognized key never produces an issue — the open tier stays open, and
/// version skew (a newer convention key reaching an older validator) never
/// hard-fails. An empty list means every recognized key is well-shaped.
///
/// Discouraged-key *warnings* are deliberately not folded in here — call
/// [`check_discouraged_open_meta_keys`] for those, so callers can treat shape
/// errors (hard) and discouraged keys (soft) with different severities.
///
/// A non-object `open_meta` (array, string, …) is itself a shape violation and
/// surfaces as one issue at the root path.
///
/// # Errors
/// Propagates a [`TemperError::Config`] if the embedded schema cannot compile.
pub fn validate_open_meta(open_meta: &serde_json::Value) -> Result<Vec<ValidationIssue>> {
    let validator = load_open_meta_schema()?;

    let issues = validator
        .iter_errors(open_meta)
        .map(|err| ValidationIssue {
            path: err.instance_path().to_string(),
            message: err.to_string(),
            auto_fixable: false,
        })
        .collect();

    Ok(issues)
}

/// Find discouraged open_meta keys (bare `slug`/`title`) that duplicate a
/// managed identity field. Returned as non-auto-fixable warnings — the caller
/// decides whether to block or merely surface them. A non-object value yields
/// no issues (shape is [`validate_open_meta`]'s job).
pub fn check_discouraged_open_meta_keys(open_meta: &serde_json::Value) -> Vec<ValidationIssue> {
    let Some(map) = open_meta.as_object() else {
        return vec![];
    };

    DISCOURAGED_OPEN_META_KEYS
        .iter()
        .filter(|(key, _)| map.contains_key(*key))
        .map(|(key, managed)| ValidationIssue {
            path: (*key).to_string(),
            message: format!(
                "open_meta key '{key}' is discouraged — canonical identity lives in the managed \
                 field '{managed}'; a bare '{key}' in open_meta drifts silently"
            ),
            auto_fixable: false,
        })
        .collect()
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
    use crate::frontmatter::DocType;

    const UNIVERSAL: &[&str] = &[
        "temper-context",
        "temper-type",
        "temper-slug",
        "temper-updated",
    ];

    let extras: &[&str] = match DocType::from_str(doc_type)? {
        DocType::Task => &["temper-stage", "temper-mode", "temper-effort"],
        DocType::Goal => &["temper-status", "temper-seq"],
        DocType::Session
        | DocType::Research
        | DocType::Concept
        | DocType::Decision
        | DocType::Fact
        | DocType::Memory
        | DocType::Question
        | DocType::Theme
        | DocType::Concern
        | DocType::Principle
        | DocType::Commitment
        | DocType::Domain => &[],
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
pub static DOC_TYPE_NAMES: &[&str] = &[
    "commitment",
    "concept",
    "concern",
    "decision",
    "domain",
    "fact",
    "goal",
    "memory",
    "principle",
    "question",
    "research",
    "session",
    "task",
    "theme",
];

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

/// Return the embedded open_meta recognized-conventions schema as a JSON value.
///
/// This is the self-describing dump behind `temper resource describe-open-meta` and the MCP
/// `describe_open_meta` tool: the schema documents which open (caller-defined) keys temper
/// recognizes, their shapes, and — via each property's `description` — whether the key is
/// FTS-indexed (and at what weight) or shape-only. The tier stays open
/// (`additionalProperties: true`), so this is guidance, not a closed vocabulary.
///
/// # Errors
/// Returns [`TemperError::Config`] if the embedded schema fails to parse (a programming error —
/// the schema ships with the binary).
pub fn open_meta_schema_value() -> Result<serde_json::Value> {
    serde_json::from_str(OPEN_META_SCHEMA)
        .map_err(|e| TemperError::Config(format!("open_meta schema JSON parse error: {e}")))
}

/// A discouraged open_meta key and the managed field that supersedes it.
#[derive(Debug, Clone, Serialize)]
pub struct DiscouragedOpenMetaKey {
    pub key: String,
    pub use_instead: String,
}

/// The self-describing open_meta convention, returned by [`describe_open_meta`] and rendered by the
/// CLI `resource describe-open-meta` command + the MCP `describe_open_meta` tool. Both surfaces share
/// this type so the guidance can never drift between them.
#[derive(Debug, Clone, Serialize)]
pub struct OpenMetaConvention {
    /// The recognized-conventions JSON Schema. Self-describing: each property's `description` states
    /// whether the key is FTS-indexed (and at what weight) or shape-only, and the schema `title`
    /// carries the convention version. The tier stays open (`additionalProperties: true`), so this is
    /// guidance, not a closed vocabulary.
    pub schema: serde_json::Value,
    /// Discouraged bare keys — absent from `schema` because the tier is open, surfaced here so callers
    /// can see them → the managed field that supersedes each.
    pub discouraged_keys: Vec<DiscouragedOpenMetaKey>,
}

/// Build the self-describing open_meta convention (schema + discouraged keys).
///
/// # Errors
/// Returns [`TemperError::Config`] if the embedded schema fails to parse (a programming error).
pub fn describe_open_meta() -> Result<OpenMetaConvention> {
    Ok(OpenMetaConvention {
        schema: open_meta_schema_value()?,
        discouraged_keys: DISCOURAGED_OPEN_META_KEYS
            .iter()
            .map(|(key, managed)| DiscouragedOpenMetaKey {
                key: (*key).to_string(),
                use_instead: (*managed).to_string(),
            })
            .collect(),
    })
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

/// Cached non-null string enum values for the `task` doc-type schema,
/// keyed by property name (e.g. `"temper-mode"` → `["plan", "build"]`).
///
/// The task schema is the single source of truth for valid `temper-mode` /
/// `temper-effort` values. This caches the parsed enums so the embedded JSON
/// is parsed at most once. JSON `null` entries in the schema enum (the schema
/// allows the field to be absent/null) are filtered out — validation only runs
/// on `Some(value)`, so callers want the concrete string set.
static TASK_ENUM_VALUES: OnceLock<BTreeMap<String, Vec<String>>> = OnceLock::new();

/// Return the allowed non-null string enum values for a property of the `task`
/// schema, or an empty slice if the property has no string enum constraint.
///
/// Backed by [`enum_fields`] (which already filters non-string entries, so JSON
/// `null` is dropped) and cached in a [`OnceLock`]. This is the schema-backed
/// replacement for hard-coded mode/effort whitelists — the JSON schema at
/// `schemas/task.schema.json` is the single source of truth.
///
/// # Panics
/// Panics if the embedded task schema fails to parse, which would indicate a
/// build-time error in the committed schema file rather than a runtime
/// condition.
pub fn task_enum_values(field: &str) -> &'static [String] {
    let map = TASK_ENUM_VALUES
        .get_or_init(|| enum_fields("task").expect("embedded task schema must parse"));
    map.get(field).map_or(&[], Vec::as_slice)
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
    // open_meta recognized-conventions tests
    // -------------------------------------------------------------------------

    #[test]
    fn open_meta_well_shaped_recognized_keys_pass() {
        let om = serde_json::json!({
            "keywords": ["thermocline", "stratification"],
            "descriptor": "Vertical temperature structure of the open ocean",
            "tags": ["concept"],
            "date": "2026-07-11",
            "relates_to": ["temper"]
        });
        let issues = validate_open_meta(&om).expect("schema compiles");
        assert!(
            issues.is_empty(),
            "well-shaped open_meta should pass: {issues:?}"
        );
    }

    #[test]
    fn open_meta_keywords_accepts_bare_string() {
        // Convention v1 blesses a bare string as well as an array.
        let om = serde_json::json!({ "keywords": "thermocline stratification" });
        let issues = validate_open_meta(&om).expect("schema compiles");
        assert!(
            issues.is_empty(),
            "bare-string keywords should pass: {issues:?}"
        );
    }

    #[test]
    fn open_meta_misshaped_descriptor_is_flagged() {
        // descriptor: 42 is stored-but-not-indexed — the exact silent footgun.
        let om = serde_json::json!({ "descriptor": 42 });
        let issues = validate_open_meta(&om).expect("schema compiles");
        assert!(
            issues.iter().any(|i| i.path.contains("descriptor")),
            "numeric descriptor should be flagged: {issues:?}"
        );
    }

    #[test]
    fn open_meta_tags_accepts_array_and_bare_string() {
        // Convention v2: `tags` is the declared synonym of `keywords` and shares its shape rules —
        // a JSON array of strings OR a bare string (both fold into the C-weight vector; the FTS
        // parser tokenizes the string). Deliberately as permissive as `keywords`: since the
        // receive-side gate (D) is the first server-side open_meta validation, hard-rejecting a
        // bare-string tag the SQL indexes fine would break existing callers (wire-contract skew).
        for value in [
            serde_json::json!({ "tags": ["concept", "design"] }),
            serde_json::json!({ "tags": "concept design" }),
        ] {
            let issues = validate_open_meta(&value).expect("schema compiles");
            assert!(
                issues.is_empty(),
                "tags array or bare string should pass: {issues:?}"
            );
        }
    }

    #[test]
    fn open_meta_misshaped_tags_is_flagged() {
        // A non-string array element is a genuine shape violation (a tag must be text).
        let om = serde_json::json!({ "tags": [1, 2] });
        let issues = validate_open_meta(&om).expect("schema compiles");
        assert!(
            issues.iter().any(|i| i.path.contains("tags")),
            "numeric tag elements should be flagged: {issues:?}"
        );
    }

    #[test]
    fn open_meta_schema_recognized_key_set_is_pinned() {
        // A hand-authored-schema "snapshot": pin the exact recognized-key set and the FTS-indexed
        // subset, so a stray add/remove or a lost FTS-indexed marker trips this test instead of
        // silently changing the convention. The migrations (v1/v2) and this schema must agree.
        let schema = open_meta_schema_value().expect("schema compiles");
        let props = schema
            .get("properties")
            .and_then(|p| p.as_object())
            .expect("open_meta schema has properties");

        let mut recognized: Vec<&str> = props.keys().map(String::as_str).collect();
        recognized.sort_unstable();
        assert_eq!(
            recognized,
            [
                "date",
                "depends_on",
                "derived_from",
                "descriptor",
                "keywords",
                "preceded_by",
                "references",
                "relates_to",
                "tags",
            ],
            "recognized open_meta key set drifted — update the schema, the docs, and this pin together"
        );

        // The FTS-indexed keys are exactly keywords/tags/descriptor (each description says so). This is
        // the classification the migrations enforce; keep them in lockstep.
        let indexed: Vec<&str> = props
            .iter()
            .filter(|(_, v)| {
                v.get("description")
                    .and_then(|d| d.as_str())
                    .is_some_and(|d| d.contains("FTS-indexed at weight"))
            })
            .map(|(k, _)| k.as_str())
            .collect();
        let mut indexed_sorted = indexed;
        indexed_sorted.sort_unstable();
        assert_eq!(
            indexed_sorted,
            ["descriptor", "keywords", "tags"],
            "the FTS-indexed key set drifted from keywords/tags/descriptor"
        );
    }

    #[test]
    fn describe_open_meta_surfaces_schema_and_discouraged_keys() {
        let conv = describe_open_meta().expect("schema compiles");
        // The schema is self-describing: recognized keys with descriptions.
        let props = conv
            .schema
            .get("properties")
            .and_then(|p| p.as_object())
            .expect("open_meta schema has properties");
        for key in ["keywords", "tags", "descriptor", "date"] {
            assert!(
                props.contains_key(key),
                "recognized key {key} must be present"
            );
        }
        // A recognized-key description marks its FTS-indexing (guidance is in the description).
        assert!(
            props["descriptor"]["description"]
                .as_str()
                .unwrap_or_default()
                .contains("FTS-indexed"),
            "descriptor description should mark it FTS-indexed"
        );
        // The open tier stays open.
        assert_eq!(
            conv.schema.get("additionalProperties"),
            Some(&serde_json::json!(true))
        );
        // Discouraged bare keys surface with their canonical replacement.
        let slug = conv
            .discouraged_keys
            .iter()
            .find(|d| d.key == "slug")
            .expect("slug is a discouraged key");
        assert_eq!(slug.use_instead, "temper-slug");
    }

    #[test]
    fn open_meta_malformed_date_is_flagged() {
        let om = serde_json::json!({ "date": "July 11, 2026" });
        let issues = validate_open_meta(&om).expect("schema compiles");
        assert!(
            issues.iter().any(|i| i.path.contains("date")),
            "non-ISO date should be flagged: {issues:?}"
        );
    }

    #[test]
    fn open_meta_unknown_key_passes_open_tier() {
        // additionalProperties: true — the open tier stays open, and version
        // skew (an unknown future key) never hard-fails.
        let om = serde_json::json!({ "some-future-key": { "nested": [1, 2, 3] } });
        let issues = validate_open_meta(&om).expect("schema compiles");
        assert!(
            issues.is_empty(),
            "unknown key should pass untouched: {issues:?}"
        );
    }

    #[test]
    fn open_meta_non_object_is_a_shape_violation() {
        let om = serde_json::json!(["not", "an", "object"]);
        let issues = validate_open_meta(&om).expect("schema compiles");
        assert!(
            !issues.is_empty(),
            "a non-object open_meta should be flagged"
        );
    }

    #[test]
    fn open_meta_discouraged_keys_warn_but_are_separate_from_shape() {
        let om = serde_json::json!({ "slug": "path-to-alpha", "title": "Path to Alpha" });
        // Shape-clean (strings are valid), so no shape issues …
        let shape = validate_open_meta(&om).expect("schema compiles");
        assert!(
            shape.is_empty(),
            "bare slug/title are shape-valid strings: {shape:?}"
        );
        // … but flagged as discouraged.
        let discouraged = check_discouraged_open_meta_keys(&om);
        assert_eq!(
            discouraged.len(),
            2,
            "both slug and title should warn: {discouraged:?}"
        );
        assert!(discouraged
            .iter()
            .any(|i| i.message.contains("temper-slug")));
        assert!(discouraged
            .iter()
            .any(|i| i.message.contains("temper-title")));
    }

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
            validate_field_value("temper-branch", "anything-goes", &schema_prop),
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

    // -------------------------------------------------------------------------
    // task_enum_values tests
    // -------------------------------------------------------------------------

    #[test]
    fn task_enum_values_mode_excludes_null() {
        assert_eq!(
            task_enum_values("temper-mode"),
            &["plan".to_string(), "build".to_string()],
            "temper-mode enum should be exactly [plan, build] with JSON null filtered out"
        );
    }

    #[test]
    fn task_enum_values_effort_excludes_null() {
        assert_eq!(
            task_enum_values("temper-effort"),
            &[
                "small".to_string(),
                "medium".to_string(),
                "large".to_string()
            ],
            "temper-effort enum should be exactly [small, medium, large] with JSON null filtered out"
        );
    }

    #[test]
    fn task_enum_values_unknown_field_is_empty() {
        assert!(
            task_enum_values("temper-nonexistent").is_empty(),
            "a property with no string enum constraint should return an empty slice"
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
