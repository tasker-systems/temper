//! Managed-metadata hash (domain-A: strips tier-1 system fields + applies
//! doc-type defaults).

use crate::frontmatter::fields::TIER1_SYSTEM_FIELDS;
use temper_core::hash::hash_canonical_json;

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

    // Fill in managed-tier doc-type defaults so both sides agree.
    crate::defaults::apply_managed_defaults(doc_type, &mut meta);

    hash_canonical_json(&meta)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        let without = json!({"temper-title": "Test"});
        let with_tier1 = json!({"temper-title": "Test", "temper-created": "2026-01-01T00:00:00Z"});
        assert_eq!(
            compute_managed_hash("task", &without),
            compute_managed_hash("task", &with_tier1),
        );
    }

    // 10. managed_hash_deterministic_regardless_of_key_order
    #[test]
    fn managed_hash_deterministic_regardless_of_key_order() {
        let v1 = json!({"temper-title": "A", "temper-stage": "backlog"});
        let v2 = json!({"temper-stage": "backlog", "temper-title": "A"});
        assert_eq!(
            compute_managed_hash("task", &v1),
            compute_managed_hash("task", &v2),
        );
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
temper-title: "My Task"
temper-slug: my-task
temper-stage: in-progress
---
"#;
        let fm = Frontmatter::try_from(content).unwrap();
        let cli_hash = compute_managed_hash("task", &fm.managed_json());

        // Simulate API path: JSON without tier-1 fields
        let api_json = json!({
            "temper-stage": "in-progress",
            "temper-title": "My Task",
            "temper-slug": "my-task",
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
temper-title: "My Task"
temper-slug: my-task
---
"#;
        let fm = Frontmatter::try_from(content).unwrap();
        let cli_hash = compute_managed_hash("task", &fm.managed_json());

        // API: JSON with explicit default temper-stage: "backlog"
        let api_json = json!({
            "temper-title": "My Task",
            "temper-slug": "my-task",
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
temper-title: "My Task"
temper-slug: my-task
temper-stage: in-progress
---
"#
                .to_string(),
                json!({
                    "temper-stage": "in-progress",
                    "temper-title": "My Task",
                    "temper-slug": "my-task",
                }),
            ),
            (
                "goal",
                r#"---
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: goal
temper-context: ctx
temper-created: "2026-01-01T00:00:00Z"
temper-title: "Ship v1"
temper-slug: ship-v1
temper-status: achieved
---
"#
                .to_string(),
                json!({
                    "temper-status": "achieved",
                    "temper-title": "Ship v1",
                    "temper-slug": "ship-v1",
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
temper-title: "Planning"
temper-slug: planning
date: "{today}"
---
"#
                ),
                // Phase 6: `date` lives in open_meta server-side, not managed_meta.
                // The API's managed_meta JSONB therefore does not include `date`;
                // CLI's `fm.managed_json()` correctly routes `date` to open via
                // `split_managed_open`, so both sides converge here.
                json!({
                    "temper-title": "Planning",
                    "temper-slug": "planning",
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
temper-title: "Survey"
temper-slug: survey
date: "{today}"
---
"#
                ),
                json!({
                    "temper-title": "Survey",
                    "temper-slug": "survey",
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
        let without = json!({"temper-title": "Ship v1"});
        let with_default = json!({"temper-title": "Ship v1", "temper-status": "active"});
        assert_eq!(
            compute_managed_hash("goal", &without),
            compute_managed_hash("goal", &with_default),
        );
    }
}
