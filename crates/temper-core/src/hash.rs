//! Unified sync-related hash computation — single source of truth.
//!
//! Both CLI and API call these functions so that a document hashed from YAML
//! frontmatter on the client side produces the same digest as one hashed from
//! JSON columns on the server side, provided the same defaults are applied.

use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Identity fields are never included in any hash tier — they identify the
/// record but aren't content.
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

// ---------------------------------------------------------------------------
// Body hash
// ---------------------------------------------------------------------------

/// SHA-256 hash of the markdown body content.
///
/// Returns `"sha256:<hex>"`.
pub fn compute_body_hash(body: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

// ---------------------------------------------------------------------------
// JSON canonicalization and hashing
// ---------------------------------------------------------------------------

/// Recursively sort all object keys so serialization is deterministic.
pub fn canonicalize_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let sorted: BTreeMap<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), canonicalize_json(v)))
                .collect();
            serde_json::to_value(sorted).unwrap_or(serde_json::Value::Object(Default::default()))
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(canonicalize_json).collect())
        }
        other => other.clone(),
    }
}

/// Canonicalize then SHA-256. Returns `"sha256:<hex>"`.
pub fn hash_canonical_json(value: &serde_json::Value) -> String {
    let canonical = canonicalize_json(value);
    let serialized = serde_json::to_string(&canonical).unwrap_or_else(|_| "{}".to_string());
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

// ---------------------------------------------------------------------------
// Managed / open hash
// ---------------------------------------------------------------------------

/// Hash managed metadata: strip tier-1 system fields, apply doc-type
/// defaults, then hash the canonical JSON.
///
/// This ensures that a file missing an optional default field (e.g.
/// `temper-stage` for tasks) hashes identically to one where the default
/// value is explicitly present.
pub fn compute_managed_hash(doc_type: &str, managed_meta: &serde_json::Value) -> String {
    let mut meta = managed_meta.clone();

    // Strip tier-1 system fields — the DB is authoritative for these.
    if let Some(obj) = meta.as_object_mut() {
        for &field in TIER1_SYSTEM_FIELDS {
            obj.remove(field);
        }
    }

    // Fill in doc-type defaults so both sides agree.
    crate::defaults::apply_doc_type_defaults(doc_type, &mut meta);

    hash_canonical_json(&meta)
}

/// Hash open (user-defined) metadata — canonical JSON, no defaults.
pub fn compute_open_hash(open_meta: &serde_json::Value) -> String {
    hash_canonical_json(open_meta)
}

// ---------------------------------------------------------------------------
// Frontmatter splitting
// ---------------------------------------------------------------------------

/// Split YAML frontmatter into (managed, open) JSON values.
///
/// - **Identity fields** (`temper-id`, `temper-provisional-id`) and
///   **tier-1 system fields** (`temper-context`, `temper-type`, …) are
///   excluded from both tiers.
/// - `temper-*` prefixed keys, `title`, `slug`, and any properties defined
///   in the doc-type schema go to **managed**.
/// - Everything else goes to **open**.
pub fn split_frontmatter_tiers(
    fm: &serde_yaml::Value,
    doc_type: &str,
) -> (serde_json::Value, serde_json::Value) {
    let Some(mapping) = fm.as_mapping() else {
        return (serde_json::json!({}), serde_json::json!({}));
    };

    let skip: HashSet<&str> = IDENTITY_FIELDS
        .iter()
        .chain(TIER1_SYSTEM_FIELDS.iter())
        .copied()
        .collect();

    // Collect doc-type schema property names so non-temper-* schema fields
    // (like `date` for sessions) route to managed_meta instead of open_meta.
    let schema_keys: HashSet<String> = crate::schema::schema_value(doc_type)
        .ok()
        .and_then(|v| v.get("properties")?.as_object().cloned())
        .map(|props| props.keys().cloned().collect())
        .unwrap_or_default();

    let mut managed = serde_json::Map::new();
    let mut open = serde_json::Map::new();

    for (key, value) in mapping {
        let Some(key_str) = key.as_str() else {
            continue;
        };
        if skip.contains(key_str) {
            continue;
        }
        let json_value = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);
        if key_str.starts_with("temper-")
            || key_str == "title"
            || key_str == "slug"
            || schema_keys.contains(key_str)
        {
            managed.insert(key_str.to_string(), json_value);
        } else {
            open.insert(key_str.to_string(), json_value);
        }
    }

    (
        serde_json::Value::Object(managed),
        serde_json::Value::Object(open),
    )
}

