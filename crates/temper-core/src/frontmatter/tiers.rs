//! Managed / open tier splitting. Routes explicitly via the known-open
//! registry rather than relying on `$ref` not being followed.

use crate::frontmatter::document::DocType;
use crate::frontmatter::fields::{IDENTITY_FIELDS, TIER1_SYSTEM_FIELDS};
use std::collections::HashSet;

/// Split a YAML frontmatter mapping into (managed_json, open_json) tiers.
///
/// Routing rules, applied in order:
/// 1. Keys in [`IDENTITY_FIELDS`] or [`TIER1_SYSTEM_FIELDS`] → dropped.
/// 2. Keys prefixed `temper-` → managed.
/// 3. Keys `title` / `slug` → managed.
/// 4. Keys in the doc-type schema's own `properties` (not base) → managed.
/// 5. Everything else → open (known open fields and unknowns both land here).
///
/// Rule 4 uses `crate::schema::schema_value` which returns the doc-type
/// schema's own `properties` object — it does NOT follow `$ref`. That is
/// deliberate: base-schema fields like `relates_to` must route to open
/// via rule 5, not via rule 4.
pub fn split_managed_open(
    fm: &serde_yaml::Value,
    doc_type: DocType,
) -> (serde_json::Value, serde_json::Value) {
    let Some(mapping) = fm.as_mapping() else {
        return (serde_json::json!({}), serde_json::json!({}));
    };

    let skip: HashSet<&str> = IDENTITY_FIELDS
        .iter()
        .chain(TIER1_SYSTEM_FIELDS.iter())
        .copied()
        .collect();

    let schema_keys: HashSet<String> = crate::schema::schema_value(doc_type.as_str())
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

        let to_managed = key_str.starts_with("temper-")
            || key_str == "title"
            || key_str == "slug"
            || schema_keys.contains(key_str);

        if to_managed {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn yaml(s: &str) -> serde_yaml::Value {
        serde_yaml::from_str(s).unwrap()
    }

    #[test]
    fn identity_fields_are_stripped() {
        let v = yaml(
            r#"
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-provisional-id: "019d8110-8ff3-70c2-85ae-57e04ed62886"
title: Hello
temper-slug: hello
"#,
        );
        let (managed, open) = split_managed_open(&v, DocType::Task);
        assert!(managed.get("temper-id").is_none());
        assert!(managed.get("temper-provisional-id").is_none());
        assert!(open.get("temper-id").is_none());
    }

    #[test]
    fn tier1_system_fields_are_stripped() {
        let v = yaml(
            r#"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
temper-updated: "2026-04-13T00:00:00Z"
temper-owner: "@me"
temper-source: manual
title: T
temper-slug: t
"#,
        );
        let (managed, open) = split_managed_open(&v, DocType::Task);
        for f in [
            "temper-type",
            "temper-context",
            "temper-created",
            "temper-updated",
            "temper-owner",
            "temper-source",
        ] {
            assert!(managed.get(f).is_none(), "{f} must not be in managed");
            assert!(open.get(f).is_none(), "{f} must not be in open");
        }
    }

    #[test]
    fn temper_prefixed_fields_go_to_managed() {
        let v = yaml(
            r#"
title: T
temper-slug: t
temper-stage: in-progress
temper-mode: build
temper-effort: medium
"#,
        );
        let (managed, _open) = split_managed_open(&v, DocType::Task);
        assert_eq!(managed["temper-stage"], json!("in-progress"));
        assert_eq!(managed["temper-mode"], json!("build"));
        assert_eq!(managed["temper-effort"], json!("medium"));
    }

    #[test]
    fn title_and_slug_go_to_managed() {
        // Mixed-form input: bare `title` exercises the legacy-bare classifier
        // (until Task 6 normalize_aliases retires the literal); `temper-slug`
        // exercises the temper-* prefix classifier.
        let v = yaml(
            r#"
title: Hello
temper-slug: hello
"#,
        );
        let (managed, open) = split_managed_open(&v, DocType::Task);
        assert_eq!(managed["title"], json!("Hello"));
        assert_eq!(managed["temper-slug"], json!("hello"));
        assert!(open.get("title").is_none());
        assert!(open.get("temper-slug").is_none());
    }

    #[test]
    fn date_routes_to_open_tier_for_sessions() {
        // `date` is no longer in any managed-tier schema (Plan Task 13);
        // it routes to open tier as user-content metadata.
        let v = yaml(
            r#"
title: My session
temper-slug: my-session
date: "2026-04-13"
"#,
        );
        let (managed, open) = split_managed_open(&v, DocType::Session);
        assert!(managed.get("date").is_none());
        assert_eq!(open["date"], json!("2026-04-13"));
    }

    #[test]
    fn known_open_relationship_fields_go_to_open() {
        let v = yaml(
            r#"
title: T
temper-slug: t
relates_to: [a, b]
depends_on: [c]
parent: p
"#,
        );
        let (managed, open) = split_managed_open(&v, DocType::Task);
        assert_eq!(open["relates_to"], json!(["a", "b"]));
        assert_eq!(open["depends_on"], json!(["c"]));
        assert_eq!(open["parent"], json!("p"));
        assert!(managed.get("relates_to").is_none());
    }

    #[test]
    fn known_open_metadata_fields_go_to_open() {
        let v = yaml(
            r#"
title: T
temper-slug: t
tags: [auth, observability]
aliases: [alt]
"#,
        );
        let (_managed, open) = split_managed_open(&v, DocType::Task);
        assert_eq!(open["tags"], json!(["auth", "observability"]));
        assert_eq!(open["aliases"], json!(["alt"]));
    }

    #[test]
    fn unknown_fields_go_to_open() {
        let v = yaml(
            r#"
title: T
temper-slug: t
custom_field: 42
another: something
"#,
        );
        let (_managed, open) = split_managed_open(&v, DocType::Task);
        assert_eq!(open["custom_field"], json!(42));
        assert_eq!(open["another"], json!("something"));
    }

    #[test]
    fn session_date_routes_to_open_tier() {
        // After Plan Task 13, `date` is open-tier user content, not
        // managed-tier. Sessions, research, decisions, and concepts all
        // emit `date` to open tier.
        let v = yaml(
            r#"
title: S
temper-slug: s
date: "2026-04-13"
"#,
        );
        let (managed, open) = split_managed_open(&v, DocType::Session);
        assert!(managed.get("date").is_none());
        assert_eq!(open["date"], json!("2026-04-13"));
    }

    #[test]
    fn non_mapping_input_returns_empty_tiers() {
        let v: serde_yaml::Value = serde_yaml::from_str("- just\n- a list\n").unwrap();
        let (managed, open) = split_managed_open(&v, DocType::Task);
        assert_eq!(managed, json!({}));
        assert_eq!(open, json!({}));
    }

    // Regression anchor: the tier-split output produces stable hashes
    // for a known task fixture. Golden hashes captured in session 2 task 10
    // when the legacy hash::split_frontmatter_tiers API was deleted. If
    // these change, either the schema or canonicalization moved; investigate
    // before regenerating.
    #[test]
    fn task_fixture_produces_stable_hashes() {
        let v = yaml(
            r#"
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
temper-updated: "2026-04-13T00:00:00Z"
title: T
temper-slug: t
temper-stage: in-progress
temper-mode: build
temper-effort: small
temper-seq: 1
relates_to: [a]
depends_on: [b]
tags: [auth]
custom: ok
"#,
        );
        let (managed, open) = split_managed_open(&v, DocType::Task);
        let managed_hash = crate::hash::compute_managed_hash("task", &managed);
        let open_hash = crate::hash::compute_open_hash(&open);

        assert_eq!(
            managed_hash, "sha256:4c01d11757eb68e3c3879a647f6db771b17d7f37d5ce4e3a815d92f626a7b550",
            "task fixture managed hash drift"
        );
        assert_eq!(
            open_hash, "sha256:5ed7693d46e893012ed1fc01ebedb6119245c3909884df348f858d2897880c42",
            "task fixture open hash drift"
        );
    }

    #[test]
    fn session_fixture_produces_stable_hashes() {
        let v = yaml(
            r#"
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: session
temper-context: temper
temper-created: "2026-04-13T00:00:00Z"
title: S
temper-slug: s
date: "2026-04-13"
relates_to: [a]
tags: [x]
"#,
        );
        let (managed, open) = split_managed_open(&v, DocType::Session);
        let managed_hash = crate::hash::compute_managed_hash("session", &managed);
        let open_hash = crate::hash::compute_open_hash(&open);

        // Refreshed in Phase 8: `date` no longer contributes to managed_hash
        // for session/research (Phase 6 / Migration A: date lives in open_meta).
        assert_eq!(
            managed_hash, "sha256:f44eaac9f600cbc3f4a4738291741e2dc6407ef066a492bb660e7214dbb5b47e",
            "session fixture managed hash drift"
        );
        assert_eq!(
            open_hash, "sha256:d0c45999dca9e425cee891ecbda105e871592025782dd4482267f909009bc36c",
            "session fixture open hash drift"
        );
    }
}
