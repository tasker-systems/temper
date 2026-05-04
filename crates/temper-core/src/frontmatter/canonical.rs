//! Canonical 5-tier display ordering for `Frontmatter::serialize()`.
//!
//! This is strictly a display concern. Hashing uses alphabetical
//! `BTreeMap` canonicalization in `crate::hash::canonicalize_json` — the
//! two algorithms are independent, and a later test locks that
//! independence in.

use crate::frontmatter::document::DocType;
use crate::frontmatter::fields::{IDENTITY_FIELDS, TIER1_SYSTEM_FIELDS};
use crate::frontmatter::registry::KNOWN_OPEN_FIELDS;
use std::collections::HashSet;

/// Reorder a frontmatter mapping into canonical 5-tier display order.
///
/// The input is not mutated; the returned value is a new mapping with
/// the same keys and values in deterministic order.
///
/// Ordering:
/// 1. Identity fields ([`IDENTITY_FIELDS`]) in fixed order.
/// 2. Tier-1 system fields ([`TIER1_SYSTEM_FIELDS`]) in fixed order.
/// 3. Managed tier — `title`, `slug`, then doc-type schema properties in
///    schema-declaration order, then any extra `temper-*` keys alphabetically.
/// 4. Known open fields ([`KNOWN_OPEN_FIELDS`]) in registry order
///    (relationships first, then metadata).
/// 5. Unknown fields in original input insertion order.
pub fn canonicalize(fm: &serde_yaml::Value, doc_type: DocType) -> serde_yaml::Value {
    let Some(input) = fm.as_mapping() else {
        return fm.clone();
    };

    // Look up a key by string in the input mapping.
    let get = |key: &str| -> Option<serde_yaml::Value> {
        for (k, v) in input.iter() {
            if k.as_str().map(|s| s == key).unwrap_or(false) {
                return Some(v.clone());
            }
        }
        None
    };

    let mut out = serde_yaml::Mapping::new();
    let mut emitted: HashSet<String> = HashSet::new();

    // Tier 1a: identity fields (fixed order).
    for &field in IDENTITY_FIELDS {
        if let Some(v) = get(field) {
            out.insert(serde_yaml::Value::String(field.to_string()), v);
            emitted.insert(field.to_string());
        }
    }

    // Tier 1b: tier-1 system fields (fixed order).
    for &field in TIER1_SYSTEM_FIELDS {
        if let Some(v) = get(field) {
            out.insert(serde_yaml::Value::String(field.to_string()), v);
            emitted.insert(field.to_string());
        }
    }

    // Tier 2: managed fields — temper-title, slug, then schema-declared order.
    for fixed in ["temper-title", "slug"] {
        if let Some(v) = get(fixed) {
            out.insert(serde_yaml::Value::String(fixed.to_string()), v);
            emitted.insert(fixed.to_string());
        }
    }
    let schema_order = schema_property_order(doc_type);
    for key in &schema_order {
        if key == "temper-title" || key == "slug" {
            continue;
        }
        if !emitted.contains(key) {
            if let Some(v) = get(key) {
                out.insert(serde_yaml::Value::String(key.clone()), v);
                emitted.insert(key.clone());
            }
        }
    }

    // Tier 2 (additional): any `temper-*` keys not yet emitted and not in
    // tier-1 system fields go here, alphabetically, as a safety net for
    // schema-declared fields we might not know about.
    let mut extra_temper: Vec<String> = input
        .iter()
        .filter_map(|(k, _)| k.as_str())
        .filter(|s| s.starts_with("temper-") && !emitted.contains(*s))
        .map(String::from)
        .collect();
    extra_temper.sort();
    for key in extra_temper {
        if let Some(v) = get(&key) {
            out.insert(serde_yaml::Value::String(key.clone()), v);
            emitted.insert(key);
        }
    }

    // Tier 3: known open fields, registry order.
    for entry in KNOWN_OPEN_FIELDS {
        let name = entry.canonical;
        if !emitted.contains(name) {
            if let Some(v) = get(name) {
                out.insert(serde_yaml::Value::String(name.to_string()), v);
                emitted.insert(name.to_string());
            }
        }
    }

    // Tier 4: unknown open fields in input order.
    for (k, v) in input.iter() {
        let Some(name) = k.as_str() else { continue };
        if !emitted.contains(name) {
            out.insert(serde_yaml::Value::String(name.to_string()), v.clone());
            emitted.insert(name.to_string());
        }
    }

    serde_yaml::Value::Mapping(out)
}