// ---------------------------------------------------------------------------
// Convenience helper
// ---------------------------------------------------------------------------

/// Convenience: parse frontmatter, split tiers, and compute both hashes.
///
/// Returns `(managed_hash, open_hash)`. If frontmatter is `None` or not a
/// valid mapping, hashes are computed over empty objects (with doc-type
/// defaults still applied for the managed hash).
pub fn compute_frontmatter_hashes_from_yaml(
    frontmatter: Option<&serde_yaml::Value>,
    doc_type: &str,
) -> (String, String) {
    if let Some(fm) = frontmatter {
        let (managed_meta, open_meta) = split_frontmatter_tiers(fm, doc_type);
        (
            compute_managed_hash(doc_type, &managed_meta),
            compute_open_hash(&open_meta),
        )
    } else {
        (
            compute_managed_hash(doc_type, &serde_json::json!({})),
            compute_open_hash(&serde_json::json!({})),
        )
    }
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Extract doc type from a vault-relative path.
///
/// E.g. `@me/temper/task/my-task.md` → `Some("task")` (second-to-last segment).
pub fn doc_type_from_vault_path(path: &str) -> Option<&str> {
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() >= 2 {
        Some(segments[segments.len() - 2])
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // 1. body_hash_deterministic
    #[test]
    fn body_hash_deterministic() {
        let h1 = compute_body_hash("hello world");
        let h2 = compute_body_hash("hello world");
        assert_eq!(h1, h2);
        assert!(h1.starts_with("sha256:"));
    }

    // 2. body_hash_differs_for_different_content
    #[test]
    fn body_hash_differs_for_different_content() {
        let h1 = compute_body_hash("hello");
        let h2 = compute_body_hash("world");
        assert_ne!(h1, h2);
    }

    // 3. canonicalize_sorts_keys
    #[test]
    fn canonicalize_sorts_keys() {
        let input = json!({"z": 1, "a": 2});
        let canonical = canonicalize_json(&input);
        let serialized = serde_json::to_string(&canonical).unwrap();
        assert_eq!(serialized, r#"{"a":2,"z":1}"#);
    }

    // 4. canonicalize_sorts_nested_keys
    #[test]
    fn canonicalize_sorts_nested_keys() {
        let input = json!({"b": {"z": 1, "a": 2}, "a": 1});
        let canonical = canonicalize_json(&input);
        let serialized = serde_json::to_string(&canonical).unwrap();
        assert_eq!(serialized, r#"{"a":1,"b":{"a":2,"z":1}}"#);
    }

    // 5. hash_canonical_json_deterministic
    #[test]
    fn hash_canonical_json_deterministic() {
        let v1 = json!({"z": 1, "a": 2});
        let v2 = json!({"a": 2, "z": 1});
        assert_eq!(hash_canonical_json(&v1), hash_canonical_json(&v2));
    }

    // 6. hash_canonical_json_empty_object
    #[test]
    fn hash_canonical_json_empty_object() {
        let h = hash_canonical_json(&json!({}));
        assert!(h.starts_with("sha256:"));
        assert!(h.len() > 10);
    }

    // 7. managed_hash_applies_defaults_before_hashing
    #[test]
    fn managed_hash_applies_defaults_before_hashing() {
        // Task without temper-stage should hash the same as one with the default "backlog"
        let without = json!({});
        let with_default = json!({"temper-stage": "backlog"});
        assert_eq!(
            compute_managed_hash("task", &without),
            compute_managed_hash("task", &with_default),
        );
    }

    // 8. managed_hash_preserves_explicit_values
    #[test]
    fn managed_hash_preserves_explicit_values() {
        let in_progress = json!({"temper-stage": "in-progress"});
        let backlog = json!({"temper-stage": "backlog"});
        assert_ne!(
            compute_managed_hash("task", &in_progress),
            compute_managed_hash("task", &backlog),
        );
    }

    // 9. managed_hash_strips_tier1_system_fields
    #[test]
    fn managed_hash_strips_tier1_system_fields() {
        let without = json!({"title": "Test"});
        let with_tier1 = json!({"title": "Test", "temper-created": "2026-01-01T00:00:00Z"});
        assert_eq!(
            compute_managed_hash("task", &without),
            compute_managed_hash("task", &with_tier1),
        );
    }

    // 10. managed_hash_deterministic_regardless_of_key_order
    #[test]
    fn managed_hash_deterministic_regardless_of_key_order() {
        let v1 = json!({"title": "A", "temper-stage": "backlog"});
        let v2 = json!({"temper-stage": "backlog", "title": "A"});
        assert_eq!(
            compute_managed_hash("task", &v1),
            compute_managed_hash("task", &v2),
        );
    }

    // 11. split_tiers_partitions_correctly
    #[test]
    fn split_tiers_partitions_correctly() {
        let yaml_str = r#"
temper-id: "abc123"
temper-type: "task"
temper-context: "ctx"
temper-stage: "backlog"
title: "My Task"
slug: "my-task"
custom-field: "value"
"#;
        let fm: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap();
        let (managed, open) = split_frontmatter_tiers(&fm, "task");

        // temper-stage, title, slug → managed
        assert!(managed.get("temper-stage").is_some());
        assert!(managed.get("title").is_some());
        assert!(managed.get("slug").is_some());

        // custom-field → open
        assert!(open.get("custom-field").is_some());

        // identity and tier-1 system fields → neither
        assert!(managed.get("temper-id").is_none());
        assert!(open.get("temper-id").is_none());
        assert!(managed.get("temper-type").is_none());
        assert!(open.get("temper-type").is_none());
        assert!(managed.get("temper-context").is_none());
        assert!(open.get("temper-context").is_none());
    }

    // 12. split_tiers_routes_schema_properties_to_managed
    #[test]
    fn split_tiers_routes_schema_properties_to_managed() {
        let yaml_str = r#"
date: "2026-01-15"
custom: "stuff"
"#;
        let fm: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap();
        let (managed, open) = split_frontmatter_tiers(&fm, "session");

        assert!(
            managed.get("date").is_some(),
            "date should be managed for sessions"
        );
        assert!(open.get("custom").is_some(), "custom should be open");
    }

    // 13. doc_type_from_vault_path_valid
    #[test]
    fn doc_type_from_vault_path_valid() {
        assert_eq!(
            doc_type_from_vault_path("@me/temper/task/my-task.md"),
            Some("task"),
        );
    }

    // 14. doc_type_from_vault_path_invalid
    #[test]
    fn doc_type_from_vault_path_invalid() {
        assert_eq!(doc_type_from_vault_path("bad-path.md"), None);
    }

    // 15. cli_and_api_path_produce_same_managed_hash
    #[test]
    fn cli_and_api_path_produce_same_managed_hash() {
        // Simulate CLI path: YAML with tier-1 fields → split → hash managed
        let yaml_str = r#"
temper-id: "abc"
temper-type: "task"
temper-context: "ctx"
temper-created: "2026-01-01T00:00:00Z"
temper-updated: "2026-01-01T00:00:00Z"
temper-owner: "user1"
temper-stage: "in-progress"
title: "My Task"
"#;
        let fm: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap();
        let (managed_from_yaml, _) = split_frontmatter_tiers(&fm, "task");
        let cli_hash = compute_managed_hash("task", &managed_from_yaml);

        // Simulate API path: JSON without tier-1 fields
        let api_json = json!({"temper-stage": "in-progress", "title": "My Task"});
        let api_hash = compute_managed_hash("task", &api_json);

        assert_eq!(cli_hash, api_hash);
    }

    // 16. cli_and_api_agree_when_defaults_absent_locally
    #[test]
    fn cli_and_api_agree_when_defaults_absent_locally() {
        // CLI: YAML missing temper-stage
        let yaml_str = r#"
title: "My Task"
"#;
        let fm: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap();
        let (managed_from_yaml, _) = split_frontmatter_tiers(&fm, "task");
        let cli_hash = compute_managed_hash("task", &managed_from_yaml);

        // API: JSON with explicit default temper-stage: "backlog"
        let api_json = json!({"title": "My Task", "temper-stage": "backlog"});
        let api_hash = compute_managed_hash("task", &api_json);

        assert_eq!(cli_hash, api_hash);
    }

    // 17. round_trip_hash_agreement_all_doc_types
    //
    // For each doc type, simulate the CLI path (YAML with tier-1 fields ->
    // split -> hash) and API path (JSON without tier-1 fields -> hash).
    // Both must produce the same managed hash.
    #[test]
    fn round_trip_hash_agreement_all_doc_types() {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

        let cases: Vec<(&str, String, serde_json::Value)> = vec![
            (
                "task",
                format!(
                    r#"
temper-id: "abc"
temper-type: "task"
temper-context: "ctx"
temper-created: "2026-01-01T00:00:00Z"
temper-stage: "in-progress"
title: "My Task"
custom-tag: "user-value"
"#
                ),
                json!({"temper-stage": "in-progress", "title": "My Task"}),
            ),
            (
                "goal",
                format!(
                    r#"
temper-id: "def"
temper-type: "goal"
temper-context: "ctx"
temper-status: "achieved"
title: "Ship v1"
"#
                ),
                json!({"temper-status": "achieved", "title": "Ship v1"}),
            ),
            (
                "session",
                format!(
                    r#"
temper-id: "ghi"
temper-type: "session"
temper-context: "ctx"
date: "{today}"
title: "Planning"
"#
                ),
                json!({"date": today.clone(), "title": "Planning"}),
            ),
            (
                "research",
                format!(
                    r#"
temper-id: "jkl"
temper-type: "research"
temper-context: "ctx"
date: "{today}"
title: "Survey"
"#
                ),
                json!({"date": today, "title": "Survey"}),
            ),
        ];

        for (doc_type, yaml_str, api_json) in &cases {
            let fm: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap();
            let (managed_from_yaml, _) = split_frontmatter_tiers(&fm, doc_type);
            let cli_hash = compute_managed_hash(doc_type, &managed_from_yaml);
            let api_hash = compute_managed_hash(doc_type, api_json);

            assert_eq!(
                cli_hash, api_hash,
                "CLI and API managed hashes must agree for doc_type={doc_type}"
            );
        }
    }

    // 18. defaults_make_hashes_converge
    //
    // A goal without `temper-status` hashes the same as a goal with
    // `temper-status: active` (the default).
    #[test]
    fn defaults_make_hashes_converge() {
        let without = json!({"title": "Ship v1"});
        let with_default = json!({"title": "Ship v1", "temper-status": "active"});
        assert_eq!(
            compute_managed_hash("goal", &without),
            compute_managed_hash("goal", &with_default),
        );
    }

    // 19. convenience_helper_matches_manual_steps
    //
    // `compute_frontmatter_hashes_from_yaml` produces the same result as
    // manually calling split_frontmatter_tiers + compute_managed_hash +
    // compute_open_hash.
    #[test]
    fn convenience_helper_matches_manual_steps() {
        let yaml_str = r#"
temper-id: "abc"
temper-type: "task"
temper-stage: "in-progress"
title: "My Task"
custom-field: "value"
"#;
        let fm: serde_yaml::Value = serde_yaml::from_str(yaml_str).unwrap();

        // Manual path
        let (managed_meta, open_meta) = split_frontmatter_tiers(&fm, "task");
        let manual_managed = compute_managed_hash("task", &managed_meta);
        let manual_open = compute_open_hash(&open_meta);

        // Convenience helper
        let (helper_managed, helper_open) = compute_frontmatter_hashes_from_yaml(Some(&fm), "task");

        assert_eq!(manual_managed, helper_managed);
        assert_eq!(manual_open, helper_open);
    }

    // 20. convenience_helper_none_frontmatter
    //
    // `None` frontmatter produces valid (non-empty) hashes.
    #[test]
    fn convenience_helper_none_frontmatter() {
        let (managed, open) = compute_frontmatter_hashes_from_yaml(None, "task");
        assert!(managed.starts_with("sha256:"), "managed hash must be valid");
        assert!(open.starts_with("sha256:"), "open hash must be valid");
        assert!(managed.len() > 10, "managed hash must not be empty");
        assert!(open.len() > 10, "open hash must not be empty");

        // Should be the same as hashing empty objects with defaults
        let expected_managed = compute_managed_hash("task", &json!({}));
        let expected_open = compute_open_hash(&json!({}));
        assert_eq!(managed, expected_managed);
        assert_eq!(open, expected_open);
    }
}
