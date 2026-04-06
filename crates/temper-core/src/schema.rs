//! Schema validation for temper vault frontmatter.
//!
//! Loads JSON Schema files embedded at compile time, validates YAML frontmatter
//! against them, detects legacy field names, finds unknown temper-* fields, and
//! computes three-tier frontmatter hashes.

use crate::error::{Result, TemperError};
use jsonschema::{Resource, Validator};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

// ---------------------------------------------------------------------------
// Embedded schemas
// ---------------------------------------------------------------------------

const BASE_SCHEMA: &str = include_str!("../schemas/base.schema.json");
const TASK_SCHEMA: &str = include_str!("../schemas/task.schema.json");
const GOAL_SCHEMA: &str = include_str!("../schemas/goal.schema.json");
const SESSION_SCHEMA: &str = include_str!("../schemas/session.schema.json");
const RESEARCH_SCHEMA: &str = include_str!("../schemas/research.schema.json");
const DECISION_SCHEMA: &str = include_str!("../schemas/decision.schema.json");
const CONCEPT_SCHEMA: &str = include_str!("../schemas/concept.schema.json");

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
// Known field sets
// ---------------------------------------------------------------------------

/// All temper-* field names that are explicitly defined across the schemas.
/// Used to detect possible typos in temper-* fields.
static KNOWN_TEMPER_FIELDS: &[&str] = &[
    "temper-id",
    "temper-type",
    "temper-context",
    "temper-created",
    "temper-updated",
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
];

/// Legacy field names that have been renamed to temper-* equivalents.
/// Maps old name → suggested new name.
static LEGACY_FIELDS: &[(&str, &str)] = &[
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
    let doc_schema_str = match doc_type {
        "task" => TASK_SCHEMA,
        "goal" => GOAL_SCHEMA,
        "session" => SESSION_SCHEMA,
        "research" => RESEARCH_SCHEMA,
        "decision" => DECISION_SCHEMA,
        "concept" => CONCEPT_SCHEMA,
        other => {
            return Err(TemperError::Config(format!(
                "unknown doctype '{other}'; expected one of: task, goal, session, research, decision, concept"
            )))
        }
    };

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

/// Compute SHA-256 hashes for the two tiers of frontmatter.
///
/// - **meta_hash** — hash of all `temper-*` fields (sorted for determinism).
/// - **open_hash** — hash of all other fields (sorted for determinism).
///
/// Both hashes are returned as `"sha256:{lowercase_hex}"` strings.
pub fn compute_frontmatter_hashes(frontmatter: &serde_yaml::Value) -> (String, String) {
    let Some(mapping) = frontmatter.as_mapping() else {
        return (hash_map(&BTreeMap::new()), hash_map(&BTreeMap::new()));
    };

    let mut meta: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut open: BTreeMap<String, serde_json::Value> = BTreeMap::new();

    for (key, value) in mapping {
        let Some(key_str) = key.as_str() else {
            continue;
        };
        let json_value = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);
        // Skip identity fields — the resource ID is tracked structurally
        // (manifest key / kb_resources.id), not as hashed metadata.
        if key_str == "temper-id" || key_str == "temper-provisional-id" {
            continue;
        }
        if key_str.starts_with("temper-") || key_str == "title" || key_str == "slug" {
            meta.insert(key_str.to_string(), json_value);
        } else {
            open.insert(key_str.to_string(), json_value);
        }
    }

    (hash_map(&meta), hash_map(&open))
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn hash_map(fields: &BTreeMap<String, serde_json::Value>) -> String {
    // Serialize to canonical JSON (BTreeMap keys are already sorted)
    let serialized = serde_json::to_string(fields).unwrap_or_else(|_| "{}".to_string());
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    let result = hasher.finalize();
    format!("sha256:{}", hex::encode(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_fields_excluded_from_managed_hash() {
        // temper-id and temper-provisional-id are identity fields tracked
        // structurally, not as hashed metadata.
        let yaml1: serde_yaml::Value =
            serde_yaml::from_str("temper-id: \"aaa\"\ntitle: \"Test\"\ncustom-field: \"value\"")
                .unwrap();
        let yaml2: serde_yaml::Value =
            serde_yaml::from_str("temper-id: \"bbb\"\ntitle: \"Test\"\ncustom-field: \"value\"")
                .unwrap();
        let yaml3: serde_yaml::Value = serde_yaml::from_str(
            "temper-provisional-id: \"ccc\"\ntitle: \"Test\"\ncustom-field: \"value\"",
        )
        .unwrap();

        let (m1, o1) = compute_frontmatter_hashes(&yaml1);
        let (m2, o2) = compute_frontmatter_hashes(&yaml2);
        let (m3, o3) = compute_frontmatter_hashes(&yaml3);

        assert_eq!(
            m1, m2,
            "different temper-id values should produce same managed hash"
        );
        assert_eq!(m1, m3, "temper-provisional-id should also be excluded");
        assert_eq!(o1, o2, "open hash should match");
        assert_eq!(o1, o3, "open hash should match");
    }
}