/// Schema property order for a doc type, in schema-declaration order.
///
/// `serde_json::Value` without the `preserve_order` feature flag stores
/// object keys in a `BTreeMap` (alphabetized), so we parse the raw schema
/// text through `serde_yaml::Value` instead — YAML is a superset of JSON
/// and `serde_yaml::Mapping` is insertion-ordered via `IndexMap`, which is
/// exactly what we need for "schema-declaration order".
///
/// The `DocType → schema text` mapping lives on [`DocType::schema_json`]
/// as the single source of truth.
fn schema_property_order(doc_type: DocType) -> Vec<String> {
    let Ok(parsed) = serde_yaml::from_str::<serde_yaml::Value>(doc_type.schema_json()) else {
        return Vec::new();
    };
    let Some(properties) = parsed.get("properties").and_then(|p| p.as_mapping()) else {
        return Vec::new();
    };

    properties
        .iter()
        .filter_map(|(k, _)| k.as_str().map(String::from))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(s: &str) -> serde_yaml::Value {
        serde_yaml::from_str(s).unwrap()
    }

    fn keys_of(v: &serde_yaml::Value) -> Vec<String> {
        v.as_mapping()
            .unwrap()
            .iter()
            .map(|(k, _)| k.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn identity_fields_come_first_in_fixed_order() {
        let v = yaml(
            r#"
title: T
slug: t
temper-provisional-id: "019d8110-8ff3-70c2-85ae-57e04ed62886"
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
"#,
        );
        let out = canonicalize(&v, DocType::Task);
        let ks = keys_of(&out);
        assert_eq!(ks[0], "temper-id");
        assert_eq!(ks[1], "temper-provisional-id");
    }

    #[test]
    fn tier1_system_fields_follow_identity_in_fixed_order() {
        let v = yaml(
            r#"
temper-title: T
slug: t
temper-updated: "2026-04-13T00:00:00Z"
temper-context: temper
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
temper-created: "2026-04-12T00:00:00Z"
"#,
        );
        let out = canonicalize(&v, DocType::Task);
        let ks = keys_of(&out);
        // Every present tier-1 key preserves the TIER1_SYSTEM_FIELDS order.
        let expected_order = [
            "temper-context",
            "temper-type",
            "temper-created",
            "temper-updated",
        ];
        let mut prev_idx = usize::MAX;
        for key in expected_order {
            if let Some(pos) = ks.iter().position(|k| k == key) {
                if prev_idx != usize::MAX {
                    assert!(pos > prev_idx, "tier1 key {key} out of order");
                }
                prev_idx = pos;
            }
        }
        // And tier1 precedes managed (temper-title).
        let first_tier1 = ks.iter().position(|k| k == "temper-context").unwrap();
        let title_idx = ks.iter().position(|k| k == "temper-title").unwrap();
        assert!(first_tier1 < title_idx, "tier1 must precede managed");
    }

    #[test]
    fn title_comes_before_slug_in_managed_tier() {
        let v = yaml(
            r#"
slug: t
temper-title: T
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
"#,
        );
        let out = canonicalize(&v, DocType::Task);
        let ks = keys_of(&out);
        let title_idx = ks.iter().position(|k| k == "temper-title").unwrap();
        let slug_idx = ks.iter().position(|k| k == "slug").unwrap();
        assert!(title_idx < slug_idx);
    }

    #[test]
    fn doc_type_schema_properties_land_in_managed_in_schema_order() {
        // task.schema.json declares: temper-type, temper-stage, temper-mode,
        // temper-effort, temper-goal, temper-seq, temper-branch, temper-pr, slug
        // in that declaration order. We assert the subset present here
        // emerges in that relative order.
        let v = yaml(
            r#"
temper-pr: pr-url
temper-mode: build
temper-stage: in-progress
title: T
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-effort: small
"#,
        );
        let out = canonicalize(&v, DocType::Task);
        let ks = keys_of(&out);
        let stage = ks.iter().position(|k| k == "temper-stage").unwrap();
        let mode = ks.iter().position(|k| k == "temper-mode").unwrap();
        let effort = ks.iter().position(|k| k == "temper-effort").unwrap();
        let pr = ks.iter().position(|k| k == "temper-pr").unwrap();
        assert!(stage < mode && mode < effort && effort < pr);
    }

    #[test]
    fn known_open_fields_follow_in_registry_order() {
        let v = yaml(
            r#"
title: T
slug: t
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
tags: [x]
relates_to: [a]
depends_on: [b]
parent: p
"#,
        );
        let out = canonicalize(&v, DocType::Task);
        let ks = keys_of(&out);
        let relates = ks.iter().position(|k| k == "relates_to").unwrap();
        let depends = ks.iter().position(|k| k == "depends_on").unwrap();
        let parent = ks.iter().position(|k| k == "parent").unwrap();
        let tags = ks.iter().position(|k| k == "tags").unwrap();
        // Registry order: relates_to < depends_on < ... < parent < tags (metadata).
        assert!(relates < depends);
        assert!(depends < parent);
        assert!(parent < tags);
    }

    #[test]
    fn unknown_fields_preserved_in_original_order() {
        let v = yaml(
            r#"
title: T
slug: t
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
zebra: 1
alpha: 2
mango: 3
"#,
        );
        let out = canonicalize(&v, DocType::Task);
        let ks = keys_of(&out);
        let zebra = ks.iter().position(|k| k == "zebra").unwrap();
        let alpha = ks.iter().position(|k| k == "alpha").unwrap();
        let mango = ks.iter().position(|k| k == "mango").unwrap();
        assert!(zebra < alpha);
        assert!(alpha < mango);
    }

    #[test]
    fn canonicalize_is_idempotent() {
        let v = yaml(
            r#"
relates_to: [a]
title: T
slug: t
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
custom: 1
"#,
        );
        let once = canonicalize(&v, DocType::Task);
        let twice = canonicalize(&once, DocType::Task);
        assert_eq!(once, twice);
    }

    #[test]
    fn canonicalize_is_deterministic_under_input_permutations() {
        let a = yaml(
            r#"
title: T
slug: t
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
temper-type: task
relates_to: [x]
tags: [y]
"#,
        );
        let b = yaml(
            r#"
tags: [y]
relates_to: [x]
temper-type: task
temper-id: "019d8110-8ff3-70c2-85ae-57e04ed62885"
slug: t
title: T
"#,
        );
        assert_eq!(
            canonicalize(&a, DocType::Task),
            canonicalize(&b, DocType::Task)
        );
    }
}
