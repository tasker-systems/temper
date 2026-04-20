//! Unified sync-related hash computation — single source of truth.
//!
//! Both CLI and API call these functions so that a document hashed from YAML
//! frontmatter on the client side produces the same digest as one hashed from
//! JSON columns on the server side, provided the same defaults are applied.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use crate::frontmatter::fields::TIER1_SYSTEM_FIELDS;

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

    // Tier-split partitioning tests previously here moved to
    // `crate::frontmatter::tiers::tests`. Removed in task 11 of session 2
    // when `split_frontmatter_tiers` was deleted.

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
    //
    // The CLI parses YAML via Frontmatter::managed_json (which strips
    // identity + tier-1 fields) and hashes the result. The API receives
    // pre-split JSON and hashes it directly. Both paths must converge.
    #[test]
    fn cli_and_api_path_produce_same_managed_hash() {
        use crate::frontmatter::Frontmatter;

        let content = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: ctx
temper-created: "2026-01-01T00:00:00Z"
temper-updated: "2026-01-01T00:00:00Z"
temper-owner: user1
title: "My Task"
slug: my-task
temper-stage: in-progress
---
"#;
        let fm = Frontmatter::try_from(content).unwrap();
        let cli_hash = compute_managed_hash("task", &fm.managed_json());

        // Simulate API path: JSON without tier-1 fields
        let api_json = json!({
            "temper-stage": "in-progress",
            "title": "My Task",
            "slug": "my-task",
        });
        let api_hash = compute_managed_hash("task", &api_json);

        assert_eq!(cli_hash, api_hash);
    }

    // 16. cli_and_api_agree_when_defaults_absent_locally
    //
    // A CLI file missing temper-stage must hash the same as an API JSON
    // with temper-stage explicitly set to the default — `compute_managed_hash`
    // applies defaults at hash time.
    #[test]
    fn cli_and_api_agree_when_defaults_absent_locally() {
        use crate::frontmatter::Frontmatter;

        let content = r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: ctx
temper-created: "2026-01-01T00:00:00Z"
title: "My Task"
slug: my-task
---
"#;
        let fm = Frontmatter::try_from(content).unwrap();
        let cli_hash = compute_managed_hash("task", &fm.managed_json());

        // API: JSON with explicit default temper-stage: "backlog"
        let api_json = json!({
            "title": "My Task",
            "slug": "my-task",
            "temper-stage": "backlog",
        });
        let api_hash = compute_managed_hash("task", &api_json);

        assert_eq!(cli_hash, api_hash);
    }

    // 17. round_trip_hash_agreement_all_doc_types
    //
    // For each doc type, the CLI path (YAML parsed via Frontmatter, then
    // fm.managed_json() hashed) must produce the same managed hash as the
    // API path (pre-split JSON hashed directly).
    #[test]
    fn round_trip_hash_agreement_all_doc_types() {
        use crate::frontmatter::Frontmatter;

        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

        let cases: Vec<(&str, String, serde_json::Value)> = vec![
            (
                "task",
                r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: ctx
temper-created: "2026-01-01T00:00:00Z"
title: "My Task"
slug: my-task
temper-stage: in-progress
---
"#
                .to_string(),
                json!({
                    "temper-stage": "in-progress",
                    "title": "My Task",
                    "slug": "my-task",
                }),
            ),
            (
                "goal",
                r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: goal
temper-context: ctx
temper-created: "2026-01-01T00:00:00Z"
title: "Ship v1"
slug: ship-v1
temper-status: achieved
---
"#
                .to_string(),
                json!({
                    "temper-status": "achieved",
                    "title": "Ship v1",
                    "slug": "ship-v1",
                }),
            ),
            (
                "session",
                format!(
                    r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: session
temper-context: ctx
temper-created: "2026-01-01T00:00:00Z"
title: "Planning"
slug: planning
date: "{today}"
---
"#
                ),
                json!({
                    "date": today.clone(),
                    "title": "Planning",
                    "slug": "planning",
                }),
            ),
            (
                "research",
                format!(
                    r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: research
temper-context: ctx
temper-created: "2026-01-01T00:00:00Z"
title: "Survey"
slug: survey
date: "{today}"
---
"#
                ),
                json!({
                    "date": today,
                    "title": "Survey",
                    "slug": "survey",
                }),
            ),
        ];

        for (doc_type, content, api_json) in &cases {
            let fm = Frontmatter::try_from(content.as_str()).unwrap();
            let cli_hash = compute_managed_hash(doc_type, &fm.managed_json());
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

    // Tests for the deleted `compute_frontmatter_hashes_from_yaml` helper
    // previously here (convenience_helper_matches_manual_steps,
    // convenience_helper_none_frontmatter) were removed in task 11 of
    // session 2. The equivalent coverage for Frontmatter::hashes() lives
    // in `crate::frontmatter::document::tests` and the golden-hash
    // regression tests in `crates/temper-core/tests/frontmatter_test.rs`.
}
