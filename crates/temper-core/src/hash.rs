//! Unified sync-related hash computation — single source of truth.
//!
//! Both CLI and API call these functions so that a document hashed from YAML
//! frontmatter on the client side produces the same digest as one hashed from
//! JSON columns on the server side, provided the same defaults are applied.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

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

/// Bare lowercase-hex SHA-256 of raw bytes — **no `sha256:` prefix**, unlike [`compute_body_hash`].
///
/// This is the segment-text identity hash the ingest wire carries in
/// `AppendBlockPayload.content_hash`, and the Rust twin of Postgres's
/// `encode(sha256(convert_to(s, 'UTF8')), 'hex')`. The append path recomputes it over the received
/// segment text and rejects a mismatch, which is the one transit-integrity check available to a
/// caller that cannot chunk or embed locally.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
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
// Open hash
// ---------------------------------------------------------------------------

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

    // The managed-hash convergence tests (7–10, 15–18) moved to
    // `temper_workflow::hash` alongside `compute_managed_hash`, since they
    // depend on doc-type defaults and the frontmatter model (domain-A).

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
}
